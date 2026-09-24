# ADR 034: Candle and Burn may execute in-process in opt-in builds

- Status: accepted
- Date: 2026-09-22
- Amends: ADR 003

## Context

ADR 033 permits an opt-in llama.cpp engine but brings a C++ toolchain into that build. Candle and Burn offer Rust-native execution with different model contracts: Candle has practical quantized Llama GGUF inference, while Burn-LM exposes a smaller set of Llama 3 architectures using Burn's named-MPK records. Neither library is a universal model-format dispatcher.

Running either library inside Hologram still forfeits process isolation. A panic, allocator failure, or backend fault can terminate the daemon serving applications, files, and registry traffic. Large model initialization also delays server startup.

## Decision

Add `CandleEngine` and `BurnEngine` behind independent, off-by-default Cargo features. Both implement only `InferenceEngine`; callers and compatibility APIs do not depend on framework types.

The Candle adapter accepts a GGUF file, a matching `tokenizer.json`, and the explicit `llama` architecture. It runs blocking decode on a dedicated thread, streams decoded text through a bounded channel, validates the prompt before committing the response, and reports observed token counts. CPU is the base feature; CUDA and Metal are explicit variants.

The Burn adapter accepts a Burn named-MPK checkpoint, a matching Llama 3 `tokenizer.model`, and one of the Llama variants implemented by Burn-LM. The first adapter is CPU-only and serializes access to the model's mutable KV cache. It returns a buffered completion, so compatibility surfaces mark streaming as emulated.

Unsupported architectures and mismatched files fail at startup. Hologram does not infer that a format accepted by one engine is accepted by another.

## Consequences

- The stock build and its dependency graph remain unchanged.
- Opt-in builds execute model code in the daemon and inherit ADR 033's crash-boundary cost.
- Candle offers a Rust-native GGUF path with native streaming, but much narrower architecture coverage than llama.cpp. Its current tokenizer graph includes Oniguruma through Candle's dependency features, so it is not a strict no-native-code build.
- Burn enables experiments and future custom/trained models without claiming general GGUF compatibility; its initial checkpoint workflow is more specialized and generation is buffered.
- Model and tokenizer files must be paired explicitly, preventing silent tokenizer substitution.
