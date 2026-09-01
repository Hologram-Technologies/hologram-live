# ADR 011: `.holo` guest contracts use the Wasm layer auxiliary tag

- Status: accepted and implemented for import-free, store-read, store-write, channel-publish, and channel-subscribe Component v1 profiles
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

- empty string: invalid;
- `hologram:guest/core-wasm@1`: explicit core-Wasm v1;
- `hologram:guest/component@1`: Hologram Component Model v1;
- `hologram:guest/component-store-read@1`: Component Model v1 with the fixed
  mediated object-store read import.
- `hologram:guest/component-store-graph-read@1`: Component Model v1 with the
  same fixed read-only import over a complete bounded typed realization
  closure.
- `hologram:guest/component-store-write@1`: Component Model v1 with the fixed
  mediated content-addressed object-store write import.
- `hologram:guest/component-channel-publish@1`: Component Model v1 with the
  fixed mediated channel publish import.
- `hologram:guest/component-channel-subscribe@1`: Component Model v1 with the
  fixed mediated channel subscribe import.

The empty alias remains the compiler default while core-Wasm v1 is current, so
existing compatible archives retain their canonical bytes and application κ.
New component archives must carry the explicit component identifier. Source
manifest schema v4 exposes this as a separate `contract` field; the compiler
maps it to `Layer.aux`. The callable `entry` remains only the function or exported
interface selection and is never parsed as a version.

The coordinated upstream validation change landed in
`Hologram-Technologies/hologram` PR 142, merge `c5e33ec`. It exports
`WASM_CONTRACT_CORE_V1`, `WASM_CONTRACT_COMPONENT_V1`, and
`Layer::wasm_with_contract`; the upstream `Layer::wasm` helper still produces
an empty tag, but Live neither emits nor accepts it. Wasm `aux` changed upstream
to admit the supported contract identifiers without changing the canonical
codec. Upstream PR 143, merge `aad544c`, adds
`WASM_CONTRACT_COMPONENT_STORE_READ_V1`; Live pins that merge and requires one
of the accepted explicit identifiers. Upstream PR 144, merge `059c39f`, adds
`WASM_CONTRACT_COMPONENT_STORE_WRITE_V1` under the same closed validation rule;
Live pins that merge. Upstream PR 145, merge `4fac0b3`, adds the publish and
subscribe selectors; Live pins that merge as well. Upstream PR 146, merge
`01c29de`, adds the distinct graph-read selector so exact-root grants are not
reinterpreted by newer runtimes; Live pins that merge.

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

`component-store-read@1` uses the `application` world under
[`../wit/store-read/`](../wit/store-read/). It retains the same guest `run`
shape and imports exactly `hologram:host/store@1.0.0`, whose `read` function
takes one object κ and returns its bytes or a public error string. It does not
import WASI or any other Hologram interface.

`component-store-graph-read@1` deliberately reuses that exact WIT world and
single read-only host import. Its distinct canonical selector changes only the
authority interpretation: provider preparation must resolve the admitted roots
through registered typed UOR realization edges before the import is linked.
This keeps guest binaries ABI-compatible while preventing exact-root archives
from silently gaining descendant access.

`component-store-write@1` uses the separate `application` world under
[`../wit/store-write/`](../wit/store-write/). It retains the same guest `run`
shape and imports exactly `hologram:host/store-write@1.0.0`. Its `write`
function accepts an expected object κ and bytes and returns success only when
the host safely materializes that exact content address. Keeping a separate
interface avoids changing the shipped `hologram:host/store@1.0.0` read shape or
granting reads to a write-only guest.

`component-channel-publish@1` and `component-channel-subscribe@1` use separate
worlds under [`../wit/channel-publish/`](../wit/channel-publish/) and
[`../wit/channel-subscribe/`](../wit/channel-subscribe/). The first imports
only `hologram:host/channel-publish@1.0.0`; the second imports only
`hologram:host/channel-subscribe@1.0.0`. Both retain the same guest `run` shape.

### Host-interface admission

The base `component@1` world imports nothing. Every host-enabled profile uses a
new contract identifier and a fixed import set. The linker is constructed only
after the application request has been admitted against the trusted effective
grant and profile-specific required fields have been checked. There is no
ambient fallback. For `component-store-read@1`, at least one requested
`storage_roots` entry is required. Admission proves those roots are contained
by the effective grant; the host retains only that admitted intersection and
checks every requested κ before touching the object store. The current safe
slice serves an explicitly named root itself. For
`component-store-graph-read@1`, the same admission first supplies the exact
root set to a local breadth-first resolver. Every first-seen object is re-hashed
against its κ. Only registered canonical realization IRIs contribute edges;
unknown or untagged objects are opaque leaves. A claimed registered type must
have a complete canonical frame. Missing members, malformed frames, or the
host's depth, object, edge, or aggregate-byte ceiling fail before linker
construction, and public errors omit object identities. The complete resolved
closure becomes the read set for the prepared lifetime. Child attenuation
remains the exact subset relation over roots; resolving a root never lets a
child request a root its delegated grant did not contain. No network resolver
is consulted. For `component-store-write@1`, the
request must also contain a nonzero `storage_quota_bytes` value. The provider
retains the exact admitted roots and a lifetime-shared remaining quota before
linker construction. Each call checks exact-root membership, verifies that the
bytes hash to the caller-supplied κ, and atomically materializes a missing blob
only when its new bytes fit the remaining quota. Existing identical blobs cost
no additional quota; rejected root, hash, quota, or store operations do not
write a partial blob. Public errors do not include the target κ.

Channel preparation similarly requires at least one exact entry in the
profile's requested `publish_channels` or `subscribe_channels` set. Admission
has already proved that request is contained by the trusted effective grant or
attenuated child grant. Every host call compares the supplied channel κ against
that retained set before touching the broker, and public failures omit the κ.

The v1 broker is runtime-owned and host-neutral. Each exact channel is a FIFO
work queue with a 64-message mailbox and a 64 KiB per-message limit. Publish is
nonblocking: it enqueues once or returns explicit backpressure without dropping
or overwriting an earlier message. `try-receive` is also nonblocking: it removes
and returns one oldest message or returns `none`. Concurrent subscribers
therefore compete atomically and delivery is at-most-once. V1 deliberately has
no broadcast, replay, acknowledgement, durable persistence, cross-process
transport, or ordering guarantee across different channels. A broker lives for
its owning executor/runtime; direct executions made through one executor and
all applications in one resident runtime share it. Broker destruction discards
unconsumed messages. Because receive never waits or registers a waiter,
invocation cancellation and stop use the existing Component epoch boundary and
leave no pending subscription state.

| Proposed interface | Required canonical capability | v1 disposition |
| --- | --- | --- |
| `hologram:host/store.read` | target κ is an admitted `storage_roots` entry | shipped only in `component-store-read@1` |
| `hologram:host/store.read` over typed closure | target κ is reachable from an admitted `storage_roots` entry through registered canonical realization edges | shipped only in `component-store-graph-read@1`; complete local bounded resolution before linking |
| `hologram:host/store-write.write` | target κ is an admitted `storage_roots` entry and newly materialized bytes fit `storage_quota_bytes` | shipped only in `component-store-write@1` |
| `hologram:host/channel-publish.publish` | channel κ is in `publish_channels` | shipped only in `component-channel-publish@1` |
| `hologram:host/channel-subscribe.try-receive` | channel κ is in `subscribe_channels` | shipped only in `component-channel-subscribe@1` |
| `hologram:host/network.fetch` | target is contained by `network_fetch_endpoints` | withheld pending a separate mediated profile |
| `hologram:host/network.announce` | target is contained by `network_announce_endpoints` | withheld pending a separate mediated profile |
| Wasm memory and execution | `memory_max_bytes`, `cpu_time_per_event_ms`, and `priority_weight` | runtime ceilings ship in base v1; nonzero admitted memory/time scalars only tighten them; priority scheduling remains deferred |
| WASI filesystem preopens | `storage_roots` plus `storage_quota_bytes` for writes | no ambient directories; deferred profile only |
| WASI HTTP or outbound sockets | no raw-socket capability | raw sockets withheld; deferred mediated profile only |
| WASI listen sockets | no raw-socket capability | raw sockets withheld; deferred mediated profile only |
| WASI clocks, random, environment, args, stdio, DNS, secrets, or process control | no canonical field | unavailable until the capability schema is extended |
| Hologram inference or model sessions | no canonical field | unavailable until a scoped model capability exists |

ADR 020 replaces the former booleans with canonical HTTPS endpoint scopes and
path-prefix attenuation. No network interface is linked in this slice. A future
profile must remain mediated and bounded; endpoint authority never implies raw
sockets. Invocation input/output replaces WASI stdin/stdout, and the runtime
does not inherit host arguments or environment variables.

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
- The import-free, exact object-read/object-write, typed graph-read, and bounded channel
  publish/subscribe Component providers, resource enforcement, and locked
  pure-Python wheel packaging are current capabilities. Native Python packages,
  transitive graph writes, durable/distributed messaging, and any
  capability-gated WASI profile remain explicit follow-up work.
- New host authority requires both a versioned contract profile and a canonical
  capability field before an import can be linked.
