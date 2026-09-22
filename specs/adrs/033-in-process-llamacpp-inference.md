# ADR 033: llama.cpp may execute in-process in opt-in builds

- Status: accepted
- Date: 2026-09-21
- Amends: ADR 003

## Context

The inference trait isolates chat and compatibility APIs from any one runtime. Ollama, vLLM, and weightc preserve a process or network boundary, but a local llama.cpp integration can provide native token streaming and exact usage without another daemon or HTTP hop.

Loading llama.cpp into the server has real costs: it requires a native C++ toolchain, and a fault in model execution can terminate the same process serving applications, files, and registry traffic. Tests also need real GGUF weights to exercise decode.

## Decision

Add `LlamaCppEngine` behind the off-by-default `llamacpp` Cargo feature. It loads one direct or catalog-imported GGUF model at startup, creates one context per request, and performs blocking decode on dedicated OS threads reached through bounded channels. A semaphore caps concurrent contexts at `llamacpp_max_concurrent_requests`. Tokenization and context validation happen before a streaming response is committed; generated pieces are then forwarded natively with exact prompt and completion token counts.

The stock build does not compile or link llama.cpp. `llamacpp-metal` and `llamacpp-cuda` are explicit GPU-backend variants. Model-backed tests run only when `HOLOGRAM_TEST_GGUF` names a fixture; construction, configuration, and catalog behavior remain unconditionally tested.

## Consequences

- Opt-in builds need CMake, Clang, and a C++ compiler.
- A native crash is a daemon crash; there is no subprocess isolation.
- Loading the model delays startup and consumes the daemon's address space.
- The tokenizer provides exact usage, and decode streams without an HTTP hop.
- The `InferenceEngine` boundary remains the only interface used by callers.
