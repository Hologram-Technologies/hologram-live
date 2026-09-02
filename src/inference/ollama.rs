//! Ollama-compatible HTTP engine (`POST {endpoint}/api/generate`).

use super::{Completion, CompletionRequest, InferenceEngine};
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
    eval_count: Option<u64>,
    eval_duration: Option<u64>,
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
        if self.model.trim().is_empty() {
            return Err(LiveError::Capability(
                "the ollama engine requires inference.default_model to name a model tag".to_owned(),
            ));
        }
        let started = Instant::now();
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
            stream: false,
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
        })
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
