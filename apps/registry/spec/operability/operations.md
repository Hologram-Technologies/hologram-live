# Operations: what the product owes its operator

2026-09-21. Each section ends in something a test can fail. Reference behaviour cited from `study-distribution.md`.

## 1. Configuration

| | Rule | Check |
|---|---|---|
| Two dialects | The Server reads `live.toml` and `HOLOGRAM_*`. In registry mode it reads `config.yml` and `REGISTRY_*` (`001` `contracts/config-table.md`). Hologram-only registry settings live in `[oci]` / `HOLOGRAM_OCI_*`, never in the reference's namespace | walk test |
| One reference page per dialect | **generated from code**: `hologram config reference --markdown` and `hologram oci settings --markdown`. Every key: type, default, environment name, class (supported, accepted and ignored, refused and why), since which version | `check-docs-settings.sh`: the committed page equals the generated one |
| Validation | `hologram config validate` and `registry serve --dry-run <config>` load, validate and print the effective configuration with secrets masked; exit 0 or 2 | test per refused key |
| Loud refusal | unknown or unsupported key: exit 2, the key, the reason, a link | `001:FR-011` |
| Secrets | never in logs, traces, metrics, `--dry-run` output or `/openapi.json` | a test greps all five for a planted secret |

New Server keys this definition adds to `live.toml` (each with a `HOLOGRAM_*` twin), so the Server owns the machinery and the Registry only maps onto it:

| Key | Default | Registry setting mapped onto it |
|---|---|---|
| `server.tls.certificate`, `server.tls.key` | unset | `http.tls.certificate`, `http.tls.key` |
| `server.tls.minimum` | `1.2` | `http.tls.minimumtls` |
| `server.debug_listen` | unset (listener off) | `http.debug.addr` |
| `server.metrics.enabled`, `server.metrics.path` | `true`, `/metrics` when the debug listener is on | `http.debug.prometheus.*` |
| `server.admin_socket` | `<data_dir>/state/admin.sock` | none |
| `server.health.storage.interval_secs`, `.threshold` | 10, 3 | `health.storagedriver.*` |
| `tracing.fields` | empty | `log.fields` |

## 2. Health, readiness, drain

On the debug listener (`http.debug.addr`; off unless set; the registry image's default file sets `:5001`, as the reference).

| Path | Answers | Use |
|---|---|---|
| `GET /debug/health` | 200 `{}` when every check passes; 503 with a JSON object naming each failed check | liveness and readiness probes, load balancers. Same shape as the reference's |
| `POST /debug/health/down`, `/up` | flips a manual check | drain before maintenance; the chart's `preStop` hook |
| `GET /healthz` on the public listener | unchanged: the Server's own health document | existing clients of the Server |

Checks in v1.0: storage (write, read back, delete a 1 KiB file under the data root every `interval`; fails after `threshold` misses), store open, certificate loaded. v1.1 adds `file`, `http`, `tcp`.

## 3. Metrics

`GET /metrics`, Prometheus text format, on the debug listener. Names follow the reference's `registry_` namespace where it has a counterpart, so existing dashboards draw **[memory: the reference's names; gate G diffs our list against a scrape of the pinned reference and the table below is corrected from it]**.

| Metric | Type | Labels |
|---|---|---|
| `registry_http_requests_total` | counter | `handler`, `method`, `code` |
| `registry_http_request_duration_seconds` | histogram | `handler`, `method` |
| `registry_http_in_flight_requests` | gauge | `handler` |
| `registry_http_request_size_bytes`, `registry_http_response_size_bytes` | histogram | `handler` |
| `registry_storage_action_seconds` | histogram | `action` (`stat`, `open`, `append`, `complete`, `link`, `tag`) |
| `hologram_oci_uploads_in_progress` | gauge | |
| `hologram_oci_upload_bytes_total`, `hologram_oci_blob_bytes_served_total` | counter | |
| `hologram_oci_digest_mismatch_total` | counter | |
| `hologram_oci_store_blobs`, `hologram_oci_store_bytes`, `hologram_oci_repositories` | gauge, refreshed every 60 s | |
| `hologram_oci_verify_damaged_objects` | gauge, set by the last `verify` | |
| `hologram_tls_certificate_not_after_seconds`, `hologram_tls_reload_failures_total` | gauge, counter | |
| `hologram_disk_free_bytes` | gauge | `path` |
| `hologram_build_info` | gauge, value 1 | `version`, `revision`, `features` |
| `process_*` | standard process collector | |

`handler` is the route name from `contracts/registry-api.md` (`blob`, `manifest`, `upload`, `tags`, `catalog`, `referrers`, `base`), never the raw path: repository names must not become label values. A Grafana dashboard JSON and four alert rules (disk under 10%, health 503 for 2 minutes, certificate expires in 14 days, any damaged object) ship in `apps/registry/docs/`.

## 4. Logs

To stdout. `text` or `json`. One access line per request at `info`: time, request id, method, handler, repository, status, bytes in and out, duration, client address, user if any, trace id. Static `log.fields` are added to every line. Level changeable at runtime through `hologram tracing`, by the administration socket. Never logged: passwords, tokens, `Authorization` headers.

## 5. Tracing

OTLP. Standard variables honoured: `OTEL_TRACES_EXPORTER` (`otlp`, `none`), `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL` (`grpc`; `http/protobuf` if the existing exporter supports it, else refused by name), `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`. They override `[telemetry]` in `live.toml`. Inbound W3C `traceparent` is continued. With nothing set, nothing leaves the process: this differs from the reference, which exports to `localhost:4318` unless told not to, and is why its image sets `OTEL_TRACES_EXPORTER=none`. Listed as D-007.

## 6. Backup and restore

| | |
|---|---|
| What to back up | the data root: `kappa/` (blobs, index), `oci/links.redb`, the layout marker. `live/` is disposable |
| Consistent copy, server stopped | copy the directory |
| Consistent copy, server running | `POST /debug/health/down`; set `storage.maintenance.readonly.enabled`; snapshot the volume or `cp -al`; undo both. Blobs are immutable once named, so only the two database files need the read-only window |
| Restore | put the directory back; start; run `hologram oci verify` |
| Logical backup | `skopeo sync` or `oras cp -r` to any registry: proven both ways by gate D |
| Check | gate G `backup.sh`: push during a read-only-window backup attempt is refused; restore elsewhere; every tag pulls; `verify` exits 0 |

## 7. Upgrade and rollback between our own versions

| Release | May change | May not change |
|---|---|---|
| Patch `1.0.x` | fixes; dependency updates | any setting, route, metric name, log field, exit code, on-disk layout |
| Minor `1.x` | new settings with safe defaults; new routes, metrics, log fields; a setting moving from refused to supported; new rows in `DIFFERENCES.md` only if they remove a difference | defaults of existing settings; anything a patch may not. On-disk layout may gain tables, never lose or reinterpret them; a minor must open data of every earlier `1.x` |
| Major `2.0` | anything, with a migration command and a guide | silently |

Downgrade: data touched by `1.(x+1)` either opens in `1.x` or is refused by the layout marker with a message naming both versions. It never half-opens. The OpenAPI document follows the same rule through `oasdiff` (`openapi-policy.md` O7).

Supported versions: the latest minor gets fixes; the previous minor gets security fixes for 6 months after the next one ships. Written with dates in `SECURITY.md`.

Check: gate G `upgrade.sh`: write the gate D corpus with release N-1, start N on it, pull every tag, run `verify`; then start N-1 on the result and assert "works" or "refused by marker".

## 8. Security posture

| Question | Answer |
|---|---|
| What is reachable with no configuration? | Binary: loopback only, `127.0.0.1:11435`. Registry image: port 5000, anonymous read and write, **as the reference**; and 5001 inside the container, not published. Server image: port 11435, every route behind a token generated at first start |
| Why is an anonymous registry on 5000 acceptable? | It is the reference's default and the one-line swap depends on it. The first log line at start says `warning: anonymous push is allowed; see <docs link>` in both text and JSON. The reference says nothing |
| Administration | Unix socket or named pipe under the data root, mode 0600. Not on any TCP listener in container modes (FR-S06). `docker exec` reaches it; the network cannot |
| Debug listener | health and metrics only; no pprof, no variables dump. Bind it to a private interface; the docs say so beside the setting |
| TLS | rustls with `ring`; 1.2 minimum, 1.3 available; no cipher choice offered (parity row 31); files reloaded on change |
| Passwords | bcrypt only, as the reference; constant work for unknown users (`001` P6 T1) |
| Build | `unsafe` forbidden in our code; dependencies locked; licence and advisory gates (`cargo deny`) on every commit |
| Disclosure | `SECURITY.md`: private address; first response in 3 working days; fix or mitigation plan in 30 days; credit offered; advisories published through GitHub Security Advisories |
| Image | distroless, no shell, no package manager; read-only root filesystem works; non-root in Kubernetes |

## 9. Resources

| | Bound | From |
|---|---|---|
| Memory per upload | one 4 MiB frame | `001` plan 9.1 |
| Memory under a 20 GB push | under 200 MB | `001:SC-007` |
| Idle memory, start time | under 30 MB, under 1 s | SC-S08 |
| Open files | 2 per active transfer plus about 20; the docs give the `ulimit` for N concurrent pulls | measured in gate G |
| Connections | no built-in cap in v1.0; `http.draintimeout` on shutdown; slow handshakes time out at 10 s | `001` P6 T2 |
| Disk | staging needs free space equal to the largest layer in flight; an `[oci] min_free_mb` setting refuses new uploads below it (default 1024) with a registry-shaped 5xx, so a full disk is met early and cleanly. Replaces the hub's `KAPPA_DISK_PRESSURE_THRESHOLD_MB` | gate G `disk-full.sh` |
| Kubernetes defaults in the chart | requests 50m CPU and 64 Mi; limit 512 Mi, no CPU limit | |

## 10. Failure modes

Each is provoked by a gate G test (FR-S21).

| Failure | The operator sees | The client sees | Recovery |
|---|---|---|---|
| Disk full, or below `min_free_mb` | health 503 `storage`; `hologram_disk_free_bytes` near 0; one error line per minute, not per request | pushes: 5xx in the registry's error shape; pulls keep working | free space; nothing to repair. Half-written uploads resume or expire |
| Store held by another process | exit 1 at start: `the store at <path> is in use by another process` | | stop the other one. In Kubernetes this is the old pod still draining: the StatefulSet ordering prevents it |
| Certificate or key unreadable at start | exit 2, names the file | | fix the file |
| Certificate unreadable at reload | old certificate stays; `hologram_tls_reload_failures_total` rises; one error line per attempt | nothing | fix the file; picked up within 5 s |
| Port taken (public or debug) | exit 3, names the setting and the address | | |
| Index file damaged (`links.redb` or the Kappa index does not open) | exit 1, names the file; points to `hologram oci adopt --rebuild-links` (rebuilds `links.redb` from tags and manifests; the Kappa index has no rebuild in v1.0: restore from backup) | | restore or rebuild, then `verify` |
| Blob bytes damaged | `verify` names the object, repositories and tags; metric set; the server keeps serving | the damaged blob downloads and the **client's** digest check fails | re-push the named tags, or restore the named files |
| Clock jumps | upload expiry uses monotonic time where it can; a backward jump never expires a live upload | nothing | |
| Killed at any point | `001` `data-model.md`, crash tables | retried push succeeds | none needed |

Exit codes, stable: 0 ok; 1 runtime failure; 2 configuration; 3 cannot bind or reach; 4 authentication; 5 capability missing [read: `src/main.rs:38-44`].

## 11. Support

`SUPPORT.md`: questions in GitHub Discussions; bugs by issue template (bug, feature, **interoperability**: which client, which version, the failing command, and whether the same command works against `registry:3`); security by the private address only; no private support, no response-time promise outside security. `CHANGELOG.md` in Keep a Changelog form, updated by every user-visible pull request; a CI check fails a pull request that touches `src/` or `apps/registry/` with no changelog line unless labelled `no-changelog`.

## 12. Gate G: operations

Runs per commit against the built image unless marked nightly.

1. `promtool check metrics` on a scrape; every metric in section 3 present; nightly: name list diffed against the pinned reference's scrape.
2. `/debug/health`: 200; after `down`, 503; after `up`, 200; disk-full test drives it to 503 within 30 s.
3. A Collector container receives one trace per request; with `OTEL_TRACES_EXPORTER=none`, none.
4. Certificate replaced on disk: new handshakes present it within 5 s; 200 parallel long pulls lose no connection.
5. `SIGTERM` during a pull: it completes; exit 0 within the drain timeout.
6. Read-only root filesystem, no `$HOME`, arbitrary non-root user: starts and serves.
7. The six failure modes of section 10 behave as written.
8. `backup.sh` and `upgrade.sh` (nightly).
9. A planted secret appears in none of: logs at `debug`, traces, metrics, `--dry-run`, the OpenAPI document.
10. The first log line on an anonymous registry is the warning.
