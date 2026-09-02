# Inference Streaming and Token Usage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `stream: true` work on the OpenAI- and Ollama-compatible HTTP surfaces for every engine, and report real token usage where an engine measures it.

**Architecture:** `src/inference.rs` splits into a module. The `InferenceEngine` trait gains a defaulted `stream_kind()` predicate and a defaulted `complete_stream()` that buffers `complete()`, so engines that cannot stream inherit emulation unchanged. `OllamaEngine` overrides both for real NDJSON streaming. Both compat modules translate the resulting event stream to their own wire format and label it with `x-hologram-stream`.

**Tech Stack:** Rust 1.94, axum 0.8 (SSE via the ungated `axum::response::sse`), tokio-stream, reqwest 0.12, utoipa 5, kameo 0.22.

**Spec:** `docs/superpowers/specs/2026-09-02-openai-streaming-usage-embeddings-design.md`

**Plan 1 of 3.** This plan implements §9 (module extraction) and §2–§5 (boundary, wire formats, mid-stream failure, usage). The llama.cpp engine (§10) and the `uor-r4` engine (§11) each get their own plan and both depend on this one landing first. §6 (embeddings) is specified but not built.

## Global Constraints

- **Never estimate token counts** (D2). Engines report counts or report nothing. The daemon does not tokenize and does not substitute zero.
- **Emit `usage` only when both counts are known** (D3). A partial pair cannot satisfy the OpenAI schema, which requires `total_tokens`.
- **Buffered engines emit a single delta** (D5). Do not split text on whitespace.
- **Both compat surfaces change together** (D4). `openai_compat` and `ollama_compat` stay in step.
- Errors use each surface's own envelope: `{"error": {message, type, code}}` for OpenAI, `{"error": "..."}` for Ollama.
- `LiveError` variants map to codes via `LiveError::code()`; `Capability` is `LIVE_CAPABILITY_MISSING`.
- Follow the existing comment density: `//!` module headers and `///` on public items explaining *why*, not *what*.

---

### Task 1: Extract `src/inference.rs` into a module

A pure file move. No behaviour changes, no new tests — the existing suite is the proof.

**Files:**
- Create: `src/inference/mod.rs`, `src/inference/echo.rs`, `src/inference/ollama.rs`, `src/inference/weightc/mod.rs`, `src/inference/weightc/session.rs`
- Delete: `src/inference.rs` (1,264 lines)

**Interfaces:**
- Consumes: nothing.
- Produces: `crate::inference::{CompletionRequest, Completion, InferenceEngine, EchoEngine, WeightcEngine, OllamaEngine, engine_from_config}` — every path identical to today, via re-exports in `mod.rs`.

- [ ] **Step 1: Confirm the suite is green before moving anything**

Run: `cargo test --locked 2>&1 | tail -20`
Expected: PASS. Record the test count; Step 6 must match it.

- [ ] **Step 2: Create the module skeleton and move the shared types**

`src/inference/mod.rs` keeps the `//!` header, `CompletionRequest`, `Completion`, the `InferenceEngine` trait, `engine_from_config`, and the shared helpers `elapsed_millis` and `stderr_tail` (both used by more than one engine, so they stay here and become `pub(crate)`).

At the top of `mod.rs`, declare and re-export so no call site changes:

```rust
mod echo;
mod ollama;
mod weightc;

pub use echo::EchoEngine;
pub use ollama::OllamaEngine;
pub use weightc::WeightcEngine;
```

Call sites that must keep compiling unchanged: `src/app.rs:91`, `src/chat.rs:5`, `src/chat.rs:85`, `src/chat.rs:123`, `src/modules/openai_compat.rs:11`, `src/modules/ollama_compat.rs:9`.

- [ ] **Step 3: Move each engine to its own file with its tests**

- `echo.rs` — `EchoEngine`, `last_user_content`, and the `echo_returns_*` tests.
- `ollama.rs` — `OllamaEngine`, `OllamaGenerateRequest`, `OllamaOptions`, `OllamaGenerateResponse`, `OllamaTagsResponse`, `OllamaTag`. No tests exist yet; Task 3 adds them.
- `weightc/mod.rs` — `WeightcEngine`, `WeightcAskOutput`, `SessionSpec`, `sampling_args`, and the `weightc_*` tests.
- `weightc/session.rs` — `SessionTable`, `WeightcSessionActor`, `SessionTurn`, `StopSession`, `TurnOutcome`, `TurnFailure`, `SessionRequestLine`, `SessionLine`, `SessionErrorLine`, `SHUTDOWN_TIMEOUT`, and the resident-session tests.

Each file gets a `//!` header naming its responsibility. Items shared between `weightc/mod.rs` and `weightc/session.rs` become `pub(super)`; items shared with `mod.rs` become `pub(crate)`.

The `fn prompt(text: &str) -> CompletionRequest` test helper (`src/inference.rs:885`) is used by the echo tests. Move it into `echo.rs` alongside them; `mod.rs`'s test module keeps `unknown_engine_is_a_config_error` and does not need it. Later tasks build `CompletionRequest` literals inline rather than depending on where this helper landed.

- [ ] **Step 4: Delete the original**

```bash
git rm src/inference.rs
```

- [ ] **Step 5: Fix visibility until it compiles**

Run: `cargo build --locked 2>&1 | tail -30`
Expected: PASS. Any error here is a missing `pub(super)` / `pub(crate)`, not a logic problem.

- [ ] **Step 6: Verify the suite is unchanged**

Run: `cargo test --locked 2>&1 | tail -20`
Expected: PASS with the **same test count as Step 1**. A different count means a test was dropped in the move — find it before continuing.

- [ ] **Step 7: Commit**

```bash
git add -A src/inference src/inference.rs
git commit -m "refactor(inference): split the engine boundary into a module

Pure file move ahead of adding streaming and two more engines. The 1,264-line
inference.rs held the trait, three engines, the resident session actor, and its
tests. mod.rs re-exports every type, so no call site changes."
```

---

### Task 2: Add `TokenUsage` to the boundary

**Files:**
- Modify: `src/inference/mod.rs`, `src/inference/ollama.rs`, `src/inference/weightc/mod.rs`, `src/inference/weightc/session.rs`
- Test: `src/inference/mod.rs` (inline `mod tests`), `src/inference/ollama.rs`

**Interfaces:**
- Consumes: `Completion` from Task 1.
- Produces: `TokenUsage { prompt_tokens: u64, completion_tokens: u64 }` with `TokenUsage::from_counts(Option<u64>, Option<u64>) -> Option<TokenUsage>` and `TokenUsage::total() -> u64`; `Completion.usage: Option<TokenUsage>`.

- [ ] **Step 1: Write the failing test**

In `src/inference/mod.rs`, inside the existing `mod tests`:

```rust
#[test]
fn usage_needs_both_counts_to_be_reportable() {
    assert_eq!(
        TokenUsage::from_counts(Some(18), Some(42)),
        Some(TokenUsage {
            prompt_tokens: 18,
            completion_tokens: 42
        })
    );
    // A partial pair cannot satisfy the OpenAI schema, which requires a total.
    assert_eq!(TokenUsage::from_counts(None, Some(42)), None);
    assert_eq!(TokenUsage::from_counts(Some(18), None), None);
    assert_eq!(TokenUsage::from_counts(None, None), None);
}

#[test]
fn usage_total_saturates_rather_than_overflowing() {
    let usage = TokenUsage {
        prompt_tokens: u64::MAX,
        completion_tokens: 1,
    };
    assert_eq!(usage.total(), u64::MAX);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked usage_needs_both_counts 2>&1 | tail -10`
Expected: FAIL — `cannot find type TokenUsage in this scope`.

- [ ] **Step 3: Add the type and wire it through `Completion`**

In `src/inference/mod.rs`:

```rust
/// Token counts an engine measured. Both fields are required: the OpenAI
/// schema needs a total, so a half-known pair is not reportable (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl TokenUsage {
    /// Reports usage only when the engine measured both halves. Never
    /// estimates and never substitutes zero (D2).
    pub const fn from_counts(prompt: Option<u64>, completion: Option<u64>) -> Option<Self> {
        match (prompt, completion) {
            (Some(prompt_tokens), Some(completion_tokens)) => Some(Self {
                prompt_tokens,
                completion_tokens,
            }),
            _ => None,
        }
    }

    pub const fn total(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}
```

Add `pub usage: Option<TokenUsage>` to `Completion`, then fix every construction site — including the test-side ones, which the compiler will point at:

- Production: `EchoEngine::complete`, `WeightcEngine::complete_one_shot`, `WeightcSessionActor::parse_receipt`, `OllamaEngine::complete`. All set `usage: None` for now; Step 5 fills in Ollama.
- Tests: the `MirrorEngine` in `src/modules/openai_compat.rs` and the one in `src/modules/ollama_compat.rs` both build a `Completion` literal, as does `src/chat.rs` around line 85. Each gets `usage: None`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked usage_ 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Parse the counts Ollama already sends**

`OllamaGenerateResponse` reads `eval_count` today only to derive a rate, then discards it. `prompt_eval_count` is never read at all. In `src/inference/ollama.rs`, add the field and build usage:

```rust
#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
    eval_duration: Option<u64>,
}
```

In `complete`, replace `usage: None` with:

```rust
usage: TokenUsage::from_counts(parsed.prompt_eval_count, parsed.eval_count),
```

- [ ] **Step 6: Add the tolerant weightc fields**

Both weightc parsers already ignore unknown fields, so adding these needs no change to the external CLI — they are absent today and populate automatically if it ever emits them.

In `src/inference/weightc/mod.rs`, add to `WeightcAskOutput`:

```rust
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
```

and set `usage: TokenUsage::from_counts(parsed.prompt_tokens, parsed.completion_tokens)` in `complete_one_shot`.

Do the same for `SessionLine` in `src/inference/weightc/session.rs` and `parse_receipt`.

- [ ] **Step 7: Run the full suite**

Run: `cargo test --locked 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/inference
git commit -m "feat(inference): carry measured token usage on Completion

TokenUsage reports only when an engine measured both halves; a partial pair
cannot satisfy the OpenAI schema, which needs a total. Ollama already sends
eval_count (read for a rate, then discarded) and prompt_eval_count (never
read); both are now parsed. The weightc parsers gain optional count fields,
which their tolerant decoding picks up if the CLI ever emits them."
```

---

### Task 3: Stub HTTP server for the Ollama engine

`OllamaEngine` has no tests. This adds the harness the streaming work depends on. No new dependencies: `axum` is already a primary dependency, so the stub binds a `TcpListener` on port 0.

**Files:**
- Modify: `src/inference/ollama.rs` (add `mod tests`)

**Interfaces:**
- Consumes: `TokenUsage`, `Completion` from Task 2.
- Produces: `tests::spawn_stub(routes) -> String` returning the stub's base URL, reused by Task 7.

- [ ] **Step 1: Write the failing test**

At the bottom of `src/inference/ollama.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;

    /// Binds an ephemeral port and serves `router`, returning its base URL.
    /// Uses axum directly rather than a new dev-dependency.
    pub(super) async fn spawn_stub(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let address = listener.local_addr().expect("read the bound address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{address}")
    }

    fn config_for(endpoint: &str) -> InferenceConfig {
        InferenceConfig {
            engine: "ollama".to_owned(),
            default_model: "test-model".to_owned(),
            ollama_endpoint: endpoint.to_owned(),
            ..InferenceConfig::default()
        }
    }

    #[tokio::test]
    async fn generate_reports_the_counts_ollama_sends() {
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "response": "hello",
                    "prompt_eval_count": 18,
                    "eval_count": 42,
                    "eval_duration": 1_000_000_000_u64,
                }))
            }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = OllamaEngine::new(&config_for(&endpoint)).expect("build the engine");

        let completion = engine
            .complete(CompletionRequest {
                prompt: "hi".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("the stub responds successfully");

        assert_eq!(completion.text, "hello");
        assert_eq!(
            completion.usage,
            Some(TokenUsage {
                prompt_tokens: 18,
                completion_tokens: 42
            })
        );
    }

    #[tokio::test]
    async fn generate_omits_usage_when_ollama_sends_no_counts() {
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({ "response": "hello" }))
            }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = OllamaEngine::new(&config_for(&endpoint)).expect("build the engine");

        let completion = engine
            .complete(CompletionRequest {
                prompt: "hi".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("the stub responds successfully");

        assert_eq!(completion.usage, None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked ollama 2>&1 | tail -20`
Expected: FAIL. If `InferenceConfig` does not derive `Default`, or its fields are not all public, fix that first — the weightc tests already build configs this way, so mirror whatever they do.

- [ ] **Step 3: Make them pass**

No production change should be needed; Task 2 Step 5 already added the parsing. If `generate_reports_the_counts_ollama_sends` fails on `usage`, the `prompt_eval_count` field is missing from `OllamaGenerateResponse`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked ollama 2>&1 | tail -20`
Expected: PASS, 2 tests.

- [ ] **Step 5: Commit**

```bash
git add src/inference/ollama.rs
git commit -m "test(inference): cover the Ollama engine with an axum stub server

The engine had no tests. The stub binds an ephemeral port using axum, already a
primary dependency, so this adds no dev-dependency. It is the harness the
native streaming path needs."
```

---

### Task 4: Report usage on the OpenAI surface

**Files:**
- Modify: `src/modules/openai_compat.rs:88-94` (`Usage`), `:76-84` (`ChatCompletion`), `:284-292` (construction), `:466-468` (existing assertion)

**Interfaces:**
- Consumes: `TokenUsage` from Task 2.
- Produces: `Usage { prompt_tokens: u64, completion_tokens: u64, total_tokens: u64 }` with `impl From<TokenUsage> for Usage`; `ChatCompletion.usage: Option<Usage>`.

- [ ] **Step 1: Write the failing test**

Replace the existing assertion at `src/modules/openai_compat.rs:466-468` — it asserts three nulls, which is exactly the shape D3 removes — and add its counterpart:

```rust
    #[tokio::test]
    async fn usage_is_omitted_when_the_engine_reports_no_counts() {
        let fixture = fixture();
        let completion = complete_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            request("", &["Hello"]),
        )
        .await
        .expect("the echo engine always completes");

        assert!(completion.usage.is_none(), "echo measures no tokens");
        let encoded = serde_json::to_value(&completion).expect("serialize");
        assert!(
            encoded.get("usage").is_none(),
            "an absent key reads as not-measured; null would violate the \
             declared int type"
        );
    }

    #[test]
    fn usage_carries_the_total_when_the_engine_measured_both_halves() {
        let usage = Usage::from(crate::inference::TokenUsage {
            prompt_tokens: 18,
            completion_tokens: 42,
        });
        assert_eq!(usage.prompt_tokens, 18);
        assert_eq!(usage.completion_tokens, 42);
        assert_eq!(usage.total_tokens, 60);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --lib openai_compat 2>&1 | tail -20`
Expected: FAIL — `usage` is not an `Option`, and `Usage: From<TokenUsage>` does not exist.

- [ ] **Step 3: Change the shape**

```rust
/// Present only when the engine measured both halves (D3). Absence reads as
/// not-measured; `0` would assert a measurement no engine made.
#[derive(Debug, Serialize, ToSchema)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl From<crate::inference::TokenUsage> for Usage {
    fn from(usage: crate::inference::TokenUsage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total(),
        }
    }
}
```

On `ChatCompletion`:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
```

In `complete_chat`, replace the three-`None` literal with `usage: completion.usage.map(Usage::from)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --lib openai_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/modules/openai_compat.rs
git commit -m "feat(openai-compat): omit usage rather than reporting null counts

The three nulls violated OpenAI's schema, which declares these as required
integers. usage is now present only when an engine measured both halves."
```

---

### Task 5: Report usage on the Ollama surface

**Files:**
- Modify: `src/modules/ollama_compat.rs` (`GenerateResponse` and `ChatResponse` structs and their construction sites)

**Interfaces:**
- Consumes: `TokenUsage` from Task 2.
- Produces: `prompt_eval_count` / `eval_count` fields on both non-streaming responses, omitted when unknown.

- [ ] **Step 1: Write the failing test**

In the `mod tests` of `src/modules/ollama_compat.rs`:

```rust
    #[tokio::test]
    async fn generate_omits_counts_when_the_engine_measures_none() {
        let fixture = fixture();
        let response = generate_core(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            GenerateRequest {
                model: String::new(),
                prompt: "Hello".to_owned(),
                stream: Some(false),
                options: None,
            },
        )
        .await
        .expect("the echo engine always completes");

        let encoded = serde_json::to_value(&response).expect("serialize");
        assert!(encoded.get("eval_count").is_none());
        assert!(encoded.get("prompt_eval_count").is_none());
    }
```

The non-streaming helpers are `generate_core` (`src/modules/ollama_compat.rs:270`) and `chat_core` (`:301`); the public handlers are `generate` (`:234`) and `chat` (`:256`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --lib ollama_compat 2>&1 | tail -20`
Expected: FAIL — the fields do not exist.

- [ ] **Step 3: Add the fields**

On both `GenerateResponse` and `ChatResponse`:

```rust
    /// Omitted unless the engine measured both halves (D3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u64>,
```

At each construction site:

```rust
        prompt_eval_count: completion.usage.map(|usage| usage.prompt_tokens),
        eval_count: completion.usage.map(|usage| usage.completion_tokens),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --lib ollama_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/modules/ollama_compat.rs
git commit -m "feat(ollama-compat): report measured token counts when available"
```

---

### Task 6: Add the streaming types and the buffered default

**Files:**
- Modify: `src/inference/mod.rs`
- Test: `src/inference/mod.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `TokenUsage`, `Completion`, `InferenceEngine` from Task 2.
- Produces: `StreamKind::{Native, Buffered}` with `header_value() -> &'static str`; `CompletionSummary { model: String, usage: Option<TokenUsage>, tokens_per_second: Option<f64>, elapsed_millis: u64 }`; `CompletionEvent::{Delta(String), Done(CompletionSummary)}`; `type CompletionStream = Pin<Box<dyn Stream<Item = Result<CompletionEvent>> + Send>>`; trait methods `stream_kind()` and `complete_stream()`.

- [ ] **Step 1: Write the failing test**

In the `mod tests` of `src/inference/mod.rs`:

```rust
    #[tokio::test]
    async fn the_buffered_default_yields_one_delta_then_done() {
        use tokio_stream::StreamExt;

        let engine = EchoEngine;
        assert_eq!(engine.stream_kind(), StreamKind::Buffered);

        // Built inline rather than via the `prompt` test helper: Task 1 moves
        // the echo tests to echo.rs, so that helper's home is not fixed here.
        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "Hello".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("the echo engine always completes");

        let mut deltas = Vec::new();
        let mut summary = None;
        while let Some(event) = stream.next().await {
            match event.expect("the buffered default never errors mid-stream") {
                CompletionEvent::Delta(text) => deltas.push(text),
                CompletionEvent::Done(done) => summary = Some(done),
            }
        }

        assert_eq!(
            deltas,
            vec!["Hello".to_owned()],
            "D5: buffered engines emit a single delta, not whitespace chunks"
        );
        assert!(summary.is_some(), "the stream must terminate with Done");
    }

    #[test]
    fn the_header_distinguishes_real_streaming_from_emulation() {
        assert_eq!(StreamKind::Native.header_value(), "native");
        assert_eq!(StreamKind::Buffered.header_value(), "emulated");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked buffered_default 2>&1 | tail -10`
Expected: FAIL — `no method named complete_stream`.

- [ ] **Step 3: Add the types**

In `src/inference/mod.rs`, alongside the existing imports add `use std::pin::Pin;` and `use tokio_stream::Stream;`, then:

```rust
/// How an engine produces token deltas. Engines that cannot stream still
/// accept `stream: true`; only the arrival schedule is reconstructed, which
/// the `x-hologram-stream` header discloses (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// Deltas arrive as the model produces them.
    Native,
    /// No incremental output; deltas are replayed from a completed response.
    Buffered,
}

impl StreamKind {
    /// Value reported in the `x-hologram-stream` response header.
    pub const fn header_value(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Buffered => "emulated",
        }
    }
}

/// Terminal record of a streamed completion.
#[derive(Debug, Clone, Default)]
pub struct CompletionSummary {
    pub model: String,
    pub usage: Option<TokenUsage>,
    pub tokens_per_second: Option<f64>,
    pub elapsed_millis: u64,
}

/// One unit of a streamed completion.
#[derive(Debug, Clone)]
pub enum CompletionEvent {
    Delta(String),
    Done(CompletionSummary),
}

pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<CompletionEvent>> + Send>>;
```

- [ ] **Step 4: Add the trait members**

Inside `pub trait InferenceEngine`:

```rust
    /// Whether deltas are real or reconstructed. Drives the
    /// `x-hologram-stream` header, so the honesty marker comes from the engine
    /// rather than being set per module.
    fn stream_kind(&self) -> StreamKind {
        StreamKind::Buffered
    }

    /// Buffered default: awaits the whole completion, then replays it as a
    /// single delta (D5). Because the completion is awaited before the stream
    /// is returned, a failure surfaces as a normal typed error with the
    /// correct status rather than a half-open stream.
    async fn complete_stream(&self, request: CompletionRequest) -> Result<CompletionStream> {
        let completion = self.complete(request).await?;
        let summary = CompletionSummary {
            model: completion.model,
            usage: completion.usage,
            tokens_per_second: completion.tokens_per_second,
            elapsed_millis: completion.elapsed_millis,
        };
        Ok(Box::pin(tokio_stream::iter(vec![
            Ok(CompletionEvent::Delta(completion.text)),
            Ok(CompletionEvent::Done(summary)),
        ])))
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --locked --lib inference 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/inference/mod.rs
git commit -m "feat(inference): add a streaming seam with a buffering default

stream_kind() and complete_stream() follow the supports_sessions() idiom: a
defaulted capability predicate plus an overridable method, so engines that
cannot stream inherit emulation without changing. The default awaits the whole
completion first, so those engines still fail with a typed error and the right
status instead of a half-open stream."
```

---

### Task 7: Native NDJSON streaming for the Ollama engine

**Files:**
- Modify: `Cargo.toml` (reqwest `stream`, tokio-stream `sync`), `src/inference/ollama.rs`
- Test: `src/inference/ollama.rs`

**Interfaces:**
- Consumes: `StreamKind`, `CompletionEvent`, `CompletionStream`, `CompletionSummary` from Task 6; `spawn_stub` from Task 3.
- Produces: `OllamaEngine::stream_kind() == StreamKind::Native` and a real `complete_stream`.

- [ ] **Step 1: Add the two feature flags**

In `Cargo.toml`, `reqwest` gains `stream` so the engine can read the body incrementally, and `tokio-stream` gains `sync` for `wrappers::ReceiverStream`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "blocking", "stream"] }
tokio-stream = { version = "0.1", features = ["net", "sync"] }
```

Run: `cargo build --locked 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 2: Write the failing test**

In the `mod tests` of `src/inference/ollama.rs`:

```rust
    #[tokio::test]
    async fn streaming_yields_each_ndjson_line_then_the_final_counts() {
        use tokio_stream::StreamExt;

        let body = concat!(
            "{\"response\":\"Hel\",\"done\":false}\n",
            "{\"response\":\"lo\",\"done\":false}\n",
            "{\"response\":\"\",\"done\":true,\"prompt_eval_count\":18,\"eval_count\":42}\n"
        );
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(move || async move { body }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = OllamaEngine::new(&config_for(&endpoint)).expect("build the engine");

        assert_eq!(engine.stream_kind(), StreamKind::Native);

        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "hi".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("the stub responds successfully");

        let mut deltas = Vec::new();
        let mut summary = None;
        while let Some(event) = stream.next().await {
            match event.expect("the stub sends well-formed lines") {
                CompletionEvent::Delta(text) => deltas.push(text),
                CompletionEvent::Done(done) => summary = Some(done),
            }
        }

        assert_eq!(deltas, vec!["Hel".to_owned(), "lo".to_owned()]);
        assert_eq!(
            summary.expect("the stream terminates with Done").usage,
            Some(TokenUsage {
                prompt_tokens: 18,
                completion_tokens: 42
            })
        );
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --locked streaming_yields_each 2>&1 | tail -10`
Expected: FAIL — the default `complete_stream` yields one delta of the whole body, and `stream_kind()` is `Buffered`.

- [ ] **Step 4: Implement the native path**

Add the line type near `OllamaGenerateResponse`:

```rust
/// One NDJSON line of a streaming `/api/generate` response. The terminal line
/// carries `done: true` and the counts.
#[derive(Debug, Deserialize)]
struct OllamaStreamLine {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
}
```

In `impl InferenceEngine for OllamaEngine`:

```rust
    fn stream_kind(&self) -> StreamKind {
        StreamKind::Native
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<CompletionStream> {
        use tokio_stream::StreamExt;

        let response = self.send_generate(&request, true).await?;
        let started = Instant::now();
        let model = self.model.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(16);

        tokio::spawn(async move {
            let mut body = response.bytes_stream();
            let mut buffered = Vec::new();
            while let Some(chunk) = body.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        let _ = sender
                            .send(Err(LiveError::Transport(format!(
                                "ollama stream failed: {error}"
                            ))))
                            .await;
                        return;
                    }
                };
                buffered.extend_from_slice(&chunk);
                // NDJSON: a line is only complete once its newline arrives, so
                // a partial tail stays buffered for the next chunk.
                while let Some(index) = buffered.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buffered.drain(..=index).collect();
                    let trimmed = String::from_utf8_lossy(&line).trim().to_owned();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let parsed: OllamaStreamLine = match serde_json::from_str(&trimmed) {
                        Ok(parsed) => parsed,
                        Err(error) => {
                            let _ = sender
                                .send(Err(LiveError::Protocol(format!(
                                    "parse ollama stream line: {error}"
                                ))))
                                .await;
                            return;
                        }
                    };
                    if !parsed.response.is_empty()
                        && sender
                            .send(Ok(CompletionEvent::Delta(parsed.response.clone())))
                            .await
                            .is_err()
                    {
                        return;
                    }
                    if parsed.done {
                        let _ = sender
                            .send(Ok(CompletionEvent::Done(CompletionSummary {
                                model: model.clone(),
                                usage: TokenUsage::from_counts(
                                    parsed.prompt_eval_count,
                                    parsed.eval_count,
                                ),
                                tokens_per_second: None,
                                elapsed_millis: elapsed_millis(started),
                            })))
                            .await;
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(receiver),
        ))
    }
```

Extract the request-building and status-checking half of the existing `complete` into `send_generate(&self, request: &CompletionRequest, stream: bool) -> Result<reqwest::Response>` and call it from both, so the two paths cannot drift. `complete` keeps `stream: false` and its existing parsing.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --locked --lib inference 2>&1 | tail -20`
Expected: PASS, including the two Task 3 tests, which prove `send_generate` did not change non-streaming behaviour.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/inference/ollama.rs
git commit -m "feat(inference): stream natively from the Ollama engine

Both paths share send_generate so the streaming and non-streaming requests
cannot drift. NDJSON lines are buffered until their newline arrives, since a
chunk boundary can split a line."
```

---

### Task 8: SSE streaming on the OpenAI surface

**Files:**
- Modify: `src/modules/openai_compat.rs`
- Test: `src/modules/openai_compat.rs` (including inverting `stream_true_is_rejected` at `:600`)

**Interfaces:**
- Consumes: `StreamKind`, `CompletionEvent`, `CompletionStream` from Task 6.
- Produces: `ChatCompletionChunk`, `ChunkChoice`, `ChunkDelta`, `StreamOptions { include_usage: bool }`; `chat_completions` returning `Response`.

- [ ] **Step 1: Write the failing test**

Replace `stream_true_is_rejected` — it is the regression guard for D1 and must be rewritten, not deleted:

```rust
    /// Was `stream_true_is_rejected`. D1 reverses that behaviour: every engine
    /// accepts stream: true, and the header says whether it was real.
    #[tokio::test]
    async fn streaming_emits_ordered_chunks_and_marks_emulation() {
        let fixture = fixture();
        let mut request = request("", &["Hello"]);
        request.stream = Some(true);

        let response = stream_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            request,
        )
        .await
        .expect("the echo engine streams by emulation");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "emulated"
        );

        let body = collect_sse(response).await;
        assert!(
            body.contains("\"role\":\"assistant\""),
            "the first chunk announces the role: {body}"
        );
        assert!(body.contains("\"content\":\"Hello\""), "{body}");
        assert!(body.contains("\"finish_reason\":\"stop\""), "{body}");
        assert!(body.trim_end().ends_with("data: [DONE]"), "{body}");
    }

    /// Reads a streaming response body to a String for assertion.
    async fn collect_sse(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read the streamed body");
        String::from_utf8(bytes.to_vec()).expect("utf-8")
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --lib openai_compat 2>&1 | tail -20`
Expected: FAIL — `stream_chat` does not exist.

- [ ] **Step 3: Add the chunk types**

```rust
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct StreamOptions {
    /// Adds a usage-bearing chunk with empty `choices` before `[DONE]`. The
    /// only standard way a streaming client can obtain counts.
    #[serde(default)]
    pub include_usage: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: ChunkDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Serialize, ToSchema)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}
```

Add to `ChatCompletionRequest`:

```rust
    #[serde(default)]
    pub stream_options: Option<StreamOptions>,
```

The `request(model, contents)` test helper builds `ChatCompletionRequest` as a
literal, so it needs `stream_options: None` added or the module stops
compiling.

- [ ] **Step 4: Branch the handler and build the chunk stream**

```rust
pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, OpenAiError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    if request.stream == Some(true) {
        return stream_chat(engine, catalog, &default_model, request).await;
    }
    let completion = complete_chat(engine, catalog, &default_model, request).await?;
    Ok(Json(completion).into_response())
}

/// Streaming half of `chat_completions`, kept free of `AppState` so tests can
/// drive it with a bare engine and catalog.
async fn stream_chat(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: ChatCompletionRequest,
) -> Result<Response, OpenAiError> {
    if request.messages.is_empty() {
        return Err(OpenAiError::invalid_request("messages must not be empty"));
    }
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let kind = engine.stream_kind();
    let include_usage = request
        .stream_options
        .as_ref()
        .is_some_and(|options| options.include_usage);
    let created = unix_seconds();
    let id = completion_id(created, &model);
    let events = engine
        .complete_stream(CompletionRequest {
            prompt: render_prompt(&request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            seed: request.seed,
            session_key: None,
        })
        .await
        .map_err(OpenAiError::from)?;

    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        use tokio_stream::StreamExt;

        let chunk = |choices, usage| ChatCompletionChunk {
            id: id.clone(),
            object: "chat.completion.chunk".to_owned(),
            created,
            model: model.clone(),
            choices,
            usage,
        };
        // `send` must be a macro, not a closure: async closures are unstable,
        // and `try_send` would silently truncate the stream whenever a slow
        // client let the 16-slot channel fill. `.send().await` applies
        // backpressure instead.
        macro_rules! send {
            ($value:expr) => {{
                let encoded = serde_json::to_string(&$value).unwrap_or_default();
                sender
                    .send(Ok::<_, std::convert::Infallible>(
                        axum::response::sse::Event::default().data(encoded),
                    ))
                    .await
            }};
        }

        let role = chunk(
            vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: Some("assistant".to_owned()),
                    content: None,
                },
                finish_reason: None,
            }],
            None,
        );
        if send!(role).is_err() {
            return;
        }

        let mut events = events;
        let mut usage = None;
        while let Some(event) = events.next().await {
            match event {
                Ok(CompletionEvent::Delta(text)) => {
                    let delta = chunk(
                        vec![ChunkChoice {
                            index: 0,
                            delta: ChunkDelta {
                                role: None,
                                content: Some(text),
                            },
                            finish_reason: None,
                        }],
                        None,
                    );
                    if send!(delta).is_err() {
                        return;
                    }
                }
                Ok(CompletionEvent::Done(summary)) => usage = summary.usage,
                Err(error) => {
                    // Status is already 200, so in-band is the only honest
                    // way to report this (§4). Return rather than break: a
                    // finish_reason of "stop" after a failure would claim a
                    // clean completion that did not happen.
                    let envelope = OpenAiErrorEnvelope {
                        error: OpenAiError::from(error).body,
                    };
                    let _ = send!(envelope);
                    let _ = sender
                        .send(Ok(axum::response::sse::Event::default().data("[DONE]")))
                        .await;
                    return;
                }
            }
        }

        let stop = chunk(
            vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta::default(),
                finish_reason: Some("stop".to_owned()),
            }],
            None,
        );
        let _ = send!(stop);

        if include_usage {
            if let Some(usage) = usage {
                let final_chunk = chunk(Vec::new(), Some(Usage::from(usage)));
                let _ = send!(final_chunk);
            }
        }

        let _ = sender
            .send(Ok(axum::response::sse::Event::default().data("[DONE]")))
            .await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(receiver);
    let mut response = axum::response::sse::Sse::new(stream).into_response();
    response.headers_mut().insert(
        "x-hologram-stream",
        axum::http::HeaderValue::from_static(kind.header_value()),
    );
    Ok(response)
}
```

`OpenAiError::body` must become `pub(crate)` for the in-band error path.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --locked --lib openai_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Add the include_usage test**

```rust
    #[tokio::test]
    async fn include_usage_adds_no_chunk_when_the_engine_measured_nothing() {
        let fixture = fixture();
        let mut request = request("", &["Hello"]);
        request.stream = Some(true);
        request.stream_options = Some(StreamOptions { include_usage: true });

        let response = stream_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            request,
        )
        .await
        .expect("the echo engine streams by emulation");

        let body = collect_sse(response).await;
        assert!(
            !body.contains("\"usage\""),
            "echo measures nothing, so no usage chunk is emitted (D3): {body}"
        );
    }
```

Run: `cargo test --locked --lib openai_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/modules/openai_compat.rs
git commit -m "feat(openai-compat): stream chat completions over SSE

Every engine now accepts stream: true; x-hologram-stream reports whether the
deltas were real or replayed. stream_options.include_usage adds the usage
chunk when an engine measured counts. The stream_true_is_rejected test is
inverted rather than deleted: it is the regression guard for D1."
```

---

### Task 9: NDJSON streaming on the Ollama surface

**Files:**
- Modify: `src/modules/ollama_compat.rs`
- Test: `src/modules/ollama_compat.rs` (inverting the rejection tests at `:600` and `:622`)

**Interfaces:**
- Consumes: `StreamKind`, `CompletionEvent` from Task 6.
- Produces: streaming `generate` and `chat` handlers returning `application/x-ndjson`.

- [ ] **Step 1: Write the failing test**

```rust
    /// Was `stream_true_is_rejected`. Ollama's own API defaults to
    /// stream: true, so rejecting it broke that ecosystem's default path.
    #[tokio::test]
    async fn generate_streams_ndjson_lines_and_marks_emulation() {
        let fixture = fixture();
        let response = stream_generate(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            GenerateRequest {
                model: String::new(),
                prompt: "Hello".to_owned(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the echo engine streams by emulation");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "emulated"
        );

        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read the streamed body");
        let body = String::from_utf8(bytes.to_vec()).expect("utf-8");
        let lines: Vec<&str> = body.lines().filter(|line| !line.is_empty()).collect();

        assert!(lines[0].contains("\"response\":\"Hello\""), "{body}");
        assert!(lines[0].contains("\"done\":false"), "{body}");
        assert!(
            lines.last().expect("a terminal line").contains("\"done\":true"),
            "{body}"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --lib ollama_compat 2>&1 | tail -20`
Expected: FAIL — `stream_generate` does not exist.

- [ ] **Step 3: Add the streaming line type and handlers**

```rust
/// One NDJSON line. `/api/generate` carries `response`; `/api/chat` carries
/// `message`. Exactly one is present per line.
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamLine {
    pub model: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<OllamaMessage>,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u64>,
}
```

Branch the handler, then build the line stream:

```rust
pub async fn generate(
    State(state): State<AppState>,
    Json(request): Json<GenerateRequest>,
) -> Result<Response, OllamaError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    if request.stream == Some(true) {
        return stream_generate(engine, catalog, &default_model, request).await;
    }
    let response = generate_core(engine, catalog, &default_model, request).await?;
    Ok(Json(response).into_response())
}

/// Streaming half of `generate`, kept free of `AppState` so tests can drive it
/// with a bare engine and catalog.
async fn stream_generate(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: GenerateRequest,
) -> Result<Response, OllamaError> {
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let kind = engine.stream_kind();
    let events = engine
        .complete_stream(completion_request(&request.prompt, request.options))
        .await
        .map_err(OllamaError::from)?;
    Ok(ndjson_response(kind, model, events, LineShape::Response))
}

/// Which field carries the text: `/api/generate` uses `response`,
/// `/api/chat` uses `message`.
#[derive(Debug, Clone, Copy)]
enum LineShape {
    Response,
    Message,
}

fn ndjson_response(
    kind: StreamKind,
    model: String,
    events: CompletionStream,
    shape: LineShape,
) -> Response {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<String, std::convert::Infallible>>(16);

    tokio::spawn(async move {
        use tokio_stream::StreamExt;

        let line = |text: Option<String>, done: bool, summary: Option<CompletionSummary>| {
            let (response, message) = match (shape, text) {
                (_, None) => (None, None),
                (LineShape::Response, Some(text)) => (Some(text), None),
                (LineShape::Message, Some(text)) => (
                    None,
                    Some(OllamaMessage {
                        role: "assistant".to_owned(),
                        content: text,
                    }),
                ),
            };
            let usage = summary.as_ref().and_then(|summary| summary.usage);
            let value = StreamLine {
                model: model.clone(),
                created_at: rfc3339_now(),
                response,
                message,
                done,
                done_reason: done.then(|| "stop".to_owned()),
                prompt_eval_count: usage.map(|usage| usage.prompt_tokens),
                eval_count: usage.map(|usage| usage.completion_tokens),
            };
            format!("{}\n", serde_json::to_string(&value).unwrap_or_default())
        };

        let mut events = events;
        let mut summary = None;
        while let Some(event) = events.next().await {
            match event {
                Ok(CompletionEvent::Delta(text)) => {
                    if sender.send(Ok(line(Some(text), false, None))).await.is_err() {
                        return;
                    }
                }
                Ok(CompletionEvent::Done(done)) => summary = Some(done),
                Err(error) => {
                    // Status is already 200, so in-band is the only honest
                    // way to report this (§4).
                    let envelope =
                        serde_json::json!({ "error": error.to_string() }).to_string();
                    let _ = sender.send(Ok(format!("{envelope}\n"))).await;
                    return;
                }
            }
        }
        let _ = sender.send(Ok(line(None, true, summary))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(receiver);
    let mut response = axum::body::Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/x-ndjson"),
    );
    response.headers_mut().insert(
        "x-hologram-stream",
        axum::http::HeaderValue::from_static(kind.header_value()),
    );
    response
}
```

`chat` branches identically, calling `ndjson_response(..., LineShape::Message)`. `rfc3339_now()` already exists in this module (used at `:295` and `:329`), so use it directly. For building a `CompletionRequest` from `options`, reuse whatever `generate_core` already does; if that logic is inline, extract it into a `completion_request` helper so the streaming and non-streaming paths cannot drift.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --lib ollama_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Invert the chat rejection test the same way**

Mirror Step 1's test for `/api/chat`, asserting `"message":{"role":"assistant"` appears rather than `"response"`.

Run: `cargo test --locked --lib ollama_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/modules/ollama_compat.rs
git commit -m "feat(ollama-compat): stream generate and chat as NDJSON

Ollama's own API defaults to stream: true, so rejecting it broke that
ecosystem's default path. Both rejection tests are inverted rather than
deleted."
```

---

### Task 10: Mid-stream failure on both surfaces

§4 applies only to native engines: buffered ones await the whole completion
before the stream is returned, so they still fail with a normal typed error and
the right status. This task proves the native case, and incidentally gives
`x-hologram-stream` its only `native` coverage in this plan.

**Files:**
- Test: `src/modules/openai_compat.rs`, `src/modules/ollama_compat.rs`

**Interfaces:**
- Consumes: `StreamKind`, `CompletionEvent`, `CompletionStream` from Task 6; `stream_chat` from Task 8; `stream_generate` from Task 9.
- Produces: no production code — a fixture engine per test module.

- [ ] **Step 1: Add the fixture engine to `src/modules/openai_compat.rs`**

Place it beside the existing `MirrorEngine`:

```rust
    /// Streams one delta, then fails. Only a native engine can fail this way;
    /// buffered engines resolve `complete()` before the stream exists.
    struct HalfwayFailingEngine;

    #[tonic::async_trait]
    impl InferenceEngine for HalfwayFailingEngine {
        fn name(&self) -> &'static str {
            "halfway-failing"
        }

        fn stream_kind(&self) -> StreamKind {
            StreamKind::Native
        }

        async fn complete(&self, _request: CompletionRequest) -> crate::error::Result<Completion> {
            Err(LiveError::Transport("this fixture only streams".to_owned()))
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> crate::error::Result<CompletionStream> {
            Ok(Box::pin(tokio_stream::iter(vec![
                Ok(CompletionEvent::Delta("Hel".to_owned())),
                Err(LiveError::Transport("the engine vanished".to_owned())),
            ])))
        }

        async fn list_models(&self) -> crate::error::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }
```

- [ ] **Step 2: Write the failing test**

```rust
    #[tokio::test]
    async fn a_mid_stream_failure_is_reported_in_band_and_marked_native() {
        let fixture = fixture();
        let mut request = request("", &["Hello"]);
        request.stream = Some(true);

        let response = stream_chat(
            Arc::new(HalfwayFailingEngine),
            fixture.catalog.clone(),
            "halfway-failing",
            request,
        )
        .await
        .expect("the stream opens before the failure occurs");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "native"
        );

        let body = collect_sse(response).await;
        assert!(body.contains(r#""content":"Hel""#), "the delta arrives first: {body}");
        assert!(
            body.contains("LIVE_TRANSPORT_UNAVAILABLE"),
            "the failure is reported in-band: {body}"
        );
        assert!(
            !body.contains(r#""finish_reason":"stop""#),
            "a failed stream must not claim a clean stop: {body}"
        );
        assert!(body.trim_end().ends_with("data: [DONE]"), "{body}");
    }
```

- [ ] **Step 3: Run test to check that it fails**

Run: `cargo test --locked --lib a_mid_stream_failure 2>&1 | tail -20`
Expected: FAIL — either `HalfwayFailingEngine` is undefined, or the body still
carries a `finish_reason` chunk because Task 8's error arm was left as a
`break`.

- [ ] **Step 4: Make it pass**

No new production code should be required; Task 8 Step 4 already returns after
the error envelope. If the `finish_reason` assertion fails, change the error arm
in `stream_chat` to `return` rather than `break`.

Run: `cargo test --locked --lib a_mid_stream_failure 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Mirror the test on the Ollama surface**

Copy `HalfwayFailingEngine` into the `mod tests` of `src/modules/ollama_compat.rs`
verbatim — it is a fixture, and the two test modules share no helper module
today. Assert instead that the body's last line parses as JSON carrying an
`error` key, and that no line carries `"done":true`.

Run: `cargo test --locked --lib ollama_compat 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/modules/openai_compat.rs src/modules/ollama_compat.rs
git commit -m "test(compat): cover mid-stream failure on both surfaces"
```

---

### Task 11: Documentation and the ADR

**Files:**
- Create: `specs/adrs/022-streaming-and-token-usage.md`
- Modify: `specs/adrs/003-inference-engine-boundary.md`, `README.md:869-880`, `README.md:917`, `README.md:1031`, `ACTUAL_CAPABILITIES.md:42`, `src/modules/openai_compat.rs:1-7`, `src/modules/ollama_compat.rs:1-5`, `apps/docs/public/openapi.json`

**Interfaces:**
- Consumes: everything above.
- Produces: no code.

- [ ] **Step 1: Write ADR 022**

Record D1–D5 with the reasoning from the spec: emulation is honest because only the arrival schedule is reconstructed; counts are never estimated because a number no engine reported is fabricated data; `usage` is omitted rather than zeroed; both surfaces move together; buffered engines emit one delta. Note that 021 is the previous highest.

- [ ] **Step 2: Update ADR 003's closing consequence**

It currently calls token streaming a "deliberate fast-follow." Change it to record that streaming landed on the boundary with an emulating default, and that the boundary itself is unchanged.

- [ ] **Step 3: Correct every "non-streaming" claim**

- `README.md` "Inference compatibility APIs": the paragraph saying `stream: true` is rejected is now wrong. Show a streaming `curl` and explain `x-hologram-stream`.
- `README.md:917`: the module table calls `openai-compat` "Non-streaming".
- `README.md:1031`: "Token streaming ... remain future work" — remove streaming from that list.
- `ACTUAL_CAPABILITIES.md:42`: describes both surfaces as non-streaming.
- Both modules' `//!` headers and their `utoipa` tag descriptions say "non-streaming subset".

- [ ] **Step 4: Regenerate the OpenAPI document**

Run: `just docs`
Expected: `apps/docs/public/openapi.json` changes — `usage` is no longer required, and the chunk schemas appear.

- [ ] **Step 5: Full verification**

Run: `cargo test --locked 2>&1 | tail -20`
Run: `cargo clippy --locked --all-targets -- -D warnings 2>&1 | tail -20`
Run: `cargo fmt --check`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add specs/adrs README.md ACTUAL_CAPABILITIES.md src/modules apps/docs/public/openapi.json
git commit -m "docs: record streaming and token usage in ADR 022

Corrects every non-streaming claim across the README, ACTUAL_CAPABILITIES, and
both module headers, and updates ADR 003, which called streaming a fast-follow."
```
