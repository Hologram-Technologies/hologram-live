# Streaming, token usage, and embeddings for the compatibility surfaces

- Date: 2026-09-02
- Status: approved design, not yet implemented
- Scope: implement streaming (§2–§4) and token usage (§5); specify embeddings (§6)
- Revised 2026-09-02 after review: the native streaming path is deferred (D6)

## Context

`openai_compat` and `ollama_compat` translate the OpenAI and Ollama HTTP APIs
onto the `InferenceEngine` boundary from ADR 003. Both surfaces reject
`stream: true` with a typed error, and both report token usage as three nulls
that no engine ever populates.

Neither gap is module-local. Streaming and token counts are properties of the
engine, and the four engine paths do not share them:

| Engine path | Streaming | Token counts |
|---|---|---|
| `echo` | none | none — no tokenizer |
| `ollama` | native NDJSON on `/api/generate` | `eval_count` parsed today but discarded; `prompt_eval_count` unread |
| `weightc` one-shot | none — `ask --json` emits one blob at process exit | none |
| `weightc` resident JSONL | only if the external CLI emits deltas | none |

The daemon carries no streaming primitives today: no SSE, no `futures` direct
dependency, and `reqwest` without the `stream` feature.

## Decisions

### D1 — Emulate streaming on engines that cannot stream, and label it

Every engine accepts `stream: true`. Engines with native support stream real
deltas; the rest complete first and then emit the result as deltas. Responses
carry `x-hologram-stream: native | emulated`.

ADR 003 and ADR 016 forbid synthesizing data — format versions, authorization
evidence, completion state, model output. Emulated streaming synthesizes none
of it: the tokens delivered are exactly the tokens the engine produced. Only
their arrival schedule is reconstructed, which is a transport detail rather
than a claim about the completion.

Rejecting instead was considered and declined. `echo` is the default engine and
OpenAI SDKs stream on the common path, so a strict daemon would return a
capability error to the first request any stock client makes — leaving the
integration ADR 016 explicitly preserves nominally present and practically
unusable. An `emulate_streaming` config toggle was also declined: it reinstates
the two-path test matrix ADR 016 collapsed, and its honest default leaves the
default engine broken regardless.

The disclosed cost: on emulated engines, time-to-first-token equals
time-to-full-completion. The header is what makes that legible.

### D2 — Never estimate token counts

The mirror of D1. A count the engine did not report is fabricated data, which
is exactly what ADR 016 forbids. Engines report counts or report nothing; the
daemon does not tokenize, and does not substitute zero.

### D3 — Omit `usage` entirely when counts are unknown

Absence reads as "not measured"; `0` would assert a measurement no engine made.
Because OpenAI's schema requires `prompt_tokens`, `completion_tokens`, and
`total_tokens` together, a partial object cannot satisfy it — so the object is
emitted only when **both** counts are known, and omitted otherwise.

This replaces `Usage { Option<u64> × 3 }` with `Option<Usage>` of plain `u64`,
a breaking change to the current response shape. The only consumer of the
existing nulls is a unit test in `src/modules/openai_compat.rs`; no desktop or
TypeScript client reads them. ADR 016 permits the break pre-release.

### D4 — Both surfaces in one landing

The engine work is shared and only the wire translation differs. The Ollama
API's own default is `stream: true`, so `ollama_compat` is the more acute gap.

Throughout this document, `ollama_compat` (the inbound HTTP surface letting
Ollama clients talk to this daemon's engines) and `OllamaEngine` (the outbound
proxy to a separate Ollama server) are distinct. D6 defers the latter only.
The `ollama_compat` surface lands in full.

### D5 — Buffered engines emit a single delta

Emulated deltas are written back-to-back with no delay, so splitting text on
whitespace conveys no timing information and invents token boundaries no
tokenizer produced. Progressive rendering should come from a real streaming
engine, not synthetic chunking.

### D6 — Defer the native streaming path

`OllamaEngine` is the only engine that can stream natively or report token
counts, and the target deployment does not run Ollama: `echo` is the default
and `weightc` is the first-party engine. Building the native override now
would add the largest item in the test plan — a stub HTTP server, the only
reason dev-dependencies would grow — to serve a path nobody executes.

Deferred: the `OllamaEngine` overrides of `stream_kind` and
`complete_stream`, its `prompt_eval_count` parsing, the `reqwest` `stream`
feature, and the stub server. Everything else lands.

The trait seam is kept rather than emulating inside the modules, so adopting
a native path later needs no boundary change. This has one honest cost:
`StreamKind::Native` ships with no implementor and the header always reports
`emulated`. A two-variant enum with one reachable variant is the kind of
speculative branch ADR 016 removes elsewhere, and it is retained here only
because it is roughly five lines and spares clients a second change when a
native engine arrives. If that trade stops looking worthwhile, collapsing to
a header-free buffered-only surface is the fallback.

The higher-value follow-up is external: if `weightc enter --jsonl` emitted
incremental delta lines and token counts, this design accepts both with no
further change here — the receipt parsers are already tolerant (§5).

## §2 Engine boundary

Approach: additive. A defaulted capability predicate plus an overridable
method, matching how `supports_sessions()` already handles heterogeneous
capability on this same trait.

```rust
pub enum StreamKind { Native, Buffered }

pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

pub struct CompletionSummary {
    pub model: String,
    pub usage: Option<TokenUsage>,
    pub tokens_per_second: Option<f64>,
    pub elapsed_millis: u64,
}

pub enum CompletionEvent {
    Delta(String),
    Done(CompletionSummary),
}

pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<CompletionEvent>> + Send>>;

// added to InferenceEngine:
fn stream_kind(&self) -> StreamKind { StreamKind::Buffered }

async fn complete_stream(&self, request: CompletionRequest)
    -> Result<CompletionStream>
{
    // default: await complete(), then yield Delta(text), Done(summary)
}
```

`Stream` comes from `tokio_stream`, already a direct dependency, so the
boundary adds no new crate. `Completion` gains `usage: Option<TokenUsage>`.
`complete()` itself is unchanged and remains the path for `stream: false`
requests; `complete_stream` is strictly additive.

`OllamaEngine` is where these members are overridden when the native path
lands (D6); at this landing every engine uses the default. `EchoEngine` and both `weightc`
paths are untouched and inherit emulation, which is why the well-tested
resident session actor — where a turn is a single kameo `ask` returning one
`TurnOutcome` — needs no rework.

The buffered default awaits the entire completion before returning the stream,
so failures on those engines surface as a normal JSON error with the correct
status code rather than a half-open SSE stream. The native path cannot offer
that guarantee; see §4.

Rejected alternatives: making `complete_stream` the sole primitive with
`complete` collecting it (forces a stream abstraction onto the actor path for
no behavioural gain), and emulating inside the compat modules with no trait
change (structurally cannot reach Ollama's native stream, and duplicates
chunking across both modules).

## §3 Wire formats

Both surfaces derive the `x-hologram-stream` header from `stream_kind()` rather
than setting it per module.

At this landing the header always reports `emulated`, since no engine
implements the native path (D6).

**OpenAI — `text/event-stream`**

```
data: {"id":…,"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}
data: {…,"choices":[{"index":0,"delta":{"content":"…"},"finish_reason":null}]}
data: {…,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}
data: [DONE]
```

`stream_options: {include_usage: true}` is supported: when requested and both
counts are known, a usage-bearing chunk with empty `choices` precedes `[DONE]`.
This is the only standard way a streaming client obtains counts, so it is what
makes §5 reachable while streaming. When counts are unknown the field is
omitted, per D3.

**Ollama — `application/x-ndjson`**

```
{"model":…,"created_at":…,"response":"…","done":false}
{"model":…,"created_at":…,"done":true,"done_reason":"stop","eval_count":42,"prompt_eval_count":18}
```

`/api/chat` substitutes `{"message":{"role":"assistant","content":"…"}}` for
`response`. Count fields are omitted from the terminal line when unknown.

## §4 Mid-stream failure

Applies to the native path only, so it is specified but unreachable until D6
is revisited; buffered engines fail before the response starts. Once bytes are on the wire the status is already 200, so in-band
delivery is the only honest option: SSE emits one `data:` event carrying the
surface's error envelope followed by `[DONE]`; NDJSON emits one
`{"error": "…"}` line. The stream then terminates.

## §5 Token usage plumbing

- **Ollama** — deferred with D6. When it lands: parse `prompt_eval_count`
  alongside the already-parsed `eval_count` (today `eval_count` is read only to
  derive a rate, then discarded), and extract usage for the streaming and
  non-streaming paths through one shared function, since `OllamaEngine` will
  hit the same endpoint with different `stream` flags. That shared parse is
  what keeps the two from drifting.
- **weightc** — add optional `prompt_tokens` / `completion_tokens` to
  `WeightcAskOutput` and `SessionLine`. Both parsers already ignore unknown
  fields, so this requires no coordination with the external CLI: the fields
  are absent today and are picked up automatically if the CLI ever emits them.
- **echo** — always `None`.

Consequently, at this landing `usage` is always omitted: `echo` never reports
counts and `weightc` does not emit the fields yet. The observable change from
today is that the key is absent rather than three nulls. It is built now
because it is cheap and because it starts working on its own, without a second
change here, the moment an engine reports counts.

No dependency changes. The `reqwest` `stream` feature is deferred with D6.

## §6 Embeddings — specified, not implemented

Same idiom as §2:

```rust
fn supports_embeddings(&self) -> bool { false }

async fn embed(&self, request: EmbeddingRequest) -> Result<Embeddings> {
    Err(LiveError::Capability(…))
}
```

A separate `EmbeddingEngine` trait was considered and declined: it would force
a second construction path through `engine_from_config` for no isolation gain.

**`POST /v1/embeddings`**

Request: `{model, input: string | string[], encoding_format: "float" |
"base64", dimensions?}`. Response: `{object: "list", data: [{object:
"embedding", index, embedding}], model, usage}`. `base64` encodes
little-endian `f32` values, which some OpenAI SDK versions request by default.
`usage` follows D3.

Per engine: Ollama implements it via `POST /api/embed` (`{"model", "input"}` →
`{"embeddings": [[…]]}`); `weightc` and `echo` return the capability error.
Since Ollama is the only capable engine, implementing this endpoint before a
capable engine exists would ship a permanent capability error; it stays
specified and unbuilt, and lands with or after D6.

**No emulation here.** Unlike arrival scheduling, a synthesized vector is
fabricated *data*, so D1 does not extend to embeddings and D2 governs instead.

New config key `inference.embedding_model`, because the chat default model is
usually not an embedding model.

## §7 Testing

No new dev-dependencies. The stub HTTP server is deferred with D6, which
removes the largest scaffolding item from this landing; `OllamaEngine` remains
untested, as it is today.

Coverage to add:

- buffered default yields exactly `Delta` then `Done`
- chunk sequencing and `[DONE]` ordering, per surface
- `x-hologram-stream` reflects `stream_kind()`; only `emulated` is reachable
  until D6 is revisited
- usage key absent when counts are unknown, which is every engine at this
  landing; the both-counts-known branch is covered at the type level via a
  `TokenUsage`-bearing summary rather than through an engine
- `stream_options.include_usage` chunk shape
- mid-stream error shape is deferred with D6, being unreachable without a
  native path
- every existing non-streaming test stays green, with one deliberate
  exception: the two tests asserting that `stream: true` is rejected
  (`stream_true_is_rejected` in each module) invert to assert a stream is
  returned. They are the regression guard for D1 and must be rewritten,
  not deleted.

The compatibility surfaces have no BDD coverage under `features/`; this design
does not add any.

## §8 Documentation

- New **ADR 022** recording D1–D6. 021 is the current highest.
- ADR 003's closing consequence calls streaming a "deliberate fast-follow" and
  needs updating.
- `README.md` "Inference compatibility APIs" states streaming is rejected.
- `ACTUAL_CAPABILITIES.md` describes both surfaces as non-streaming.
- The module table in `README.md` labels `openai-compat` non-streaming.
- Both modules' `//!` headers and their `utoipa` tag descriptions say
  "non-streaming subset".
- Regenerate `apps/docs/public/openapi.json` via `just docs`; it currently
  declares `usage` required.
