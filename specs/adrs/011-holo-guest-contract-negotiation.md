# ADR 011: `.holo` guest contracts use the Wasm layer auxiliary tag

- Status: accepted and implemented for import-free Component v1
- Date: 2026-08-25

## Context

The canonical `AppManifest` identifies each layer with its kind, content κ,
callable `entry`, and kind-specific `aux` string. Core-Wasm v1 predates explicit
contract negotiation, so Wasm `aux` is empty and the runtime interprets the
module with its import-free `core-wasm-v1` contract. A component-model ABI
cannot safely be inferred from an export name, filename, archive extension, or
the module bytes. Those values either are not canonical application identity or
do not distinguish the callable from its host contract.

Adding another field to the canonical realization would require a new upstream
codec and migration. An archive extension is unsuitable because extensions are
packaging metadata and are deliberately excluded from application identity.
Adding a new layer kind for every ABI would also turn version negotiation into
an unbounded format-enum problem.

## Decision

The existing canonical Wasm layer `aux` string is the guest-contract selector.
Its values are exact, namespaced identifiers:

- empty string: legacy alias for `hologram:guest/core-wasm@1`;
- `hologram:guest/core-wasm@1`: explicit core-Wasm v1;
- `hologram:guest/component@1`: Hologram Component Model v1.

The empty alias remains the compiler default while core-Wasm v1 is current, so
existing compatible archives retain their canonical bytes and application κ.
New component archives must carry the explicit component identifier. Source
manifest schema v4 exposes this as a separate `contract` field; the compiler
maps it to `Layer.aux`. The callable `entry` remains only the function or exported
interface selection and is never parsed as a version.

The coordinated upstream validation change landed in
`Hologram-Technologies/hologram` PR 142, merge `c5e33ec`. It exports
`WASM_CONTRACT_CORE_V1`, `WASM_CONTRACT_COMPONENT_V1`, and
`Layer::wasm_with_contract`, while `Layer::wasm` retains the empty compatibility
tag. Wasm `aux` changed
from “must be empty” to “empty or a well-formed supported contract identifier.”
The canonical codec does not change. Older runtimes already reject non-empty
Wasm `aux`, which is the required fail-closed behavior. Live pins the merge and
can emit either accepted explicit identifier.

### Negotiation

Planning resolves the effective contract before provider preparation:

1. normalize empty Wasm `aux` to `hologram:guest/core-wasm@1`;
2. look up the exact `(LayerKind::WasmCodemodule, contract)` pair in a
   closed runtime registry;
3. validate the payload against that contract before any layer starts;
4. reject an unknown contract without falling back to core Wasm, WASI, or an
   older major version.

The identifier's major version fixes the canonical ABI. Compatible
implementation fixes within a major do not change the identifier. A breaking
world, import set, type, lifecycle, or completion change requires a new major
identifier. Component type checking remains exact even when the registry
supports that major.

`core-wasm@1` continues to use the contract in ADR 004. `component@1` uses the
WIT package and `application` world checked into
[`../wit/hologram-application-v1.wit`](../wit/hologram-application-v1.wit). It
exports one `run` function with one `list<u8>` input and one `list<u8>` output.
It imports no host interfaces. A successful result maps to application
completion `returned`; a guest error or component trap is a typed operation
error. Component v1 does not create a numeric process exit status.

### Host-interface admission

The base `component@1` world imports nothing. Future profiles must use a new
contract identifier and declare a fixed import set. The linker is constructed
only after the application request has been admitted against the trusted
effective grant. There is no ambient fallback.

| Proposed interface | Required canonical capability | v1 disposition |
| --- | --- | --- |
| `hologram:host/store.read` | target κ is under `storage_roots` | withheld from base world |
| `hologram:host/store.write` | target κ is under `storage_roots` and `storage_quota_bytes > 0` | withheld from base world |
| `hologram:host/channel.publish` | channel κ is in `publish_channels` | withheld from base world |
| `hologram:host/channel.subscribe` | channel κ is in `subscribe_channels` | withheld from base world |
| `hologram:host/network.fetch` | `network_fetch` | withheld from base world |
| `hologram:host/network.announce` | `network_announce` | withheld from base world |
| Wasm memory and execution | `memory_max_bytes`, `cpu_time_per_event_ms`, and `priority_weight` | runtime ceilings ship in base v1; nonzero admitted memory/time scalars only tighten them; priority scheduling remains deferred |
| WASI filesystem preopens | `storage_roots` plus `storage_quota_bytes` for writes | no ambient directories; deferred profile only |
| WASI HTTP or outbound sockets | `network_fetch` | raw sockets withheld; deferred mediated profile only |
| WASI listen sockets | `network_announce` | raw sockets withheld; deferred mediated profile only |
| WASI clocks, random, environment, args, stdio, DNS, secrets, or process control | no canonical field | unavailable until the capability schema is extended |
| Hologram inference or model sessions | no canonical field | unavailable until a scoped model capability exists |

`network_fetch` and `network_announce` are currently booleans, so they are not
sufficient authority for unrestricted raw sockets. A future profile should
prefer mediated Hologram interfaces or first introduce endpoint-scoped
capabilities. Invocation input/output replaces WASI stdin/stdout, and the
runtime does not inherit host arguments or environment variables.

Python uses this unchanged base world. The source compiler pins
`componentize-py 0.25.0` and passes `--stub-wasi`, which replaces CPython's WASI
imports inside the generated guest rather than linking them in Live. ADR 012
admits SHA-256-locked platform-independent wheels through a private build path;
those compiler inputs do not add runtime imports. The result is type-checked
and executed like any other import-free Component v1 payload. The stubbed PRNG
seed is deterministic only within one built component and must not be treated
as secure randomness or reproducible build input.

### Diagnostics

- Known but runtime-unsupported contract: `LIVE_CAPABILITY_MISSING`, naming the
  contract and layer position. Component v1 is supported; future known profiles
  may use this status until their providers land.
- Unknown source identifier: `LIVE_CONFIG_INVALID` before archive emission.
  Unknown canonical manifest identifier: `LIVE_HOLO_INVALID` during manifest
  validation. Neither reaches provider preparation.
- Known contract with a malformed payload, wrong world, wrong export, or
  undeclared import: `LIVE_PROTOCOL_ERROR` during preparation.
- Declared import whose required authority is absent from the admitted grant:
  `LIVE_AUTHORIZATION_DENIED` before linker construction or instantiation,
  naming the interface and capability field but not secret grant contents.
- A host interface with no canonical capability mapping:
  `LIVE_CAPABILITY_MISSING`; it is never linked optimistically.

Direct and resident modes use the same normalization, registry, type checks,
admission mapping, and error codes.

## Consequences

- Guest-contract choice becomes part of canonical application identity without
  changing the existing manifest codec.
- Existing core-Wasm archives and κ values remain unchanged.
- Component archives fail closed on older runtimes and never silently execute
  under the core-Wasm ABI.
- The import-free Component provider, resource enforcement, and locked
  pure-Python wheel packaging are current capabilities. Native Python packages
  and any capability-gated WASI profile remain explicit follow-up work.
- New host authority requires both a versioned contract profile and a canonical
  capability field before an import can be linked.
