# Current sprint: M3.1a bounded Component v1 execution

## Sprint status

- State: active
- Started: 2026-08-25
- Last reviewed: 2026-08-25
- Durable milestone: [M3.1a — Component-model and Python/WASI proof](plans/holo-application-runtime.md#m31a-component-model-and-pythonwasi-proof)
- Goal: execute the import-free `hologram:guest/component@1` contract directly
  and resident without weakening core-Wasm v1 or granting ambient WASI
- Exit signal: a valid Component v1 echo application executes on both targets,
  malformed components and guest errors are typed, and memory, fuel, input,
  output, deadline, and cancellation bounds are enforced before the provider is
  advertised as available

The completed selector tracker remains in Git history. Durable requirements
stay in [`plans/holo-application-runtime.md`](plans/holo-application-runtime.md),
and the accepted negotiation design is
[`adrs/011-holo-guest-contract-negotiation.md`](adrs/011-holo-guest-contract-negotiation.md).

## Runtime policy

- Component v1 is the exact exported
  `hologram:application/application@1.0.0` world and has no imports.
- Each input is instantiated in a fresh Wasmtime store; compiled components may
  remain warm, but guest memory and state never cross invocation boundaries.
- Runtime-owned ceilings apply even when capability scalar `0` means
  unspecified: 64 MiB linear memory, 100 million fuel units, 1 MiB input, 1 MiB
  output, and a two-second wall deadline per invocation.
- A nonzero admitted `memory_max_bytes` or `cpu_time_per_event_ms` may only
  tighten the runtime ceiling; archive requests never expand host authority.
- Fuel is the deterministic compute bound. Epoch interruption terminates a
  timed-out or cancelled synchronous call, and each prepared component owns an
  engine plus serialization boundary so interruption is isolated.
- No WASI, filesystem, network, clock, randomness, environment, or Hologram
  host interface is linked in this slice.
- Legacy `hologram:guest/core-wasm@1` execution and canonical bytes remain
  unchanged.

## Implementation

- [x] Add the generated Component v1 host bindings to the runtime crate.
- [x] Add a Component provider selected only by the exact Component v1 tag.
- [x] Register the provider for direct and resident plans.
- [x] Compile and type-check the component during transactional preparation.
- [x] Execute one WIT `run` call per input and preserve one output per input.
- [x] Map guest-declared errors, malformed components, and traps to typed Live
  errors without exposing payload bytes.
- [x] Keep resident components compiled and warm while giving every input a
  fresh store.

## Resource and cancellation gates

- [x] Enforce the memory ceiling with Wasmtime's store limiter.
- [x] Enforce fuel for every fresh store.
- [x] Reject oversized inputs before guest allocation.
- [x] Reject oversized outputs before returning them to the application.
- [x] Enforce the wall deadline and interrupt the guest on expiry.
- [x] Interrupt in-flight guest execution when the invocation future is
  cancelled.
- [x] Prove a timed-out/cancelled call cannot interrupt another application or
  the core-Wasm provider.

## Conformance and delivery

- [x] Add a valid echo Component fixture and direct execution proof.
- [x] Add resident load/invoke/unload execution proof.
- [x] Add malformed contract, guest error, input/output, fuel, memory, deadline,
  and cancellation tests.
- [x] Update README, architecture, security, actual-capabilities, and website
  Component documentation.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Run formatting, focused tests, all-target tests, Clippy, BDD, optimized
  build, release smoke, WIT conformance, and docs.
- [ ] Land the reviewable PR and return the repository to clean `main`.

## Deferred discoveries

- [ ] `DISC-017` — Package and execute dependency-free Python through the
  Component v1 provider once the bounded host contract is proven.
- [ ] `DISC-018` — Define a capability-gated WASI profile only after the
  import-free provider works directly and resident.
