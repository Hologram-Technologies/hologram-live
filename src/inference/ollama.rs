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

pub struct OllamaEngine {
    endpoint: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| LiveError::Transport(format!("build ollama client: {error}")))?;
        Ok(Self {
            endpoint: config.ollama_endpoint.trim_end_matches('/').to_owned(),
            model: config.default_model.clone(),
            client,
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
        let response = self
            .client
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
}
