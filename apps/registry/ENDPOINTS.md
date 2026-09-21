# Endpoints: what Hologram Registry reuses, and what it adds

Rule: the registry adds an endpoint only where a protocol it must speak fixes the path. Everything else uses what
Hologram Live already serves. Checked against `Hologram-Technologies/hologram-live` main at `b359ff1`
(33 HTTP paths found by search, one gRPC service).

## Added: the registry protocol, and nothing else

| Path | Why it cannot be an existing endpoint |
|---|---|
| `/v2/`, `/v2/_catalog`, `/v2/<name>/tags/list`, `/v2/<name>/manifests/<ref>`, `/v2/<name>/blobs/<digest>`, `/v2/<name>/blobs/uploads/[<uuid>]`, `/v2/<name>/referrers/<digest>` | `docker`, `containerd`, `helm`, `oras` and `cosign` have these paths compiled in. "Change the image name and nothing else" means the client side cannot change, so neither can the paths. They are one module, `dev.hologram.live.oci`, mounted as three router entries. |

## Reused: no new endpoint

| Need | Existing endpoint | Where the reference puts it | State |
|---|---|---|---|
| Health, for the image's `HEALTHCHECK` and for Kubernetes probes | `GET /healthz` | `:5001/debug/health`, a second port | Works. The registry volume is opened before the listener binds, so a registry that cannot serve never reports healthy. |
| API reference | `GET /openapi.json`, `GET /docs` | none | Done: the module contributes its eight paths (`src/modules/oci/openapi.rs`). |
| Is the registry on, and at which version | `GET /api/v1/modules`, `GET /api/v1/capabilities` | none | Works; held by `tests/oci_http.rs`. |
| Operator commands (`garbage-collect`, `import`, `verify`) | the CLI, and for a running server the existing operations channel (gRPC `hologram.live.v1`) | the CLI | Planned in P8. No HTTP admin route is added; the plan's `admin.rs` is dropped. |
| Log level at run time | the existing tracing operations | a restart | Works, unchanged. |
| Login for everything that is not the registry | the existing bearer token | none | Works, unchanged; `/v2/` authenticates itself because its clients need the registry's own challenge (ADR 026). |

## Not there in either: to be added once, for the whole server

| Need | Note |
|---|---|
| `GET /metrics` | Hologram Live has no metrics route today (the only mention is a test of the 404). The reference serves Prometheus metrics on its debug port. This belongs to the server, not to the registry module: one route, all modules report into it. |

## One store behind both APIs: a decision, not yet taken

`/api/v1/objects`, `/api/v1/files`, `/api/v1/holo` and `/api/v1/models` read and write through one seam,
`RegistryProvider` (five methods; two implementations today: the local BLAKE3 store and the remote Kappa client).
The registry volume could be a third. Then a layer pushed with `docker push` is readable at
`/api/v1/objects/<id>`, a model stored through the object API is pullable with `docker pull`, and the hub, which
speaks the object API, moves to the new store without changing a call.

What stands in the way: that seam passes whole objects in memory (`put_object(bytes)`), and the registry's rule is
that no layer is ever in memory. So the provider would be limited to objects under a size cap, or the seam would
gain a streaming pair first. Recommended after the v1 gates are green: it changes no `/v2/` behaviour and it is the
cleanest route for the hub swap.
