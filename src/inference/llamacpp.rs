//! In-process GGUF inference through `llama-cpp-2`.
//!
//! This module is compiled only with the `llamacpp` feature. Model loading is
//! paid once at startup; each completion gets a dedicated OS thread and llama
//! context so blocking native decode never occupies Tokio's worker pool.

use super::{
    Completion, CompletionEvent, CompletionRequest, CompletionStream, CompletionSummary,
    InferenceEngine, StreamKind, TokenUsage,
};
use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::ModelInfo;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio_stream::StreamExt;

const DEFAULT_MAX_TOKENS: u32 = 256;

fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<std::result::Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| LiveError::Capability(format!("llama.cpp backend init failed: {error}")))
}

#[derive(Debug)]
pub struct LlamaCppEngine {
    model: Arc<LlamaModel>,
    model_path: PathBuf,
    model_id: String,
    model_label: String,
    n_ctx: u32,
    decode_slots: Arc<tokio::sync::Semaphore>,
}

struct PreparedRequest {
    tokens: Vec<llama_cpp_2::token::LlamaToken>,
    max_tokens: u32,
    temperature: Option<f32>,
    seed: Option<u64>,
}

impl LlamaCppEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let model_path = PathBuf::from(&config.model_path);
        if !model_path.is_file() {
            return Err(LiveError::Capability(format!(
                "inference.model_path {} is not a file",
                model_path.display()
            )));
        }
        let params = LlamaModelParams::default().with_n_gpu_layers(config.n_gpu_layers);
        let model =
            LlamaModel::load_from_file(backend()?, &model_path, &params).map_err(|error| {
                LiveError::Capability(format!("load GGUF model {}: {error}", model_path.display()))
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
            model: Arc::new(model),
            model_path,
            model_id,
            model_label,
            n_ctx: config.n_ctx,
            decode_slots: Arc::new(tokio::sync::Semaphore::new(
                config.llamacpp_max_concurrent_requests,
            )),
        })
    }

    fn prepare(&self, request: CompletionRequest) -> Result<PreparedRequest> {
        let tokens = self
            .model
            .str_to_token(&request.prompt, AddBos::Always)
            .map_err(|error| LiveError::Protocol(format!("tokenize llama.cpp prompt: {error}")))?;
        if tokens.is_empty() {
            return Err(LiveError::Protocol(
                "llama.cpp tokenizer returned an empty prompt".to_owned(),
            ));
        }
        let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let required = u64::try_from(tokens.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::from(max_tokens));
        if required > u64::from(self.n_ctx) {
            return Err(LiveError::Capability(format!(
                "llama.cpp request needs {required} context tokens but inference.n_ctx is {}",
                self.n_ctx
            )));
        }
        Ok(PreparedRequest {
            tokens,
            max_tokens,
            temperature: request.temperature,
            seed: request.seed,
        })
    }

    async fn start_decode(&self, request: CompletionRequest) -> Result<CompletionStream> {
        let prepared = self.prepare(request)?;
        let model = self.model.clone();
        let model_id = self.model_id.clone();
        let n_ctx = self.n_ctx;
        let permit = self
            .decode_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| {
                LiveError::Capability(format!("acquire llama.cpp decode slot: {error}"))
            })?;
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("hologram-llamacpp-decode".to_owned())
            .spawn(move || {
                let _permit = permit;
                decode_into(&model, &model_id, n_ctx, prepared, &sender, ready_sender);
            })
            .map_err(|error| LiveError::Capability(format!("start llama.cpp worker: {error}")))?;
        ready_receiver.await.map_err(|_| {
            LiveError::Capability("llama.cpp worker stopped during initialization".to_owned())
        })??;
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }
}

fn decode_into(
    model: &LlamaModel,
    model_id: &str,
    n_ctx: u32,
    prepared: PreparedRequest,
    sender: &tokio::sync::mpsc::Sender<Result<CompletionEvent>>,
    ready: tokio::sync::oneshot::Sender<Result<()>>,
) {
    let Some(n_ctx) = NonZeroU32::new(n_ctx) else {
        let _ = ready.send(Err(LiveError::Config(
            "inference.n_ctx must be greater than zero".to_owned(),
        )));
        return;
    };
    let mut context = match model.new_context(
        match backend() {
            Ok(backend) => backend,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        },
        LlamaContextParams::default().with_n_ctx(Some(n_ctx)),
    ) {
        Ok(context) => context,
        Err(error) => {
            let _ = ready.send(Err(LiveError::Capability(format!(
                "create llama.cpp context: {error}"
            ))));
            return;
        }
    };
    let mut batch = LlamaBatch::new(prepared.tokens.len().max(1), 1);
    let last = prepared.tokens.len().saturating_sub(1);
    for (position, token) in prepared.tokens.iter().copied().enumerate() {
        let is_last = position == last;
        let position = match i32::try_from(position) {
            Ok(position) => position,
            Err(error) => {
                let _ = ready.send(Err(LiveError::Capability(format!(
                    "llama.cpp prompt position overflow: {error}"
                ))));
                return;
            }
        };
        if let Err(error) = batch.add(token, position, &[0], is_last) {
            let _ = ready.send(Err(LiveError::Capability(format!(
                "build llama.cpp prompt batch: {error}"
            ))));
            return;
        }
    }
    if let Err(error) = context.decode(&mut batch) {
        let _ = ready.send(Err(LiveError::Capability(format!(
            "decode llama.cpp prompt: {error}"
        ))));
        return;
    }
    if ready.send(Ok(())).is_err() {
        return;
    }

    let started = Instant::now();
    let mut sampler = sampler(prepared.temperature, prepared.seed);
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut produced = 0_u32;
    while produced < prepared.max_tokens {
        let token = sampler.sample(&context, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = match model.token_to_piece(token, &mut decoder, true, None) {
            Ok(piece) => piece,
            Err(error) => {
                send_decode_error(sender, format!("decode llama.cpp token: {error}"));
                return;
            }
        };
        produced = produced.saturating_add(1);
        if !piece.is_empty()
            && sender
                .blocking_send(Ok(CompletionEvent::Delta(piece)))
                .is_err()
        {
            return;
        }
        if produced == prepared.max_tokens {
            break;
        }
        batch.clear();
        let position = prepared
            .tokens
            .len()
            .saturating_add(usize::try_from(produced.saturating_sub(1)).unwrap_or(usize::MAX));
        let position = match i32::try_from(position) {
            Ok(position) => position,
            Err(error) => {
                send_decode_error(sender, format!("llama.cpp position overflow: {error}"));
                return;
            }
        };
        if let Err(error) = batch.add(token, position, &[0], true) {
            send_decode_error(sender, format!("build llama.cpp token batch: {error}"));
            return;
        }
        if let Err(error) = context.decode(&mut batch) {
            send_decode_error(sender, format!("decode llama.cpp token: {error}"));
            return;
        }
    }
    let elapsed_millis = super::elapsed_millis(started);
    #[allow(clippy::cast_precision_loss)]
    let tokens_per_second =
        (elapsed_millis > 0).then(|| f64::from(produced) / (elapsed_millis as f64 / 1000.0));
    let _ = sender.blocking_send(Ok(CompletionEvent::Done(CompletionSummary {
        model: model_id.to_owned(),
        usage: Some(TokenUsage {
            prompt_tokens: prepared.tokens.len().try_into().unwrap_or(u64::MAX),
            completion_tokens: u64::from(produced),
        }),
        tokens_per_second,
        elapsed_millis,
    })));
}

fn sampler(temperature: Option<f32>, seed: Option<u64>) -> LlamaSampler {
    match temperature.filter(|value| value.is_finite() && *value > 0.0) {
        Some(temperature) => LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature),
            LlamaSampler::dist(seed.map_or(0, seed32)),
        ]),
        None => LlamaSampler::greedy(),
    }
}

fn seed32(seed: u64) -> u32 {
    let bytes = seed.to_le_bytes();
    u32::from_le_bytes([
        bytes[0] ^ bytes[4],
        bytes[1] ^ bytes[5],
        bytes[2] ^ bytes[6],
        bytes[3] ^ bytes[7],
    ])
}

fn send_decode_error(sender: &tokio::sync::mpsc::Sender<Result<CompletionEvent>>, message: String) {
    let _ = sender.blocking_send(Err(LiveError::Capability(message)));
}

#[tonic::async_trait]
impl InferenceEngine for LlamaCppEngine {
    fn name(&self) -> &'static str {
        "llamacpp"
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
            "llama.cpp stream ended without a completion summary".to_owned(),
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
            engine: "llamacpp".to_owned(),
            source: self.model_path.display().to_string(),
            size,
            created_at_millis: 0,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_model() -> Option<PathBuf> {
        std::env::var_os("HOLOGRAM_TEST_GGUF").map(PathBuf::from)
    }

    #[test]
    fn backend_initializes_once() {
        let first = backend().expect("backend");
        let second = backend().expect("same backend");
        assert!(std::ptr::eq(first, second));
    }

    #[test]
    fn missing_model_is_a_typed_error() {
        let config = InferenceConfig {
            engine: "llamacpp".to_owned(),
            model_path: "/nonexistent/model.gguf".to_owned(),
            ..InferenceConfig::default()
        };
        let error = LlamaCppEngine::new(&config).expect_err("missing model");
        assert!(error.to_string().contains("nonexistent"), "{error}");
    }

    #[tokio::test]
    async fn real_model_streams_and_reports_exact_usage_when_configured() {
        let Some(path) = fixture_model() else {
            return;
        };
        let config = InferenceConfig {
            engine: "llamacpp".to_owned(),
            model_path: path.display().to_string(),
            n_ctx: 512,
            ..InferenceConfig::default()
        };
        let engine = LlamaCppEngine::new(&config).expect("engine");
        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "Hello".to_owned(),
                max_tokens: Some(8),
                ..CompletionRequest::default()
            })
            .await
            .expect("stream");
        let mut done = None;
        while let Some(event) = stream.next().await {
            if let CompletionEvent::Done(summary) = event.expect("decode event") {
                done = Some(summary);
            }
        }
        let usage = done.expect("done").usage.expect("exact usage");
        assert!(usage.prompt_tokens > 0);
        assert!(usage.completion_tokens <= 8);
    }
}
