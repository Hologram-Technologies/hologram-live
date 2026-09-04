# llama.cpp Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `llamacpp` inference engine that executes GGUF models in-process, behind an off-by-default Cargo feature.

**Architecture:** A fifth `InferenceEngine` implementation. `llama-cpp-2` bindings run the model inside the daemon, so the engine compiles only under the `llamacpp` feature and the default build stays pure-Rust and ADR 003 compliant. Decode is blocking work on a dedicated thread reached over bounded channels. Because the tokenizer is in-process, this is the first engine that can always report exact token counts.

**Tech Stack:** Rust 1.94, `llama-cpp-2` 0.1.146 (API verified against the vendored source at `~/.cargo/registry/src/*/llama-cpp-2-0.1.146/`), tokio, kameo.

**Spec:** `docs/superpowers/specs/2026-09-02-openai-streaming-usage-embeddings-design.md` — §10 and D8.

**Plan 2 of 3.** Depends on Plan 1's boundary (`StreamKind`, `CompletionEvent`, `CompletionStream`, `TokenUsage`, `complete_stream`). **Do not start until PR #60 has merged**, or this plan's base shifts underneath it. Plan 3 (`uor-r4`, §11) is independent of this one.

## Global Constraints

- **Never estimate token counts** (D2). This engine owns the tokenizer, so counts are exact: `prompt_tokens` is the encoded prompt length, `completion_tokens` the decoded count. Never substitute an estimate.
- **The default build stays pure-Rust.** Everything here is behind `#[cfg(feature = "llamacpp")]`. A default `cargo build` must not require a C++ toolchain, and a default `cargo test` must pass without one.
- **Selecting an uncompiled engine is a typed config error** naming the missing feature — never a panic, never a silent fallback to `echo`.
- **Decode must never run on the async runtime** that serves every other HTTP route.
- Match the existing comment density: `//!` module headers and `///` on public items explaining *why*.

## Verified API facts

Read from the vendored crate source, not recalled. These shape the design:

- `LlamaBackend::init() -> Result<LlamaBackend>` is **process-global**; it must happen exactly once.
- `LlamaModel::load_from_file(&LlamaBackend, path, &LlamaModelParams) -> Result<Self, LlamaModelLoadError>`
- `model.new_context<'a>(&'a self, &LlamaBackend, LlamaContextParams) -> Result<LlamaContext<'a>, _>`
- **`LlamaContext<'a>` holds `pub model: &'a LlamaModel`.** A struct therefore **cannot** own both a model and a context — that is self-referential. The model must outlive its contexts within a single scope, which is why the decode thread owns the model and creates contexts locally.
- `context.decode(&mut LlamaBatch) -> Result<(), DecodeError>`
- `LlamaSampler::chain_simple([...])`, `::temp(f32)`, `::dist(u32)`, `::greedy()`; `sampler.sample(&ctx, idx) -> LlamaToken`; `sampler.accept(token)`
- `model.str_to_token(...)`, `model.token_to_str(...)`, `model.is_eog_token(token)`
- `context.embeddings_seq_ith(i) -> Result<&[f32]>` exists, so §6 embeddings is servable by this engine later — **out of scope here**.

---

### Task 1: GGUF support in the model catalog

`ModelCatalog::import` requires a **directory** containing `manifest.json` and hard-codes `engine: "weightc"`. GGUF is a single file with neither. A prerequisite, not a detail.

**Files:**
- Modify: `src/models.rs`
- Test: `src/models.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `ModelCatalog`, `ModelManifest`, `ModelInfo` as they exist.
- Produces: `ModelCatalog::import_gguf(&self, source: &Path) -> Result<ModelInfo>` recording `engine: "llamacpp"`, and `ModelCatalog::artifact_file(&self, id: &str) -> Result<PathBuf>`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn gguf_import_records_a_single_file_and_the_llamacpp_engine() {
        let fixture = fixture();
        let source = fixture.temporary.path().join("tiny.gguf");
        // GGUF magic plus filler; the catalog stores bytes, it does not parse.
        std::fs::write(&source, b"GGUF\x03\x00\x00\x00rest-of-file").expect("write gguf");

        let info = fixture.catalog.import_gguf(&source).expect("import");

        assert_eq!(info.engine, "llamacpp");
        assert_eq!(info.name, "tiny");
        assert!(info.size > 0);
        let stored = fixture.catalog.artifact_file(&info.id).expect("artifact file");
        assert!(stored.is_file(), "a gguf artifact is one file, not a directory");
        assert_eq!(
            std::fs::read(&stored).expect("read stored"),
            b"GGUF\x03\x00\x00\x00rest-of-file"
        );
    }

    #[test]
    fn gguf_import_rejects_a_directory() {
        let fixture = fixture();
        let source = fixture.temporary.path().join("not-a-file.gguf");
        std::fs::create_dir_all(&source).expect("dir");
        let error = fixture.catalog.import_gguf(&source).expect_err("must reject");
        assert!(error.to_string().contains("not a .gguf file"), "{error}");
    }
```

Mirror whatever fixture helper `src/models.rs`'s existing tests use. If there is none, build the catalog the way `src/modules/openai_compat.rs`'s `fixture()` does: an `ObjectStore` plus a `ModelCatalog` under a `tempfile::tempdir()`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --lib models::tests::gguf 2>&1 | tail -10`
Expected: FAIL — `no method named import_gguf`.

- [ ] **Step 3: Implement**

```rust
    /// Import a single-file GGUF model for the in-process `llamacpp` engine.
    ///
    /// Unlike `.wcpu` artifacts, which are directories carrying their own
    /// `manifest.json`, a GGUF model is one opaque blob. The catalog records it
    /// with a synthesized manifest so both kinds list identically.
    pub fn import_gguf(&self, source: &Path) -> Result<ModelInfo> {
        if !source.is_file() {
            return Err(LiveError::Protocol(format!(
                "{} is not a .gguf file",
                source.display()
            )));
        }
        let bytes = std::fs::read(source).map_err(|error| LiveError::io(source, error))?;
        let name = source
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "model".to_owned());
        let manifest = ModelManifest {
            name,
            engine: "llamacpp".to_owned(),
            source: source.display().to_string(),
            size: bytes.len() as u64,
            files: vec![GGUF_FILE_NAME.to_owned()],
        };
        let encoded = serde_json::to_vec_pretty(&manifest)?;
        let metadata = self.store.put(
            MODEL_KIND,
            MODEL_MEDIA_TYPE,
            Some(manifest.name.clone()),
            &encoded,
        )?;
        let destination = self.artifact_path(&metadata.id);
        std::fs::create_dir_all(&destination)
            .map_err(|error| LiveError::io(&destination, error))?;
        let file = destination.join(GGUF_FILE_NAME);
        std::fs::write(&file, &bytes).map_err(|error| LiveError::io(&file, error))?;
        Ok(model_info(metadata, &manifest))
    }

    /// Path of a single-file artifact (GGUF). Directory-shaped artifacts use
    /// `artifact_dir` instead.
    pub fn artifact_file(&self, id: &str) -> Result<PathBuf> {
        let file = self.artifact_dir(id)?.join(GGUF_FILE_NAME);
        if !file.is_file() {
            return Err(LiveError::NotFound(format!(
                "model {id} has no {GGUF_FILE_NAME}; it may be a .wcpu artifact"
            )));
        }
        Ok(file)
    }
```

Add `const GGUF_FILE_NAME: &str = "model.gguf";` beside the other module constants. Storing it inside the per-digest directory keeps one artifact-root layout for both kinds.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --lib models 2>&1 | tail -10`
Expected: PASS. The existing `.wcpu` import tests must be untouched and still green.

- [ ] **Step 5: Commit**

```bash
git add src/models.rs
git commit -m "feat(models): import single-file GGUF artifacts

The catalog assumed every model is a .wcpu directory carrying its own
manifest.json. GGUF is one opaque blob, so it gets a synthesized manifest and
a single-file accessor while sharing the per-digest artifact root."
```

---

### Task 2: The feature flag, the dependency, and the typed config error

**Files:**
- Modify: `Cargo.toml`, `src/config.rs`, `DEPENDENCIES.md`
- Test: `src/config.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: a `llamacpp` Cargo feature; `engine = "llamacpp"` accepted by validation; a typed error when the feature is absent; `InferenceConfig` fields `model_path`, `n_ctx`, `n_gpu_layers`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn llamacpp_engine_is_accepted_only_when_the_feature_is_compiled() {
        let mut config = AppConfig::default();
        config.inference.engine = "llamacpp".to_owned();
        config.inference.model_path = "/models/tiny.gguf".to_owned();
        let result = config.validate();
        if cfg!(feature = "llamacpp") {
            assert!(result.is_ok(), "{result:?}");
        } else {
            let error = result.expect_err("a build without the feature must refuse");
            assert!(
                error.to_string().contains("--features llamacpp"),
                "the error must name the missing feature: {error}"
            );
        }
    }

    #[test]
    fn an_unknown_engine_lists_llamacpp_among_the_supported_set() {
        let mut config = AppConfig::default();
        config.inference.engine = "nonsense".to_owned();
        let error = config.validate().expect_err("unknown engine");
        assert!(error.to_string().contains("llamacpp"), "{error}");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --lib config::tests::llamacpp 2>&1 | tail -10`
Expected: FAIL — validation rejects `llamacpp`.

- [ ] **Step 3: Add the feature and the optional dependency**

In `Cargo.toml`:

```toml
[dependencies]
llama-cpp-2 = { version = "0.1.146", optional = true }

[features]
bdd = []
# Executes GGUF model weights inside the daemon process (ADR 023). Off by
# default: it needs a C++ toolchain and forfeits the crash isolation every
# subprocess engine has.
llamacpp = ["dep:llama-cpp-2"]
```

- [ ] **Step 4: Accept the engine, and fail honestly without the feature**

In `src/config.rs`, replace the engine match arm:

```rust
        match self.inference.engine.as_str() {
            "echo" | "weightc" | "ollama" => {}
            "llamacpp" => {
                if !cfg!(feature = "llamacpp") {
                    return Err(LiveError::Config(
                        "inference.engine \"llamacpp\" needs a build with \
                         --features llamacpp; this binary was built without it"
                            .to_owned(),
                    ));
                }
                if self.inference.model_path.trim().is_empty() {
                    return Err(LiveError::Config(
                        "inference.model_path must name a .gguf file when \
                         inference.engine is \"llamacpp\""
                            .to_owned(),
                    ));
                }
            }
            other => {
                return Err(LiveError::Config(format!(
                    "unsupported inference.engine {other:?}; \
                     expected echo, weightc, ollama, or llamacpp"
                )))
            }
        }
```

Add to `InferenceConfig`: `pub model_path: String` (default `String::new()`), `pub n_ctx: u32` (default `4096`), `pub n_gpu_layers: u32` (default `0`). A GGUF model is selected by path rather than by the `blake3:` digest `weightc` uses, so `default_model` is not the right knob here.

- [ ] **Step 5: Run tests in both builds**

Run: `cargo test --locked --lib config 2>&1 | tail -10`
Run: `cargo test --locked --features llamacpp --lib config 2>&1 | tail -10`
Expected: PASS both times. The test branches on `cfg!(feature = ...)`, so it asserts the correct thing in each build.

- [ ] **Step 6: Record the dependency**

Add to `DEPENDENCIES.md`'s table: `llama-cpp-2` — *optional, `llamacpp` feature only: in-process GGUF execution*. Then amend the paragraph stating the daemon runs third-party code as subprocesses "rather than as loaded native code" to note this feature-gated exception, pointing at ADR 023.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/config.rs DEPENDENCIES.md
git commit -m "feat(config): accept engine = llamacpp behind an off-by-default feature

Selecting it in a binary built without the feature is a typed config error
naming the missing flag, not a panic and not a silent fallback to echo."
```

---

### Task 3: Backend init and model loading

**Files:**
- Create: `src/inference/llamacpp.rs`
- Modify: `src/inference/mod.rs` (module declaration, re-export, `engine_from_config` arm)

**Interfaces:**
- Consumes: `InferenceConfig`, `InferenceEngine`, `LiveError`.
- Produces: `pub struct LlamaCppEngine` with `LlamaCppEngine::new(&InferenceConfig) -> Result<Self>`, and a private `fn backend() -> Result<&'static LlamaBackend>`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Exercising a real model needs real weights, so those tests are gated on
    /// a fixture path and skip when it is absent. That keeps the default
    /// `cargo test` run weight-free.
    fn fixture_model() -> Option<std::path::PathBuf> {
        std::env::var_os("HOLOGRAM_TEST_GGUF").map(std::path::PathBuf::from)
    }

    #[test]
    fn the_backend_initializes_exactly_once() {
        let first = backend().expect("backend init");
        let second = backend().expect("backend init is idempotent");
        assert!(
            std::ptr::eq(first, second),
            "llama.cpp's backend is process-global and must init exactly once"
        );
    }

    #[test]
    fn a_missing_model_file_is_a_typed_error_at_construction() {
        let mut config = crate::config::InferenceConfig::default();
        config.engine = "llamacpp".to_owned();
        config.model_path = "/nonexistent/model.gguf".to_owned();
        let error = LlamaCppEngine::new(&config).expect_err("missing file must fail");
        assert!(error.to_string().contains("nonexistent"), "{error}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --features llamacpp --lib inference::llamacpp 2>&1 | tail -10`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement the module skeleton**

```rust
//! In-process GGUF execution via `llama-cpp-2` (ADR 023).
//!
//! Compiled only under the `llamacpp` feature. Unlike every other engine, this
//! one runs model weights inside the daemon: there is no subprocess to isolate
//! a crash, which is the cost D8 accepted in exchange for exact token counts
//! and native streaming with no HTTP hop.

use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use std::path::PathBuf;
use std::sync::OnceLock;

/// `llama.cpp`'s backend is process-global; initializing it twice is undefined.
/// A `OnceLock` makes the single init explicit rather than trusting callers.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<std::result::Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| LiveError::Capability(format!("llama.cpp backend init failed: {error}")))
}

pub struct LlamaCppEngine {
    model_path: PathBuf,
    model_label: String,
    n_ctx: u32,
    n_gpu_layers: u32,
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
        // Fail at construction rather than on the first request, so an
        // operator learns about a bad model at startup, not mid-conversation.
        backend()?;
        Ok(Self {
            model_label: model_path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "gguf".to_owned()),
            model_path,
            n_ctx: config.n_ctx,
            n_gpu_layers: config.n_gpu_layers,
        })
    }
}
```

In `src/inference/mod.rs`:

```rust
#[cfg(feature = "llamacpp")]
mod llamacpp;
#[cfg(feature = "llamacpp")]
pub use llamacpp::LlamaCppEngine;
```

and in `engine_from_config`:

```rust
        #[cfg(feature = "llamacpp")]
        "llamacpp" => Ok(Arc::new(LlamaCppEngine::new(config)?)),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --features llamacpp --lib inference::llamacpp 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Verify the default build is untouched**

Run: `cargo test --locked --lib 2>&1 | grep -E "^test result:"`
Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: PASS, with the same test count as before this task — the module must be entirely absent without the feature.

- [ ] **Step 6: Commit**

```bash
git add src/inference Cargo.toml
git commit -m "feat(inference): add the llamacpp engine skeleton and backend init

llama.cpp's backend is process-global, so a OnceLock makes the single
initialization explicit. Model problems surface at construction, so an
operator learns at startup rather than mid-conversation."
```

---

### Task 4: Blocking decode on a dedicated thread

The task the verified API shapes most. `LlamaContext<'a>` borrows `&'a LlamaModel`, so **no struct may hold both**. The model is loaded on the decode thread, contexts are created there, and both die with the thread.

**Files:**
- Modify: `src/inference/llamacpp.rs`

**Interfaces:**
- Consumes: Task 3's `LlamaCppEngine` and `backend()`.
- Produces: `impl InferenceEngine for LlamaCppEngine` with a working `complete()`, `name()`, `list_models()`, and exactly-populated `TokenUsage`.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn completion_reports_exact_counts_from_the_in_process_tokenizer() {
        let Some(path) = fixture_model() else {
            eprintln!("skipping: set HOLOGRAM_TEST_GGUF to a small .gguf to run this");
            return;
        };
        let mut config = crate::config::InferenceConfig::default();
        config.engine = "llamacpp".to_owned();
        config.model_path = path.display().to_string();
        config.n_ctx = 512;
        let engine = LlamaCppEngine::new(&config).expect("build the engine");

        let completion = engine
            .complete(CompletionRequest {
                prompt: "Hello".to_owned(),
                max_tokens: Some(8),
                ..CompletionRequest::default()
            })
            .await
            .expect("completion");

        let usage = completion
            .usage
            .expect("this engine owns the tokenizer, so counts are always known");
        assert!(usage.prompt_tokens > 0, "the prompt was encoded");
        assert!(
            usage.completion_tokens > 0 && usage.completion_tokens <= 8,
            "completion_tokens must be the real decoded count, bounded by \
             max_tokens: {usage:?}"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --features llamacpp --lib inference::llamacpp 2>&1 | tail -10`
Expected: FAIL — `InferenceEngine` is not implemented. Without `HOLOGRAM_TEST_GGUF` the test skips; set it to see the real failure.

- [ ] **Step 3: Implement the decode thread**

```rust
/// One decode request handed to a worker thread.
struct DecodeJob {
    request: CompletionRequest,
    reply: tokio::sync::mpsc::Sender<Result<CompletionEvent>>,
}

impl LlamaCppEngine {
    /// Runs one completion on a dedicated OS thread.
    ///
    /// `LlamaContext` borrows its `LlamaModel`, so the two cannot live in a
    /// struct together — the model is loaded here and the context borrows it
    /// for the life of the call. Decode is also blocking CPU/GPU work and must
    /// never occupy the runtime that serves every HTTP route.
    fn spawn_decode(&self, job: DecodeJob) {
        let path = self.model_path.clone();
        let label = self.model_label.clone();
        let (n_ctx, n_gpu_layers) = (self.n_ctx, self.n_gpu_layers);
        std::thread::spawn(move || {
            let send = |event: Result<CompletionEvent>| {
                let _ = job.reply.blocking_send(event);
            };
            let backend = match backend() {
                Ok(backend) => backend,
                Err(error) => return send(Err(error)),
            };
            let model_params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
            let model = match LlamaModel::load_from_file(backend, &path, &model_params) {
                Ok(model) => model,
                Err(error) => {
                    return send(Err(LiveError::Capability(format!(
                        "load {}: {error}",
                        path.display()
                    ))))
                }
            };
            if let Err(error) = decode_into(&model, backend, n_ctx, &label, &job.request, &send) {
                send(Err(error));
            }
        });
    }
}
```

`decode_into` tokenizes with `model.str_to_token`, builds a `LlamaBatch`, then loops: `context.decode(&mut batch)`, `sampler.sample(&context, index)`, stop on `model.is_eog_token(token)` or `max_tokens`, detokenize with `model.token_to_str`. It sends one `CompletionEvent::Delta` per token and a terminal `CompletionEvent::Done` carrying `TokenUsage { prompt_tokens: prompt_len as u64, completion_tokens: produced as u64 }`.

Build the sampler from the request:

```rust
    let sampler = match (request.temperature, request.seed) {
        (Some(temperature), Some(seed)) => LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature),
            LlamaSampler::dist(seed as u32),
        ]),
        (Some(temperature), None) => LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature),
            LlamaSampler::dist(0),
        ]),
        _ => LlamaSampler::greedy(),
    };
```

`complete()` drives `spawn_decode`, concatenates the deltas, and returns a `Completion` carrying the terminal summary's usage. **Task 5 reuses this same machinery** — do not write a second decode path, which is how the two would drift.

- [ ] **Step 4: Run the test with a fixture model**

Run: `HOLOGRAM_TEST_GGUF=/path/to/tiny.gguf cargo test --locked --features llamacpp --lib inference::llamacpp 2>&1 | tail -15`
Expected: PASS. Any small instruct GGUF works; ~50–100 MB keeps the run fast.

- [ ] **Step 5: Verify the default build again**

Run: `cargo test --locked --lib 2>&1 | grep -E "^test result:"`
Expected: PASS, count unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/inference/llamacpp.rs
git commit -m "feat(inference): decode GGUF models on a dedicated thread

LlamaContext borrows its LlamaModel, so neither can be held in a struct
together; the thread owns both for the life of a request. Decode is blocking
CPU/GPU work and never touches the runtime serving HTTP. Token counts come
from the in-process tokenizer, so they are exact rather than reported."
```

---

### Task 5: Native streaming

**Files:**
- Modify: `src/inference/llamacpp.rs`

**Interfaces:**
- Consumes: Task 4's decode machinery.
- Produces: `stream_kind() -> StreamKind::Native` and a real `complete_stream`.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn streaming_yields_tokens_as_they_are_decoded() {
        use tokio_stream::StreamExt;

        let Some(path) = fixture_model() else {
            eprintln!("skipping: set HOLOGRAM_TEST_GGUF to a small .gguf to run this");
            return;
        };
        let mut config = crate::config::InferenceConfig::default();
        config.engine = "llamacpp".to_owned();
        config.model_path = path.display().to_string();
        config.n_ctx = 512;
        let engine = LlamaCppEngine::new(&config).expect("build the engine");
        assert_eq!(engine.stream_kind(), StreamKind::Native);

        let mut stream = engine
            .complete_stream(CompletionRequest {
                prompt: "Hello".to_owned(),
                max_tokens: Some(8),
                ..CompletionRequest::default()
            })
            .await
            .expect("stream opens");

        let mut deltas = Vec::new();
        let mut summary = None;
        while let Some(event) = stream.next().await {
            match event.expect("no mid-stream failure with a valid model") {
                CompletionEvent::Delta(text) => deltas.push(text),
                CompletionEvent::Done(done) => summary = Some(done),
            }
        }

        assert!(
            deltas.len() > 1,
            "a native engine emits one delta per token, not one buffered \
             delta: {deltas:?}"
        );
        assert!(summary.expect("terminal Done").usage.is_some());
    }
```

The `deltas.len() > 1` assertion is what distinguishes this from the buffered default. Without it, the test would pass even if `complete_stream` fell through to the trait's emulating implementation.

- [ ] **Step 2: Run test to verify it fails**

Run: `HOLOGRAM_TEST_GGUF=/path/to/tiny.gguf cargo test --locked --features llamacpp --lib streaming_yields_tokens 2>&1 | tail -10`
Expected: FAIL — `stream_kind` is `Buffered` and exactly one delta arrives.

- [ ] **Step 3: Implement**

```rust
    fn stream_kind(&self) -> StreamKind {
        StreamKind::Native
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<CompletionStream> {
        let (reply, receiver) = tokio::sync::mpsc::channel(16);
        self.spawn_decode(DecodeJob { request, reply });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }
```

**Honour the boundary contract** documented on `complete_stream` in `src/inference/mod.rs`: anything knowable before the first delta must be returned as `Err` from this method, never yielded into the stream. Model loading currently happens on the thread, so a corrupt model would surface in-band after a committed 200. Either load and validate before returning the stream, or rely on Task 3's construction-time check as the guarantee and document that the only in-band failures are genuine mid-decode ones. Pick one and say which in the code.

- [ ] **Step 4: Run tests to verify they pass**

Run: `HOLOGRAM_TEST_GGUF=/path/to/tiny.gguf cargo test --locked --features llamacpp --lib inference::llamacpp 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/inference/llamacpp.rs
git commit -m "feat(inference): stream llama.cpp tokens natively

Deltas leave the decode loop as they are produced, so this is the first engine
whose x-hologram-stream header reads native without an HTTP hop."
```

---

### Task 6: ADR 023 and documentation

**Files:**
- Create: `specs/adrs/023-in-process-inference.md`
- Modify: `specs/adrs/003-inference-engine-boundary.md`, `README.md`, `ACTUAL_CAPABILITIES.md`, `install.sh`, `install.ps1`

**Interfaces:** none.

- [ ] **Step 1: Write ADR 023**

Record D8 in the house style (Status / Context / Decision / Consequences — compare ADRs 016 and 022). It **amends ADR 003's central decision** — *"the daemon never executes model weights in-process"* — and must say so explicitly, scoping the amendment to builds carrying the `llamacpp` feature.

State the costs plainly rather than burying them: a C++ toolchain requirement for opt-in builds; the loss of the crash isolation every subprocess engine has, since a segfault takes down a daemon also hosting wasmtime archives, files, applications, and the registry; and the fact that the default test suite never covers this engine because it needs real weights. State the compensating benefits: exact token counts from an in-process tokenizer, and native streaming with no HTTP hop.

- [ ] **Step 2: Amend ADR 003**

Add a Consequences line recording that ADR 023 amends the in-process decision for opt-in builds, and that the boundary itself is unchanged — this is a fifth implementation of the same trait, not a new seam.

- [ ] **Step 3: Update the operator-facing docs**

`README.md`'s engine list and `live.toml` sample gain `llamacpp` with `model_path`, `n_ctx`, and `n_gpu_layers`, plus the `--features llamacpp` build instruction and its toolchain requirement. `ACTUAL_CAPABILITIES.md` gains the engine. `install.sh` and `install.ps1` are cargo-only source distributions — document that the default install is unaffected and that the feature build additionally needs cmake and a C++ compiler.

- [ ] **Step 4: Verify both builds**

Run: `cargo test --locked 2>&1 | grep -E "^test result:"`
Run: `cargo test --locked --features llamacpp 2>&1 | grep -E "^test result:"`
Run: `cargo clippy --locked --all-targets -- -D warnings`
Run: `cargo clippy --locked --features llamacpp --all-targets -- -D warnings`
Run: `cargo fmt --check`
Expected: all PASS. Both builds must be lint-clean — feature-gated code is not exempt.

- [ ] **Step 5: Commit**

```bash
git add specs/adrs README.md ACTUAL_CAPABILITIES.md install.sh install.ps1
git commit -m "docs: record in-process inference in ADR 023

Amends ADR 003's never-in-process decision for opt-in builds only, and states
the costs: a C++ toolchain, no crash isolation, and an engine the default test
suite cannot cover."
```

---

## Testing note

Both in-process engines share a problem: exercising them needs real weights. Tests here are gated on `HOLOGRAM_TEST_GGUF` and skip when it is absent, so the default `cargo test` stays weight-free and fast. The consequence — the default suite never covers this engine — is an accepted cost of D8 and belongs in ADR 023, not hidden in a test file. CI should run the gated job separately with a small fixture model.

The parts needing no weights must be tested unconditionally: backend single-init, the typed config error when the feature is missing, GGUF catalog import, and model-path validation.

## Deliberately out of scope

- **Sessions.** §10 proposes reusing the `weightc` kameo session actor and LRU for KV-cache contexts. That is a meaningful chunk of work, and this plan does not attempt it: `supports_sessions()` stays `false` and every request is one-shot. Add it once the one-shot path is proven.
- **Embeddings.** `context.embeddings_seq_ith` exists, so §6 is servable by this engine — but §6 remains specified-and-unbuilt by choice.
- **`uor-r4`.** Plan 3, independent of this one.
