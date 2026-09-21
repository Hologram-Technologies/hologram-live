# Feature Specification: Hologram Server v1, a standalone product

> **Superseded in part, 21 September 2026: one product.** Hologram Server and Hologram Registry are now one product, **Hologram Registry**. Read this file as the registry's operability requirements. "Hologram Server" is an engineering name for the binary. The `server` image is dropped from v1.0; the `registry` image is also published to Docker Hub. One board: org project 3, "Hologram Registry Roadmap". The decision is recorded in `apps/registry/README.md`.

**Feature Branch**: `002-hologram-server-v1`

**Created**: 2026-09-21

**Status**: Specified, clarified and checked. No open decisions. Stops before `plan`.

**Input**: "Ship Hologram Server v1 as a robust and professional standalone product inside `Hologram-Technologies/hologram-live`. Equivalent feature set, interoperability and form factor to the server behind Docker Registry. Seamless for the Docker and Kubernetes ecosystem. Interoperable with the CNCF ecosystem (Artifact Hub) and with OpenAPI. Strictly defined scope, features and form factor. It first powers Hologram Registry."

**Read with**: `boundary.md` (what is Server, what is Registry), `parity-matrix.md` (every reference feature, with a verdict), `interop-matrix.md`, `form-factor.md`, `operations.md`, `openapi-policy.md`. Requirements of the Registry stay in `../v1/spec.md` and are cited by id (`001:FR-007`), never restated.

Terms. **Reference** = CNCF Distribution v3.1.1, image `registry:3`, pinned by digest. **Operator** = person who runs it. **Stranger** = operator with Docker, ten minutes, and no access to us. **Gate** = automated check that blocks a release.

## The product in four lines

Hologram Server is one binary, `hologram`, that a stranger can download, run, secure, observe, upgrade and get help with, without reading the source. Hologram Registry is that binary started as a registry, packaged so the reference's compose file runs with one line changed. v1.0 matches everything the reference's image, default configuration and deployment guide use (tier 1), and is measurably better in eleven places. The proxy cache and token login follow in v1.1; S3 storage and notifications in v1.2.

## Words the brief used, and what they mean here

| Brief said | Means, testably |
|---|---|
| robust | FR-S19 to FR-S21: it survives kill, full disk and upgrade, each with a test |
| professional | the 13 gaps in `study-hologram-server.md` are closed: SC-S02 |
| standalone | a stranger completes journey J1 in 5 minutes with nothing but the README: SC-S01 |
| equivalent | tier 1 of `parity-matrix.md`, proven by gates A to H |
| seamless | journeys J1 to J6 of `interop-matrix.md`, each under its time limit |
| fully interoperable with CNCF | the 22 rows of `interop-matrix.md` section 2, each with a script. A project not in the table is not claimed |
| equivalent to Docker Registry, as the ecosystem sees it | the 30 rows of `interop-matrix.md` section 4: peer registries, image tools, UIs, scanners, CI and GitOps |
| interoperable with OpenAPI | rules O1 to O10 of `openapi-policy.md` |

## Clarifications

### Session 2026-09-21

- Q: What is the v1.0 feature scope? → A: Tier 1. Proxy cache and token login in v1.1; S3 storage and notifications in v1.2.
- Q: How are releases versioned and tagged? → A: One version, one tag family, `server-v<version>`.
- Q: Which licence does the repository ship? → A: `MIT OR Apache-2.0`; add both texts.

Earlier answers that still hold: `001` Clarifications (copy and import only; upstream pull requests plus carried patches; v1 is a foundation; no cut list; two engineers; built in the parent repository as `apps/registry`).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Try it in five minutes, swap it in one line (Priority: P1)

A Docker user reads the first screen of the README, types one command, pushes an image, pulls it back, and removes every trace. Then they change the `image:` line of the compose file they already run, and nothing else changes.

**Why this priority**: it is the product's claim. Everything else assumes it.

**Independent Test**: `try.sh` and `compose-swap.sh` against the released image, on a machine that has never seen it.

**Acceptance Scenarios**:

1. **Given** Docker and no other preparation, **When** the user runs the three commands on the README's first screen, **Then** an image is pushed and pulled with an unchanged digest, in under 5 minutes including reading.
2. **Given** that trial, **When** the user runs `docker rm -fv <name>`, **Then** no container, volume, file or process of ours remains.
3. **Given** the reference's documented compose files (basic, TLS, htpasswd), **When** only `image:` is changed, **Then** each starts and serves, as `001:SC-001`.
4. **Given** the reference image's **default** configuration file, unchanged, **When** our image starts with it, **Then** it starts: no key in that file is refused (this fails today against the `001` key table; see `impact-on-001.md`).

---

### User Story 2 - Run it on Kubernetes (Priority: P2)

A platform engineer installs it with Helm or one manifest, points the cluster's container runtime at it, and pods pull from it, with a private certificate authority and a pull secret.

**Why this priority**: most private registries now live in or beside a cluster. Without this the product is a laptop tool.

**Independent Test**: `kubernetes.sh`, `manifest.sh`, `mirror-containerd.sh`, `pullsecret.sh` in a kind cluster.

**Acceptance Scenarios**:

1. **Given** a cluster, **When** `helm install registry oci://ghcr.io/hologram-technologies/charts/registry` runs with no values, **Then** within 180 s a pod can pull an image pushed to it.
2. **Given** the chart, **When** `replicaCount: 2` is set, **Then** the install is refused by the chart's schema with a message that says the store has one writer.
3. **Given** a `hosts.toml` for containerd and a private CA, **When** a pod references an image by the mirror's upstream name, **Then** it is pulled from us.
4. **Given** htpasswd login, **When** a pod uses an `imagePullSecrets` entry made by `kubectl create secret docker-registry`, **Then** it pulls; without it, it is refused.
5. **Given** a rolling update of the chart, **When** the pod is replaced, **Then** the old pod drains within `http.draintimeout`, the new one opens the store only after the old one has released it, and an upload in progress resumes (`001:FR-006`).

---

### User Story 3 - Operate it like any other server (Priority: P3)

The operator scrapes metrics, probes health, reads structured logs, sends traces, rotates a certificate without a restart, upgrades to the next release and rolls back, and knows what each failure looks like.

**Why this priority**: it is what separates a product from a binary.

**Independent Test**: gate G (`operations.md`, last section).

**Acceptance Scenarios**:

1. **Given** the default image configuration, **When** Prometheus scrapes `:5001/metrics`, **Then** the output passes `promtool check metrics` and holds every metric named in `operations.md`.
2. **Given** a running server, **When** the data disk is filled, **Then** `/debug/health` answers 503 within 30 s, pushes fail with a registry-shaped error, and pulls of existing images keep working.
3. **Given** new certificate files written over the old, **When** 5 s pass, **Then** new connections present the new certificate and no connection was dropped.
4. **Given** data written by release N-1, **When** release N starts on it, **Then** every tag pulls; and when N-1 is started again on that data after N wrote to it, it either works or refuses by layout version, never corrupts.
5. **Given** `SIGTERM`, **When** requests are in flight, **Then** they complete within the drain timeout and the process exits 0.
6. **Given** `OTEL_EXPORTER_OTLP_ENDPOINT` set to a collector, **When** a push runs, **Then** one trace with spans for each request arrives; with `OTEL_TRACES_EXPORTER=none`, nothing is sent.

---

### User Story 4 - Trust what was downloaded (Priority: P4)

A cautious adopter verifies the signature and provenance of the image, reads the licence, finds where to report a vulnerability and which versions are supported, and checks that the API document tells the truth.

**Why this priority**: adoption in a company goes through someone whose job is to ask these questions.

**Independent Test**: gate F and gate H.

**Acceptance Scenarios**:

1. **Given** a released image, **When** `cosign verify` runs with the documented identity, **Then** it verifies; `gh attestation verify` shows build provenance; an SBOM is attached.
2. **Given** any artefact (image, tarball, chart), **When** it is unpacked, **Then** it contains the licence texts and a third-party notice listing every dependency and its licence.
3. **Given** `SECURITY.md`, **When** it is read, **Then** it names a private reporting address, a response time, and the supported versions with dates.
4. **Given** the OpenAPI document of a release, **When** the contract test runs against that release, **Then** every documented operation exists and every served route is documented.

---

### User Story 5 - Use it from the tools already in place (Priority: P5)

CI pipelines, GitOps controllers, signers, scanners and peer registries work unchanged.

**Why this priority**: a registry is a meeting point; it is judged by what can meet there.

**Independent Test**: every script in `interop-matrix.md` at its cadence.

**Acceptance Scenarios**:

1. **Given** each of the 20 clients at its pinned version, **When** its script runs, **Then** it exits 0.
2. **Given** Flux and Argo CD in kind, **When** they reconcile a chart hosted by us, **Then** the release is installed.
3. **Given** a chart, an annotated image and an `artifacthub-repo.yml` pushed to us, **When** a self-hosted Artifact Hub adds the repository, **Then** both packages are listed and the publisher is verified.

---

### User Story 6 - Run Hologram Server for itself (Priority: P6)

A developer who wants the Server's other modules (objects, files, `.holo` apps, models, chat) runs the server image or binary, sees what it is on the first screen, and finds the API reference.

**Why this priority**: the Server is the platform; but in v1 only the Registry has a user waiting.

**Independent Test**: `server-try.sh`.

**Acceptance Scenarios**:

1. **Given** `docker run -p 11435:11435 ghcr.io/hologram-technologies/server:1`, **When** the user opens `/docs`, **Then** the API reference renders, and the container printed the generated token's location once.
2. **Given** `hologram` with no arguments, **When** it runs, **Then** it prints at most 24 lines: what it is, five commands, the docs address; exit code 0.
3. **Given** a default build without the `oci` feature, **When** `cargo tree` runs, **Then** no Kappa crate and no new C library is present (`001` plan, section 0).

---

### Edge Cases

- The container has no `$HOME` and a read-only root filesystem: it starts, writing only under the data volume.
- Port 5001 is taken: the debug listener fails the start with a message naming `http.debug.addr`; it is not skipped.
- The certificate is replaced by a file that does not parse: the old one stays in use, an error is logged once per attempt, health stays 200, a metric counts the failure.
- The administration socket path is too long for the platform: the start fails and names the path.
- `hologram oci verify` is run inside the container while TLS is on: it works, because administration does not use the public listener.
- Two Server images of different versions are started on one volume: the second fails to open the store and says why.
- A metrics scrape during a 20 GB push: answers in under 1 s.
- A client speaks HTTP to the TLS port: answered as the reference answers (**B:`http-on-tls-port`**).
- The OpenAPI document is requested from a Registry-mode container: it is served, and contains only the routes that are on.

## Requirements *(mandatory)*

### Functional Requirements: Server

- **FR-S01**: `hologram` with no arguments MUST print at most 24 lines (what it is, five starting commands, the documentation address) and exit 0. `registry` with no arguments MUST do the same for the registry commands.
- **FR-S02**: Each release MUST publish downloadable binaries for the systems in `form-factor.md`, and an install one-liner per system that **downloads**, verifies the checksum and installs. Building from source MUST NOT be the documented way to install.
- **FR-S03**: Each release MUST publish two images from one build, `server` and `registry`, for `linux/amd64` and `linux/arm64`, with the metadata in `form-factor.md`.
- **FR-S04**: Each release MUST publish a Helm chart as an OCI artifact and one plain Kubernetes manifest. The chart MUST refuse more than one replica.
- **FR-S05**: The Server MUST serve TLS from a certificate and key file, minimum TLS 1.2, settable to 1.3, and MUST begin using replaced files within 5 s without dropping connections. (Supersedes the listener half of `001:FR-008`; the setting names stay with the Registry.)
- **FR-S06**: Administration operations (shutdown, verify, any future operator command) MUST be reachable only through a Unix socket or, on Windows, a named pipe, inside the data directory, mode 0600. In container modes they MUST NOT be reachable from the public listener. (Supersedes `001:FR-021`'s loopback port.)
- **FR-S07**: With a data directory configured, the Server MUST write nothing outside it, MUST start with no `$HOME`, and MUST run with a read-only root filesystem.
- **FR-S08**: A module MUST be able to authenticate its own routes; such routes MUST still get a request id, a trace span and an access log line.
- **FR-S09**: The Server MUST offer a second listener carrying `/debug/health` (200 or 503, with `/down` and `/up` for manual drain), and `/metrics` in the Prometheus text format with the metrics listed in `operations.md`. The storage health check MUST be a real write, read and delete.
- **FR-S10**: The Server MUST honour the standard `OTEL_*` variables for trace export, including `OTEL_TRACES_EXPORTER=none`, and MUST write logs as text or JSON with configurable static fields.
- **FR-S11**: The Server MUST publish one OpenAPI 3.1 document that covers every HTTP route it serves in the running configuration, `/v2/` included, and obeys rules O1 to O7 of `openapi-policy.md`.
- **FR-S12**: A contract test MUST prove, on every commit, that the running Server matches its document in both directions (rules O8 to O10).
- **FR-S13**: Every released image, binary and chart MUST carry a checksum, a cosign signature, an SBOM and build provenance, verifiable by the commands in `form-factor.md`.
- **FR-S14**: The repository MUST contain the licence texts it declares. Every artefact MUST contain them and a third-party notice. A licence gate MUST fail the build on a dependency whose licence is not on the allowed list, or that has no licence file of its own unless listed as an accepted exception with a reason.
- **FR-S15**: `SECURITY.md` MUST name a private reporting address, a first-response time of 3 working days, and a supported-versions table with dates.
- **FR-S16**: A versioning and support policy MUST state what a patch, a minor and a major release may change (`operations.md`), and a `CHANGELOG.md` MUST be updated by every user-visible pull request.
- **FR-S17**: The repository MUST offer issue templates for bug, feature and interoperability reports, and a `SUPPORT.md` saying where help is and is not given.
- **FR-S18**: The README's first screen MUST say what Hologram Server is, show the three-command trial, and link to the Registry, the API reference and the security policy. The documentation site MUST have sections "Server" and "Registry".
- **FR-S19**: Data written by the previous minor release MUST open in the current one. A downgrade MUST either work or be refused by layout version. Both are tested at every release.
- **FR-S20**: On `SIGTERM` or `SIGINT` the Server MUST stop accepting, drain in-flight requests within the drain timeout, and exit 0. Exit codes MUST be documented and stable.
- **FR-S21**: For each failure in the table in `operations.md` (disk full, store locked, certificate unreadable, port taken, corrupt index, clock jump), the Server MUST behave as the table says, and a test MUST provoke each.
- **FR-S22**: The default build MUST stay free of the registry's dependencies (`001` plan, section 0, rules 1 and 2).
- **FR-S24**: Both settings references (Server, Registry) MUST be generated from the code, and a validate command (`hologram config validate`, `registry serve --dry-run <config>`) MUST print the effective configuration with secrets masked.
- **FR-S25**: Backup and restore MUST be documented and tested: a copy taken in a read-only window restores elsewhere with every tag pullable and `verify` clean (`operations.md` section 6).
- **FR-S23**: One version number and one tag family MUST publish every artefact. Decided 2026-09-21: one tag family, `server-v<version>`, publishes binaries, both images and the chart; the eight gates are a condition of that release. `001`'s `registry-v*` tags are dropped.

### Functional Requirements: Registry, added to `001` by this definition

- **FR-R23**: The Registry MUST honour `tags.maxtags`.
- **FR-R24**: The Registry MUST map `http.debug.addr`, `http.debug.prometheus.*` and `health.storagedriver.*` onto FR-S09, and MUST start with the reference image's default configuration file unchanged.
- **FR-R25**: The Registry MUST apply `log.fields` as static log attributes.
- **FR-R26**: Artifacts Artifact Hub needs (semver-tagged charts, annotated images, the `artifacthub.io` metadata tag with its layer media type) MUST round-trip unchanged.
- **FR-R27**: Every row of `interop-matrix.md` MUST have a script that runs at the row's cadence. A red per-commit or nightly row blocks a release; per-release rows are recorded in the release checklist.
- **FR-R29**: `Location` headers MUST honour `X-Forwarded-Proto` and `X-Forwarded-Host` exactly as the reference does behind a TLS-terminating proxy (**B:`forwarded-headers`**). Found by the equivalence checklist; absent from `001`.
- **FR-R30**: The type selectors the guide's own commands use, `REGISTRY_AUTH=htpasswd` and `REGISTRY_STORAGE=filesystem`, MUST be accepted. Any other value is refused by name. Found by the equivalence checklist: with `001`'s rule that an unknown `REGISTRY_*` variable stops the start, the guide's htpasswd compose file would not start.
- **FR-R31**: With the reference's `http.headers` recipe for CORS, a browser on another origin MUST be able to list repositories and tags, read a manifest and delete a tag; an `OPTIONS` preflight under `/v2/` MUST be answered before authentication, and `Docker-Content-Digest` and `Link` MUST be exposable. Added 21 September for web UIs (`interop-matrix.md` §4).
- **FR-R32**: The Registry MUST behave as an upstream for proxying registries (Harbor proxy-cache, Nexus, Artifactory, Quay mirroring): `HEAD` on manifests and blobs with `Docker-Content-Digest`, `ETag` and `Content-Length`; conditional requests; `Range`; identical answers for a tag that has not moved. Added 21 September (`interop-matrix.md` §4).
- **FR-R28**: v1.0 feature scope is tier 1 of `parity-matrix.md`. Decided 2026-09-21: tier 1 in v1.0 (80 engineer days); pull-through proxy and token login in v1.1 (19.5 days); S3 storage and notifications in v1.2 (19 days). Full equality (146 days) was considered and declined.

### Key Entities

- **Public listener**: serves product routes; plain or TLS.
- **Debug listener**: serves health and metrics; plain; off unless configured (on in the registry image's default file, as the reference).
- **Administration socket**: serves operator commands; never networked.
- **Release**: one version, one tag; binaries, two images, a chart, a manifest, the OpenAPI document, checksums, signatures, SBOMs, provenance, `DIFFERENCES.md`, `CHANGELOG.md`.
- **Gate**: A to E from `001`; **F** form factor, **G** operations, **H** OpenAPI, defined in `form-factor.md`, `operations.md`, `openapi-policy.md`.
- **Journey**: J1 to J6, a timed script a user could follow by hand.

## Success Criteria *(mandatory)*

- **SC-S01**: A stranger completes J1 in under 5 minutes using only the README's first screen. Measured with three people who have not seen the product, before release; in CI the same commands finish in under 20 s.
- **SC-S02**: All 13 Server gaps in `study-hologram-server.md` are closed, each by its requirement's check.
- **SC-S03**: Journeys J2 to J6 finish under their limits on every commit or night, per `interop-matrix.md`.
- **SC-S04**: Every per-commit and nightly row of `interop-matrix.md` due at v1.0 (20 client rows, 17 project rows, 7 OpenAPI rows) is green for 3 consecutive nights before release; every per-release row (4 projects, 1 OpenAPI) is recorded green in the release checklist. One project row (CloudEvents) waits for v1.2.
- **SC-S05**: The OpenAPI document has 0 schema errors, 0 lint errors, 0 undocumented routes, 0 documented routes that do not exist.
- **SC-S06**: `cosign verify`, `gh attestation verify` and checksum verification succeed for every artefact of the release, run from a clean machine.
- **SC-S07**: Upgrade from the previous release and the downgrade rule pass (from v1.1 onward; at v1.0 the test runs against the release candidate).
- **SC-S08**: Registry image: compressed size under 40 MB, ready to serve in under 1 s, idle memory under 30 MB **[assumption: measured once the image exists; the reference is about 10 MB and we will be larger because the binary carries the other modules]**.
- **SC-S09**: Each of the six failure modes in `operations.md` is provoked by a test and behaves as written.
- **SC-S10**: `001:SC-010` still holds: five gates green on the release and hub.uor.foundation serving from it. Gates F, G and H are added to that release condition.

## Out of Scope

Every line says what the user does instead. Settings named here are refused at start, by name.

| Out of v1.0 | Instead |
|---|---|
| Pull-through proxy cache | **v1.1.** Until then: run the reference as a mirror beside us |
| Token authentication (`auth.token`) | **v1.1.** Until then: htpasswd, or a front proxy that authenticates |
| Mutual TLS, `http.prefix`, `http.net: unix`, manifest URL and platform validation, file, HTTP and TCP health checks | **v1.1** |
| S3-compatible storage, redirects to the object store, more than one replica | **v1.2.** Until then: a persistent disk and one replica |
| Notifications (webhooks, CloudEvents) | **v1.2.** Until then: poll `tags/list`, or trigger from CI |
| `linux/s390x`, `linux/riscv64` | **v1.2** |
| GCS and Azure native storage | never in 1.x. GCS through its S3-compatible endpoint from v1.2; otherwise a managed disk |
| Let's Encrypt inside the server | never in 1.x. cert-manager in Kubernetes; Caddy or certbot on a host. FR-S05 picks up renewed files |
| Redis cache, in-memory storage, `silly` auth, CloudFront and redirect middleware, mail log hooks, logstash format, cipher suite selection, `/debug/vars`, pprof | never. `parity-matrix.md` rows 15, 17, 19, 28, 31, 41, 46, 47 say what to use |
| `linux/arm/v6`, `arm/v7`, `ppc64le` | never in 1.x. Run the reference there |
| A shell in the image | `docker exec <c> hologram …`, or an ephemeral debug container |
| A Go library, a registry UI, search, vulnerability scanning, replication rules, quotas, projects and roles | not a registry server's job. Harbor or zot on top, or beside |
| A hosted service | run it yourself; none is planned |
| Everything in `001`'s out-of-scope list not reversed above (Hugging Face, running models through the registry, merging the object API with the registry store) | as `001` says |
| Hologram Desktop | its own product and release train |

## Assumptions

- The reference is Distribution v3.1.1 [ran: GitHub API]. Its image facts were read from the Dockerfile and `config-dev.yml` at `main` `5b354e6`, not from the published image, because Docker was not reachable. `001` P2 T1 checks the published image and corrects this spec if they differ.
- Day estimates in `parity-matrix.md` are estimates. 29 new days on top of `001`'s 51: 80 engineer days, about 41 working days with two engineers. There is no cut list; the date moves.
- wasmtime supports `s390x` and `riscv64` and not 32-bit ARM or `ppc64le` **[memory]**. If wrong, rows 55 and 56 of the parity matrix change.
- Tool version floors in `interop-matrix.md` are from memory; `001` P2 T4 confirms them.
- The other maintainer of `hologram-live` agrees to a second listener, an administration socket and a licence gate in the Server. These join the three agreements `001` decision 8 already needs.
- Decided 2026-09-21: the licence stays `MIT OR Apache-2.0`, as the package declares; both texts are added to the repository and to every artefact (FR-S14).

## Done check

- **Asked**: a standalone, equivalent, seamless, interoperable, strictly scoped product. Each adjective is replaced by a gate or a journey (table at the top). Scope is in three tiers with 61 rows, each with a verdict.
- **Skeptic**: "this is a registry with extra steps; why a Server product at all?" Because every item in FR-S02 to FR-S21 has to be built for the Registry anyway; writing them as Server requirements costs nothing more and lets the next product reuse them. "Why would I switch?" For v1.0: eleven checked improvements, none of them a reason alone. `001` already says so. The reason arrives with one store for images, models and objects, after v1.
- **Not verified**: the published reference image; tool version floors; wasmtime's target list; every day estimate; whether `artifacthub.io` itself (not a self-hosted copy) can reach a given deployment, which depends on the operator's network, not on us.
