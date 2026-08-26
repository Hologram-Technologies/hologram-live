# Current sprint: M3.1a Python Component v1 packaging

## Sprint status

- State: active
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M3.1a — Component-model and Python/WASI proof](plans/holo-application-runtime.md#m31a-component-model-and-pythonwasi-proof)
- Goal: compile a dependency-free, locked Python project into an import-free
  `hologram:guest/component@1` Wasm layer and execute the resulting `.holo`
  directly and resident
- Exit signal: `hologram compile` invokes a pinned isolated Python component
  toolchain, the emitted component passes the exact existing world check, and
  a teaching example executes on both runtime targets without Docker or
  ambient WASI

The completed bounded-Component tracker remains in Git history. Durable
requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
the accepted negotiation design is
[`adrs/011-holo-guest-contract-negotiation.md`](adrs/011-holo-guest-contract-negotiation.md).

## Packaging policy

- The source profile is `wasi-component`, but this slice uses
  `componentize-py --stub-wasi`: every WASI import is replaced inside the guest
  and the emitted component still has the import-free Component v1 contract.
- Pin `componentize-py` to `0.25.0`; invoke it through an isolated `uvx`
  environment with the developer virtual environment, user site, and
  `PYTHONPATH` withheld.
- Require `pyproject.toml`, `uv.lock`, and `src/` under the declared project;
  reject escaping paths and symlinks using the existing Python source boundary.
- Accept dependency-free locks only in this slice. A lock containing an
  additional package fails before componentization with guidance to the next
  pure-Python dependency milestone or the explicit rootfs profile.
- Generate the WIT adapter in a private temporary directory. Application code
  keeps the existing `module:function`, `bytes -> bytes` entrypoint instead of
  importing generated Hologram bindings.
- The output is an ordinary `WasmCodemodule` with entry `run` and contract
  `hologram:guest/component@1`; `.holo` canonical identity and the runtime
  provider registry need no new layer kind or selector.
- `--stub-wasi` makes Python randomness deterministic and unsuitable for
  security-sensitive randomness. Filesystem, network, clocks, environment,
  arguments, stdio, DNS, secrets, and process control remain unavailable.

## Compiler and schema

- [x] Add `wasi-component` to the Python source profile and validate it only on
  schema-v4 Component v1 Wasm layers.
- [x] Add `hologram app init --template python --profile wasi-component` with
  the exact generated layer kind, entry, and contract.
- [x] Compile through pinned `componentize-py 0.25.0 --stub-wasi` and emit its
  component bytes directly as the Wasm layer payload.
- [x] Refuse dependency-bearing locks and ambient Python search paths with
  actionable typed diagnostics.
- [x] Keep `rootfs` source manifests and their OCI compiler behavior compatible.

## Runtime and conformance

- [x] Admit the bounded internal instance/table/memory counts required by the
  bundled CPython component without increasing its 64 MiB byte ceiling.
- [x] Prove the emitted component has no imports and exports the exact
  `hologram:application/application@1.0.0` world.
- [x] Execute the dependency-free Python example directly.
- [x] Import, load, invoke, and unload the same archive resident.
- [x] Prove a dependency lock and malformed Python entry fail before archive
  emission with typed diagnostics.
- [x] Preserve core-Wasm v1 and the Rust Component v1 conformance suite.

## Documentation and delivery

- [x] Add the dependency-free Python Component example and a repeatable demo
  command.
- [x] Update README, website Python guide, architecture/security capability
  matrices, and `.holo` format documentation where the new compiler profile is
  user-visible.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Run formatting, focused tests, all-target tests, Clippy, BDD, optimized
  build, release smoke, WIT conformance, and docs.
- [ ] Land the reviewable PR and return the repository to clean `main`.

## Deferred discoveries

- [ ] `DISC-017b` — Resolve and package a locked pure-Python dependency into an
  isolated Component v1 build path.
- [ ] `DISC-018` — Define a capability-gated WASI profile only when an
  application needs real host interfaces; do not broaden the import-free base
  world.
