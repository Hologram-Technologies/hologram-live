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
logical edges and layer positions remain distinct. Planning defaults to at most
256 logical layers, 64 logical applications, a maximum child depth of 16, 512
unique resolved objects, and 4 GiB of cumulative resolved bytes across the
complete tree. Child manifests, requested and delegated capability objects,
and layers use the same verified κ resolver. Traversal is iterative, detects a
repeated application κ on an ancestor path, and retains distinct logical edges
when physical objects are deduplicated.

`ApplicationPlan` is the strict, executable form: construction fails if any
required root or child object is missing, malformed, over a limit, or has an
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

### Completion and exit amendment (M3.1)

Provider invocation returns byte outputs and a separate typed completion. A
completion is either `returned`, meaning the callable returned successfully but
has no process status, or `exited { code }`, meaning the provider observed a
real numeric exit status. Providers may not infer a status from output bytes.
The import-free core-Wasm v1 provider always reports `returned`; the direct
Python OCI provider reports `exited { code: 0 }` only after its container
processes actually succeed. Traps, nonzero provider processes, protocol errors,
and lifecycle failures remain typed operation errors and do not produce a
successful completion.

Only the root application's declared primary is invoked, so only its completion
can become the application-run completion. Child primaries and non-primary
layers are lifecycle-managed dependencies and never supply a competing exit
status. Wasm and rootfs are currently exit-bearing roles. View, Tensor, and
InferenceModel layers are explicitly non-exit-bearing and cannot be selected as
the runtime primary. A direct run returns completion only if primary invocation
and reverse-order shutdown both succeed. A resident `returned` or `exited`
completion ends that request but leaves the admitted application resident.

Prepare or start failure in any dependency fails the application transaction
and rolls it back. An observed autonomous non-primary failure after startup must
transition the application to `failed`, fail active or subsequent invocation,
and trigger reverse-order cleanup; it must not be reinterpreted as primary
completion. The current connected non-primary providers do not yet expose an
autonomous failure callback, so that notification mechanism must land with the
first such provider rather than being simulated.

`HoloRunResult` requires completion as `{ "kind": "returned" }` or
`{ "kind": "exited", "code": N }`. Missing completion is a protocol error.

Provider-owned resident handles remain opaque and are stored by the runtime.
Shared status reports expose lifecycle state, resident bytes, and typed failure
details. Existing bounded mailboxes and backpressure remain mandatory.

### Capability admission amendment (M2)

`AppManifest.requires` is an application-controlled request. Planning must
resolve it, prove its κ, require canonical upstream `CapabilitySet` bytes, and
decode it into typed requested capabilities; it must never promote those bytes
into authority. Execution constructs a separate `EffectiveGrant` from trusted
host context and applies upstream `Capabilities::admits` after complete
non-child resolution but before any provider `prepare` call. Providers receive
only that effective grant.

Strict-contract amendment (2026-08-26): requested and delegated capability
objects always require canonical upstream encoding. The canonical empty set is
the deny-all representation; a zero-byte object is malformed even when its κ
matches.

Ordinary direct and local-service execution use the canonical empty local
baseline, which grants no storage roots, channels, or network flags. A direct
local file may opt into a source-schema grant through the explicit
`--development-grant` flag. A resident runtime may use
`holo.development_grant` from host configuration only while listening on
loopback. Catalog and remote run requests carry no grant field and cannot
self-assert authority. For each child edge, strict planning retains distinct
canonical delegated and requested capability objects. Execution walks those
edges parent-before-child: the trusted parent grant must admit the delegation,
and that delegation must admit the child's request. Only then does the
delegation become the child's effective grant. Amplification and under-granted
requests fail before any root or child provider prepares.

Denial returns `LIVE_AUTHORIZATION_DENIED` with application, request, and grant
identities plus aggregate non-secret capability facts. Allow/deny traces use
the same identities and trusted-source label. Successful raw run results expose
the request κ, effective-grant κ, grant source, and allow outcome additively in
JSON and Protobuf/gRPC; capability source documents and secret values are never
included.

Every evaluated application request and child delegation also crosses an
awaited JSONL audit boundary before provider preparation. Its typed record
contains the authenticated principal, relation, application and optional parent
application κ, requested or delegated capability κ, effective-grant κ, trusted
grant-source label, and allow/deny outcome. It never contains raw capability
documents, storage roots, channels, tokens, authorization headers, or payload
bytes. Direct execution uses the stable `local-cli` principal; service
execution uses the authenticated request principal. Allowed execution fails
closed if persistence fails. A denied authorization remains the primary typed
error if recording that denial also fails, with the audit failure emitted as an
error trace. Run and resident records carry the non-secret authorization
evidence additively across JSON/HTTP and Protobuf/gRPC.

### Child lifecycle amendment (M2)

The lifecycle order is a depth-first pre-order projection of the canonical
application tree. For each application, its layers appear in manifest order,
followed by each child subtree in child-manifest order. Preparation and start
both use this same flattened order. Normal stop and every prepare/start
rollback use its exact reverse. Thus a parent layer is active before its first
child, siblings remain in manifest order, and deeper descendants are owned by
their nearest parent without becoming independent resident applications.

Every provider context names the logical application κ and receives only that
application's admitted effective grant. Layer positions remain local to their
application, so lifecycle records pair application index/κ with layer position
instead of treating position as globally unique. Status aggregates all root
and child layers. Loading or direct execution succeeds only after the entire
tree starts; unload and one-shot cleanup stop the entire tree.

Only the root manifest's primary layer is invoked by the current application
call. Child primaries are lifecycle-managed dependencies and are not
automatically invoked. A child prepare or start failure fails the parent start
and rolls back the complete prepared prefix. A child stop failure contributes
to the parent's stop failure. A root invocation failure remains primary while
reverse-tree cleanup failures are attached as rollback diagnostics. This is
the current exit-propagation boundary until a future guest contract introduces
an explicit child invocation or supervision channel.

## Consequences

- Fat and thin archives can be correlated by `application_kappa` without
  conflating their physical object IDs or footer integrity values.
- Inspection and planning remain possible without initializing an execution
  engine.
- Missing or unsupported non-primary layers prevent all application starts.
- Existing direct and resident Wasm paths must migrate through the same plan
  before additional providers are enabled.
- Child applications execute only after complete closure resolution and
  transitive grant attenuation; they share the parent's transactional lifetime
  and are not independently addressable resident records.
- An archive cannot grant authority to itself, and remote callers cannot opt
  into the local development escape hatch.
- The async, `Send` provider contract may require actor adapters for
  thread-affine desktop or platform engines, but keeps the runtime boundary
  deterministic and portable.
