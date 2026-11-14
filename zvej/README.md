# zvej — vej over OpenZiti

`zvej` hosts [vej](../README.md) as an [OpenZiti](https://openziti.io) service. vej then opens **no network port at all**. Only enrolled Ziti identities that your network's policies allow can reach it, from anywhere, without opening a firewall port.

Plain `vej` contains no OpenZiti code. `zvej` is a separate, small Go program built on the official OpenZiti Go SDK. It runs vej as a child process on a private Unix socket and forwards requests to it.

```
 client (Ziti identity) ──OpenZiti overlay──▶ zvej ──private Unix socket──▶ vej serve --no-tcp
```

## What callers can do

Requests through zvej arrive on vej's **remote API** (see [Remote Access](../README.md#-remote-access)):

- **Allowed:** decisions, batch, generation, `/health`, `/v1/models`, `/v1/ps`, `/api/tags`.
- **Not available:** model switching, unloading, shutdown, the TypeSafe proxy and the playground. These return 404.
- **Ignored:** per-request `keep_alive` and `model`, unless enabled under `[remote]` in vej's config file.
- **Identity:** OpenZiti authenticates every caller. zvej logs each request with the caller's identity name:
  ```
  level=INFO msg="Ziti request" caller=laptop method=POST path=/v1/systemone status=200 elapsed_ms=212
  ```
- **Unauthorized identities:** an identity without a Dial policy can't see the service at all; it gets `service 'vej' not found`.

## Set up the OpenZiti side

With the `ziti` CLI logged in to your controller:

```bash
# The identity zvej runs as, and one per client
ziti edge create identity vej-host -a vej-hosts -o vej-host.jwt
ziti edge create identity laptop   -a vej-clients -o laptop.jwt

# The service, plus who may host it (Bind) and who may call it (Dial)
ziti edge create config vej-intercept intercept.v1 \
  '{"protocols":["tcp"],"addresses":["vej.ziti"],"portRanges":[{"low":80,"high":80}]}'
ziti edge create service vej --configs vej-intercept
ziti edge create service-policy vej-bind Bind --service-roles @vej --identity-roles '#vej-hosts'
ziti edge create service-policy vej-dial Dial --service-roles @vej --identity-roles '#vej-clients'

# On the vej machine
ziti edge enroll vej-host.jwt -o vej-host.json
```

Your network also needs edge-router policies that let these identities and the service use a router. Many networks already have `#all` policies.

The `intercept.v1` config is only for clients using a tunneler (see below). SDK clients dial the service by name.

## Run zvej

```bash
zvej serve --identity vej-host.json -- -m gemma-4-e2b
```

Arguments after `--` go to `vej serve`. zvej adds `--no-tcp`, `--remote-socket` and `--remote-token-optional` itself, so it rejects `--host`, `--port` and those three flags.

### Scale to zero

```bash
zvej serve --identity vej-host.json --lazy -- -m gemma-4-e2b --keep-alive 5m --idle-exit 30m
```

- **Resident cost:** zvej stays connected to the overlay and uses little memory.
- **Start on demand:** vej starts on the first request.
- **Model release:** the model unloads after 5 minutes without requests.
- **Exit and restart:** vej exits after 30 minutes without requests. The next request starts it again, paying the model-load time once.

### Options

| Option | Default | Meaning |
| :--- | :--- | :--- |
| `--identity FILE` | required | Enrolled identity that hosts the service |
| `--service NAME` | `vej` | Service to host |
| `--vej PATH` | `vej` next to `zvej`, else on `PATH` | vej executable |
| `--lazy` | off | Start vej on the first request instead of at startup |
| `--require-vej-token` | off | Also require a vej bearer token (`vej remote token create`) on top of the Ziti identity |
| `--socket PATH` | private temp directory | Socket between zvej and vej |
| `--start-timeout` | `1m` | How long vej may take to open its socket |

If vej crashes or exits, the next request starts it again. If vej fails to start (for example, the model isn't downloaded), zvej logs the reason. Callers get a 503 for the next 10 seconds before zvej tries again. At startup without `--lazy`, zvej exits right away instead, so mistakes surface immediately.

## Clients

- **Any app, including Dart/Flutter:** run a tunneler on the device, such as Ziti Desktop Edge (macOS/Windows), Ziti Mobile Edge (iOS/Android) or `ziti-edge-tunnel` (Linux), with the client's identity. Then use the intercept address:
  ```dart
  final vej = VejClient(baseUrl: 'http://vej.ziti');
  ```
- **Go, without a tunneler:** see [`examples/client`](examples/client/main.go). With the SDK's HTTP client, the URL host is the service name:
  ```bash
  go run ./examples/client --identity laptop.json POST /v1/systemone \
    '{"state":"I was charged twice","choices":["refund","shipping"]}'
  ```
- **Python, Node, Java, Swift, C#:** the [OpenZiti SDKs](https://openziti.io/docs/reference/developer/sdk) can dial the service directly.

## Run zvej in the background

`vej service install` manages plain vej only. It rejects `--no-tcp`, because on-demand start and `vej service status` need a TCP port. Run zvej under launchd or systemd yourself with `zvej serve ...` as the command; it stops vej when it receives SIGTERM.

If zvej is killed with SIGKILL, vej keeps running on its private socket until it is stopped. With `--idle-exit`, it exits on its own once idle.

## Development

```bash
go test -race ./...    # unit tests (a fake vej stands in for the real one)
zvej/e2e/run.sh        # from the repo root: real vej + zvej on a throwaway OpenZiti network
```

The end-to-end script runs `ziti edge quickstart` in Docker, so it needs Docker and the `ziti` CLI. It checks:
- an authorized and an unauthorized identity;
- that admin routes are absent;
- that vej has no TCP listener;
- the audit log;
- clean shutdown;
- scale to zero.
