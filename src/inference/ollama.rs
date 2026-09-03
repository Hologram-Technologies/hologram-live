//! Ollama-compatible HTTP engine (`POST {endpoint}/api/generate`).

use super::{
    Completion, CompletionEvent, CompletionRequest, CompletionStream, CompletionSummary,
    InferenceEngine, StreamKind, TokenUsage,
};
use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::ModelInfo;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Cap on a single buffered NDJSON line while streaming. A legitimate Ollama
/// stream line is a few hundred bytes; 1 MiB is generous headroom while
/// still bounding a broken or hostile upstream that never sends a newline
/// from inflating the buffer without limit (bounded only in wall-clock by
/// the client timeout otherwise, not in bytes).
const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;

pub struct OllamaEngine {
    endpoint: String,
    model: String,
    /// Used by the non-streaming path. `.timeout()` is a deadline on the
    /// *entire* request including reading the whole response body, which is
    /// exactly right for a single buffered `complete()` call: either the
    /// full answer lands within `request_timeout_secs` or it didn't.
    client: reqwest::Client,
    /// Used only by `complete_stream`. A native stream can legitimately run
    /// far longer than `request_timeout_secs` as long as tokens keep
    /// arriving, so this client carries no `.timeout()` at all — instead
    /// `.read_timeout()` bounds each individual read, failing only after
    /// `request_timeout_secs` of silence. Do not collapse this back into
    /// `client`: doing so would cut healthy long-running streams dead at
    /// the whole-request deadline.
    stream_client: reqwest::Client,
}

impl OllamaEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| LiveError::Transport(format!("build ollama client: {error}")))?;
        let stream_client = reqwest::Client::builder()
            .read_timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| {
                LiveError::Transport(format!("build ollama streaming client: {error}"))
            })?;
        Ok(Self {
            endpoint: config.ollama_endpoint.trim_end_matches('/').to_owned(),
            model: config.default_model.clone(),
            client,
            stream_client,
        })
    }

    /// Builds and sends the shared `/api/generate` request, checking the
    /// response status before returning it. Both `complete` and
    /// `complete_stream` call this so the request body they send — and the
    /// point at which a non-2xx status becomes a typed error — cannot drift
    /// between the two paths. `stream` is the only difference between them.
    async fn send_generate(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<reqwest::Response> {
        if self.model.trim().is_empty() {
            return Err(LiveError::Capability(
                "the ollama engine requires inference.default_model to name a model tag".to_owned(),
            ));
        }
        let options = if request.max_tokens.is_some()
            || request.temperature.is_some()
            || request.seed.is_some()
        {
            Some(OllamaOptions {
                num_predict: request.max_tokens,
                temperature: request.temperature,
                seed: request.seed,
            })
        } else {
            None
        };
        let body = OllamaGenerateRequest {
            model: &self.model,
            prompt: &request.prompt,
            stream,
            options,
        };
        let client = if stream {
            &self.stream_client
        } else {
            &self.client
        };
        let response = client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("ollama generate: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LiveError::Transport(format!(
                "ollama generate failed ({status}): {}",
                super::stderr_tail(body.trim())
            )));
        }
        Ok(response)
    }
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
    eval_duration: Option<u64>,
}

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

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTag>,
}

#[derive(Debug, Deserialize)]
struct OllamaTag {
    name: String,
    #[serde(default)]
    size: u64,
}

#[tonic::async_trait]
impl InferenceEngine for OllamaEngine {
    fn name(&self) -> &'static str {
        "ollama"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let started = Instant::now();
        let response = self.send_generate(&request, false).await?;
        let parsed: OllamaGenerateResponse = response
            .json()
            .await
            .map_err(|error| LiveError::Protocol(format!("parse ollama response: {error}")))?;
        // Rates are informational; sub-token precision is not meaningful here.
        #[allow(clippy::cast_precision_loss)]
        let tokens_per_second = match (parsed.eval_count, parsed.eval_duration) {
            (Some(count), Some(duration)) if duration > 0 => {
                Some(count as f64 / (duration as f64 / 1_000_000_000.0))
            }
            _ => None,
        };
        Ok(Completion {
            text: parsed.response,
            model: self.model.clone(),
            tokens_per_second,
            elapsed_millis: super::elapsed_millis(started),
            usage: TokenUsage::from_counts(parsed.prompt_eval_count, parsed.eval_count),
        })
    }

    fn stream_kind(&self) -> StreamKind {
        StreamKind::Native
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<CompletionStream> {
        use tokio_stream::StreamExt;

        // Status is checked inside `send_generate` before we return here, so
        // any rejected request or upstream failure that is knowable up front
        // surfaces as a typed `Err` from this method — never as an item
        // inside the stream. Only once that succeeds do we commit to
        // returning a stream at all.
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
                // A chunk can legitimately carry several complete lines that
                // together exceed the cap — the drain loop below consumes
                // them and the buffer shrinks back down. What must never
                // happen is a single line with no newline growing past the
                // cap: that is the unbounded-buffer case, so check for it
                // here before draining.
                if buffered.len() > MAX_STREAM_LINE_BYTES && !buffered.contains(&b'\n') {
                    let _ = sender
                        .send(Err(LiveError::Protocol(format!(
                            "ollama stream line exceeded {MAX_STREAM_LINE_BYTES} bytes without a newline"
                        ))))
                        .await;
                    return;
                }
                // NDJSON: a line is only complete once its newline arrives, so
                // a partial tail (split across chunk boundaries) stays
                // buffered for the next chunk rather than being parsed early.
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
                                elapsed_millis: super::elapsed_millis(started),
                            })))
                            .await;
                        return;
                    }
                }
            }
            // The byte stream ended (connection closed, proxy reset, killed
            // model process, ...) without a `done: true` line ever arriving.
            // Falling off the end here without sending anything would let
            // the channel just close and `ReceiverStream` end silently — the
            // exact "neither Done nor Err" case the contract on
            // `complete_stream` forbids, since a consumer could not tell a
            // truncated answer from a normal short one. Report it instead.
            let _ = sender
                .send(Err(LiveError::Protocol(
                    "ollama stream ended before a done line".to_owned(),
                )))
                .await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.endpoint))
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("ollama tags: {error}")))?;
        if !response.status().is_success() {
            return Err(LiveError::Transport(format!(
                "ollama tags failed ({})",
                response.status()
            )));
        }
        let parsed: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|error| LiveError::Protocol(format!("parse ollama tags: {error}")))?;
        Ok(parsed
            .models
            .into_iter()
            .map(|tag| ModelInfo {
                id: tag.name.clone(),
                name: tag.name,
                engine: "ollama".to_owned(),
                source: self.endpoint.clone(),
                size: tag.size,
                created_at_millis: 0,
            })
            .collect())
    }
}

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

    #[tokio::test]
    async fn streaming_errs_when_the_body_ends_without_a_done_line() {
        use tokio_stream::StreamExt;

        // No `done: true` line: simulates a killed model process, a proxy
        // reset, or a connection that closes mid-stream.
        let body = concat!(
            "{\"response\":\"Hel\",\"done\":false}\n",
            "{\"response\":\"lo\",\"done\":false}\n"
        );
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(move || async move { body }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = OllamaEngine::new(&config_for(&endpoint)).expect("build the engine");

        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "hi".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("the stub responds successfully");

        let mut deltas = Vec::new();
        let mut error = None;
        while let Some(event) = stream.next().await {
            match event {
                Ok(CompletionEvent::Delta(text)) => deltas.push(text),
                Ok(CompletionEvent::Done(_)) => {
                    panic!("a truncated stream must never send Done")
                }
                Err(received) => {
                    error = Some(received);
                    break;
                }
            }
        }

        assert_eq!(deltas, vec!["Hel".to_owned(), "lo".to_owned()]);
        assert!(
            error.is_some(),
            "truncation before a done line must surface as Err, not a silent end"
        );
        assert!(
            stream.next().await.is_none(),
            "nothing follows the terminal Err"
        );
    }

    #[tokio::test]
    async fn streaming_reassembles_a_json_line_split_across_chunk_boundaries() {
        use tokio_stream::StreamExt;

        // Two body frames whose split falls *inside* the first JSON object's
        // `response` field, plus a second object in the same frame as the
        // tail of the first. Proves the buffer — not `bytes_stream()`'s
        // chunking — determines line boundaries: a `chunk.split('\n')`
        // implementation would misparse `"{\"response\":\"Hel` as its own
        // line and fail this test.
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(|| async {
                let frames = vec![
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                        b"{\"response\":\"Hel",
                    )),
                    Ok(axum::body::Bytes::from_static(
                        b"lo\",\"done\":false}\n{\"response\":\"\",\"done\":true,\"prompt_eval_count\":1,\"eval_count\":2}\n",
                    )),
                ];
                axum::body::Body::from_stream(tokio_stream::iter(frames))
            }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = OllamaEngine::new(&config_for(&endpoint)).expect("build the engine");

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
            match event.expect("the reassembled lines are well-formed") {
                CompletionEvent::Delta(text) => deltas.push(text),
                CompletionEvent::Done(done) => summary = Some(done),
            }
        }

        assert_eq!(deltas.concat(), "Hello");
        assert_eq!(
            summary.expect("the stream terminates with Done").usage,
            Some(TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 2
            })
        );
    }

    #[tokio::test]
    async fn streaming_errs_on_a_line_that_never_terminates_within_the_byte_cap() {
        use tokio_stream::StreamExt;

        // A hostile or broken upstream that never sends a newline must not be
        // allowed to inflate the line buffer without limit. Serve 2 MiB of
        // `x` with no `\n` anywhere, split across several chunks so the
        // cap must be enforced incrementally rather than on one giant read.
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(|| async {
                let chunk = axum::body::Bytes::from(vec![b'x'; 256 * 1024]);
                let frames: Vec<Result<axum::body::Bytes, std::io::Error>> =
                    std::iter::repeat_with(|| Ok(chunk.clone()))
                        .take(8)
                        .collect();
                axum::body::Body::from_stream(tokio_stream::iter(frames))
            }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = OllamaEngine::new(&config_for(&endpoint)).expect("build the engine");

        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "hi".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("the stub responds successfully");

        let event = stream
            .next()
            .await
            .expect("the stream yields the terminal Err rather than ending silently");
        let error = match event {
            Ok(CompletionEvent::Delta(_)) => {
                panic!("a newline-free body carries no complete NDJSON line")
            }
            Ok(CompletionEvent::Done(_)) => {
                panic!("a newline-free body must never reach a done line")
            }
            Err(received) => received,
        };

        let message = error.to_string();
        assert!(
            message.contains("1048576"),
            "the error should name the byte limit that was exceeded, got: {message}"
        );
        assert!(
            stream.next().await.is_none(),
            "nothing follows the terminal Err"
        );
    }

    #[tokio::test]
    async fn streaming_survives_past_the_whole_request_timeout_as_long_as_tokens_keep_arriving() {
        use tokio_stream::StreamExt;

        // Each gap between sends is well under the configured timeout, but
        // the *total* wall time across the whole response exceeds it. A
        // client built with `.timeout(request_timeout_secs)` (a deadline on
        // the entire request) would be cut dead partway through; a client
        // built with `.read_timeout(request_timeout_secs)` bounds only the
        // silence between reads and lets a still-healthy stream keep going.
        let router = axum::Router::new().route(
            "/api/generate",
            axum::routing::post(|| async {
                let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<axum::body::Bytes>>(4);
                tokio::spawn(async move {
                    let lines = [
                        "{\"response\":\"H\",\"done\":false}\n",
                        "{\"response\":\"e\",\"done\":false}\n",
                        "{\"response\":\"l\",\"done\":false}\n",
                        "{\"response\":\"l\",\"done\":false}\n",
                        "{\"response\":\"o\",\"done\":false}\n",
                        "{\"response\":\"!\",\"done\":false}\n",
                        "{\"response\":\"\",\"done\":true,\"prompt_eval_count\":1,\"eval_count\":2}\n",
                    ];
                    for line in lines {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        if tx
                            .send(Ok(axum::body::Bytes::from_static(line.as_bytes())))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
                axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
            }),
        );
        let endpoint = spawn_stub(router).await;
        let mut config = config_for(&endpoint);
        // Total wall time (7 * 250ms = 1.75s) comfortably exceeds this,
        // while every individual gap (250ms) stays a 4x margin under it —
        // wide enough to survive scheduling jitter under full-suite load.
        config.request_timeout_secs = 1;
        let engine = OllamaEngine::new(&config).expect("build the engine");

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
            match event.expect(
                "a stream with steady token arrivals must not be cut by the whole-request timeout",
            ) {
                CompletionEvent::Delta(text) => deltas.push(text),
                CompletionEvent::Done(done) => summary = Some(done),
            }
        }

        assert_eq!(deltas.concat(), "Hello!");
        assert!(
            summary.is_some(),
            "the stream must reach Done despite exceeding request_timeout_secs in total"
        );
    }
}
