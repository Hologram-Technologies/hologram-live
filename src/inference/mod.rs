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
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio_stream::Stream;

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
    pub usage: Option<TokenUsage>,
}

/// Token counts an engine measured. Both fields are required: the `OpenAI`
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
    /// A chunk of generated text. Concatenating every `Delta` in arrival
    /// order reconstructs the full completion text.
    Delta(String),
    /// The terminal event: the stream will not yield anything after this.
    Done(CompletionSummary),
}

/// A completion delivered incrementally.
///
/// `Send + 'static`: the stream outlives the `complete_stream` call that
/// created it (it is returned, then polled independently), so implementors
/// must move owned values into it rather than borrowing `&self` or the
/// request.
///
/// Contract binding on every implementor of [`InferenceEngine::complete_stream`],
/// not just the buffered default:
/// - Anything knowable before the first delta (auth failure, a rejected
///   request, an upstream connection error) must be returned as `Err` from
///   `complete_stream` itself, never yielded as an item inside the stream.
///   That is what lets a caller commit to a 200 response only once it knows
///   the request is actually going to produce output.
/// - A stream ends with exactly one [`CompletionEvent::Done`], or with an
///   `Err` item and no `Done` — never both, and never neither.
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<CompletionEvent>> + Send>>;

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

    /// Whether deltas are real or reconstructed. Drives the
    /// `x-hologram-stream` header, so the honesty marker comes from the engine
    /// rather than being set per module.
    fn stream_kind(&self) -> StreamKind {
        StreamKind::Buffered
    }

    /// Streams a completion. Implementors — including any future native
    /// override — must uphold this contract:
    /// - Anything knowable before the first delta (a rejected request, an
    ///   upstream connection or auth failure) is returned as `Err` from this
    ///   method itself, never yielded into the stream. A caller that commits
    ///   to a 200 response on `Ok` must be able to trust that the stream will
    ///   actually produce output.
    /// - The stream ends with exactly one [`CompletionEvent::Done`], or with
    ///   an `Err` item and no `Done` — never both.
    /// - The returned [`CompletionStream`] is `'static`, so implementors move
    ///   owned values in rather than borrowing `&self`.
    ///
    /// Buffered default: awaits the whole completion, then replays it as a
    /// single delta (D5). Because the completion is awaited before the stream
    /// is returned, a failure surfaces as a normal typed error with the
    /// correct status rather than a half-open stream — this is what the
    /// contract above requires, made unavoidable by construction.
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

    #[tokio::test]
    async fn the_buffered_default_yields_one_delta_then_done() {
        use tokio_stream::StreamExt;

        let engine = EchoEngine;
        assert_eq!(engine.stream_kind(), StreamKind::Buffered);

        // Built inline rather than via the `prompt` test helper: Task 1 moves
        // the echo tests to echo.rs, so that helper's home is not fixed here.
        // Multi-word so a `split_whitespace()` implementation would yield more
        // than one delta and fail this test — a single-word prompt would let
        // "loses no text" masquerade as "does not chunk".
        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "Hello there".to_owned(),
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
            vec!["Hello there".to_owned()],
            "D5: buffered engines emit a single delta carrying the whole text, not whitespace chunks"
        );
        assert!(summary.is_some(), "the stream must terminate with Done");
    }

    #[test]
    fn the_header_distinguishes_real_streaming_from_emulation() {
        assert_eq!(StreamKind::Native.header_value(), "native");
        assert_eq!(StreamKind::Buffered.header_value(), "emulated");
    }
}
