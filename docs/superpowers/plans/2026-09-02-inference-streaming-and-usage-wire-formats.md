# Inference Streaming and Token Usage Implementation Plan — Part B

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the compatibility-surface portion of streaming and token usage after the shared engine boundary and native Ollama stream from Part A.

**Prerequisite:** Complete [Part A, Tasks 1–7](2026-09-02-inference-streaming-and-usage.md). Its global constraints apply unchanged here.

**Spec:** `docs/superpowers/specs/2026-09-02-openai-streaming-usage-embeddings-design.md`

**Plan 1 of 3, Part B.** This file covers Tasks 8–11: OpenAI SSE, Ollama NDJSON, mid-stream failure behavior on both surfaces, and documentation.

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
        let send = |sender: &tokio::sync::mpsc::Sender<_>, value: &ChatCompletionChunk| {
            let encoded = serde_json::to_string(value).unwrap_or_default();
            sender.try_send(Ok::<_, std::convert::Infallible>(
                axum::response::sse::Event::default().data(encoded),
            ))
        };

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
        if send(&sender, &role).is_err() {
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
                    if send(&sender, &delta).is_err() {
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
                    let encoded = serde_json::to_string(&envelope).unwrap_or_default();
                    let _ = sender.try_send(Ok(
                        axum::response::sse::Event::default().data(encoded)
                    ));
                    let _ = sender
                        .try_send(Ok(axum::response::sse::Event::default().data("[DONE]")));
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
        let _ = send(&sender, &stop);

        if include_usage {
            if let Some(usage) = usage {
                let final_chunk = chunk(Vec::new(), Some(Usage::from(usage)));
                let _ = send(&sender, &final_chunk);
            }
        }

        let _ = sender.try_send(Ok(axum::response::sse::Event::default().data("[DONE]")));
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
    let response = generate_response(engine, catalog, &default_model, request).await?;
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

`chat` branches identically, calling `ndjson_response(..., LineShape::Message)`. Reuse whatever helper the module already has for building a `CompletionRequest` from `options`; if there is none, extract one from `generate_response` so both paths share it. `rfc3339_now()` is the module's existing timestamp helper — if the non-streaming responses build `created_at` inline, extract that into a function first.

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
