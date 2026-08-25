# ADR 004: `.holo` execution is an in-process Wasm runtime

## Status

Accepted.

## Decision

`HoloRuntime` executes `.holo` archives whose primary layer is Wasm. The daemon compiles the layer with wasmtime, keeps the compiled module resident under a supervised Kameo actor with a bounded mailbox, and serves `holo.load`, `holo.unload`, `holo.run`, and `holo.resident` against it. Each run instantiates a fresh `Store`; residency means the module stays compiled and warm, not that guest state persists between runs.

The guest contract is core Wasm with no WASI: the module exports `memory`, `holo_alloc(len)`, and `holo_run(ptr, len)`, returning a packed output pointer and length. Contract violations and traps surface as typed errors, never as panics in the daemon.

The upstream `uor-hologram` CPU backend remains excluded (it needs unstable AVX-512 intrinsics), and `tensor`/`rootfs` layers keep returning `LIVE_CAPABILITY_MISSING`. Compiled `weightc` artifacts and mvm microVMs are the candidate future engines for those kinds.

## Consequences

- `hologram run` and resident sessions work for Wasm-layer archives with no external process and no `unsafe` in this crate.
- Guest code is sandboxed by the Wasm boundary; it receives only the run inputs and returns bytes.
- Adding an engine means teaching `HoloRuntime::load` to route another layer kind to a new backend; the operation IDs, wire types, and CLI do not change.
- Per-run instantiation keeps guests stateless; a future session API can add stateful instances without changing the wire contract.
