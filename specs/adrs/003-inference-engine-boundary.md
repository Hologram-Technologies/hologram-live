# ADR 003: Inference runs behind an engine boundary

## Status

Accepted.

## Decision

The daemon never executes model weights in-process. Chat and model management call an `InferenceEngine` trait selected by `[inference].engine` in `live.toml`. Three engines ship in v1:

- `echo` — local fallback that repeats the user message; remains the default.
- `weightc` — spawns the external `weightc ask <artifact-dir> <prompt> --json` one-shot CLI against an imported `.wcpu` artifact directory, bounded by `request_timeout_secs`.
- `ollama` — proxies `POST /api/generate` to an Ollama-compatible HTTP endpoint, streaming natively when asked to.

Imported `.wcpu` artifact directories are copied under `data_dir/models/<digest>/` and recorded in the content-addressed object store as manifest JSON with `kind = "model"`. The daemon renders conversation history as a plain `role: content` transcript; engines apply their own chat templates.

## Consequences

- Chat behavior changes only through configuration; the echo demo path is preserved exactly for existing clients, tests, and the desktop.
- An unconfigured engine or missing model returns `LIVE_CAPABILITY_MISSING` instead of simulating a response.
- Model execution inherits the external engine's isolation and performance characteristics; the daemon only mediates configuration, timeouts, and storage.
- OpenAI/Ollama-compatible HTTP API surfaces were a deliberate fast-follow on top of this boundary. Resident `weightc enter --jsonl` sessions (one supervised process per conversation, LRU-capped) have since landed as an opt-in engine mode. Token streaming has since landed too (ADR 022): every engine accepts `stream: true`, with engines that cannot stream natively emulating it by completing first and replaying the result as deltas. The boundary itself is unchanged.
