# vej fix plan

Work packages from a full-codebase review (architecture, quality, performance, security) on
2026-11-13 at commit `1aa2322` (`main`). Each package is self-contained so an agent with no
prior context can pick one up. Line numbers refer to `1aa2322` and will drift: find code by
the function names given.

## Status

Update this table when you start or finish a package (`todo` → `in progress` → `done`), and
fill in "Merged" when the branch lands on `main`. "Done" means committed and verified on its
branch; a dependency counts as met only once it is merged.

| ID  | Package                                          | Priority | Depends on | Status | Branch                    | Merged                         |
| --- | ------------------------------------------------ | -------- | ---------- | ------ | ------------------------- | ------------------------------ |
| P1  | Server: concurrency permit, Jev proxy client     | High     | —          | done   | `fix/p1-inference-permit` | `8bcbd2e`                      |
| P2  | Engine: shared parsing, input limits, errors     | High     | —          | done   | `fix/p2-engine-refactor`  | `5d4ba8c`                      |
| P3  | Model manager: safe loads                        | Medium   | P2 step 1  | done   | `fix/p3-model-manager`    | `9b1112a`                      |
| P4  | Model download integrity                         | Medium   | —          | todo   |                           |                                |
| P5  | Config: `--config`, model overrides, errors      | Medium   | P2, P3, P7 | todo   |                           |                                |
| P6  | CI on pull requests; release workflow hardening  | Medium   | —          | done   | `ci/p6-pr-ci`             | `5eebe34`                      |
| P7  | Documentation drift                              | Low      | —          | done   | `fix/p7-docs-drift`       | `840c23b`                      |
| P8  | Playground: escaping, model list, API key        | Low      | —          | done   | `fix/p8-playground`       | `31cdc74`                      |
| P9  | Dart SDK cleanup                                 | Low      | —          | done   | `fix/p9-dart-sdk`         | `ed1f31a`                      |
| P10 | Unix socket creation permissions                 | Low      | —          | done   | `fix/p10-socket-perms`    | `c223464`                      |
| P11 | Performance: reuse inference contexts (optional) | Optional | P2, P3     | done   | `perf/p11-context-reuse` | `00de006`                 |

**Branch stack.** P1 (`8bcbd2e`), P2 (`5d4ba8c`), P3 (`9b1112a`), P6 (`5eebe34`) and P11
(`00de006`) are merged into `main`. With P2, P3 and P7 merged, P5's prerequisites are met.
Merged branches and their worktrees have been removed.

**Parallel waves.** Wave 1: P1, P2, P4, P6, P7, P8, P9, P10 (they touch different files).
Wave 2: P3 (after P2 step 1), then P5 (after P2, P3 and P7). Wave 3: P11. Packages that touch
the same file are sequenced so merges stay clean; the "Files" line of each package lists what
it may edit.

## Rules for every package

- **Branch** from up-to-date `main`: `fix/<id>-<slug>` (e.g. `fix/p1-inference-permit`;
  CI-only work may use `ci/`, as P6 did with `ci/p6-pr-ci`).
  Commit with conventional messages (`fix:`, `feat:`, `refactor:`, `test:`, `docs:`, `ci:`).
  Do not push, open PRs, or change git config (the repo already has a local noreply email)
  unless the user asks.
- **Test first.** Write the failing test, watch it fail, then fix. Server and lifecycle tests
  use the fakes in `src/test_support.rs` (`FakeLoader`, `FakeModel`, `Probe`); follow
  the patterns in `src/server/tests.rs` and `src/model_manager.rs` tests.
- **No model downloads.** GGUF files are 0.8–5 GB. Logic that needs a real model goes in an
  opt-in test that is skipped unless `VEJ_TEST_MODEL=/path/to/model.gguf` is set (see P2).
- **Style:** no `unwrap`/`expect` on production paths, functions under ~50 lines, files under
  800 lines, `cargo fmt`, clippy clean with `-D warnings`.
- **Verify before marking done** (all must pass):

  ```sh
  cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
  (cd zvej && go vet ./... && go test ./...)          # Go 1.26.11 (zvej/go.mod)
  (cd dart_vej && dart analyze && dart test)
  (cd java_vej && mvn -q test)
  ```

  In a worktree, export `CARGO_TARGET_DIR=/Users/moscac/dev/moscait/vej/target` and
  `GGML_NATIVE=OFF` so llama.cpp isn't rebuilt per worktree (cargo locks the directory).

- **Downstream consumer:** wordstudy (`github.com/carljmosca/wordstudy`) uses dart_vej
  `evaluateQuestionsBatch` with `noul` questions and `score` questions whose criteria are maps
  keyed `'0'..'5'` (dart_vej sends those as a list), plus `POST /v1/generate` and
  `POST /api/generate` (`keep_alive: 0` to unload). Do not change those behaviors except where
  a package says so.

---

## P1 — Server: concurrency permit, Jev proxy client

**Files:** `src/server/handlers.rs`, `src/server/tests.rs`.

- [x] **1. The inference-slot permit is released when a client disconnects, while inference
  keeps running.** In `run_inference`, `_slot` (the `OwnedSemaphorePermit`) lives in the async fn,
  but the work runs in `spawn_blocking`. If hyper drops the handler future (client
  disconnect; the Java SDK closes the connection on its 30 s timeout), the permit is freed
  while the blocking job continues, so more than `max_concurrent` inferences can run at once.
  Each one allocates a KV cache.
  - [x] **Fix:** move the permit into the closure, e.g.
    `spawn_blocking(move || { let _slot = slot; work(&lease) })`.
  - [x] **Test:** `FakeLoader::with_eval_delay(300ms)` and `max_concurrent = 1`. Send request A
    through the router with `tokio::time::timeout(50ms, …)` so its future is dropped, then
    immediately send request B. Assert `probe.max_concurrent_evaluations() == 1`. It must be
    2 before the fix.
- [x] **2. `jev_proxy_handler` builds a new `reqwest::Client` per request with no timeout.**
  Use one shared client (`std::sync::OnceLock` or a field on `AppState`) with a connect
  timeout (5 s) and a request timeout (60 s). Map a timeout to 504 and other failures to 502,
  and keep the JSON error body.

**Done when** both tests pass and the full verification passes.

**Done** on branch `fix/p1-inference-permit` (2 commits, ending `8cf0b0e`), merged into `main`
at `8bcbd2e`.

---

## P2 — Engine: shared question parsing, input limits, typed errors

**Files:** `src/engine.rs` (to be split into `src/engine/`), `src/server/handlers.rs` (error
mapping and request validation only), `src/test_support.rs`, `src/main.rs` (only if
`DecisionModel` signatures change). One agent, steps in order, one commit per step.

**Context.** `src/engine.rs` is 1,258 lines and has 3 tests. `evaluate_unified_with_context`
(single requests, about lines 545–757) and `evaluate_batch` (about lines 765–1110) each
contain their own copy of the question-to-choices parsing, and the copies have drifted.
Behavior confirmed against a running daemon with llama-3.2-1b:

| Input                                              | `/v1/systemone`          | `/v1/batch`              |
| -------------------------------------------------- | ------------------------ | ------------------------ |
| `choice` question, `criteria: ["Paris","London"]`  | `choice: "Paris"`        | `choice: "A"`            |
| `score` question with no criteria                  | defaults to levels 0..=5 | 500 "must define criteria" |
| `type: "boolean"` with no criteria                 | 500                      | true/false defaults      |
| `score` with numeric `levels: N`                   | N levels                 | ignored, then 500        |
| `score` array display in prompt                    | `0) 0: text`             | `0) text`                |

### Step 1 — Split the module and extract pure functions (no behavior change yet)

Convert `src/engine.rs` into a module directory, for example:

- [x] `engine/mod.rs`: `DecisionEngine`, `EngineLoader`, `DecisionModel`, `llama_backend()`
- [x] `engine/types.rs`: request/response serde types
- [x] `engine/choices.rs`: question parsing (pure, no llama types)
- [x] `engine/prompt.rs`: chat templates for decisions and generation
- [x] `engine/scoring.rs`: label scheme plus softmax over candidate logits
- [x] `engine/generate.rs`: `generate_text`

Each file should stay under ~400 lines.
- [x] In `choices.rs`, add
  `build_choices(spec: &QuestionSpec) -> Result<Vec<ChoiceItem>, InvalidRequest>` and
  `format_instructions(spec: &QuestionSpec) -> Option<String>`, and make both the single and
  batch paths call them.

### Step 2 — One canonical behavior, used by both paths

- [x] **Array criteria for non-score questions:** key = the option text (single-path behavior).
  This changes `/v1/batch` output for array criteria from letters to text. wordstudy does not
  use array criteria for `choice` questions, but state the change in the commit message.
- [x] **Score array display:** use the option text, or the index when the text is empty (batch
  behavior, which wordstudy's prompts rely on).
- [x] **Score without criteria:** levels 0..=5 on both paths.
- [x] **Numeric `levels: N`:** supported on both paths, bounded by step 3.
- [x] **`noul` or `boolean` without criteria:** true/false defaults on both paths.
- [x] **Tests:** table-driven unit tests for `build_choices` covering every row of the table above,
  plus map criteria, empty criteria, and non-string values.

### Step 3 — Input limits (memory exhaustion from a single request)

Today:

- `levels: 1000000000000000000` (an integer, not a float) loops `for lvl in 0..n` and
  exhausts memory.
- The batch context is sized from the request (`create_context_with_size(max_chunk_tokens +
  256, 32)`). A 2 MB body is about 500k tokens, which for a 1B model means a KV cache of about
  16 GB.
- Generation sizes its context as prompt tokens + `max_gen`.
- More than 16 lettered choices get the label `?` and silently can never win.

The remote API reaches all of these, without a token when zvej runs with its default
`--remote-token-optional`. Add an `engine/limits.rs` with named constants and validate as
early as possible:

- [x] Lettered choices: extend the letters to A–Z and reject more than 26 with 400.
- [x] Numeric or score levels: at most 101 (0..=100). Questions per request: at most 32. Batch
  items: at most 256. Items × questions: at most 1024.
- [x] Shape checks (empty choices, empty questions, the counts above) need no model. Add
  `validate()` on `DecisionRequest` and `BatchDecisionRequest` and call it in the handlers
  before `run_inference`, so invalid requests never load a model.
- [x] Token checks run after tokenizing. Reject a decision prompt longer than the context with 400
  ("input is N tokens; the limit is M"). In the batch path, build chunks by a token budget as
  well as the 32-sequence cap (a `MAX_BATCH_TOKENS` constant, e.g. 8192), so context size is
  bounded by the budget and never by the request.
- [x] Generation: cap the context at `max(n_ctx, 8192)` and at the model's `n_ctx_train()`.
  Reject longer prompts with 400. wordstudy sends long generation prompts, so do not cap at
  `n_ctx` (2048).
- [x] Keep axum's 2 MB default body limit, and set it explicitly with `DefaultBodyLimit` in
  `public_routes` so it is visible.

### Step 4 — Client errors return 400, not 500

- [x] Add `pub struct InvalidRequest(pub String)` (implementing `std::error::Error` and `Display`)
  and return it, wrapped in `anyhow`, for every request-shape problem.
- [x] In `run_inference`, map errors that `downcast_ref::<InvalidRequest>()` matches to 400
  with the message; everything else stays 500. The Java and Dart SDKs already surface any
  non-2xx code, so no SDK change is needed.
- [x] Tests: empty choices, empty questions, too many levels, and an oversized batch all
  return 400 (currently 500; see `evaluation_errors_return_an_error_status` for the pattern).

### Step 5 — Scoring: never score by a shared first token

`score_choices_from_logits` takes `tokens.first()` of each label variant (about line 431).
For a multi-token variant that first token can be shared across choices. On tokenizers that
split digits (the Gemma family, including the default `gemma-4-e2b`), "1" and "10" share a
first token, so their scores collide. Llama 3 is unaffected because it keeps numbers up to 3
digits as one token (verified).

- [x] **Fix:** compute a `LabelScheme` once per question from the tokenizer:
  - boolean when the choices are true/false and each has a single-token variant;
  - numeric when every key has a single-token variant and those tokens are distinct across
    choices;
  - otherwise letters A–Z.
- [x] Use the same scheme in `format_prompt` and in scoring, and only accept variants that encode
  to exactly one token. Keep the tokenizer behind a small trait or closure so the selection
  logic is unit-testable with a fake tokenizer.

### Step 6 — Neutralize special tokens in user text (prompt injection)

`LlamaModel::str_to_token` in llama-cpp-2 0.1.151 always calls `llama_tokenize` with
`parse_special = true` (see the crate's `src/model.rs` near line 326). So text like
`<|eot_id|><|start_header_id|>system<|end_header_id|>` inside `state`, `instruction`,
`criteria`, or a generation prompt becomes real control tokens. Anyone who writes text that
vej later classifies (emails, user content) can steer the decision.

- [x] First check whether a newer llama-cpp-2 exposes tokenizing with `parse_special = false`. If
  it does, tokenize user segments with special parsing off, template scaffolding with it on,
  and concatenate the token vectors.
- [x] Otherwise, at model load build the set of special-token strings: iterate
  `model.tokens(true)` and keep tokens whose `token_attr()` has `Control` or `UserDefined`
  and whose text is at least 2 characters. Store the set on `DecisionEngine`. Before
  formatting, run every user-supplied string through `neutralize(text, &specials)`, which
  inserts U+200B after the first character of each occurrence. Use the `aho-corasick` crate if
  the naive replace is slow for large vocabularies.
- [x] **Tests:** a unit test of `neutralize` with a fixed set, plus an opt-in real-model test
  (step 8) that a neutralized `<|eot_id|>` tokenizes to no control tokens.

### Step 7 — Generation: no app-specific system prompt; honor `model`

- [x] `format_generation_prompt` hard-codes "You are a helpful, accurate Bible study assistant…".
  That belongs to wordstudy, not the engine. The maintainer has approved a general default.
  Replace it with one named constant `DEFAULT_GENERATE_SYSTEM_PROMPT` in `engine/prompt.rs`
  holding exactly:

  > You are a precise, concise assistant. Follow the user's instructions exactly. When asked
  > for JSON, respond with valid JSON only, with no surrounding text or markdown.

  Keep the JSON clause: callers rely on `/v1/generate` returning bare JSON when they ask for
  it.
- [x] Add an optional `system` field to `GenerateRequest` (Ollama's `/api/generate` uses the same
  name) and pass it to `generate_text`, so callers can set their own persona. When `system` is
  absent or blank, use the default above. Unit-test `format_generation_prompt` for every
  template with and without a custom system prompt. Extend `FakeModel::generate_text`'s echo
  so a server test can assert the `system` value reaches the model.
- **Downstream (wordstudy): done.** wordstudy sends its Bible-study text as `system` on both
  `/v1/generate` calls (`buildVejGenerateBody` in `lib/services/browser_bridge_stub.dart`);
  merged into wordstudy `main` as `d0a77a3`. vej 0.1.1 ignores the field, so it was safe to
  ship first. Still mention the default-prompt change in the vej release notes.
- [x] `generate_handler` ignores the request's `model` field, which wordstudy sends. Add
  `model: Option<String>` and route it through `policy.requested_model(...)` the same way
  `decision_handler` does.

### Step 8 — Single-request questions use the batch decode; opt-in real-model tests

- [x] Have `evaluate_unified` with `questions` build the same tasks as the batch path and decode
  them in one call, so both paths share all code. Measured on an M2 Pro with llama-3.2-1b:
  4 questions take 294 ms serially vs 214 ms batched.
- [x] Add `tests/real_model.rs` (or `#[ignore]` tests) that run only when `VEJ_TEST_MODEL` is set.
  They should check that a single request and a one-item batch return identical
  `answers`/`probabilities` for the same input, and cover the special-token neutralization
  from step 6. Document how to run them in `README.md`.

**Done when** every step is committed, the full verification passes, and the opt-in tests
pass locally with a small model if one is available.

**Done** on branch `fix/p2-engine-refactor` (8 commits on top of P1, ending `e439d16`), merged into
`main` at `5d4ba8c`.

---

## P3 — Model manager: safe loads

**Files:** `src/model_manager.rs`, `src/test_support.rs`, plus the `impl ModelLoader for
EngineLoader` block in the engine module (start after P2 step 1 has merged to avoid conflicts).

- [x] **1. A failed load evicts the working model.** In `ModelManager::acquire`, `evict_and_drain()`
  runs before `load_blocking()`. A request naming a catalog model that isn't downloaded, or
  `POST /v1/models/switch` with a bad path, unloads the serving model and then fails; the
  next request pays a full reload.
  - [x] **Fix:** add `fn check(&self, model_id: &str) -> Result<()>` to the `ModelLoader` trait
    (`EngineLoader` resolves the path and confirms the file exists, without loading). Call it
    after choosing `model_id` and before `evict_and_drain()`.
  - [x] **Test:** a `FakeLoader` option that makes `check` fail for given ids. Assert that after a
    failed `acquire(Some("missing"))` the previous model is still loaded and no drop event was
    recorded.
- [x] **2. A cancelled request can leave a load running unsupervised.** If the caller's future is
  dropped during `load_blocking().await`, `load_lock` is released while the blocking load
  continues. The next request starts a second load, so two models can be resident at once.
  - [x] **Fix:** run evict + load + install in a `tokio::spawn`ed task that owns the load guard
    (e.g. `Arc<tokio::sync::Mutex<()>>` with `lock_owned()`, and `acquire(self: &Arc<Self>,
    …)`), and await its `JoinHandle`. If the caller goes away, the task still finishes and the
    returned lease drops normally.
  - [x] **Test:** with `with_load_delay(200ms)`, wrap `acquire` in a 20 ms timeout so it is
    dropped, then `acquire` again. Assert exactly one `load` event and never two concurrent
    loads (add a concurrent-load counter to `Probe`).
- [x] **3. Keep `failed_load_is_reported_and_can_be_retried` and
  `failed_switch_keeps_the_previous_model_preferred` passing.**

**Done** on branch `fix/p3-model-manager` (worktree `../vej-p3`, commits `ab8c06a` and
`a60e30d`), merged into `main` at `9b1112a` with all 167 tests passing.

---

## P4 — Model download integrity

**Files:** `src/models.rs`, `src/main.rs` (only `models list` output if needed).

`pull_model` downloads from mutable `.../resolve/main/...` URLs and never verifies the bytes.
llama.cpp's GGUF parser has had memory-safety bugs, so a swapped upstream file is a real risk.

- [ ] 1. Replace `hf_url` in `ModelSpec` with `repo`, `revision` (a full commit SHA), `filename`
   and `sha256`. Build the URL as
   `https://huggingface.co/{repo}/resolve/{revision}/{filename}`.
- [ ] 2. Get the values from public metadata, without downloading models:
   - commit: `curl -s https://huggingface.co/api/models/<repo>` → `.sha`
   - file hash: `curl -s https://huggingface.co/api/models/<repo>/tree/<sha>` → the entry for
     `filename` → `.lfs.oid`
   - Cross-check any models already in the cache (macOS: `~/Library/Caches/vej/models/`) with
     `shasum -a 256`.
- [ ] 3. Hash with `sha2` while streaming, then `sync_all`, then compare before the rename. On a
   mismatch, or any error, delete the `.tmp` file (use a drop guard) and report expected vs
   actual hash.
- [ ] 4. Tests: a unit test for the hashing/verification helper using a small in-memory stream, and
   a test that every `RECOMMENDED_MODELS` entry has a 40-character revision and a 64-character
   lowercase hex `sha256`.

---

## P5 — Config: `--config`, model overrides, parse errors

**Files:** `src/config.rs`, `src/cli.rs` (one new global flag), `src/main.rs`,
`src/service/spec.rs`, `src/service/mod.rs`, the engine loader, `vej.toml` (renamed),
`README.md` and `LOGIC.md` (config sections only). Start after P2, P3 and P7 merge: P7 also
edits `vej.toml`, `README.md` and `src/cli.rs`.

The maintainer has decided both questions below. This is a breaking change for vej 0.2.0:
list it under "Breaking changes" in the commit message and the release notes.

- [ ] **1. Stop reading `./vej.toml` from the current directory.** `get_config_path` checks the
   current directory before the user config dir. Results then depend on where vej is started,
   and running vej inside an untrusted checkout adopts that checkout's host, token and model
   settings. Nothing that deploys vej relies on it: launchd and systemd start vej in `/`,
   zvej passes flags, and wordstudy passes flags plus `VEJ_CACHE_DIR`.
   - [ ] Add a global `--config <PATH>` flag on `Cli` (`#[arg(long, global = true)]`).
   - [ ] New precedence: `--config` > `VEJ_CONFIG` > `<user config dir>/vej/config.toml`. Never
     the current directory.
   - [ ] Make the resolution pure and unit-testable, e.g.
     `resolve_config_path(explicit, env_var, user_config_dir) -> PathBuf`, and test each level
     of precedence.
   - [ ] When no explicit config was given and `./vej.toml` exists, warn once: "found ./vej.toml
     but it is no longer read; use --config ./vej.toml".
   - [ ] An explicit `--config` or `VEJ_CONFIG` file that doesn't exist is an error. A missing
     default user config still falls back to defaults, as today.
   - [ ] `vej config init` writes to the user config path by default, with `--path <PATH>` to
     choose another location (keep `--force`). `vej config path` prints the resolved path and
     says which rule chose it.
   - [ ] `vej service install --config <PATH>` appends `--config <absolute path>` to the service's
     `serve` arguments (in `serve_args` in `src/service/spec.rs`), so it shows up in
     `vej service status`. Keep the existing `VEJ_CONFIG` passthrough.
   - [ ] `git mv vej.toml vej.example.toml`, and update README and LOGIC.md to show
     `vej config init` and `--config` instead of a `vej.toml` in the working directory.
   - **Downstream (wordstudy): done** (wordstudy `main`, `d0a77a3`). wordstudy starts vej
     with `VEJ_CONFIG` pointing at a wordstudy-owned, comment-only file it creates
     (`<app data>/Wordstudy/vej-config.toml`), so its built-in vej never reads the user's
     personal vej config. It uses `VEJ_CONFIG` rather than `--config` because vej 0.1.1
     rejects unknown flags. Keep `VEJ_CONFIG` honored, and keep an existing comment-only file
     parsing as all defaults.
- [ ] **2. Implement the `[models.<id>]` overrides.** `path` and `template` are parsed into
   `Config::models` but never read. They are the only way to give a custom GGUF the right chat
   template: today `-m /path/x.gguf` guesses the template from the filename, so a Mistral or
   Phi file silently gets ChatML. They also let a catalog id point at a different
   quantization.
   - [ ] Add a `ModelResolver`, built from `Config::models` and the config file's directory, that
     maps a query (catalog id, alias, custom name or file path) to
     `(PathBuf, Option<TemplateStyle>)`.
   - [ ] Override keys are canonicalized with `find_model_spec`, so aliases work. Keys that aren't
     in the catalog define new names, and those entries must have a `path`.
   - [ ] Resolve a relative `path` against the config file's directory, not the current directory.
   - [ ] `template` alone (no `path`) is allowed for catalog models. Add `FromStr` for
     `TemplateStyle` (Llama3, ChatML, Gemma, Gemma4, case-insensitive). An unknown template
     fails when the config loads.
   - [ ] A missing `path` file fails when that model is resolved. `serve` resolves its startup
     model before loading, so a bad override for that model still stops startup.
   - [ ] Use the resolver everywhere `resolve_model_path` is called today: `handle_serve`,
     `handle_decide`, `EngineLoader` (load, and P3's `check`), and `service install`.
   - [ ] New names are usable locally (`-m <name>`, `POST /v1/models/switch`). Remote requests can
     still only select catalog models (`catalog_model` in handlers); keep that, and add a test
     that a remote request naming a custom override is ignored.
   - [ ] Document the section in `vej.example.toml` and the README, with one catalog-override
     example and one custom-model example.
- [ ] **3. A config parse error silently falls back to defaults** (`main.rs`,
   `load_config().unwrap_or_else`). Make it fatal for `serve`, `decide` and `service install`.
   Other commands may keep the warning.

---

## P6 — CI on pull requests; release workflow hardening

**Files:** `.github/workflows/ci.yml` (new), `.github/workflows/release.yml`,
`.github/dependabot.yml` (new).

The only workflow runs on release tags, which is how a Windows compile break first appeared at
v0.1.0. Add `ci.yml` on `pull_request` and pushes to `main`:

- [x] **rust** (matrix: `macos-latest`, `ubuntu-latest`): `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`. Copy the Linux apt step
  (`cmake build-essential`), `GGML_NATIVE=OFF`, and `Swatinem/rust-cache` from `release.yml`.
  The llama.cpp build is slow, so caching matters.
- [x] **windows** (`windows-latest`): `cargo check --all-targets` (x86_64 only).
- [x] **go**: `actions/setup-go` with `go-version-file: zvej/go.mod`; `go vet ./...`,
  `go test ./...`, and `govulncheck ./...` in `zvej/`.
- [x] **dart**: `dart-lang/setup-dart`; in `dart_vej/` run `dart pub get`,
  `dart analyze --fatal-infos`, `dart test`.
- [x] **java**: `actions/setup-java` (temurin 17); `mvn -B test` in `java_vej/`.
- [x] **audit**: `cargo audit` (install with `cargo install cargo-audit --locked`, or use
  `rustsec/audit-check`).

In `release.yml`:

- [x] Set top-level `permissions: contents: read` and give `contents: write` only to the
  `release` job.
- [x] Pin every third-party action to a full commit SHA with the tag as a comment
  (`uses: actions/checkout@<sha> # v4`). Look up SHAs with
  `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha`.
- [x] Add `dependabot.yml` for `github-actions`, `cargo`, `gomod` (`/zvej`), `pub` (`/dart_vej`)
  and `maven` (`/java_vej`), on a weekly schedule.

The workflow files can't be run locally. Check them with `actionlint` if it's available, and
keep job names stable so branch protection can require them later.

**Done** on branch `ci/p6-pr-ci` (worktree `../vej-p6`, 3 commits ending `b6308e5`), not
merged. actionlint passes, and every command the CI jobs run passes locally on the branch.
Running the new checks found two problems on `main` that would have failed CI immediately, so
the branch also fixes them (outside the planned files):
- `cddb188`: `cargo audit` found 5 vulnerabilities (`rustls` RUSTSEC-2026-0346 to 0349,
  `rustls-webpki` RUSTSEC-2026-0382) plus `rustix`/`tokio` unsound and `yoke-derive` yanked.
  Targeted, semver-compatible `cargo update -p` of those crates only.
- `7f3d786`: `govulncheck` found zvej reaching 30 Go 1.26.0 standard-library vulnerabilities,
  also present in released zvej binaries. `zvej/go.mod` now says `go 1.26.11` (setup-go
  ignores the `toolchain` directive). One unreachable `golang.org/x/crypto` finding remains;
  its fix is not released yet.
Also added: an `npm` (`/playground`) entry in Dependabot. CI uses pinned `govulncheck@v1.9.0`
and `taiki-e/install-action` for `cargo-audit`. The workflows run for the first time when
pushed; watch the first run, especially the Windows check and the llama.cpp build times.

---

## P7 — Documentation drift

**Files:** `README.md`, `LOGIC.md`, `vej.toml`, `src/cli.rs` (help strings and one test value).

- [x] `gemma-2-2b` and `gemma-2-9b` appear in `README.md` (8 places), `LOGIC.md` (2), `vej.toml`
  (3) and `src/cli.rs` (help text at the `model` arg of `ServeArgs` and `ModelsCommands::Pull`,
  plus the `serve_flags_override_config` test) but are not in `RECOMMENDED_MODELS`
  (`src/models.rs`). The catalog is the source of truth: llama-3.2-1b, smollm2-1.7b,
  llama-3.2-3b, llama-3.1-8b, granite-4.2-3b, gemma-4-e2b. Update the docs to match. The
  playground list belongs to P8.
- [x] Latency claims disagree: `vej.toml` says "~6-12ms" and README says 150–220 ms for
  llama-3.2-1b. Measured on an Apple M2 Pro with Metal: about 64 ms engine `latency_ms` and
  about 84 ms end to end for a short 3-choice decision. Use one consistent statement that
  names the hardware, or drop the specific numbers.
- [x] In `vej.toml`, fix the preset comments. Leave the `[models.<id>]` section and the file name
  alone: P5 implements the overrides and renames the file to `vej.example.toml`.

**Done** on branch `fix/p7-docs-drift` (commit `a172c82`), merged to `main` in `840c23b`.
Note for P5: the `[models.gemma-2-2b]` example section in `vej.toml` still names a model that
isn't in the catalog; replace it when rewriting that section.

---

## P8 — Playground: HTML escaping, model list, API key storage

**Files:** `playground/src/app.js`, `playground/index.html`, `playground/src/presets.js`.

- [x] 1. Many `innerHTML` templates interpolate unescaped user and server data: question names,
   criteria, instructions, model ids, verdicts, option labels, and responses relayed from
   TypeSafe through `/v1/proxy/jev`. Add an `escapeHtml` helper that escapes `& < > " '`, and
   apply it to every interpolated value, including those inside attributes such as
   `value="${qName}"`. Prefer `textContent` where no markup is needed.
- [x] 2. The model `<select>` hard-codes a list that includes models not in the catalog
   (`gemma-2-2b`, `gemma-2-9b`) and misses `granite-4.2-3b`. Populate it from `GET /v1/models`
   (`id`, `is_downloaded`, `is_active`), falling back to the catalog ids when offline. Drop
   the per-model latency labels.
- [x] 3. The TypeSafe API key is kept in `localStorage`, so anything later served from the same
   origin (`http://127.0.0.1:8080`) can read it. Keep it in memory by default, with an opt-in
   "remember" checkbox that uses `sessionStorage`. Remove any old `localStorage` key on load.
- [x] 4. Bump the `app.js?v=` cache-busting query in `index.html`. The playground is embedded in the
   binary with `rust-embed`, so run `cargo build`, start `vej serve`, open `/playground`, and
   check that a decision renders and that a question named `<img src=x onerror=alert(1)>`
   displays as text. `npm ci && npm run build` must succeed.

**Done** on branch `fix/p8-playground` (commit `ef6bdb0`), merged to `main` in `31cdc74`.
Escaping lives in the new `playground/src/html.js` (embedded by the existing `src/*` rule).
Also beyond the plan: exported code snippets read the key from `TYPESAFE_API_KEY` instead of
embedding it, and storage access is wrapped so blocked storage cannot break the page. Verified
with a headless-Chromium run against `vej serve` plus `npm ci && npm run build` and `cargo test`.

---

## P9 — Dart SDK cleanup

**Files:** `dart_vej/**`. Release as 0.3.0 because `evaluate`'s return type changes.
wordstudy pins a dart_vej git ref, so nothing changes for it until the pin is bumped.

- [x] 1. `lib/src/prompt_template.dart` (`PromptBuilder`, `TemplateStyle`, exported from
   `lib/vej.dart`) duplicates the server's prompt templates and will drift. Mark both
   `@Deprecated('Prompts are built by the vej server; removed in 0.4.0')`.
- [x] 2. `evaluate()` returns `Future<dynamic>`. Add `sealed class EvaluationResult`, implemented by
   `DecisionResult` and `JevResult` (the Java SDK already has `EvaluationResult`), and return
   `Future<EvaluationResult>`.
- [x] 3. The five request methods repeat the same `try { … } catch (e)` block. That bare
   catch turns JSON parse failures into `VejConnectionException`. Extract private `_getJson` and
   `_postJson` helpers:
   - catch `http.ClientException` and `TimeoutException` as `VejConnectionException`;
   - report a wrong-shaped body as `VejException('Invalid response…')`, checking the shape with
     pattern matching instead of catching `TypeError`.
- [x] 4. Update `CHANGELOG.md` and `pubspec.yaml`. Add tests with `package:http/testing`
   `MockClient` for the parse-error and timeout paths.

**Done** on branch `fix/p9-dart-sdk` (commit `daecefc`), merged to `main` in `ed1f31a`.
Wrong-shaped responses are detected by the new `lib/src/json_reading.dart` (pattern-matched
reads that throw `FormatException`), and `BatchItemResult.result` is typed as
`EvaluationResult` too. wordstudy was checked against this version through a temporary path
override (analyze and vej tests pass); bump its `ref` in `pubspec.yaml` when adopting 0.3.0.

---

## P10 — Unix socket creation permissions

**Files:** `src/activation.rs`.

- [x] `bind_unix_socket` binds, then calls `set_permissions(0o600)`, so for an instant the socket
  has umask-derived permissions. zvej's default path sits in a private 0700 temp directory, but
  explicit `--remote-socket` paths do not. Set `libc::umask(0o177)` immediately before
  `UnixListener::bind` and restore the previous mask right after (this runs during startup,
  before the server spawns work). Keep the explicit `chmod`. Also warn when the parent directory
  is group- or world-writable. The existing test
  `binds_an_owner_only_socket_and_removes_it_on_drop` must keep passing; add one for the
  warning.

**Done** on branch `fix/p10-socket-perms` (commit `9826b68`), merged to `main` in `c223464`.
Uses a 0o077 umask instead of 0o177: the umask is process-wide, and 0o177 would give a
directory created concurrently mode 0600 (no execute bit, so unusable). All umask changes go
through one lock (`with_umask`); without it the umask test was flaky under parallel tests.
Missing socket directories are now created as 0700.

---

## P11 — Performance: reuse inference contexts (optional; measure first)

**Files:** the engine module and `src/server/handlers.rs`. Start after P2 and P3 merge.

Every decision creates a fresh `LlamaContext` (`create_context` in `evaluate_unified`), which
allocates the KV cache and compute buffers. Measured on M2 Pro with llama-3.2-1b: about 84 ms
end to end vs about 64 ms engine time, so roughly 20 ms per request is overhead, most of it
context creation. The comment "eliminates expensive context recreation" only holds within one
request.

- [x] **Approach:** `LlamaContext<'a>` borrows the model, so keep contexts in worker threads. When
  a model loads, start `max_concurrent` workers. Each holds an `Arc` to the engine, creates one
  default-size context, and serves jobs from a channel, replying on a oneshot. Jobs that need a
  larger context (big batches, generation) create one on demand as today. Dropping the engine
  closes the channel so workers exit. Lease, drain, and `keep_alive = 0` semantics must not
  change; the model-manager and server tests must still pass.
- [x] **Measure** before and after against a local `vej serve -m llama-3.2-1b --port 47931`:
  median of 8 `POST /v1/systemone` with
  `{"state":"I want my money back, the item arrived broken.","choices":["refund","support","billing"]}`,
  comparing wall time with the response's `latency_ms`.
- [x] **Done when** the overhead (wall minus `latency_ms`) drops below 5 ms with no regression in
  `latency_ms`. Record the numbers in the commit message. If the gain is under 10 ms, don't
  merge; report the numbers instead.

**Done** on branch `perf/p11-context-reuse` (commit `6026eb4`, worktree `../vej-p11`).
Split immutable state into `EngineCore` so worker threads in `src/engine/pool.rs` (`ContextPool`)
hold `Arc<EngineCore>` without reference cycles back to `DecisionEngine`; dropping
`DecisionEngine` shuts down the queue and joins all workers before releasing `Arc<LlamaModel>`.
Measured on M2 Pro (`llama-3.2-1b`, port 47931, median of 8 runs after 1 warmup):
- Before: wall 97.84 ms, `latency_ms` 64.08 ms, overhead 33.90 ms (`decision=refund`, `conf=0.6542`)
- After: wall 64.53 ms, `latency_ms` 63.93 ms, overhead 0.58 ms (`decision=refund`, `conf=0.6542`)
Gain: -33.32 ms overhead (-33.31 ms wall time), meeting the < 5 ms overhead and >= 10 ms gain
thresholds.

**Follow-up idea (not part of this plan):** reuse the KV cache for the shared system and state
prefix across the questions of one request.
