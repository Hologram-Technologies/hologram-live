# ADR 004: `.holo` execution is an in-process Wasm runtime

## Status

Accepted.

## Decision

`HoloRuntime` executes `.holo` archives whose primary layer is Wasm. After the runtime resolves the complete non-child application plan, the closed provider registry selects `wasmtime-direct` or `wasmtime-resident` for every Wasm layer. Providers prepare and start in manifest order, the declared primary position receives `run`, and stop or rollback proceeds in reverse order. The primary is not required to be position zero. Resident providers keep each compiled module under a supervised Kameo actor with a bounded mailbox and serve `holo.load`, `holo.unload`, `holo.run`, and `holo.resident`. Each run instantiates a fresh `Store`; residency means modules stay compiled and warm, not that guest state persists between runs.

The guest contract is named `core-wasm-v1` and has no imports or WASI. The
module exports `memory`, `holo_alloc(len: i32) -> i32`, and the function named
by the canonical Wasm layer's manifest `entry` with signature
`(ptr: i32, len: i32) -> i64`. The result packs the output pointer and length.
`holo_run` is the generator default; the
runtime provider does not hard-code it when a manifest entry is available.
Direct and resident providers resolve and type-check the declared entry during
preparation, before any layer starts, and use that same entry for invocation.

Core-Wasm v1 returns one byte output for each input but does not carry a numeric
process exit status. A returned value means successful completion and a trap is
a typed `LIVE_PROTOCOL_ERROR`; the runtime does not reinterpret output bytes as
an exit code. A future ABI and host interfaces require explicit version
negotiation. They must not overload the callable entry string, and imports may
only be linked from an admitted effective capability grant.
[ADR 011](011-holo-guest-contract-negotiation.md) defines that negotiation:
empty canonical Wasm `aux` remains the core-v1 alias, while explicit namespaced
contract identifiers use the same identity-bearing tag and fail closed on
older runtimes.

The upstream `uor-hologram` CPU backend remains excluded (it needs unstable AVX-512 intrinsics), and `tensor`/`rootfs` layers keep returning `LIVE_CAPABILITY_MISSING`. Compiled `weightc` artifacts and mvm microVMs are the candidate future engines for those kinds.

## Consequences

- `hologram run` and resident sessions work for Wasm-layer archives with no external process and no `unsafe` in this crate.
- Guest code is sandboxed by the Wasm boundary; it receives only the run inputs and returns bytes.
- The manifest entry is authoritative application identity and selects only an
  export in the resolved module, never a provider or host function.
- Adding an engine means implementing the async provider boundary for another closed layer kind and registering it for the supported execution target; the operation IDs, wire types, and CLI do not change.
- Per-run instantiation keeps guests stateless; a future session API can add stateful instances without changing the wire contract.
