#!/usr/bin/env bash
# End-to-end test: vej hosted by zvej on a throwaway OpenZiti network.
#
# Needs Docker (runs `ziti edge quickstart` in a container), the `ziti` CLI on the host,
# a release build of vej, and a downloaded model. From the repository root:
#
#   cargo build --release
#   (cd zvej && go build -o ../target/release/zvej . \
#             && go build -o ../target/release/zvej-example-client ./examples/client)
#   zvej/e2e/run.sh
#
# The host only makes outgoing connections, so no firewall prompts appear.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
VEJ=${VEJ:-$ROOT/target/release/vej}
ZVEJ=${ZVEJ:-$ROOT/target/release/zvej}
CLIENT=${CLIENT:-$ROOT/target/release/zvej-example-client}
MODEL=${MODEL:-llama-3.2-1b}
ZITI_IMAGE=${ZITI_IMAGE:-openziti/ziti-cli:1.5.11}
CONTAINER=zvej-e2e-ziti
WORK=$(mktemp -d "${TMPDIR:-/tmp}/zvej-e2e.XXXXXX")
ZVEJ_PID=

failures=0
pass() { echo "  PASS  $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }
section() { echo; echo "== $*"; }
expect() { # expect DESCRIPTION PATTERN TEXT
  if grep -qE -- "$2" <<<"$3"; then pass "$1"; else fail "$1"; echo "        got: ${3:0:300}"; fi
}

cleanup() {
  [ -n "$ZVEJ_PID" ] && kill "$ZVEJ_PID" 2>/dev/null && wait "$ZVEJ_PID" 2>/dev/null
  docker rm -f "$CONTAINER" >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT

call() { "$CLIENT" --identity "$WORK/$1.json" "${@:2}" 2>&1; }
vej_pids() { pgrep -f "^$VEJ serve --no-tcp" || true; }

start_zvej() { # start_zvej LOG ZVEJ_ARGS... -- VEJ_ARGS...
  local log=$1; shift
  "$ZVEJ" serve --identity "$WORK/vej-host.json" --vej "$VEJ" "$@" >"$WORK/$log" 2>&1 &
  ZVEJ_PID=$!
  for _ in $(seq 1 120); do
    grep -q "Hosting OpenZiti service" "$WORK/$log" && return 0
    kill -0 "$ZVEJ_PID" 2>/dev/null || break
    sleep 0.5
  done
  echo "zvej did not start:"; cat "$WORK/$log"; exit 1
}

stop_zvej() {
  kill "$ZVEJ_PID" && wait "$ZVEJ_PID"
  local status=$?
  ZVEJ_PID=
  return $status
}

for bin in "$VEJ" "$ZVEJ" "$CLIENT"; do
  [ -x "$bin" ] || { echo "missing $bin (see the build steps at the top)"; exit 1; }
done

section "OpenZiti network (quickstart in Docker)"
docker rm -f "$CONTAINER" >/dev/null 2>&1
docker run -d --name "$CONTAINER" -p 127.0.0.1:1280:1280 -p 127.0.0.1:3022:3022 \
  "$ZITI_IMAGE" edge quickstart --ctrl-address localhost --router-address localhost >/dev/null
for _ in $(seq 1 90); do
  curl -sk https://localhost:1280/edge/client/v1/version >/dev/null && break
  sleep 1
done
export ZITI_CONFIG_DIR=$WORK/ziti-cli # keep the host's ziti CLI login untouched
for _ in $(seq 1 30); do
  ziti edge login localhost:1280 -u admin -p admin -y >/dev/null 2>&1 && break
  sleep 1
done
ziti edge create identity vej-host -a vej-hosts -o "$WORK/vej-host.jwt" >/dev/null
ziti edge create identity laptop -a vej-clients -o "$WORK/laptop.jwt" >/dev/null
ziti edge create identity stranger -o "$WORK/stranger.jwt" >/dev/null
for id in vej-host laptop stranger; do
  ziti edge enroll "$WORK/$id.jwt" -o "$WORK/$id.json" >/dev/null 2>&1
done
ziti edge create service vej >/dev/null
ziti edge create service-policy vej-bind Bind --service-roles @vej --identity-roles '#vej-hosts' >/dev/null
ziti edge create service-policy vej-dial Dial --service-roles @vej --identity-roles '#vej-clients' >/dev/null
ziti edge create edge-router-policy all --edge-router-roles '#all' --identity-roles '#all' >/dev/null 2>&1
ziti edge create service-edge-router-policy all --edge-router-roles '#all' --service-roles '#all' >/dev/null 2>&1
ls "$WORK"/{vej-host,laptop,stranger}.json >/dev/null && pass "network up, three identities enrolled"

section "zvej hosting vej (vej started at once)"
start_zvej zvej.log -- -m "$MODEL" --keep-alive 5m
VEJ_PID=$(vej_pids)
[ -n "$VEJ_PID" ] && pass "vej running (pid $VEJ_PID)" || fail "vej running"
if lsof -nP -a -p "$VEJ_PID" -iTCP -sTCP:LISTEN >/dev/null 2>&1; then
  fail "vej opened a TCP listener"
else
  pass "vej has no TCP listener"
fi

section "authorized identity"
expect "GET /health" '^HTTP 200' "$(call laptop GET /health)"
expect "POST /v1/systemone decides" '^HTTP 200 .*"decision":"refund"' \
  "$(call laptop POST /v1/systemone '{"state":"I was charged twice for my order","choices":["refund","shipping"]}')"
expect "GET /v1/ps shows the model" "^HTTP 200 .*$MODEL" "$(call laptop GET /v1/ps)"

section "admin routes don't exist over OpenZiti"
for route in /v1/unload /v1/shutdown /v1/models/switch; do
  expect "POST $route -> 404" '^HTTP 404' "$(call laptop POST $route '{"model":"llama-3.2-3b"}')"
done
expect "GET /playground/ -> 404" '^HTTP 404' "$(call laptop GET /playground/)"
expect "remote keep_alive 0 is ignored" "^HTTP 200 .*$MODEL" \
  "$(call laptop POST /api/generate '{"keep_alive":0}' >/dev/null; call laptop GET /v1/ps)"

section "identity without a Dial policy"
STRANGER=$(call stranger GET /health)
echo "        stranger got: ${STRANGER:0:160}"
expect "stranger is refused by OpenZiti" '^client: ' "$STRANGER"
expect "stranger never reached vej" '^$' "$(grep 'caller=stranger' "$WORK/zvej.log")"

section "audit log"
expect "zvej logs the Ziti caller" 'caller=laptop .*path=/v1/systemone status=200' "$(cat "$WORK/zvej.log")"

section "shutdown"
SOCKET_DIR=$(grep -oE 'socket=[^ ]+' "$WORK/zvej.log" | head -1 | cut -d= -f2 | xargs dirname)
stop_zvej && pass "zvej exits cleanly on SIGTERM" || fail "zvej exit status"
sleep 0.5
[ -z "$(vej_pids)" ] && pass "vej stopped with zvej" || fail "vej still running"
[ ! -e "$SOCKET_DIR" ] && pass "private socket directory removed" || fail "socket directory $SOCKET_DIR remains"

section "scale to zero: --lazy plus vej --idle-exit"
start_zvej lazy.log --lazy -- -m "$MODEL" --idle-exit 3s
[ -z "$(vej_pids)" ] && pass "vej not started before the first request" || fail "vej started eagerly"
expect "first request starts vej" '^HTTP 200' "$(call laptop GET /health)"
[ -n "$(vej_pids)" ] && pass "vej running after the request" || fail "vej not running"
sleep 6
[ -z "$(vej_pids)" ] && pass "vej exited after 3s idle" || fail "vej still running after idle-exit"
expect "next request starts vej again" '^HTTP 200 .*"decision"' \
  "$(call laptop POST /v1/systemone '{"state":"the app crashes on login","choices":["refund","technical"]}')"
expect "zvej logged two vej starts" '2' "$(grep -c 'Started vej' "$WORK/lazy.log")"
stop_zvej >/dev/null

echo
if [ "$failures" -eq 0 ]; then echo "ALL PASSED"; else echo "$failures FAILED"; fi
exit $((failures > 0))
