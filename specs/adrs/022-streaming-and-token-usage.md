# ADR 022: Streaming and token usage on the compatibility surfaces

- Status: accepted and implemented
- Date: 2026-09-02

## Context

`openai_compat` and `ollama_compat` translate the OpenAI and Ollama HTTP APIs
onto the `InferenceEngine` boundary from ADR 003. Both surfaces used to reject
`stream: true` with a typed error, and both reported token usage as three
nulls that no engine ever populated.

Neither gap was module-local. Streaming and token counts are properties of the
engine, and the engine paths do not share them: `echo` has no tokenizer and no
incremental output; `weightc`'s one-shot CLI emits one blob at process exit;
`ollama` streams native NDJSON and reports `eval_count`/`prompt_eval_count`,
but the daemon parsed neither into the response.

## Decision

### D1 — Emulate streaming on engines that cannot stream, and label it

Every engine accepts `stream: true`. Engines with native support stream real
deltas; the rest complete first and then emit the result as deltas. Responses
carry `x-hologram-stream: native | emulated`, derived from the engine's
`stream_kind()` rather than set per module.

ADR 003 and ADR 016 forbid synthesizing data — format versions, authorization
evidence, completion state, model output. Emulated streaming synthesizes none
of it: the tokens delivered are exactly the tokens the engine produced. Only
their arrival schedule is reconstructed, which is a transport detail rather
than a claim about the completion.

Rejecting instead was considered and declined. `echo` is the default engine
and OpenAI SDKs stream on the common path, so a strict daemon would return a
capability error to the first request any stock client makes — leaving the
integration ADR 016 explicitly preserves nominally present and practically
unusable. An `emulate_streaming` config toggle was also declined: it
reinstates the two-path test matrix ADR 016 collapsed, and its honest default
leaves the default engine broken regardless.

The disclosed cost: on emulated engines, time-to-first-token equals
time-to-full-completion. The header is what makes that legible.

### D2 — Never estimate token counts

The mirror of D1. A count the engine did not report is fabricated data, which
is exactly what ADR 016 forbids. Engines report counts or report nothing; the
daemon does not tokenize, and does not substitute zero.

### D3 — Omit `usage` entirely when counts are unknown

Absence reads as "not measured"; `0` would assert a measurement no engine
made, and `null` would violate OpenAI's declared integer type for the count
fields. Because OpenAI's schema requires `prompt_tokens`, `completion_tokens`,
and `total_tokens` together, a partial object cannot satisfy it — so the
object is emitted only when **both** counts are known, and omitted otherwise.

This replaces `Usage { Option<u64> × 3 }` with `Option<Usage>` of plain
`u64`, a breaking change to the response shape. ADR 016 permits the break
pre-release.

### D4 — Both surfaces move together

The engine work is shared and only the wire translation differs. The Ollama
API's own default is `stream: true`, so `ollama_compat` was the more acute
gap; both surfaces land in the same change rather than staggering the fix.

Throughout this record, `ollama_compat` (the inbound HTTP surface letting
Ollama clients talk to this daemon's engines) and `OllamaEngine` (the
outbound proxy to a separate Ollama server) are distinct.

### D5 — Buffered engines emit a single delta

Emulated deltas are written back-to-back with no delay, so splitting text on
whitespace conveys no timing information and invents token boundaries no
tokenizer produced. Progressive rendering should come from a real streaming
engine, not synthetic chunking. A buffered stream is therefore exactly
`Delta(text)` followed by `Done(summary)` — never more than one delta.

### The streaming contract on `InferenceEngine::complete_stream`

Every implementor, not just the buffered default, must uphold:

- Anything knowable before the first delta — a rejected request, an upstream
  connection or auth failure — is returned as `Err` from `complete_stream`
  itself, never yielded as an item inside the stream. That is what lets a
  caller commit to a 200 response only once it knows the request will
  actually produce output.
- A stream ends with exactly one `CompletionEvent::Done`, or with an `Err`
  item and no `Done` — never both, and never neither.

## Consequences

- Mid-stream failures are necessarily reported in-band: once bytes are on the
  wire the status is already 200, so a native engine that fails partway
  through cannot fall back to an HTTP error status. SSE emits one `data:`
  event carrying the surface's error envelope followed by `[DONE]`; NDJSON
  emits one `{"error": "..."}` line. The stream then terminates without a
  `finish_reason: "stop"` or `done_reason: "stop"`, so a failed stream can
  never be mistaken for a clean completion.
- Clients that only read the terminal frame for usage must instead read
  `stream_options.include_usage` (OpenAI) or the terminal NDJSON line
  (Ollama); both are still omitted, per D3, when the engine reported nothing.
- The two tests that used to prove `stream: true` was rejected
  (`stream_true_is_rejected` in each module) now assert the opposite: that a
  stream is returned. They remain the regression guard for D1.
- Buffered engines cannot offer real time-to-first-token; `x-hologram-stream:
  emulated` is what keeps that cost disclosed rather than silently absorbed.
