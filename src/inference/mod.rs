//! Inference engine boundary.
//!
//! The daemon never executes model weights in-process. Chat (and later the
//! OpenAI/Ollama-compatible modules) call an [`InferenceEngine`]; engines
//! either echo locally or delegate to an external engine — the `weightc`
//! one-shot CLI over `.wcpu` artifact directories, or an Ollama-compatible
//! HTTP endpoint.

mod echo;
mod ollama;
mod weightc;

pub use echo::EchoEngine;
pub use ollama::OllamaEngine;
pub use weightc::WeightcEngine;

use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::{ModelCatalog, ModelInfo};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub seed: Option<u64>,
    /// Resident-session routing key. Engines without session support ignore
    /// it and run one-shot.
    pub session_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub tokens_per_second: Option<f64>,
    pub elapsed_millis: u64,
}

#[tonic::async_trait]
pub trait InferenceEngine: Send + Sync {
    fn name(&self) -> &'static str;
    /// Whether the engine holds per-session context itself. When true, chat
    /// sends the raw new user turn plus a `session_key` instead of rendering
    /// the transcript into every prompt.
    fn supports_sessions(&self) -> bool {
        false
    }
    async fn complete(&self, request: CompletionRequest) -> Result<Completion>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    /// Release engine-owned resources such as resident session children.
    /// Called during daemon shutdown alongside plugin teardown.
    async fn shutdown(&self) {}
}

pub fn engine_from_config(
    config: &InferenceConfig,
    catalog: Arc<ModelCatalog>,
    mailbox_capacity: usize,
) -> Result<Arc<dyn InferenceEngine>> {
    match config.engine.as_str() {
        "echo" => Ok(Arc::new(EchoEngine)),
        "weightc" => Ok(Arc::new(WeightcEngine::new(
            config,
            catalog,
            mailbox_capacity,
        ))),
        "ollama" => Ok(Arc::new(OllamaEngine::new(config)?)),
        other => Err(LiveError::Config(format!(
            "unsupported inference.engine {other:?}; expected echo, weightc, or ollama"
        ))),
    }
}

pub(crate) fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn stderr_tail(text: &str) -> &str {
    const LIMIT: usize = 512;
    if text.len() <= LIMIT {
        return text;
    }
    let mut start = text.len() - LIMIT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_engine_is_a_config_error() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let store = Arc::new(
            crate::store::ObjectStore::open(temporary.path().join("store")).expect("store"),
        );
        let catalog =
            Arc::new(ModelCatalog::open(store, temporary.path().join("models")).expect("catalog"));
        let config = InferenceConfig {
            engine: "surprise".to_owned(),
            ..InferenceConfig::default()
        };
        let error = engine_from_config(&config, catalog, 8)
            .err()
            .expect("must fail");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
    }
}
