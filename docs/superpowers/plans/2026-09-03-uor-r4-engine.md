# `uor-r4` Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect `.holo` v4 `InferenceModel` layers to the inference boundary, so chat and both compatibility surfaces can serve models compiled by `hologram-ai`.

**Architecture:** A fifth `InferenceEngine` implementation, compiled behind an off-by-default `uor-r4` feature. Unlike every other engine, the model is identified by a `blake3:` κ resolved through the existing archive verify/cache path rather than a filesystem path or a remote tag. This is the adapter ADR 009 reserved: *"A future adapter may connect that facade to Live's chat and OpenAI/Ollama surfaces."*

**Tech Stack:** Rust 1.94, `uor-r4-api` (pinned-`rev` git dependency on the sibling workspace, consumed the way `uor-hologram` already is), kameo, tokio.

**Spec:** `docs/superpowers/specs/2026-09-02-openai-streaming-usage-embeddings-design.md` — §11 and D9.

**Plan 3 of 3.** Depends on Plan 1's boundary. Independent of Plan 2 (llama.cpp). **Do not start until PR #60 has merged**, and not before Assumption A1 below is resolved.

---

## Corrections to §11

The spec's §11 was written before anyone read `uor-r4`'s API. Three of its claims are wrong, and the plan below supersedes them. Update §11 when this lands.

**§11 said `stream_kind() = Native`, "with deltas forwarded from the decode loop."** The natural entry point, `R4Engine::generate_into(&mut self, seed: &[u32], out: &mut [u32]) -> Result<GenerateStatus, ObservedBound>`, fills a caller-provided buffer and returns once — there is no token callback, so it yields `Buffered` behaviour. Native streaming *is* achievable by driving the lower-level `predict_decision(&window) -> PredictDecision` in a loop and emitting a delta per `PredictDecision::Serve(outcome)`, which is exactly what `generate_into` does internally. That is a real choice with a real cost: it couples this engine to `uor-r4` internals rather than its stable entry point. **Task 5 takes the buffered path first and leaves native streaming as a follow-up**, so the engine lands against the stable API.

**§11 said "Token counts are exact, since the tokenizer is in-process."** True, but not for the reason given. `generate_into` takes `seed: &[u32]` — *tokens*, not text — so the caller must tokenize anyway, using the `Tokenizer` re-exported from `uor-r4-api`. `prompt_tokens` is therefore the length of the encoded seed and `completion_tokens` is `GenerateStatus.count`. Exact, but only because we do the tokenizing ourselves.

**§11 never mentioned abstention, and it is this engine's most distinctive behaviour.** `generate_into` "stops at the first abstention (returning the count so far and the abstaining status) and never emits a guessed token." `GenerateStatus.abstained` is a first-class outcome with no equivalent on either HTTP surface. Presenting an abstention as a normal completion would be the same class of dishonesty as fabricating a token count — the thing D2 exists to forbid. See Task 6.

**A fourth thing §11 missed:** `generate_into` truncates its seed to the last `WINDOW` tokens (`seed[seed.len().saturating_sub(WINDOW)..]`). Long prompts are silently shortened. Whatever this engine does, it must not let a client believe the whole prompt was considered.

---

## Assumptions

Stated because they are unresolved, not because they are safe. **A1 blocks Task 3.**

**A1 — the bundle layout is unknown to this plan.** `R4Engine` is constructed from `EngineParts { graph: &[u8], signature_artifact: &[u8], tokenizer: Option<&[u8]>, score_report: Option<&[u8]> }` — four separate byte slices. ADR 009 says the `InferenceModel` layer's `content` is *"the κ of one opaque, provider-owned model bundle"* — a single blob. `uor-r4-api`'s `release_bundle` module exposes `ReleaseBundleManifest`, `BundleComponentDigests`, and `validate()`, but **no unpacker that turns one blob into `EngineParts`**.

So one of these must be true, and nobody in this repo knows which:

1. the bundle blob is an archive (tar/zip) that Live unpacks into the four components — which makes it not opaque to Live, contradicting ADR 009's wording; or
2. `uor-r4` has or gains a loader taking the blob and returning `EngineParts`, keeping it opaque; or
3. the layer's `content` addresses a manifest that references further κs, and Live resolves each through the object store.

**Resolve this with the `hologram-ai` maintainers before starting Task 3.** Option 2 is the one consistent with ADR 009 and the one to ask for. Task 3 is written against it and says so.

**A2 — no test fixture exists yet.** Exercising this engine needs a `.holo` v4 archive carrying an `InferenceModel` layer, and only `hologram-ai` can produce one. Weight-dependent tests are gated on `HOLOGRAM_TEST_R4_HOLO` and skip when absent, mirroring Plan 2. Everything not needing weights — κ resolution, the `aux` engine check, service-entry derivation, the ambiguous-entry failure, config validation — is tested unconditionally and forms the bulk of this plan.

**A3 — `uor-r4-api` is the right crate and it builds standalone.** It is the workspace member exporting `R4Engine`, `InferenceRequest`, `EngineParts`, and `Tokenizer`. Not yet verified: whether depending on it alone pulls a tractable subgraph, and whether it is `Send` enough for the actor path. Task 1 verifies this before any design commits to it.

---

## Global Constraints

- **Never estimate token counts** (D2), and never present an abstention as a completion. Both are the same rule: do not assert what was not observed.
- **The default build stays pure-Rust and unchanged.** Everything is behind `#[cfg(feature = "uor-r4")]`; a default `cargo build`/`cargo test` must be unaffected.
- **Selecting an uncompiled engine is a typed config error** naming the missing feature.
- **`aux` must be checked before executing.** ADR 009 requires the provider to "enforce bundle and engine compatibility": a layer tagged for a different engine is a typed error, never a best-effort attempt.
- **Scope boundary:** this connects the provider to *chat and the compatibility surfaces only*. `hologram run` and resident load keep returning `LIVE_CAPABILITY_MISSING` for model archives. ADR 009's update must say which surfaces are live, or that error stops telling anyone what is actually missing.

---

### Task 1: Verify the dependency is tractable (spike)

A spike, not a feature. Its output is an answer recorded in the plan, and **its code is thrown away**. Doing this first avoids designing five tasks around a crate that cannot be depended on.

**Files:** none committed except a note.

- [ ] **Step 1: Add the dependency temporarily**

```toml
uor-r4-api = { git = "https://github.com/<org>/uor-r4", rev = "<pin>", optional = true }
```

Get the org and a current rev from the sibling checkout at `/Users/auser/work/uor/hologram/uor-r4` (`git -C … remote -v` and `git -C … rev-parse HEAD`).

- [ ] **Step 2: Answer four questions**

Run: `cargo build --locked --features uor-r4 2>&1 | tail -30`

1. Does it build, and how large is the added dependency subgraph (`cargo tree -p uor-r4-api --depth 1`)? `DEPENDENCIES.md` is an explicit budget; a crate dragging in dozens of transitive deps is a conversation, not a detail.
2. Is `R4Engine` `Send`? The engine will be owned by a worker thread, and `generate_into` takes `&mut self`, so it cannot be shared behind an `Arc` without a lock.
3. Does the build require any native toolchain? Unlike llama.cpp this should be pure Rust — confirm rather than assume, since it changes the install story.
4. Is there a loader from a single bundle blob to `EngineParts` (Assumption A1, option 2)? Grep the crate for one.

- [ ] **Step 3: Record the answers and revert**

Append the findings to this plan under "Spike findings", commit *only* that documentation change, and `git checkout Cargo.toml Cargo.lock`. If question 1 or 2 comes back badly, **stop and escalate** — the remaining tasks assume a workable answer.

---

### Task 2: The feature flag, config, and typed error

**Files:**
- Modify: `Cargo.toml`, `src/config.rs`, `DEPENDENCIES.md`
- Test: `src/config.rs`

**Interfaces:**
- Produces: a `uor-r4` Cargo feature; `engine = "uor-r4"` accepted by validation; `InferenceConfig::service_entry`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn uor_r4_engine_is_accepted_only_when_the_feature_is_compiled() {
        let mut config = AppConfig::default();
        config.inference.engine = "uor-r4".to_owned();
        config.inference.default_model =
            "blake3:0000000000000000000000000000000000000000000000000000000000000000".to_owned();
        let result = config.validate();
        if cfg!(feature = "uor-r4") {
            assert!(result.is_ok(), "{result:?}");
        } else {
            let error = result.expect_err("a build without the feature must refuse");
            assert!(
                error.to_string().contains("--features uor-r4"),
                "the error must name the missing feature: {error}"
            );
        }
    }

    #[test]
    fn uor_r4_requires_a_kappa_not_a_path() {
        let mut config = AppConfig::default();
        config.inference.engine = "uor-r4".to_owned();
        config.inference.default_model = "/models/some.gguf".to_owned();
        let error = config.validate().expect_err("a path is not a kappa");
        assert!(error.to_string().contains("blake3:"), "{error}");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --lib config::tests::uor_r4 2>&1 | tail -10`
Expected: FAIL — validation rejects `uor-r4`.

- [ ] **Step 3: Implement**

Extend the engine match arm in `src/config.rs` with a `"uor-r4"` case mirroring the existing structure: a `cfg!(feature = "uor-r4")` guard producing a typed error naming the flag, then validate that `default_model` starts with `blake3:` and is followed by 64 hex characters. `src/config.rs`'s `validate_holo_resident` already validates κ strings in exactly this shape — reuse that logic rather than writing a second validator.

Add `pub service_entry: String` (default `String::new()`) to `InferenceConfig`, used only when an archive carries more than one `InferenceModel` layer (Task 4).

Add the feature:

```toml
[features]
uor-r4 = ["dep:uor-r4-api"]
```

- [ ] **Step 4: Run tests in both builds**

Run: `cargo test --locked --lib config 2>&1 | tail -10`
Run: `cargo test --locked --features uor-r4 --lib config 2>&1 | tail -10`
Expected: PASS both.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/config.rs DEPENDENCIES.md
git commit -m "feat(config): accept engine = uor-r4 behind an off-by-default feature

Unlike every other engine, this one names its model by blake3 kappa rather
than a path or a remote tag, so validation enforces that shape."
```

---

### Task 3: Resolve a κ to an engine bundle

**Blocked on Assumption A1.** Written against A1 option 2 — `uor-r4` exposes a loader from one bundle blob to `EngineParts`. If the answer turns out to be option 1 or 3, this task's Step 3 changes and the rest of the plan does not.

**Files:**
- Create: `src/inference/uor_r4.rs`
- Modify: `src/inference/mod.rs`

**Interfaces:**
- Produces: `pub struct UorR4Engine`, `UorR4Engine::new(&InferenceConfig, Arc<ModelCatalog>, Arc<ObjectStore>) -> Result<Self>`.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn a_kappa_that_is_not_a_model_archive_is_a_typed_error() {
        let fixture = fixture();
        let mut config = crate::config::InferenceConfig::default();
        config.engine = "uor-r4".to_owned();
        config.default_model = format!("blake3:{}", "0".repeat(64));
        let error = UorR4Engine::new(&config, fixture.catalog.clone(), fixture.store.clone())
            .expect_err("an unknown kappa must fail");
        assert!(
            error.to_string().contains("not found") || error.to_string().contains("0000"),
            "the error must name the kappa it could not resolve: {error}"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --features uor-r4 --lib inference::uor_r4 2>&1 | tail -10`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement resolution**

The module header states plainly what this engine is:

```rust
//! Executes `.holo` v4 `InferenceModel` layers in-process (ADR 023, ADR 009).
//!
//! This is the adapter ADR 009 reserved when it said a future adapter may
//! connect the `hologram-ai` facade to Live's chat and OpenAI/Ollama surfaces.
//! Unlike every other engine, the model is a content address rather than a
//! path or a remote tag: the archive is resolved and verified through the
//! existing cache, and its layer's `aux` tag is checked before anything runs.
```

`new` resolves `config.default_model` through the archive verify/cache path, locates the `InferenceModel` layer(s), and returns a typed error naming the κ when resolution fails. Loading the bundle and constructing `R4Engine` happens on the worker thread (Task 5), not here — `R4Engine::generate_into` takes `&mut self`, so it cannot be shared.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --features uor-r4 --lib inference::uor_r4 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Verify the default build is untouched**

Run: `cargo test --locked --lib 2>&1 | grep -E "^test result:"`
Expected: PASS, count unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/inference Cargo.toml
git commit -m "feat(inference): resolve uor-r4 models by content address

The archive is resolved through the existing verify and cache path rather
than a filesystem path, so a model's identity is its kappa."
```

---

### Task 4: Enforce engine compatibility and service-entry selection

ADR 009 requires the provider to "enforce bundle and engine compatibility". This is the task that honours it.

**Files:**
- Modify: `src/inference/uor_r4.rs`

**Interfaces:**
- Produces: `aux` validation and `entry` selection, both typed failures.

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn a_layer_tagged_for_another_engine_is_refused() {
        // Fixture archive whose InferenceModel layer carries aux = "other-engine".
        let fixture = model_archive_with_engine("other-engine");
        let error = engine_for(&fixture).expect_err("a foreign engine tag must be refused");
        assert!(error.to_string().contains("other-engine"), "{error}");
        assert!(error.to_string().contains("uor-r4"), "{error}");
    }

    #[tokio::test]
    async fn a_single_inference_model_layer_supplies_the_entry() {
        let fixture = model_archive_with_entry("ai.default");
        let engine = engine_for(&fixture).expect("one layer needs no explicit entry");
        assert_eq!(engine.service_entry(), "ai.default");
    }

    #[tokio::test]
    async fn ambiguous_entries_must_be_named_rather_than_guessed() {
        let fixture = model_archive_with_entries(&["ai.default", "ai.small"]);
        let error = engine_for(&fixture).expect_err("two layers must not be resolved silently");
        assert!(error.to_string().contains("inference.service_entry"), "{error}");
        assert!(error.to_string().contains("ai.default"), "the error lists the choices: {error}");
    }
```

These need archive fixtures rather than model weights: an archive with an `InferenceModel` layer whose `aux`/`entry` differ. Build them with the same helpers `src/holo.rs`'s tests use (`Layer::inference_model(address_bytes(...), "ai.default", "uor-r4")` appears at `src/holo.rs:2068` and `src/application_plan.rs:1420`). **These tests need no `hologram-ai` fixture and must pass unconditionally** — they are the bulk of this engine's real logic.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --features uor-r4 --lib inference::uor_r4 2>&1 | tail -10`
Expected: FAIL.

- [ ] **Step 3: Implement**

Check `aux == "uor-r4"` before anything else and produce a typed error naming both the declared engine and the expected one — the same shape ADR 009 already mandates for the unconnected case. Derive `entry` when exactly one `InferenceModel` layer exists; with more than one, require `inference.service_entry` and, when it is absent or unmatched, fail with an error listing the available entries. Never pick one.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked --features uor-r4 --lib inference::uor_r4 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/inference/uor_r4.rs
git commit -m "feat(inference): enforce uor-r4 bundle and engine compatibility

ADR 009 requires the provider to check compatibility rather than attempt a
best-effort run. An ambiguous service entry is named, never guessed."
```

---

### Task 5: Generation on a worker thread

**Files:**
- Modify: `src/inference/uor_r4.rs`

**Interfaces:**
- Produces: `impl InferenceEngine for UorR4Engine` with `complete()`, exact `TokenUsage`, and `stream_kind() == StreamKind::Buffered`.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn completion_reports_exact_counts_from_the_bundle_tokenizer() {
        let Some(archive) = fixture_archive() else {
            eprintln!("skipping: set HOLOGRAM_TEST_R4_HOLO to a .holo v4 model archive");
            return;
        };
        let engine = engine_for_archive(&archive).expect("build the engine");

        let completion = engine
            .complete(CompletionRequest {
                prompt: "Hello".to_owned(),
                max_tokens: Some(8),
                ..CompletionRequest::default()
            })
            .await
            .expect("completion");

        let usage = completion.usage.expect("we tokenize the seed ourselves, so counts are known");
        assert!(usage.prompt_tokens > 0, "the seed was encoded");
        assert!(
            usage.completion_tokens <= 8,
            "completion_tokens is GenerateStatus.count, bounded by max_tokens: {usage:?}"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `HOLOGRAM_TEST_R4_HOLO=/path/to/model.holo cargo test --locked --features uor-r4 --lib inference::uor_r4 2>&1 | tail -10`
Expected: FAIL — `InferenceEngine` is not implemented. Without the fixture the test skips.

- [ ] **Step 3: Implement**

`R4Engine::generate_into` takes `&mut self` and is blocking, so it runs on a dedicated worker thread reached over a bounded channel, exactly as Plan 2's llama.cpp engine does and for the same two reasons: it is blocking work that must not occupy the runtime serving HTTP, and its `&mut self` receiver rules out sharing behind an `Arc`.

The sequence: encode the prompt with the bundle's `Tokenizer` into a seed `Vec<u32>`; allocate `out` sized by `max_tokens`; call `generate_into(&seed, &mut out)`; decode `out[..status.count]` back to text. Report
`TokenUsage { prompt_tokens: seed.len() as u64, completion_tokens: status.count as u64 }`.

**Two behaviours to surface, not swallow:**

`generate_into` truncates its seed to the last `WINDOW` tokens. When `seed.len() > WINDOW`, log at `warn` naming both lengths. A client must not be left believing the whole prompt was considered.

`ObservedBound` from `check_window` is a typed failure, not a panic — map it to `LiveError::Protocol`.

`stream_kind()` stays the default `Buffered`. Native streaming would mean driving `predict_decision` per token instead of using the stable `generate_into` entry point; that is a deliberate follow-up, recorded below.

- [ ] **Step 4: Run tests to verify they pass**

Run: `HOLOGRAM_TEST_R4_HOLO=/path/to/model.holo cargo test --locked --features uor-r4 --lib inference::uor_r4 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/inference/uor_r4.rs
git commit -m "feat(inference): generate from uor-r4 bundles on a worker thread

generate_into takes &mut self and blocks, so the engine cannot be shared
behind an Arc and must not occupy the HTTP runtime. Counts are exact because
we encode the seed ourselves with the bundle's tokenizer."
```

---

### Task 6: Surface abstention honestly

The most interesting task, and the one with no precedent on either surface. `generate_into` "never emits a guessed token": it stops at the first abstention and reports `GenerateStatus.abstained`. Returning that as a normal completion would assert something that did not happen — the same class of dishonesty D2 forbids for token counts.

**Files:**
- Modify: `src/inference/uor_r4.rs`, `src/inference/mod.rs`, both compat modules

**Interfaces:**
- Produces: an abstention signal on `CompletionSummary`, and its rendering on both surfaces.

- [ ] **Step 1: Decide the representation**

Recommended, and consistent with how this project already resolved the same tension for emulated streaming: **return the partial text, and mark it out of band.**

- Add `pub abstained: bool` to `CompletionSummary` and `Completion` (default `false`; every other engine leaves it false).
- On the OpenAI surface, the final chunk carries `finish_reason: "abstain"` rather than `"stop"`. Non-standard, but a client reading `finish_reason` gets a true answer instead of a false one, and OpenAI SDKs surface unknown values as plain strings rather than failing.
- On the Ollama surface, the terminal line carries `done_reason: "abstain"`.
- Both surfaces also set `x-hologram-abstained: true`, mirroring the `x-hologram-stream` precedent: an out-of-band marker that never breaks a strict client's parse.

Emitting `"stop"` for an abstention is the one option to reject outright — it is the exact failure the header precedent exists to avoid.

- [ ] **Step 2: Write the failing tests**

```rust
    #[tokio::test]
    async fn an_abstention_is_not_reported_as_a_clean_stop() {
        let engine = Arc::new(AbstainingEngine);
        let fixture = fixture();
        let mut request = request("", &["Hello"]);
        request.stream = Some(true);

        let response = stream_chat(engine, fixture.catalog.clone(), "abstain", request)
            .await
            .expect("the stream opens");

        assert_eq!(
            response.headers().get("x-hologram-abstained").map(|v| v.to_str().unwrap()),
            Some("true")
        );
        let body = collect_sse(response).await;
        assert!(body.contains("\"finish_reason\":\"abstain\""), "{body}");
        assert!(
            !body.contains("\"finish_reason\":\"stop\""),
            "an abstention must never claim a clean stop: {body}"
        );
    }
```

`AbstainingEngine` is a fixture whose `complete_stream` yields one `Delta` and a `Done` with `abstained: true` — no `uor-r4` dependency, so this test runs in the default build and guards the behaviour for every engine.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --locked --lib an_abstention 2>&1 | tail -10`
Expected: FAIL — the field does not exist.

- [ ] **Step 4: Implement**

Thread `abstained` from `GenerateStatus` through `Completion`/`CompletionSummary` to both surfaces' terminal chunk and header. Every other engine reports `false`, so nothing else changes shape.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --locked 2>&1 | grep -E "^test result:"`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/inference src/modules
git commit -m "feat(inference): surface uor-r4 abstention rather than faking a stop

The engine never emits a guessed token; when it declines, the surfaces say so
via finish_reason/done_reason and an x-hologram-abstained header instead of
claiming a completion that did not happen."
```

---

### Task 7: ADR updates and documentation

**Files:**
- Modify: `specs/adrs/009-inference-model-holo-v4.md`, `specs/adrs/023-in-process-inference.md` (or create if Plan 2 has not landed), `README.md`, `ACTUAL_CAPABILITIES.md`, `DEPENDENCIES.md`

- [ ] **Step 1: Update ADR 009**

It currently mandates `LIVE_CAPABILITY_MISSING` until a provider is connected. Record that the provider is now connected **to chat and the compatibility surfaces only**, and that `hologram run` and resident load deliberately still return that error. Without this the remaining capability error becomes ambiguous about what is actually missing — which is the whole reason the scope boundary is worth writing down.

- [ ] **Step 2: Record the in-process decision**

If Plan 2 landed, extend ADR 023 to cover this engine too: pure Rust, so no C++ toolchain, but the same forfeit of crash isolation. If Plan 2 has not landed, write ADR 023 covering D8 and D9 together.

- [ ] **Step 3: Update operator docs**

`README.md`'s engine list and `live.toml` sample gain `uor-r4` with `default_model` (a κ) and `service_entry`. `ACTUAL_CAPABILITIES.md` gains the engine and drops "inference-model provider invocation" from future work **for the chat and compatibility surfaces only** — the application-runtime paths are still unconnected and the wording must not overclaim. `DEPENDENCIES.md` gains `uor-r4-api` as an optional, feature-gated dependency.

- [ ] **Step 4: Verify both builds**

Run: `cargo test --locked 2>&1 | grep -E "^test result:"`
Run: `cargo test --locked --features uor-r4 2>&1 | grep -E "^test result:"`
Run: `cargo clippy --locked --all-targets -- -D warnings`
Run: `cargo clippy --locked --features uor-r4 --all-targets -- -D warnings`
Run: `cargo fmt --check`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add specs/adrs README.md ACTUAL_CAPABILITIES.md DEPENDENCIES.md
git commit -m "docs: connect the uor-r4 provider in ADR 009

Records which surfaces are live and which deliberately still return
LIVE_CAPABILITY_MISSING, so the remaining error stays unambiguous."
```

---

## Deliberately out of scope

- **Native streaming.** Requires driving `predict_decision` per token instead of the stable `generate_into` entry point, coupling this engine to `uor-r4` internals. Worth doing once the buffered path is proven, and worth asking whether `uor-r4` would expose a callback form instead.
- **The application-runtime paths.** `hologram run` and resident load on a model archive keep returning `LIVE_CAPABILITY_MISSING`. Connecting those is a separate piece of work with its own session and authorization semantics.
- **Witnesses.** `InferenceWitness` and `include_witness` offer a replayable proof summary per token. There is no place on the OpenAI or Ollama wire formats to put it, and inventing one is a design conversation, not a detail.
- **Sessions.** `supports_sessions()` stays `false`. ADR 009 mentions preserving model-session lifecycle semantics; that lands with the application-runtime work, not here.
