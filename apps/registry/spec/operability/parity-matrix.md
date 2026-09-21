# Parity matrix: Distribution v3.1.1 against Hologram Server v1

2026-09-21. Reference: Distribution **v3.1.1**, the image `registry:3` pinned by digest in `apps/registry/gates/reference.env` (`001` P2 T1). Why this one: it is the current release, and it is what `docker pull registry:3` gives an operator today.

## What "equivalent" means, as a test

Full feature equality with Distribution costs **146 engineer days** (bottom of this page). It would also mean building the parts of Distribution its own users complain about most. So equivalence is defined by tier, and each tier has a check that can fail:

| Tier | Definition | Release | Check |
|---|---|---|---|
| 1 | Everything the reference's **image, default configuration and deployment guide use in a command or a file**. The guide also *mentions* Let's Encrypt, S3 and token login, one paragraph and a link each [read: `dist:docs/content/about/deploying.md:258-264`, `:176`, `:477-486`]; no command in it uses them, so they are tier 2. An operator who followed the guide changes one line | **v1.0** | gates A to E of `001`, plus gate F (form factor) and gate G (operations) defined in `spec.md` |
| 2 | Documented features with real production use that the guide does not need | **v1.1**, **v1.2**, named per row | each ships with its own gate B scenarios |
| 3 | Test-only, deprecated, or replaced by something every operator already has | **never**; the row says what to do instead | the setting is refused at start, by name (`001` FR-011) |

Days are **new** engineer days beyond the 51 already in the `001` plan **[assumption: estimates]**. "in 001" means the cost is already counted there. Owner: **S** Server, **R** Registry (`boundary.md`).

## A. API and data

| # | Feature | Own | Verdict | Days | Reason | Check |
|---|---|---|---|---|---|---|
| 1 | OCI Distribution 1.1: pull, push, chunked, monolithic, mount | R | v1 | in 001 | the product | gate A |
| 2 | Tags list, catalogue, paging, `catalog.maxentries` | R | v1 | in 001 | Harbor and Zot sync need it | gates A, E |
| 3 | `tags.maxtags` paging cap (new in 3.1) | R | v1 | 0.25 | one constant beside row 2 | gate B `tags-paging` |
| 4 | Referrers API | R | v1 | in 001 | cosign, notation, oras | gate A, gate C |
| 5 | Delete manifests and blobs; `storage.delete.enabled` | R | v1 | in 001 | | gate A, B |
| 6 | Docker manifest v2, lists; OCI manifests, indexes, artifacts | R | v1 | in 001 | | gates A, D |
| 7 | Schema 1 manifests | R | never | 0 | removed in Distribution 3 too [memory] | gate B `manifest-put-invalid` |
| 8 | Same errors, headers, statuses | R | v1 | in 001 | | gate B |
| 9 | `validation.manifests.urls`, `.indexes.platforms` | R | v1.1 | 1.5 | rare; changes what a push accepts, so it is refused until built | own scenarios |
| 10 | Go client and server libraries | | never | n/a | we are not a library; platform builders stay on Distribution | |

## B. Storage

| # | Feature | Own | Verdict | Days | Reason | Check |
|---|---|---|---|---|---|---|
| 11 | `filesystem` driver | R | v1 | in 001 | on the Kappa store; the layout differs, migration is by copy and `import` | gate D, `001` FR-012 |
| 12 | `s3` driver (and every S3-compatible store: MinIO, Ceph, R2) | R | v1.2 | 12 | the largest single gap for production users; needs a second backend under the store adapter | gates A, D on MinIO |
| 13 | `gcs` driver | R | never (1.x) | 6 | use GCS's S3-compatible endpoint with row 12, or a disk | refused by name |
| 14 | `azure` driver | R | never (1.x) | 6 | no S3 endpoint exists; use a managed disk. Revisit at 2.0 on demand | refused by name |
| 15 | `inmemory` driver | R | never | 1 | tests only. Use a `tmpfs` volume | refused by name |
| 16 | `storage.redirect` (send clients to the object store) | R | v1.2 | with 12 | only meaningful with row 12 | |
| 17 | `middleware.storage.cloudfront`, `.redirect`; registry and repository middleware | R | never | 4 | Go plug-in points. Put a CDN in front | refused by name |
| 18 | `cache.blobdescriptor: inmemory` | R | v1, ignored | 0 | our index is local; nothing to cache. It is in the reference's default file | walk test |
| 19 | `cache.blobdescriptor: redis`, `redis.*` | R | never | 3 | exists to share a cache between replicas; v1 is one writer | refused by name |
| 20 | `maintenance.uploadpurging.*` | R | v1 | in 001 | | `001` P7 T5 |
| 21 | `maintenance.readonly` | R | v1 | in 001 | | gate B `readonly-mode` |
| 22 | `storage.tag.concurrencylimit`, `filesystem.maxthreads` | R | v1, ignored | 0 | tuning knobs for another engine | walk test |
| 23 | `garbage-collect`, `--dry-run`, `--delete-untagged`, `--quiet` | R | v1 | in 001 | refuses beside a live server (listed difference) | property test |
| 24 | Many replicas over one store | R | v1.2 | with 12 | needs row 12; v1 is single-writer and the chart enforces one replica | |

## C. Authentication and transport

| # | Feature | Own | Verdict | Days | Reason | Check |
|---|---|---|---|---|---|---|
| 25 | Anonymous | R | v1 | in 001 | the reference's default | gate B |
| 26 | `auth.htpasswd` | R | v1 | in 001 | the guide's method | gate C `login.sh` |
| 27 | `auth.token` (JWT from an external service; `jwks`, `rootcertbundle`, `signingalgorithms`, `autoredirect`) | R | **v1.1** | 6 | how Keycloak, `docker_auth` and GitLab-style front ends plug in. Not used by the guide. It is also where Distribution breaks most (#1978, #4533) | own scenarios; gate C with `docker_auth` |
| 28 | `auth.silly` | R | never | 0.5 | tests only | refused by name |
| 29 | TLS: `certificate`, `key`, `minimumtls` | **S** | v1 | in 001 | moved to the Server (`boundary.md` conflict 5) | gate C `tls.sh` |
| 30 | `http.tls.clientcas`, `clientauth` (mutual TLS) | S | v1.1 | 1.5 | cheap with rustls; nobody needs it on day one | own test |
| 31 | `http.tls.ciphersuites` | S | never | 0.5 | rustls ships only safe suites; refusing beats pretending | refused by name |
| 32 | `http.tls.letsencrypt` | S | never (1.x) | 4 | in Kubernetes: cert-manager. On a host: Caddy or certbot writing the two files rows 29 reads. The reload in FR-S05 makes renewals seamless | refused by name |
| 33 | `http.addr`, `host`, `relativeurls`, `headers`, `draintimeout`, `http2.disabled` | S+R | v1 | in 001 | | gate B `location-form` |
| 33a | `X-Forwarded-Proto`, `X-Forwarded-Host` honoured when building `Location`, as the guide's load balancing section requires [read: `deploying.md:343-390`] | R | v1 | 0.5 | every registry behind an ingress or Caddy needs it, the hub included. Missing from `001` | gate B `forwarded-headers` |
| 34 | `http.h2c.enabled` | S | v1, ignored | 0 | already on; gRPC needs it | walk test |
| 35 | `http.secret` | R | v1, ignored | 0 | upload state is server-side | `DIFFERENCES.md` note |
| 36 | `http.prefix` | R | v1.1 | 1 | changes every URL; needs its own gate B run | |
| 37 | `http.net: unix` | S | v1.1 | 0.5 | | |

## D. Observability and health

| # | Feature | Own | Verdict | Days | Reason | Check |
|---|---|---|---|---|---|---|
| 38 | **`http.debug.addr`: second listener** | S | **v1** | 3 (rows 38 to 41 together) | the reference **image** turns it on at `:5001` by default. Refusing it would refuse our own default file. Reverses `001` | gate G |
| 39 | `/debug/health`, and `/down`, `/up` for manual drain | S | v1 | | load balancers and the community chart's probes use it | gate G |
| 40 | `http.debug.prometheus`: `/metrics` in the Prometheus text format | S | v1 | | metric names follow the reference's `registry_*` where a counterpart exists, so existing dashboards draw | gate G: `promtool check metrics`; name list diffed against the reference |
| 41 | `/debug/vars`, pprof | S | never | 0 | Go runtime internals. Use `/metrics` and `hologram tracing` | 404 on those paths, listed |
| 42 | `health.storagedriver` | S | v1 | 0.5 | a real write-read-delete probe feeding row 39 | gate G: fill the disk, health goes 503 |
| 43 | `health.file`, `.http`, `.tcp` | S | v1.1 | 1 | used for drain files and dependency checks; rare | |
| 44 | OpenTelemetry traces, configured by `OTEL_*` | S | v1 | 1 | the Server already exports OTLP; it must also honour the standard variables, including `OTEL_TRACES_EXPORTER=none`, which the image sets | gate G with a collector container |
| 45 | `log.level`, `formatter` text and json, `accesslog.disabled`, `fields` | S+R | v1 | in 001 | `fields` become static attributes on every line (was "ignored" in `001`; they cost nothing and log pipelines key on `service`) | walk test |
| 46 | `log.formatter: logstash` | S | never | 0.5 | json is what collectors read now | refused by name |
| 47 | `log.hooks` (mail on panic) | S | never | 2 | alert from the log pipeline | refused by name |

## E. Integration

| # | Feature | Own | Verdict | Days | Reason | Check |
|---|---|---|---|---|---|---|
| 48 | `proxy`: pull-through cache of one upstream, `ttl`, credentials, `exec` helper | R | **v1.1** | 8 | the most wanted feature and the least finished (#1431, #2367, #3725, #4281). Worth doing well, not fast. Kubernetes mirror setups need it | own gate: kill the upstream mid-pull; expire a blob mid-pull |
| 49 | `notifications`: webhooks with retry, backoff, filters | R | v1.2 | 5 | what Harbor-like systems and CI triggers use. Offered in the reference's envelope **and** as CloudEvents | own scenarios; a CloudEvents validator |

## F. Form factor

| # | Feature | Own | Verdict | Days | Reason | Check |
|---|---|---|---|---|---|---|
| 50 | One static binary; `serve`, `garbage-collect`, `--version` | S+R | v1 | in 001 | | gate F |
| 51 | Image: entry point, command, port, volume, default file equal to the reference's | R | v1 | in 001 | default file corrected from the source (`boundary.md` conflict 4) | gate F: `docker inspect` JSON compared |
| 52 | Image has a shell | R | never | 0 | distroless. Use `docker exec … hologram …` or a debug container. Listed in the docs, not in `DIFFERENCES.md` | |
| 53 | Runs as root | R | v1, same | 0 | a volume made by the reference is root-owned; non-root would break the one-line swap. The chart sets a non-root user and an `fsGroup` | `compose-swap.sh` |
| 54 | Architectures `linux/amd64`, `linux/arm64` | S | v1 | in 001 | | gate F |
| 55 | `linux/s390x`, `linux/riscv64` | S | v1.2 | 2 | wasmtime supports both [memory]; needs cross builds and an emulated smoke test | |
| 56 | `linux/arm/v6`, `arm/v7`, `ppc64le` | S | never (1.x) | blocked | wasmtime has no 32-bit ARM or ppc64le backend [memory]. Run Distribution there | |
| 57 | Release tarballs with README and LICENSE inside, a checksum beside each | S | v1 | 0.5 | | gate F `check-artifacts.sh` |
| 58 | Windows and macOS binaries | S | v1 | in 001 | **more** than the reference, which ships Linux only | gate F |
| 59 | Helm chart | R | v1 | 1.5 over 001 | the reference has none of its own; ours is signed, schema-checked and listed on Artifact Hub | gate F, `interop-matrix.md` |
| 60 | systemd recipe; nginx and Apache recipes | S | v1 (docs) | in ops docs | a unit file ships in the tarball | docs check |

## G. Where v1 is deliberately better

Each is real on day one only if its check is green.

| # | Better at | Own | Days | Check |
|---|---|---|---|---|
| B1 | Bytes verified as written; a lie leaves nothing behind | R | in 001 | `001` FR-020 tests; gate B `digest-mismatch` |
| B2 | `hologram oci verify`: re-hash everything, name what is damaged | R | in 001 | `names_every_flipped_byte` |
| B3 | An upload survives a server restart | R | in 001 | `an_upload_resumes_after_the_process_is_killed` |
| B4 | Garbage collection cannot break an image; refuses where unsafe | R | in 001 | property test, 256 cases |
| B5 | Unknown or unsupported settings stop the start, by name | S+R | in 001 | walk test; gate B `env-unknown-key` |
| B6 | TLS and login with no front proxy; certificates reload without a restart | S | 0.5 | gate G: replace the files, new handshakes use the new certificate within 5 s |
| B7 | An OpenAPI 3.1 document for every route, `/v2/` included, checked against the running server | S | 5.5 | `openapi-policy.md`, rules O1 to O10 |
| B8 | Signed images and binaries, SBOM, build provenance | S | 2 | gate F: `cosign verify`, `gh attestation verify` |
| B9 | A current security policy, a written support window, a changelog | S | 1.5 (with first screen and templates) | docs check; the policy names an address and dates |
| B10 | Upgrades tested: data written by the previous release opens in this one | S | 1 | gate G `upgrade.sh` |
| B11 | Licence clarity: LICENSE files and a third-party notice in every artefact | S | 1 | gate F: `cargo deny check licenses`; files present in image and tarball |

## H. Ecosystem work that is not a Distribution feature

| # | Work | Own | Days | Where defined |
|---|---|---|---|---|
| E1 | Server image and its quick start | S | 0.5 | `form-factor.md` |
| E2 | Plain Kubernetes manifest; containerd and CRI-O mirror journeys in gate C | R | 1.5 | `interop-matrix.md` |
| E3 | CI journeys: GitLab service, buildx cache export and import | R | 1 | `interop-matrix.md` |
| E4 | CNCF nightly: notation, Flux, Argo CD; Dragonfly per release | R | 3 | `interop-matrix.md` |
| E5 | Artifact Hub: hosting test and our own listing | R | 1 | `interop-matrix.md` |
| E6 | Operations documentation, settings reference generated from code | S | 3 | `operations.md` |

## Totals

| | Rows | Days |
|---|---|---|
| Distribution features compared (A to F) | 61 | |
| … **v1** (19 already in `001`, 9 with new work, 6 accepted and ignored or free) | 34 | 7.25 new |
| … **later**, named: v1.1 (7 rows), v1.2 (5 rows) | 12 | 19.5 + 19 = 38.5 |
| … **never**, each with what to do instead | 15 | 27.5 if they were built |
| Better than the reference (G) | 11 | 11.5 new |
| Ecosystem work (H) | 6 | 10 |
| **New v1 work beyond `001`** | | **28.75, called 29** |

**Cost and date.** `001` is 51 engineer days and 27 working days. This bar adds 29: **80 engineer days, about 41 working days** for two engineers (eight weeks and a day). Almost all of the 29 sits on engineer B's track or after gate A, so the critical path grows by about 14 days, not 29. There is no cut list: the date moves by 14 working days.

**Full equality**, for comparison: 80 + 38.5 + 27.5 = **146 engineer days**, about 73 working days. Recommendation: do not. Ship tier 1 as v1.0, then the proxy and token authentication as v1.1 (19.5 days), then S3 and notifications as v1.2 (19 days). **The maintainer's decision 1.**
