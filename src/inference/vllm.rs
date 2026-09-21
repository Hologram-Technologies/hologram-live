//! vLLM engine over its OpenAI-compatible completions API.

use super::{
    Completion, CompletionEvent, CompletionRequest, CompletionStream, CompletionSummary,
    InferenceEngine, StreamKind, TokenUsage,
};
use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::ModelInfo;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;

pub struct VllmEngine {
    endpoint: String,
    model: String,
    token: Option<String>,
    client: reqwest::Client,
    stream_client: reqwest::Client,
}

impl VllmEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        crate::util::install_crypto_provider();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| LiveError::Transport(format!("build vLLM client: {error}")))?;
        let stream_client = reqwest::Client::builder()
            .read_timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| {
                LiveError::Transport(format!("build vLLM streaming client: {error}"))
            })?;
        Ok(Self {
            endpoint: config.vllm_endpoint.trim_end_matches('/').to_owned(),
            model: config.default_model.clone(),
            token: std::env::var(&config.vllm_token_env).ok(),
            client,
            stream_client,
        })
    }

    fn request_builder(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        let request = client.request(method, format!("{}{path}", self.endpoint));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    async fn send_completion(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<reqwest::Response> {
        if self.model.trim().is_empty() {
            return Err(LiveError::Capability(
                "the vllm engine requires inference.default_model to name a served model"
                    .to_owned(),
            ));
        }
        let body = VllmCompletionRequest {
            model: &self.model,
            prompt: &request.prompt,
            stream,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            seed: request.seed,
            stream_options: stream.then_some(StreamOptions {
                include_usage: true,
            }),
        };
        let client = if stream {
            &self.stream_client
        } else {
            &self.client
        };
        let response = self
            .request_builder(client, reqwest::Method::POST, "/v1/completions")
            .json(&body)
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("vLLM completion: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LiveError::Transport(format!(
                "vLLM completion failed ({status}): {}",
                super::stderr_tail(body.trim())
            )));
        }
        Ok(response)
    }
}

#[derive(Debug, Serialize)]
struct VllmCompletionRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Deserialize)]
struct VllmCompletionResponse {
    #[serde(default)]
    model: String,
    #[serde(default)]
    choices: Vec<VllmChoice>,
    usage: Option<VllmUsage>,
}

#[derive(Debug, Deserialize)]
struct VllmChoice {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct VllmStreamChunk {
    #[serde(default)]
    model: String,
    #[serde(default)]
    choices: Vec<VllmChoice>,
    usage: Option<VllmUsage>,
    error: Option<VllmError>,
}

#[derive(Debug, Deserialize)]
struct VllmUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

impl VllmUsage {
    const fn into_usage(self) -> Option<TokenUsage> {
        TokenUsage::from_counts(self.prompt_tokens, self.completion_tokens)
    }
}

#[derive(Debug, Deserialize)]
struct VllmError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct VllmModelsResponse {
    #[serde(default)]
    data: Vec<VllmModel>,
}

#[derive(Debug, Deserialize)]
struct VllmModel {
    id: String,
    #[serde(default)]
    created: u64,
}

#[tonic::async_trait]
impl InferenceEngine for VllmEngine {
    fn name(&self) -> &'static str {
        "vllm"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let started = Instant::now();
        let response = self.send_completion(&request, false).await?;
        let parsed: VllmCompletionResponse = response
            .json()
            .await
            .map_err(|error| LiveError::Protocol(format!("parse vLLM response: {error}")))?;
        let text = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LiveError::Protocol("vLLM response has no choices".to_owned()))?
            .text;
        Ok(Completion {
            text,
            model: if parsed.model.is_empty() {
                self.model.clone()
            } else {
                parsed.model
            },
            tokens_per_second: None,
            elapsed_millis: super::elapsed_millis(started),
            usage: parsed.usage.and_then(VllmUsage::into_usage),
        })
    }

    fn stream_kind(&self) -> StreamKind {
        StreamKind::Native
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<CompletionStream> {
        use tokio_stream::StreamExt;

        let response = self.send_completion(&request, true).await?;
        let started = Instant::now();
        let fallback_model = self.model.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            let mut body = response.bytes_stream();
            let mut buffered = Vec::new();
            let mut model = fallback_model;
            let mut usage = None;
            while let Some(chunk) = body.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        let _ = sender
                            .send(Err(LiveError::Transport(format!(
                                "vLLM stream failed: {error}"
                            ))))
                            .await;
                        return;
                    }
                };
                buffered.extend_from_slice(&chunk);
                if buffered.len() > MAX_STREAM_LINE_BYTES && !buffered.contains(&b'\n') {
                    let _ = sender
                        .send(Err(LiveError::Protocol(format!(
                            "vLLM stream line exceeded {MAX_STREAM_LINE_BYTES} bytes"
                        ))))
                        .await;
                    return;
                }
                while let Some(index) = buffered.iter().position(|byte| *byte == b'\n') {
                    if index > MAX_STREAM_LINE_BYTES {
                        let _ = sender
                            .send(Err(LiveError::Protocol(format!(
                                "vLLM stream line exceeded {MAX_STREAM_LINE_BYTES} bytes"
                            ))))
                            .await;
                        return;
                    }
                    let line: Vec<u8> = buffered.drain(..=index).collect();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim();
                    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                        continue;
                    };
                    if data == "[DONE]" {
                        let _ = sender
                            .send(Ok(CompletionEvent::Done(CompletionSummary {
                                model,
                                usage,
                                tokens_per_second: None,
                                elapsed_millis: super::elapsed_millis(started),
                            })))
                            .await;
                        return;
                    }
                    let parsed: VllmStreamChunk = match serde_json::from_str(data) {
                        Ok(parsed) => parsed,
                        Err(error) => {
                            let _ = sender
                                .send(Err(LiveError::Protocol(format!(
                                    "parse vLLM stream event: {error}"
                                ))))
                                .await;
                            return;
                        }
                    };
                    if let Some(error) = parsed.error {
                        let _ = sender
                            .send(Err(LiveError::Transport(format!(
                                "vLLM stream failed: {}",
                                error.message
                            ))))
                            .await;
                        return;
                    }
                    if !parsed.model.is_empty() {
                        model = parsed.model;
                    }
                    if let Some(measured) = parsed.usage.and_then(VllmUsage::into_usage) {
                        usage = Some(measured);
                    }
                    for choice in parsed.choices {
                        if !choice.text.is_empty()
                            && sender
                                .send(Ok(CompletionEvent::Delta(choice.text)))
                                .await
                                .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            let _ = sender
                .send(Err(LiveError::Protocol(
                    "vLLM stream ended before [DONE]".to_owned(),
                )))
                .await;
        });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .request_builder(&self.client, reqwest::Method::GET, "/v1/models")
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("vLLM models: {error}")))?;
        if !response.status().is_success() {
            return Err(LiveError::Transport(format!(
                "vLLM models failed ({})",
                response.status()
            )));
        }
        let parsed: VllmModelsResponse = response
            .json()
            .await
            .map_err(|error| LiveError::Protocol(format!("parse vLLM models: {error}")))?;
        Ok(parsed
            .data
            .into_iter()
            .map(|model| ModelInfo {
                id: model.id.clone(),
                name: model.id,
                engine: "vllm".to_owned(),
                source: self.endpoint.clone(),
                size: 0,
                created_at_millis: model.created.saturating_mul(1000),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Json;
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use serde_json::{json, Value};
    use tokio_stream::StreamExt;

    async fn spawn_stub(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub");
        let address = listener.local_addr().expect("stub address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{address}")
    }

    fn config_for(endpoint: &str) -> InferenceConfig {
        InferenceConfig {
            engine: "vllm".to_owned(),
            default_model: "org/model".to_owned(),
            vllm_endpoint: endpoint.to_owned(),
            ..InferenceConfig::default()
        }
    }

    #[tokio::test]
    async fn completion_maps_request_response_and_usage() {
        let router = axum::Router::new().route(
            "/v1/completions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["model"], "org/model");
                assert_eq!(body["prompt"], "hello");
                assert_eq!(body["max_tokens"], 7);
                Json(json!({
                    "model": "org/model",
                    "choices": [{"text": " world"}],
                    "usage": {"prompt_tokens": 2, "completion_tokens": 1}
                }))
            }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = VllmEngine::new(&config_for(&endpoint)).expect("engine");
        let completion = engine
            .complete(CompletionRequest {
                prompt: "hello".to_owned(),
                max_tokens: Some(7),
                ..CompletionRequest::default()
            })
            .await
            .expect("completion");
        assert_eq!(completion.text, " world");
        assert_eq!(
            completion.usage,
            Some(TokenUsage {
                prompt_tokens: 2,
                completion_tokens: 1
            })
        );
    }

    #[tokio::test]
    async fn streaming_is_native_and_ends_with_reported_usage() {
        let body = concat!(
            "data: {\"model\":\"org/model\",\"choices\":[{\"text\":\"Hel\"}]}\n\n",
            "data: {\"model\":\"org/model\",\"choices\":[{\"text\":\"lo\"}]}\n\n",
            "data: {\"model\":\"org/model\",\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        let router = axum::Router::new().route(
            "/v1/completions",
            post(move |Json(request): Json<Value>| async move {
                assert_eq!(request["stream"], true);
                assert_eq!(request["stream_options"]["include_usage"], true);
                body
            }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = VllmEngine::new(&config_for(&endpoint)).expect("engine");
        assert_eq!(engine.stream_kind(), StreamKind::Native);
        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "hi".to_owned(),
                ..CompletionRequest::default()
            })
            .await
            .expect("stream");
        let mut text = String::new();
        let mut summary = None;
        while let Some(event) = stream.next().await {
            match event.expect("event") {
                CompletionEvent::Delta(delta) => text.push_str(&delta),
                CompletionEvent::Done(done) => summary = Some(done),
            }
        }
        assert_eq!(text, "Hello");
        assert_eq!(
            summary.expect("done").usage,
            Some(TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 2
            })
        );
    }

    #[tokio::test]
    async fn model_listing_uses_the_vllm_endpoint() {
        let router = axum::Router::new().route(
            "/v1/models",
            get(|| async { Json(json!({"data": [{"id": "org/model", "created": 42}]})) }),
        );
        let endpoint = spawn_stub(router).await;
        let engine = VllmEngine::new(&config_for(&endpoint)).expect("engine");
        let models = engine.list_models().await.expect("models");
        assert_eq!(models[0].id, "org/model");
        assert_eq!(models[0].engine, "vllm");
        assert_eq!(models[0].created_at_millis, 42_000);
    }

    #[tokio::test]
    async fn authentication_and_rejections_are_forwarded_safely() {
        let router = axum::Router::new().route(
            "/v1/completions",
            post(|headers: HeaderMap| async move {
                assert_eq!(
                    headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok()),
                    Some("Bearer test-key")
                );
                (axum::http::StatusCode::UNAUTHORIZED, "denied")
            }),
        );
        let endpoint = spawn_stub(router).await;
        let mut config = config_for(&endpoint);
        "PATH".clone_into(&mut config.vllm_token_env);
        let previous = std::env::var("PATH").expect("PATH");
        // PATH is always present but not suitable as the expected fixture,
        // so this test instead proves a configured token is attached using a
        // private constructor value without mutating process-wide environment.
        let mut engine = VllmEngine::new(&config).expect("engine");
        engine.token = Some("test-key".to_owned());
        let error = engine
            .complete(CompletionRequest::default())
            .await
            .expect_err("401");
        assert!(error.to_string().contains("401"), "{error}");
        assert!(
            !error.to_string().contains(&previous),
            "token leaked: {error}"
        );
    }
}
