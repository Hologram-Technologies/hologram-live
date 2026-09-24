//! In-process GGUF inference through Hugging Face Candle.
//!
//! The first adapter intentionally supports the quantized Llama-family model
//! implementation only. Candle does not provide a universal architecture
//! dispatcher: adding another family is an explicit code change, not a claim
//! that every GGUF accepted by llama.cpp works here.

use super::{
    Completion, CompletionEvent, CompletionRequest, CompletionStream, CompletionSummary,
    InferenceEngine, StreamKind, TokenUsage,
};
use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::ModelInfo;
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_llama::{self, ModelWeights};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokenizers::Tokenizer;
use tokio_stream::StreamExt;

const DEFAULT_MAX_TOKENS: u32 = 256;

#[derive(Debug)]
pub struct CandleEngine {
    model: Arc<Mutex<ModelWeights>>,
    tokenizer: Tokenizer,
    device: Device,
    model_path: PathBuf,
    model_id: String,
    model_label: String,
    n_ctx: u32,
}

struct PreparedRequest {
    tokens: Vec<u32>,
    max_tokens: u32,
    temperature: Option<f32>,
    seed: u64,
}

impl CandleEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        if config.model_architecture != "llama" {
            return Err(LiveError::Config(
                "candle currently requires inference.model_architecture = \"llama\"".to_owned(),
            ));
        }
        let model_path = regular_file(&config.model_path, "inference.model_path")?;
        let tokenizer_path = regular_file(&config.tokenizer_path, "inference.tokenizer_path")?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            LiveError::Capability(format!(
                "load Candle tokenizer {}: {error}",
                tokenizer_path.display()
            ))
        })?;
        let device = device()?;
        let mut file =
            std::fs::File::open(&model_path).map_err(|error| LiveError::io(&model_path, error))?;
        let content = gguf_file::Content::read(&mut file).map_err(|error| {
            LiveError::Capability(format!(
                "read Candle GGUF {}: {error}",
                model_path.display()
            ))
        })?;
        let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(|error| {
            LiveError::Capability(format!(
                "load Candle Llama model {}: {error}",
                model_path.display()
            ))
        })?;
        let model_label = model_path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "gguf".to_owned());
        let model_id = if config.default_model.trim().is_empty() {
            model_path.display().to_string()
        } else {
            config.default_model.clone()
        };
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            tokenizer,
            device,
            model_path,
            model_id,
            model_label,
            n_ctx: config.n_ctx,
        })
    }

    fn prepare(&self, request: CompletionRequest) -> Result<PreparedRequest> {
        let encoding = self
            .tokenizer
            .encode(request.prompt, true)
            .map_err(|error| LiveError::Protocol(format!("tokenize Candle prompt: {error}")))?;
        let tokens = encoding.get_ids().to_vec();
        if tokens.is_empty() {
            return Err(LiveError::Protocol(
                "Candle tokenizer returned an empty prompt".to_owned(),
            ));
        }
        let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let required = tokens.len().saturating_add(max_tokens as usize);
        let limit = usize::try_from(self.n_ctx)
            .unwrap_or(usize::MAX)
            .min(quantized_llama::MAX_SEQ_LEN);
        if required > limit {
            return Err(LiveError::Capability(format!(
                "Candle request needs {required} context tokens but the configured/model limit is {limit}"
            )));
        }
        Ok(PreparedRequest {
            tokens,
            max_tokens,
            temperature: request.temperature,
            seed: request.seed.unwrap_or(299_792_458),
        })
    }

    async fn start_decode(&self, request: CompletionRequest) -> Result<CompletionStream> {
        let prepared = self.prepare(request)?;
        let model = self.model.clone();
        let tokenizer = self.tokenizer.clone();
        let device = self.device.clone();
        let model_id = self.model_id.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("hologram-candle-decode".to_owned())
            .spawn(move || {
                decode_into(
                    &model,
                    tokenizer,
                    &device,
                    &model_id,
                    prepared,
                    &sender,
                    ready_sender,
                );
            })
            .map_err(|error| LiveError::Capability(format!("start Candle worker: {error}")))?;
        ready_receiver.await.map_err(|_| {
            LiveError::Capability("Candle worker stopped during initialization".to_owned())
        })??;
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }
}

fn decode_into(
    model: &Mutex<ModelWeights>,
    tokenizer: Tokenizer,
    device: &Device,
    model_id: &str,
    prepared: PreparedRequest,
    sender: &tokio::sync::mpsc::Sender<Result<CompletionEvent>>,
    ready: tokio::sync::oneshot::Sender<Result<()>>,
) {
    let mut model = match model.lock() {
        Ok(model) => model,
        Err(_) => {
            let _ = ready.send(Err(LiveError::Capability(
                "Candle model lock is poisoned".to_owned(),
            )));
            return;
        }
    };
    model.clear_kv_cache();
    if prepared.max_tokens == 0 {
        if ready.send(Ok(())).is_ok() {
            let _ =
                sender.blocking_send(Ok(done(model_id, prepared.tokens.len(), 0, Instant::now())));
        }
        return;
    }
    let input = match Tensor::new(prepared.tokens.as_slice(), device).and_then(|v| v.unsqueeze(0)) {
        Ok(input) => input,
        Err(error) => {
            let _ = ready.send(Err(candle_error("build prompt tensor", error)));
            return;
        }
    };
    let logits = match model.forward(&input, 0).and_then(|v| v.squeeze(0)) {
        Ok(logits) => logits,
        Err(error) => {
            let _ = ready.send(Err(candle_error("decode prompt", error)));
            return;
        }
    };
    let sampling = match prepared
        .temperature
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        Some(temperature) => Sampling::All {
            temperature: f64::from(temperature),
        },
        None => Sampling::ArgMax,
    };
    let mut logits_processor = LogitsProcessor::from_sampling(prepared.seed, sampling);
    let mut next_token = match logits_processor.sample(&logits) {
        Ok(token) => token,
        Err(error) => {
            let _ = ready.send(Err(candle_error("sample first token", error)));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    let started = Instant::now();
    let eos = eos_tokens(&tokenizer);
    let mut output = TokenOutputStream::new(tokenizer);
    let mut produced = 0_u32;
    while produced < prepared.max_tokens {
        produced = produced.saturating_add(1);
        if eos.contains(&next_token) {
            break;
        }
        match output.next_token(next_token) {
            Ok(Some(text)) if !text.is_empty() => {
                if sender
                    .blocking_send(Ok(CompletionEvent::Delta(text)))
                    .is_err()
                {
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                send_error(sender, error);
                return;
            }
        }
        if produced == prepared.max_tokens {
            break;
        }
        let input = match Tensor::new(&[next_token], device).and_then(|v| v.unsqueeze(0)) {
            Ok(input) => input,
            Err(error) => {
                send_error(sender, candle_error("build token tensor", error));
                return;
            }
        };
        let index = prepared
            .tokens
            .len()
            .saturating_add(produced as usize)
            .saturating_sub(1);
        let logits = match model.forward(&input, index).and_then(|v| v.squeeze(0)) {
            Ok(logits) => logits,
            Err(error) => {
                send_error(sender, candle_error("decode token", error));
                return;
            }
        };
        next_token = match logits_processor.sample(&logits) {
            Ok(token) => token,
            Err(error) => {
                send_error(sender, candle_error("sample token", error));
                return;
            }
        };
    }
    match output.decode_rest() {
        Ok(Some(rest)) if !rest.is_empty() => {
            if sender
                .blocking_send(Ok(CompletionEvent::Delta(rest)))
                .is_err()
            {
                return;
            }
        }
        Ok(_) => {}
        Err(error) => {
            send_error(sender, error);
            return;
        }
    }
    let _ = sender.blocking_send(Ok(done(model_id, prepared.tokens.len(), produced, started)));
}

fn done(
    model: &str,
    prompt_tokens: usize,
    completion_tokens: u32,
    started: Instant,
) -> CompletionEvent {
    let elapsed_millis = super::elapsed_millis(started);
    #[allow(clippy::cast_precision_loss)]
    let tokens_per_second = (elapsed_millis > 0)
        .then(|| f64::from(completion_tokens) / (elapsed_millis as f64 / 1000.0));
    CompletionEvent::Done(CompletionSummary {
        model: model.to_owned(),
        usage: Some(TokenUsage {
            prompt_tokens: prompt_tokens.try_into().unwrap_or(u64::MAX),
            completion_tokens: u64::from(completion_tokens),
        }),
        tokens_per_second,
        elapsed_millis,
    })
}

fn device() -> Result<Device> {
    #[cfg(feature = "candle-cuda")]
    {
        return Device::new_cuda(0)
            .map_err(|error| LiveError::Capability(format!("initialize Candle CUDA: {error}")));
    }
    #[cfg(all(not(feature = "candle-cuda"), feature = "candle-metal"))]
    {
        return Device::new_metal(0)
            .map_err(|error| LiveError::Capability(format!("initialize Candle Metal: {error}")));
    }
    #[cfg(not(any(feature = "candle-cuda", feature = "candle-metal")))]
    {
        Ok(Device::Cpu)
    }
}

fn regular_file(value: &str, field: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !Path::new(&path).is_file() {
        return Err(LiveError::Capability(format!(
            "{field} {} is not a file",
            path.display()
        )));
    }
    Ok(path)
}

fn eos_tokens(tokenizer: &Tokenizer) -> HashSet<u32> {
    let vocabulary = tokenizer.get_vocab(true);
    ["</s>", "<|end_of_text|>", "<|eot_id|>", "<|endoftext|>"]
        .into_iter()
        .filter_map(|token| vocabulary.get(token).copied())
        .collect()
}

fn candle_error(action: &str, error: candle_core::Error) -> LiveError {
    LiveError::Capability(format!("Candle {action}: {error}"))
}

fn send_error(sender: &tokio::sync::mpsc::Sender<Result<CompletionEvent>>, error: LiveError) {
    let _ = sender.blocking_send(Err(error));
}

struct TokenOutputStream {
    tokenizer: Tokenizer,
    tokens: Vec<u32>,
    previous: usize,
    current: usize,
}

impl TokenOutputStream {
    fn new(tokenizer: Tokenizer) -> Self {
        Self {
            tokenizer,
            tokens: Vec::new(),
            previous: 0,
            current: 0,
        }
    }

    fn decode(&self, tokens: &[u32]) -> Result<String> {
        self.tokenizer
            .decode(tokens, true)
            .map_err(|error| LiveError::Protocol(format!("decode Candle tokens: {error}")))
    }

    fn next_token(&mut self, token: u32) -> Result<Option<String>> {
        let old = if self.tokens.is_empty() {
            String::new()
        } else {
            self.decode(&self.tokens[self.previous..self.current])?
        };
        self.tokens.push(token);
        let text = self.decode(&self.tokens[self.previous..])?;
        if text.len() > old.len()
            && text.chars().last().is_some_and(char::is_alphanumeric)
            && text.is_char_boundary(old.len())
        {
            let delta = text[old.len()..].to_owned();
            self.previous = self.current;
            self.current = self.tokens.len();
            Ok(Some(delta))
        } else {
            Ok(None)
        }
    }

    fn decode_rest(&self) -> Result<Option<String>> {
        let old = if self.tokens.is_empty() {
            String::new()
        } else {
            self.decode(&self.tokens[self.previous..self.current])?
        };
        let text = self.decode(&self.tokens[self.previous..])?;
        Ok((text.len() > old.len() && text.is_char_boundary(old.len()))
            .then(|| text[old.len()..].to_owned()))
    }
}

#[tonic::async_trait]
impl InferenceEngine for CandleEngine {
    fn name(&self) -> &'static str {
        "candle"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let mut stream = self.start_decode(request).await?;
        let mut text = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                CompletionEvent::Delta(delta) => text.push_str(&delta),
                CompletionEvent::Done(summary) => {
                    return Ok(Completion {
                        text,
                        model: summary.model,
                        tokens_per_second: summary.tokens_per_second,
                        elapsed_millis: summary.elapsed_millis,
                        usage: summary.usage,
                    });
                }
            }
        }
        Err(LiveError::Protocol(
            "Candle stream ended without a completion summary".to_owned(),
        ))
    }

    fn stream_kind(&self) -> StreamKind {
        StreamKind::Native
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<CompletionStream> {
        self.start_decode(request).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let size = std::fs::metadata(&self.model_path)
            .map_err(|error| LiveError::io(&self.model_path, error))?
            .len();
        Ok(vec![ModelInfo {
            id: self.model_id.clone(),
            name: self.model_label.clone(),
            engine: "candle".to_owned(),
            source: self.model_path.display().to_string(),
            size,
            created_at_millis: 0,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_is_a_typed_error() {
        let config = InferenceConfig {
            engine: "candle".to_owned(),
            model_path: "/nonexistent/model.gguf".to_owned(),
            tokenizer_path: "/nonexistent/tokenizer.json".to_owned(),
            model_architecture: "llama".to_owned(),
            ..InferenceConfig::default()
        };
        let error = CandleEngine::new(&config).expect_err("missing model");
        assert!(error.to_string().contains("nonexistent"), "{error}");
    }
}
