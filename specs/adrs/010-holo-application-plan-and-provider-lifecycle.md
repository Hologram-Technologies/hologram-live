# ADR 010: Runtime-owned application plans and transactional provider lifecycle

## Status

Accepted.

## Context

A `.holo` archive has three related but non-interchangeable identities. It can
also describe several ordered layers and child applications even though the
current runtime resolves and starts only one primary Wasm layer. Continuing to
resolve payloads inside individual executors would make completeness depend on
the selected provider, allow a non-primary missing object to be discovered only
after another layer starts, and couple archive parsing to Wasmtime or a future
model, view, tensor, or rootfs engine.

The application-directory extension is useful for inspection, but it is a
derived index. Making it execution truth would create two manifests. Likewise,
a read-only planning command must be able to explain missing content and
unsupported providers, while execution must never accept those conditions.

Provider work may perform asynchronous I/O and must remain usable by the native
multi-threaded runtime. At the same time, provider-specific engine types must
not leak into shared planning and protocol models.

## Decision

### Identity vocabulary

Hologram Live uses these exact names:

- **archive object κ** (`archive_kappa`, and the existing inspection field
  `kappa`) is `blake3` over every byte of the physical archive. Fat and thin
  variants normally have different values. Catalog lookup, import, verify,
  remove, load, resident records, and run results use this identity because
  they operate on a particular stored archive.
- **archive footer fingerprint** (`archive_fingerprint`) is the integrity value
  read from the archive footer. It verifies the physical section layout but is
  not a catalog object ID or an application ID.
- **application κ** (`application_kappa`) is `address_bytes` over
  `AppManifest::canonicalize()`. It is stable when packaging metadata or
  embedded content changes without changing the canonical manifest. It is
  absent for structural archives with no application manifest.

The core `HoloIdentity` record contains all three values for a compiled
application. Inspection exposes `application_kappa` additively and retains
`kappa` with its existing physical meaning. Human text, JSON, errors, traces,
and audit fields must use the explicit names when more than one identity is in
scope. Errors about a stored or resident variant name `archive_kappa`; planning
and manifest-graph errors name `application_kappa`; content failures name the
missing content κ and its referring edge. No existing `kappa` field may be
silently repurposed.

### Plan ownership and resolution

The runtime owns an `ApplicationPlan`. It is built from the verified archive
and the canonical `AppManifest`, not from CLI arguments, HTTP representations,
or the application directory. A planning attempt decodes and validates the
manifest once and records:

- the identity record and verified manifest;
- every ordered layer, its closed `LayerKind`, content κ, entry, auxiliary
  value, primary status, resolved bytes, and resolution source;
- the required capability-set object;
- child application and delegated-capability edges;
- provider selection and typed blockers.

Resolution order is embedded content followed by the configured local content
store. Network, registry, or peer fetching requires an explicitly configured
future resolver and is never an implicit fallback. Every resolved payload is
re-hashed before use. Equal κ values share one resolved object while their
logical edges and layer positions remain distinct. Root planning defaults to at
most 256 layers, 512 unique resolved objects, and 4 GiB of cumulative resolved
bytes. Recursive child planning will additionally enforce a maximum depth of
32, the cumulative root object/byte limits, and cycle detection when M2 defines
capability attenuation; until then, child edges are visible blockers and are
not ignored.

`ApplicationPlan` is the strict, executable form: construction fails if any
required non-child object is missing, malformed, over a limit, or has an
unsupported provider. A separate serializable plan report is explanatory. It
may contain blockers and partial resolution facts so `hologram holo plan` can
remain useful for an application that cannot yet execute. Execution can begin
only from a blocker-free strict plan.

### Provider boundary

Providers are selected by the closed upstream `LayerKind`; arbitrary strings
do not select code. The shared provider boundary receives resolved layer data,
the effective capability grant, resource budgets, and narrow host interfaces.
It does not expose Wasmtime, weightc, WebView, container, or microVM types.

Native provider operations are asynchronous, object-safe, `Send`, and
`Sync`. Returned futures are `Send` so lifecycle work may move across Tokio
worker threads. Platform adapters that cannot satisfy this contract must stay
behind a platform-local actor which implements the native boundary; they do not
weaken the shared contract.

The lifecycle phases are:

1. `prepare` validates provider compatibility and constructs inert prepared
   state without making the layer externally available;
2. `start` activates prepared state;
3. `invoke` handles a named callable entry, while `attach` represents a
   non-invoked surface where appropriate;
4. `stop` releases running or prepared state and is safe to retry where the
   provider can make it idempotent.

Preparation and start follow manifest order. Normal stop follows reverse
manifest order. If prepare or start fails, all previously prepared or started
layers are stopped in reverse order; the original failure remains primary and
rollback failures are attached as diagnostics. No provider is prepared until
the whole strict plan has resolved. Unsupported kinds fail before any start.
The primary exit-bearing layer supplies application exit status; non-exit
layers do not invent one.

Provider-owned resident handles remain opaque and are stored by the runtime.
Shared status reports expose lifecycle state, resident bytes, and typed failure
details. Existing bounded mailboxes and backpressure remain mandatory.

## Consequences

- Fat and thin archives can be correlated by `application_kappa` without
  conflating their physical object IDs or footer integrity values.
- Inspection and planning remain possible without initializing an execution
  engine.
- Missing or unsupported non-primary layers prevent all application starts.
- Existing direct and resident Wasm paths must migrate through the same plan
  before additional providers are enabled.
- Child applications remain explicit blockers in M1 and become executable only
  after M2 defines grants, attenuation, recursion, and exit propagation.
- The async, `Send` provider contract may require actor adapters for
  thread-affine desktop or platform engines, but keeps the runtime boundary
  deterministic and portable.
