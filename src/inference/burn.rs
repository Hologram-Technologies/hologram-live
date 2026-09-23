//! In-process Llama inference through Tracel's Burn-LM implementation.
//!
//! Burn checkpoints use Burn's named-MPK record format and are not compatible
//! with GGUF. The adapter supports the four Llama 3 variants exposed by
//! `burn-lm-llama`; selecting the architecture is therefore mandatory.

use super::{Completion, CompletionRequest, InferenceEngine, TokenUsage};
use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::ModelInfo;
use burn::backend::NdArray;
use burn_lm_inference::{GeneratedItemEmitter, TextGenerationListener};
use burn_lm_llama::generation::{Sampler, TopP};
use burn_lm_llama::tokenizer::{Tiktoken, Tokenizer as BurnTokenizer};
use burn_lm_llama::{Llama, LlamaConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const DEFAULT_MAX_TOKENS: u32 = 256;
type CpuBackend = NdArray<f32, i32>;
type BurnLlama = Llama<CpuBackend, Tiktoken>;

#[derive(Debug)]
pub struct BurnEngine {
    model: Arc<Mutex<BurnLlama>>,
    model_path: PathBuf,
    model_id: String,
    model_label: String,
    n_ctx: u32,
}

impl BurnEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let model_path = PathBuf::from(&config.model_path);
        if !model_path.is_file() {
            return Err(LiveError::Capability(format!(
                "inference.model_path {} is not a file",
                model_path.display()
            )));
        }
        let tokenizer_path = PathBuf::from(&config.tokenizer_path);
        if !tokenizer_path.is_file() {
            return Err(LiveError::Capability(format!(
                "inference.tokenizer_path {} is not a file",
                tokenizer_path.display()
            )));
        }
        let checkpoint = model_path.to_string_lossy();
        let tokenizer = tokenizer_path.to_string_lossy();
        let max_seq_len = usize::try_from(config.n_ctx).map_err(|error| {
            LiveError::Config(format!("convert inference.n_ctx for Burn: {error}"))
        })?;
        let device = Default::default();
        let model = match config.model_architecture.as_str() {
            "llama3.2-1b" => LlamaConfig::load_llama3_2_1b::<CpuBackend>(
                &checkpoint,
                &tokenizer,
                max_seq_len,
                &device,
            ),
            "llama3.2-3b" => LlamaConfig::load_llama3_2_3b::<CpuBackend>(
                &checkpoint,
                &tokenizer,
                max_seq_len,
                &device,
            ),
            "llama3.1-8b" => LlamaConfig::load_llama3_1_8b::<CpuBackend>(
                &checkpoint,
                &tokenizer,
                max_seq_len,
                &device,
            ),
            "llama3-8b" => LlamaConfig::load_llama3_8b::<CpuBackend>(
                &checkpoint,
                &tokenizer,
                max_seq_len,
                &device,
            ),
            other => {
                return Err(LiveError::Config(format!(
                    "unsupported Burn model architecture {other:?}"
                )))
            }
        }
        .map_err(|error| {
            LiveError::Capability(format!(
                "load Burn checkpoint {}: {error}",
                model_path.display()
            ))
        })?;
        let model_label = model_path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "burn-llama".to_owned());
        let model_id = if config.default_model.trim().is_empty() {
            model_path.display().to_string()
        } else {
            config.default_model.clone()
        };
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            model_path,
            model_id,
            model_label,
            n_ctx: config.n_ctx,
        })
    }
}

#[tonic::async_trait]
impl InferenceEngine for BurnEngine {
    fn name(&self) -> &'static str {
        "burn"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let model = self.model.clone();
        let model_id = self.model_id.clone();
        let n_ctx = self.n_ctx;
        tokio::task::spawn_blocking(move || complete_blocking(&model, &model_id, n_ctx, request))
            .await
            .map_err(|error| LiveError::Capability(format!("join Burn worker: {error}")))?
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let size = std::fs::metadata(&self.model_path)
            .map_err(|error| LiveError::io(&self.model_path, error))?
            .len();
        Ok(vec![ModelInfo {
            id: self.model_id.clone(),
            name: self.model_label.clone(),
            engine: "burn".to_owned(),
            source: self.model_path.display().to_string(),
            size,
            created_at_millis: 0,
        }])
    }
}

fn complete_blocking(
    model: &Mutex<BurnLlama>,
    model_id: &str,
    n_ctx: u32,
    request: CompletionRequest,
) -> Result<Completion> {
    let mut model = model
        .lock()
        .map_err(|_| LiveError::Capability("Burn model lock is poisoned".to_owned()))?;
    let prompt_tokens = model.tokenizer.encode(&request.prompt, false, false).len();
    if prompt_tokens == 0 {
        return Err(LiveError::Protocol(
            "Burn tokenizer returned an empty prompt".to_owned(),
        ));
    }
    let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    let required = prompt_tokens.saturating_add(max_tokens as usize);
    if required > n_ctx as usize {
        return Err(LiveError::Capability(format!(
            "Burn request needs {required} context tokens but inference.n_ctx is {n_ctx}"
        )));
    }
    if max_tokens == 0 {
        return Ok(Completion {
            text: String::new(),
            model: model_id.to_owned(),
            tokens_per_second: None,
            elapsed_millis: 0,
            usage: Some(TokenUsage {
                prompt_tokens: prompt_tokens.try_into().unwrap_or(u64::MAX),
                completion_tokens: 0,
            }),
        });
    }

    model.reset();
    let (emitter, handle) = GeneratedItemEmitter::init(TextGenerationListener::default());
    let temperature = request
        .temperature
        .filter(|value| value.is_finite() && *value > 0.0)
        .map_or(0.0, f64::from);
    let mut sampler = if temperature > 0.0 {
        Sampler::TopP(TopP::new(0.9, request.seed.unwrap_or(299_792_458)))
    } else {
        Sampler::Argmax
    };
    let started = Instant::now();
    let generated = model.generate(
        &request.prompt,
        max_tokens as usize,
        temperature,
        &mut sampler,
        emitter,
    );
    let text = handle.join();
    let generated = generated
        .map_err(|error| LiveError::Capability(format!("Burn generation failed: {error:?}")))?;
    let elapsed_millis = super::elapsed_millis(started);
    #[allow(clippy::cast_precision_loss)]
    let tokens_per_second =
        (elapsed_millis > 0).then(|| generated.tokens as f64 / (elapsed_millis as f64 / 1000.0));
    Ok(Completion {
        text,
        model: model_id.to_owned(),
        tokens_per_second,
        elapsed_millis,
        usage: Some(TokenUsage {
            prompt_tokens: prompt_tokens.try_into().unwrap_or(u64::MAX),
            completion_tokens: generated.tokens.try_into().unwrap_or(u64::MAX),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_is_a_typed_error() {
        let config = InferenceConfig {
            engine: "burn".to_owned(),
            model_path: "/nonexistent/model.mpk".to_owned(),
            tokenizer_path: "/nonexistent/tokenizer.model".to_owned(),
            model_architecture: "llama3.2-1b".to_owned(),
            ..InferenceConfig::default()
        };
        let error = BurnEngine::new(&config).expect_err("missing model");
        assert!(error.to_string().contains("nonexistent"), "{error}");
    }
}
