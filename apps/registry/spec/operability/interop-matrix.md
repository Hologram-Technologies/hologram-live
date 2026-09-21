# Interoperability matrix

2026-09-21. "Works" is never a feeling. Each row says what works means and which script proves it. Scripts live in `apps/registry/gates/clients/` (gate C of `001`) unless a gate letter says otherwise.

**Versions.** The exact version of every tool is pinned in `apps/registry/gates/clients/VERSIONS` and installed from there in CI. The floor below is the oldest version v1 promises; the pin is the newest stable on the day `001` P2 starts, and moves by reviewed pull request. Floors are **[memory]**: P2 T4 confirms each against the tool's release page and corrects this table.

**Quarantine.** A red row may be switched off only by a pull request that links the upstream issue, for at most 14 days, and the release notes name it. There is no silent skip.

**When.** **C** per commit. **N** nightly. **R** per release, by hand, recorded in the release checklist.

## 1. Clients and tools: Docker, Kubernetes, CI

"Seamless" means these journeys, each with a time limit, image already pulled:

| Journey | Limit | Script |
|---|---|---|
| J1 Try it: `docker run -d -p 5000:5000 ghcr.io/hologram-technologies/registry:1`, push, pull, remove every trace with `docker rm -fv` | 5 min for a first-time user; 20 s in CI | `try.sh` |
| J2 Swap it into the reference's documented compose file, one line changed | 300 s | `compose-swap.sh` (`001` SC-001) |
| J3 Run it on Kubernetes: `helm install`, or `kubectl apply -f registry.yaml`; a pod pulls from it | 180 s after the chart is fetched | `kubernetes.sh`, `manifest.sh` |
| J4 Use it as a local registry for kind and k3d, by their documented recipes, unchanged | 180 s | `kind.sh`, `k3d.sh` |
| J5 Use it as a cluster mirror: containerd `hosts.toml`, CRI-O `registries.conf`; private CA; `imagePullSecrets` | 300 s | `mirror-containerd.sh`, `mirror-crio.sh`, `pullsecret.sh` |
| J6 Use it from CI: GitHub Actions service container; GitLab CI service; buildx `--cache-to type=registry` then `--cache-from` | each under 120 s | `service-container` job, `gitlab.sh`, `buildx-cache.sh` |

| # | Client | Floor | Works means | Script | When |
|---|---|---|---|---|---|
| 1 | Docker Engine and CLI | 24.0 | `login`, `push`, `pull`, `manifest inspect`; digests unchanged | `docker.sh`, `login.sh` | C |
| 2 | Docker Buildx, BuildKit | 0.12 | two-platform `--push`; registry cache export and import (cache manifests use non-image media types) | `docker.sh`, `buildx-cache.sh` | C |
| 3 | Docker Compose | v2.20 | J2 with three documented files | `compose-swap.sh` | C |
| 4 | containerd (`ctr`), `nerdctl` | 1.7 and 2.0 | pull, push; `hosts.toml` mirror with a private CA | `containerd.sh`, `mirror-containerd.sh` | C |
| 5 | CRI-O, `crictl` | 1.29 | pull through `registries.conf` mirror | `mirror-crio.sh` | N |
| 6 | Kubernetes kubelet | the three newest minor versions on release day | pod reaches `Running` from our image; with `imagePullSecrets` (htpasswd); with a private CA | `kubernetes.sh`, `pullsecret.sh` | C (kind), N (matrix of three minor versions) |
| 7 | kind | 0.23 | documented local-registry recipe, unchanged | `kind.sh` | C |
| 8 | k3d, k3s | 5.6 | `registries.yaml` recipe, unchanged | `k3d.sh` | N |
| 9 | Podman, Buildah | 5.0 | push, pull, `--tls-verify` both ways | `podman.sh` | N |
| 10 | Skopeo | 1.14 | `copy --all` in and out; `sync` | `skopeo.sh`, gate D | C |
| 11 | crane, go-containerregistry | 0.19 | copy, digest, `ls`, `catalog` | `crane.sh` | C |
| 12 | ORAS CLI | 1.2 | push artifact with `artifactType`; `attach`; `discover` through referrers; `cp -r` | `oras.sh` | C |
| 13 | Helm | 3.14 | `push`, `pull`, `install` from `oci://` | `helm.sh` | C |
| 14 | cosign | 2.2 | sign, verify; referrers mode and tag fallback | `cosign.sh` | C |
| 15 | notation | 1.1 | sign, verify, list through referrers | `notation.sh` | N |
| 16 | Ollama | pinned | `push`, `pull` of a model | `ollama.sh` | N |
| 17 | GitHub Actions | | J6 service container | `gate-c-service` job | C |
| 18 | GitLab CI | 16 | J6 as a `services:` entry, pushed to from `docker:dind` | `gitlab.sh` (runs `gitlab-runner exec`-style locally in a container) | N |
| 19 | Trivy, Grype | current | scan an image by registry reference | `scan.sh` | N |
| 20 | `curl` | any | every route in `contracts/registry-api.md` | gate B | C |

Twenty clients: 12 proven per commit, 8 nightly.

## 2. CNCF projects

Maturity levels **[docs: `cncf.io/projects`, fetched 2026-09-21]** unless marked.

| # | Project | Level | Works means | Proof | When |
|---|---|---|---|---|---|
| 1 | OCI Distribution spec 1.1 | not CNCF (Linux Foundation, OCI) | conformance suite, every category | gate A | C |
| 2 | OCI Image spec 1.1, artifacts (`artifactType`, `subject`) | not CNCF (OCI) | manifests, indexes, artifacts, empty config round-trip with digests unchanged | gates A, D | C |
| 3 | Kubernetes | Graduated | section 1 rows 6 to 8 | | C, N |
| 4 | containerd | Graduated | section 1 row 4 | | C |
| 5 | CRI-O | Graduated | section 1 row 5 | | N |
| 6 | Helm | Graduated | charts as OCI artifacts in; **our** chart out (below) | `helm.sh` | C |
| 7 | Harbor | Graduated | copy in and out with digests unchanged; Harbor replication pulling from us | gates D, E | N |
| 8 | Distribution | Sandbox | the reference itself | gates B, D | C |
| 9 | zot | Sandbox [memory] | copy in and out; zot `sync` pulling from us | gates D, E | C (copy), N (sync) |
| 10 | ORAS | Sandbox [memory] | section 1 row 12 | | C |
| 11 | Notary Project | Incubating | section 1 row 15 | | N |
| 12 | Sigstore (cosign) | **not CNCF** (OpenSSF) | section 1 row 14. Also signs our own release | | C |
| 13 | Flux | Graduated | `OCIRepository` and `HelmRepository` of type `oci` reconcile from us, with a pull secret and a private CA | `flux.sh` in kind | N |
| 14 | Argo CD | Graduated | an `Application` from an OCI Helm chart hosted by us syncs | `argocd.sh` in kind | N |
| 15 | Dragonfly | Graduated | a Dragonfly cluster configured with us as origin serves a pull with digests unchanged | `dragonfly.sh` | R (it needs a multi-node setup) |
| 16 | Artifact Hub | Incubating | both directions, below | `artifacthub.sh` | N; listing checked R |
| 17 | Prometheus | Graduated | `/metrics` parses; names listed in `operations.md` exist | `promtool check metrics` | C (gate G) |
| 18 | OpenTelemetry | Graduated | traces reach a Collector over OTLP; `OTEL_*` honoured, including `OTEL_TRACES_EXPORTER=none` | collector container | C (gate G) |
| 19 | CloudEvents | Graduated | notifications in CloudEvents 1.0 format | | **v1.2**, with notifications |
| 20 | Kyverno, OPA Gatekeeper | Graduated | image verification policies that read signatures from us admit a signed image and refuse an unsigned one | `kyverno.sh` | R |
| 21 | cert-manager | Graduated [memory] | the chart mounts a `Certificate` secret; renewal is picked up without a restart | `certmanager.sh` | N |
| 22 | in-toto, SLSA provenance | Graduated / OpenSSF | our own release carries build provenance | gate F | R |

Twenty-two rows: 18 are CNCF projects, 4 are not and are marked. 9 proven per commit, 8 nightly, 4 per release, 1 waits for v1.2.

**Pushback: "fully interoperable with the CNCF ecosystem" cannot be tested.** The landscape has over two hundred projects and most never talk to a registry. The testable version is this table: every CNCF project that pushes to, pulls from, mirrors, signs into, scans, or observes a registry, each with a script. A project not listed is not claimed.

### Artifact Hub, both directions

Source: Artifact Hub repository documents [docs: `github.com/artifacthub/hub/docs`, fetched today].

**Hosting.** Artifact Hub indexes an OCI repository by listing its tags and pulling manifests and layers, so it needs only what gate A already proves, plus the rules below. It supports 29 repository kinds; the ones that may live in an OCI registry are those whose document gives an `oci://` URL form.

| Kind | Hosted on us | Works means |
|---|---|---|
| Helm charts, `oci://host/namespace/chart` | yes | each chart version is a tag that is valid semver; Artifact Hub lists tags and pulls each |
| Container images, `oci://host/[namespace]/repository` | yes | tags carry the required `io.artifacthub.package.readme-url` and `org.opencontainers.image.*` annotations or labels; index-level annotations are read for multi-platform images |
| Repository metadata (verified publisher, ownership claim) | yes | `oras push host/ns/name:artifacthub.io --config /dev/null:application/vnd.cncf.artifacthub.config.v1+yaml artifacthub-repo.yml:application/vnd.cncf.artifacthub.repository-metadata.layer.v1.yaml`; the special tag `artifacthub.io` must be pullable and its layer media type preserved |
| Other OCI-capable kinds (the document of each kind says so: for example Kubewarden and Kyverno policies, Tekton bundles, KCL modules, Inspektor gadgets, Headlamp plugins) | yes, by the same mechanism | tag list, manifest, layers; no registry feature beyond OCI artifacts |
| Signatures shown in the UI | yes | cosign signatures are found by tag fallback or referrers |

Limits to state honestly: Artifact Hub must **reach** the registry. `artifacthub.io` cannot index a private or loopback registry; a self-hosted Artifact Hub can. Proof: `artifacthub.sh` pushes a chart, an annotated image and the metadata artifact, then performs the same three calls Artifact Hub makes and checks the media types survive; nightly, a self-hosted Artifact Hub (its own chart, in kind) adds the repository and must list both packages.

**Being listed.** Our chart is published to `oci://ghcr.io/hologram-technologies/charts/registry`, signed with cosign, with `artifacthub-repo.yml` pushed under the `artifacthub.io` tag (verified publisher), and `Chart.yaml` annotations `artifacthub.io/images`, `artifacthub.io/signKey`, `artifacthub.io/changes`, `artifacthub.io/license`, `artifacthub.io/links`. Our images carry the container-image annotations above. `ah lint` runs in CI. "Official" status is requested after the first release; it is Artifact Hub's call, not a requirement.

## 3. OpenAPI tooling

Policy in `openapi-policy.md`. What interoperates with what:

| # | Tool | Works means | When |
|---|---|---|---|
| 1 | Official OpenAPI 3.1 JSON Schema (`github.com/OAI/OpenAPI-Specification`, `schemas/v3.1`) | the document validates | C |
| 2 | Spectral, ruleset `spectral:oas` plus our ten rules | zero errors, zero warnings | C |
| 3 | `vacuum` | same ruleset, second implementation, zero errors | C |
| 4 | OpenAPI Generator, `python` and `go` | both clients generate and compile; one call each against a running server succeeds | C |
| 5 | `oasdiff` | no breaking change against the last release's document, or the version is a major | C |
| 6 | Schemathesis | every documented operation answers with a documented status and a body matching its schema; 0 undocumented 5xx | N |
| 7 | Scalar (served at `/docs`), Swagger UI, Redoc | each renders the document with no error | R |
| 8 | Route completeness test (ours) | every route the router serves is in the document, and the reverse | C |

Eight checks: 6 per commit, 1 nightly, 1 per release.

## 4. The Docker Registry ecosystem

Added 21 September 2026. Intention: Hologram Registry is equivalent to Docker Registry **as the ecosystem sees it**. Everything that imports from, consumes or integrates with a registry over the Docker Registry HTTP API V2 (the OCI Distribution specification) must work unchanged, the way it does against Harbor or `registry:3`.

On the roadmap this is **one item**, "Works with everything that works with Docker Registry". It is one capability (the same protocol), and the breakdown below is a test matrix, not a plan. Rows already proven by another section point to it and are not counted twice.

The list came from the maintainer. Tool names marked **[unverified]** were not checked to exist or to be maintained; they are claimed only after someone runs them.

| # | Tool | Kind | Works means | Proof | When |
|---|---|---|---|---|---|
| 1 | Docker Distribution (`registry:3`) | peer registry | copy both ways, digests unchanged; the differential reference | gates B, D | C |
| 2 | zot | peer | copy both ways; zot `sync` pulls from us | gates D, E | C, N |
| 3 | Harbor | peer | copy both ways; a Harbor replication rule pulls from us; a Harbor **proxy-cache project** with us as upstream serves a pull | gates D, E; `harbor-proxy.sh` | N |
| 4 | Project Quay | peer | copy both ways with skopeo; Quay **repository mirroring** (it uses skopeo underneath) pulls from us | `quay.sh` | N (copy), R (mirroring) |
| 5 | Sonatype Nexus Repository (community edition) | peer, proxy | a Nexus **docker proxy repository** with us as the remote serves a pull; a hosted repo copies to us | `nexus.sh` | N |
| 6 | JFrog Container Registry / Artifactory | peer, proxy | a **remote repository** with us as upstream serves a pull | by hand; its licence terms decide whether CI may run it | R |
| 7 | GitLab Container Registry | peer | copy both ways against gitlab.com with a token | by hand | R |
| 8 | GitHub Container Registry | peer | copy both ways | gate D | N |
| 9 | Docker Hub | peer | copy both ways | gate D | N |
| 10 | AWS ECR, Google Artifact Registry, Azure Container Registry | peer | copy both ways | gate D, by hand | R |
| 11 | skopeo, crane, ORAS | image tools | section 1 rows 10 to 12 | | C |
| 12 | regclient: `regctl`, `regsync` | image tool, mirror | `regctl image copy`, `tag ls`, `manifest get`; a `regsync` job mirrors a repository **from** us and **to** us | `regclient.sh` | C |
| 13 | image-syncer | mirror | one sync job in each direction | `image-syncer.sh` | N |
| 14 | docker-mirror, container-image-replicator, image-mover **[unverified]** | mirror | not claimed until run | | on request |
| 15 | A web UI for a plain registry (Joxit `docker-registry-ui` as the test subject; added by us because it is the common one) | UI | from a browser origin: list repositories, list tags, show a manifest, delete a tag. Needs CORS and `OPTIONS` answered, see below | `ui.sh` (headless preflight and calls) | N |
| 16 | Dockery, Container Hub, DockDeck **[unverified]** | UI | not claimed until run; expected to need exactly what row 15 proves | | on request |
| 17 | Drydock (update monitoring) **[unverified]** | consumer | reads tags and digests on a schedule; sees a new digest after a push | by hand | R |
| 18 | Trivy, Grype and Syft | scanner | section 1 row 19 | | N |
| 19 | Docker Scout | scanner | analyses an image by registry reference, with login | by hand; needs a Docker account | R |
| 20 | Snyk, Aqua, JFrog Xray, Prisma Cloud | scanner, commercial | not claimed. Run when a user with a licence asks; they read over the same routes as row 18 | | on request |
| 21 | Kubernetes, containerd, CRI-O, Podman | runtime | section 1 rows 4 to 9 | | C, N |
| 22 | GitHub Actions, GitLab CI, buildx cache | CI | section 1 rows 2, 17, 18 | | C, N |
| 23 | Tekton | CI | one Task pushes an image to us and a later Task pulls it | `tekton.sh` in kind | N |
| 24 | Argo Workflows, Argo CD | CI, GitOps | a workflow step pulls from us; Argo CD syncs an OCI chart we host (section 2 row 14) | `argo.sh` in kind | N |
| 25 | Flux, notation | GitOps, signing | section 2 rows 11, 13 | | N |
| 26 | Jenkins | CI | the Docker Pipeline plugin logs in, pushes, pulls | by hand, in a container | R |
| 27 | Harness | CI platform | its "Docker Registry" connector validates and pulls | by hand; needs an account | R |
| 28 | Docker Registry HTTP API V2 / OCI Distribution 1.1 | protocol | conformance, every category; same session as the reference | gates A, B | C |
| 29 | Authentication: Basic | protocol | the reference's challenge, byte for byte | gate B `auth-*`, gate C `login.sh` | C |
| 30 | Authentication: Bearer token from a token service | protocol | **v1.1.** Until then a tool that insists on a token service does not work; none of the tools above is known to insist when the registry offers Basic **[assumption]** | | v1.1 |

Thirty rows. 12 are new work (rows 3 proxy-cache, 4, 5, 6, 7, 12, 13, 15, 17, 19, 23 and 24, 26 and 27); the rest point at proof that sections 1 and 2 or the gates already give. By fastest cadence: 7 per commit, 11 nightly, 8 per release by hand, 3 on request, 1 waits for v1.1.

### What this asks of the product that nothing asked before

| Need | Because | Where it lands |
|---|---|---|
| **CORS that works.** `http.headers` can set `Access-Control-Allow-Origin`, `-Methods`, `-Headers`, `-Expose-Headers` (`Docker-Content-Digest` and `Link` must be exposed), and an `OPTIONS` preflight under `/v2/` is answered **before** authentication | a browser UI on another origin cannot list or delete otherwise. `registry:3` relies on the same `http.headers` recipe | `001` config table (`http.headers` is already Supported); gate B scenario `options` extended with a preflight; FR-R31 |
| **Being a good upstream for a proxy.** `HEAD` on manifests and blobs with `Docker-Content-Digest`, `ETag`, `Content-Length`; `Range`; stable answers for a tag that did not move | Nexus, Artifactory, Harbor proxy-cache and Quay mirroring all poll upstream with `HEAD` and conditional requests | `contracts/registry-api.md` routes 4, 5, 8, 9; FR-R32 |
| **Catalogue, tag paging and delete behave exactly as the reference** | UIs and sync tools walk `_catalog` and `tags/list` with `n` and `last`, and delete by digest | gate A; gate B `catalog-paging`, `tags-paging`, `delete-*` (already planned) |
| **Token authentication** | the wider ecosystem's default for hosted registries | v1.1, already decided |

New requirements, added to `spec.md`: **FR-R31** CORS and unauthenticated preflight; **FR-R32** upstream behaviour for proxies (`HEAD`, `ETag`, conditional requests, `Range`).

Cost: about 3.5 plan days **[estimate]** (four peer scripts 1.5, regclient and image-syncer 0.5, UI and CORS 0.5, Tekton and Argo 0.5, the by-hand checklist 0.5). v1.0 becomes about 83 plan days. At the measured pace it does not move Friday's candidate; the by-hand rows complete before `1.0.0`.

## Totals

| Table | Rows | C | N | R | later |
|---|---|---|---|---|---|
| Clients and tools | 20 | 12 | 8 | 0 | 0 |
| CNCF and adjacent projects | 22 | 9 | 8 | 4 | 1 |
| OpenAPI checks | 8 | 6 | 1 | 1 | 0 |
