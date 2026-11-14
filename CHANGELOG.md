# Changelog

All notable changes to vej. The Dart SDK keeps its own log in
[`dart_vej/CHANGELOG.md`](dart_vej/CHANGELOG.md). Earlier releases are described on the
[GitHub releases page](https://github.com/moscait/vej/releases).

## 0.2.0 (unreleased)

A hardening release from a full-codebase review: safer model loading and downloads, input
limits on every endpoint, prompt-injection defenses, a persistent inference context, and a
config system that no longer depends on the working directory.

### Breaking changes

- **Configuration file lookup.** `./vej.toml` in the working directory is no longer read.
  vej uses the first of `--config <PATH>`, `$VEJ_CONFIG`, then
  `<user config dir>/vej/config.toml`. When a `./vej.toml` exists and no explicit config was
  given, vej warns and ignores it. An explicit `--config` or `VEJ_CONFIG` file that does not
  exist is an error; a missing user config still means defaults. The repository's example
  file is now `vej.example.toml`.
- **Config errors are fatal for `serve`, `decide` and `service install`.** A file that fails
  to parse, or an invalid `[models]` section, stops those commands instead of silently
  falling back to defaults. Other commands warn and continue.
- **`vej config init` writes to the user config path** (or `--path <PATH>`), not the working
  directory. `vej config path` now says which rule chose the path.
- **Model ids with dots must be quoted in `[models.<name>]` headers**, for example
  `[models."llama-3.2-1b"]`. TOML splits unquoted dotted keys, so the old form configured a
  model called `llama-3`; vej now reports this with the quoted form. Empty sections copied
  from the old example file still load.
- **Default generation system prompt.** `/v1/generate` and `/api/generate` no longer use a
  Bible-study persona. The default is a general "precise, concise assistant" that still
  returns bare JSON when asked. Callers that want a persona send it in the new `system`
  field.
- **`/v1/batch` keys for `choice` questions with array criteria** are now the option text
  (`"Paris"`) instead of letters (`"A"`), matching `/v1/systemone`.
- **Malformed requests return 400, not 500**, and inputs are bounded: at most 26 lettered
  choices, 101 score levels, 32 questions per request, 256 batch items, 1024 item×question
  tasks, a 2 MB body, and prompts that fit the context window.
- **dart_vej 0.3.0.** `evaluate()` returns a sealed `EvaluationResult` instead of `dynamic`,
  and wrong-shaped responses throw `VejException` rather than `VejConnectionException`.
  `PromptBuilder` and `TemplateStyle` are deprecated. Consumers that pin a git ref are
  unaffected until they bump it.

### Added

- **`[models.<name>]` overrides are implemented.** Point a catalog model at another GGUF
  file, fix the chat template of a custom file (`Llama3`, `ChatML`, `Gemma`, `Gemma4`), or
  give a custom file a name usable with `-m`, `default_model` and `POST /v1/models/switch`.
  Relative paths resolve against the config file's directory. Remote clients can still only
  select catalog models.
- **`vej service install --config <PATH>`** records the config path in the service's
  `serve` arguments; the `VEJ_CONFIG` passthrough is unchanged.
- **Verified model downloads.** Each catalog entry pins a Hugging Face commit and the file's
  SHA-256. `vej models pull` streams through a hasher and only moves the file into the
  cache when the digest matches; a mismatch deletes the partial file and reports both
  digests.
- **`system` and `model` on generation requests.** `/v1/generate` and `/api/generate`
  accept a `system` prompt and honor `model` the same way decisions do.
- **Opt-in real-model tests.** Set `VEJ_TEST_MODEL` to run `tests/real_model.rs` against a
  local GGUF (see README).
- **CI on pull requests** for Rust (macOS, Linux, Windows check), Go, Dart and Java, plus
  `cargo audit` and `govulncheck`. Release workflow permissions are minimal, actions are
  pinned to commits, and Dependabot is enabled.

### Fixed

- **Concurrency limit held on disconnect.** A client that disconnected mid-inference freed
  its slot while the work kept running, so more than `max_concurrent` inferences could run
  at once. The permit now lives with the blocking job.
- **TypeSafe proxy** reuses one HTTP client with connect and request timeouts, mapping
  timeouts to 504 and other failures to 502.
- **A failed model load no longer evicts the working model.** Requests naming a model that
  is not downloaded, or a bad `POST /v1/models/switch` path, fail without unloading what is
  serving. A cancelled request can no longer leave a load running unsupervised, so two
  models are never resident at once.
- **Scoring never collides on a shared first token.** The label scheme (boolean, numeric or
  letters A–Z) is chosen per question from the tokenizer and only single-token labels are
  scored. On digit-splitting tokenizers such as Gemma, `1` and `10` used to share a score.
- **Prompt injection through special tokens.** Control tokens such as `<|eot_id|>` inside
  `state`, `instruction`, criteria or generation prompts are neutralized before
  tokenization, so user-supplied text cannot inject chat-template structure.
- **Single requests with `questions` use the batch decode**, sharing one code path and
  about 25% less time for four questions.
- **Remote Unix socket** is created owner-only from the first instant (umask), missing
  socket directories are created `0700`, and a group- or world-writable parent directory
  logs a warning.
- **Playground:** all rendered data is HTML-escaped, the model list comes from
  `GET /v1/models`, and the TypeSafe API key stays in memory (session storage on opt-in)
  instead of `localStorage`. Exported snippets read the key from `TYPESAFE_API_KEY`.
- **Catalog and docs** agree again: the removed `gemma-2` models are gone from the docs and
  playground, latency claims name the hardware, and catalog sizes reflect the pinned files
  (granite-4.2-3b is 2.24 GB and gemma-4-e2b is 3.11 GB).
- **Dependencies:** `rustls`, `rustls-webpki`, `rustix`, `tokio` and `yoke-derive`
  advisories; zvej builds with Go 1.26.11 for standard-library security fixes. Dependabot
  bumps: llama-cpp-2 0.1.161 (the engine now tokenizes through the new vocab API),
  reqwest 0.13, sha2 0.11, toml 1.2, getrandom 0.4, the GitHub Actions used by CI and
  release, the Java SDK's Jackson, JUnit and Maven plugins, and the playground's Vite 8.

### Performance

- **Persistent inference contexts.** Worker threads keep one llama context per
  `max_concurrent` slot instead of allocating a KV cache per request. On an M2 Pro with
  llama-3.2-1b, per-request overhead fell from about 34 ms to under 1 ms (wall time 97.8 ms
  to 64.5 ms) with identical results.

### Upgrade notes

- Move a `./vej.toml` to the user config path (`vej config init`, then copy your settings)
  or pass `--config ./vej.toml`.
- Quote dotted ids in `[models.<name>]` headers.
- If you relied on the Bible-study default when generating, send that text as `system`.
- Batch callers that matched `/v1/batch` answers against letters should match option text.
- wordstudy already sends `system` and starts vej with `VEJ_CONFIG` pointing at its own
  file; bump its dart_vej ref to adopt 0.3.0.
