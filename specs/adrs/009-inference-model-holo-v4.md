# ADR 009: Adopt `.holo` v4 inference-model layers before connecting a provider

## Status

Accepted.

## Context

Hologram Live originally pinned the v3 archive contract, whose closed layer set was Wasm, tensor plan, root filesystem, and view. The sibling `hologram-ai` project compiles pinned Hugging Face or local model sources through `uor-r4`, builds a deterministic R4 inference bundle, and publishes a complete `.holo` v4 archive containing a non-exit-bearing `InferenceModel` service layer. Live's existing weightc chat engine instead consumes imported `.wcpu` directories and does not define a deterministic `.holo` payload.

Treating either artifact as a `TensorPlan` would erase the distinction between a general tensor program and a callable model service. Copying a `.wcpu` directory into an archive without a canonical bundle contract would also make identity depend on filesystem traversal and packaging choices.

## Decision

Live upgrades its archive/space dependency to the additive v4 contract. Readers continue accepting v2 and v3; new writers emit v4. An `InferenceModel` layer carries:

- `content`: the κ of one opaque, provider-owned model bundle;
- `entry`: a unique callable service name such as `ai.default`;
- `aux`: the engine identifier, exposed by Live as `engine`;
- no primary/exit semantics.

Live can import, verify, cache, list, and inspect complete model `.holo` files produced by `hologram-ai`. `hologram ai inspect` reports service metadata without loading an engine. The low-level source manifest also accepts a prebuilt `inference-model` payload for archive assembly, but model-source acquisition, R4 compilation, and bundle validation remain owned by `hologram-ai`.

Until a typed model provider is connected, `hologram run` and resident load return `LIVE_CAPABILITY_MISSING` naming the declared service and engine. Live does not simulate inference and does not reinterpret model layers as executable primaries.

## Consequences

- Existing v2/v3 archives remain readable, while newly compiled non-model applications are physically v4 archives.
- The verified application directory gains an optional `engine` field and the gRPC representation adds the same field additively.
- `hologram-ai` is the preferred compiler/runtime path for R4G1 model archives. A future adapter may connect that facade to Live's chat and OpenAI/Ollama surfaces.
- Weightc remains an existing chat provider over `.wcpu`. Packaging it in `InferenceModel` requires a deterministic single-blob contract and provider validation; this ADR does not invent one.
- A future `hologram ai infer` command must invoke a real provider, enforce bundle and engine compatibility, and preserve model-session lifecycle semantics.
