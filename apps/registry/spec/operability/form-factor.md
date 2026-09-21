# Form factor: exactly what ships

> **Superseded in part, 21 September 2026: one product.** Hologram Server and Hologram Registry are now one product, **Hologram Registry**. Read this file as the registry's operability requirements. "Hologram Server" is an engineering name for the binary. The `server` image is dropped from v1.0; the `registry` image is also published to Docker Hub. One board: org project 3, "Hologram Registry Roadmap". The decision is recorded in `apps/registry/README.md`.

2026-09-21. One version, one tag (`server-v<version>`, pending decision 2), one build. Reference column: Distribution v3.1.1 [read: `dist:Dockerfile`, `dist:docker-bake.hcl`, `dist:cmd/registry/config-dev.yml` at `5b354e6`; not checked against the published image].

## 1. Images

Both are built from one build stage and hold the same binary.

| | `registry` image | `server` image | Reference |
|---|---|---|---|
| Name | `ghcr.io/hologram-technologies/registry` | `ghcr.io/hologram-technologies/server` | `registry` (Docker Hub), `distribution/distribution` |
| Tags | `1.0.0`, `1.0`, `1`, `latest`; every tag also by digest in the release notes | same | `3.1.1`, `3.1`, `3`, `latest` |
| Base | `gcr.io/distroless/cc-debian12` (gnu). musl and `scratch` only if it links with no patch (`001` decision 6) | same | `alpine:3.23` with `ca-certificates` |
| Shell | none | none | yes |
| User | root, as the reference, so a volume made by the reference opens. The chart overrides to a non-root user | `nonroot` (65532) | root |
| Entry point | `["registry"]` (hard link to `hologram`) | `["hologram"]` | `["registry"]` |
| Command | `["serve", "/etc/distribution/config.yml"]` | `["serve"]` | same as ours |
| Default config | `/etc/distribution/config.yml`, equal to the reference's **key for key**, with one change: `log.level: info`. Listed in `DIFFERENCES.md` as D-006: debug logging by default is a development setting | `/etc/hologram/live.toml`: listen `0.0.0.0:11435`, data `/var/lib/hologram`, token generated at first start | `config-dev.yml`: debug logs, delete **on**, debug listener `:5001` with Prometheus, purge off |
| Ports | `EXPOSE 5000` (and 5001 is used by the default file, as the reference; not exposed, as the reference) | `EXPOSE 11435` | `EXPOSE 5000` |
| Volume | `/var/lib/registry` | `/var/lib/hologram` | `/var/lib/registry` |
| Environment | `OTEL_TRACES_EXPORTER=none` | same | same |
| Also on the path | `hologram`, for `docker exec <c> hologram oci verify` | | |
| Architectures | `linux/amd64`, `linux/arm64`, built on native runners | same | 7 platforms (parity rows 54 to 56) |
| Labels | `org.opencontainers.image.{title,description,version,revision,created,source,url,documentation,licenses,vendor}`; `io.artifacthub.package.readme-url`, `.logo-url`, `.maintainers`, `.license`, `.keywords`, `.alternative-locations` | same | `org.opencontainers.image.*` |
| Annotations | the same keys at **index** level (Artifact Hub reads the index for multi-platform images) | same | |

## 2. Binaries

| System | Artefact | Reference |
|---|---|---|
| Linux x86_64, aarch64 (gnu) | `hologram_<ver>_linux_<arch>.tar.gz` | yes, 7 Linux targets |
| macOS aarch64, x86_64 | `hologram_<ver>_darwin_<arch>.tar.gz` | none |
| Windows x86_64 | `hologram_<ver>_windows_amd64.zip` | none |

Each archive holds: the binary, `README.md`, `LICENSE-APACHE`, `LICENSE-MIT`, `THIRD-PARTY-NOTICES.txt`, `DIFFERENCES.md`, `CHANGELOG.md`, and on Linux `hologram-registry.service` (a systemd unit). Naming follows the reference's `registry_<ver>_<os>_<arch>` pattern so scripts that fetch one can fetch the other. A system that the `001` P0 verdict drops is removed here and named in the release notes.

## 3. Kubernetes

| | Ours | Reference |
|---|---|---|
| Helm chart | `oci://ghcr.io/hologram-technologies/charts/registry`, chart name `registry`. Installs: one **StatefulSet** with `replicas: 1`, one PVC (default 10 Gi), one Service (5000), optional Ingress, optional ServiceMonitor, Secret for htpasswd, Secret or cert-manager `Certificate` for TLS, ConfigMap for `config.yml` | none first-party; community `twuni/docker-registry` [memory] |
| Values | names follow the community chart where one exists (`persistence.*`, `secrets.htpasswd`, `tlsSecretName`, `configData`, `service.*`, `ingress.*`, `resources`, `securityContext`), so a values file moves over | |
| Replicas | `values.schema.json` sets `replicaCount` `maximum: 1`. The store has one writer (redb file lock). A **StatefulSet**, not a Deployment: a rolling Deployment starts the new pod before the old one stops, and the new pod cannot open the store. Update strategy: delete, then create. Downtime per upgrade: the restart, about 1 s plus image pull | the reference scales out over S3; with the filesystem driver it has the same limit and does not say so |
| Security context | `runAsNonRoot: true`, user and `fsGroup` 65532, `readOnlyRootFilesystem: true`, all capabilities dropped | |
| Probes | liveness and readiness on the debug listener's `/debug/health`; `preStop` calls `/debug/health/down`, then waits the drain timeout | |
| Plain manifest | `registry.yaml` attached to the release: the chart rendered with defaults. `kubectl apply -f <release URL>` | none |
| Cluster wiring, documented and tested | containerd `hosts.toml`; CRI-O `registries.conf`; private CA by DaemonSet or node image; `imagePullSecrets`; kind and k3d recipes unchanged | the reference's docs cover none of these |

## 4. Supply chain, per artefact

| Item | How | Verify with |
|---|---|---|
| Checksums | `SHA256SUMS`, plus `<file>.sha256` beside each file as the reference does | `sha256sum -c SHA256SUMS` |
| Signatures | cosign keyless (GitHub OIDC) on both images, the chart, and `SHA256SUMS` | `cosign verify ghcr.io/hologram-technologies/registry:1 --certificate-identity-regexp '^https://github.com/Hologram-Technologies/hologram-live/' --certificate-oidc-issuer https://token.actions.githubusercontent.com` |
| SBOM | SPDX JSON, from the Cargo lock file, attached to each image as an OCI referrer and to the release | `cosign download sbom`, or `oras discover` |
| Provenance | GitHub artifact attestations (SLSA build provenance) for images and archives | `gh attestation verify <file> --repo Hologram-Technologies/hologram-live` |
| Kappa pin | `third_party/kappa/README.md` in the release (`001:FR-019`) | `scripts/check-kappa-pin.sh` |
| Licences | both licence texts and `THIRD-PARTY-NOTICES.txt` inside every image (`/licenses/`) and archive | gate F |

The reference publishes checksums only. No signatures, SBOM or provenance were found in its release tooling [read: `dist:Dockerfile`, `dist:docker-bake.hcl`; not an exhaustive search of its workflows].

## 5. Install one-liners

| Want | Type |
|---|---|
| Try the registry | `docker run -d -p 5000:5000 --name registry ghcr.io/hologram-technologies/registry:1` |
| Remove every trace | `docker rm -fv registry` |
| Swap it in | change `image: registry:3` to `image: ghcr.io/hologram-technologies/registry:1` |
| Kubernetes, Helm | `helm install registry oci://ghcr.io/hologram-technologies/charts/registry` |
| Kubernetes, manifest | `kubectl apply -f https://github.com/Hologram-Technologies/hologram-live/releases/download/server-v1.0.0/registry.yaml` |
| Binary, Linux and macOS | `curl -fsSL https://github.com/Hologram-Technologies/hologram-live/releases/latest/download/install.sh \| sh` : downloads the archive for the system, checks its sha256, installs to `~/.local/bin`. Replaces today's script, which builds from source [read: `install.sh:9-15`] |
| Binary, Windows | the same for `install.ps1` |
| The Server | `docker run -d -p 11435:11435 -v hologram:/var/lib/hologram ghcr.io/hologram-technologies/server:1` |
| Already installed | `hologram update` (exists today; verified by BLAKE3) |

## 6. In the repository

"Standalone within this repository" means (`001` plan, section 0, extended):

| Path | Holds | Owner |
|---|---|---|
| `src/` | the Server, registry module included, behind the `oci` feature | Server |
| `apps/registry/` | the Registry as a product: Dockerfile, default `config.yml`, `DIFFERENCES.md`, `chart/`, `docs/`, `gates/` | Registry |
| `apps/server/` | **new**: the Server image's Dockerfile and default `live.toml`, quick start | Server |
| `apps/desktop/`, `apps/model-hub/`, `apps/docs/` | unchanged | their own |
| `LICENSE-APACHE`, `LICENSE-MIT`, `SECURITY.md`, `SUPPORT.md`, `CHANGELOG.md`, `.github/ISSUE_TEMPLATE/` | **new or rewritten** | Server |
| `README.md` | first screen rewritten: what Hologram Server is; the trial; links to Registry, Desktop, API reference, security. The desktop screenshot moves below | Server |
| `.github/workflows/` | `release-server.yml` publishes everything on `server-v*`; `gates.yml`, `gates-nightly.yml`, `registry-os.yml` path-filtered as `001` says; `desktop` and `docs` trains untouched | |

## 7. Gate F: form factor

Runs against the release candidate's artefacts; blocks the release.

1. `docker inspect` of the `registry` image equals the pinned reference's for `Entrypoint`, `Cmd`, `ExposedPorts`, `Volumes`, and `Env` contains `OTEL_TRACES_EXPORTER=none`.
2. The image's default file, diffed key by key against the reference's, differs only in rows listed in `DIFFERENCES.md`.
3. Both images list `linux/amd64` and `linux/arm64`; each starts and serves under emulation.
4. Every expected archive exists; every checksum, signature and attestation verifies from a clean container that has only `cosign`, `gh` and `sha256sum`.
5. Licence files and the notice exist in every artefact; `cargo deny check licenses` passes.
6. `helm lint`, `ah lint`, chart schema refuses `replicaCount: 2`; the rendered manifest equals the attached `registry.yaml`.
7. `try.sh` (J1) and `server-try.sh` pass on the published tags, by digest.
8. Size, start time and idle memory are measured and written to the release notes (SC-S08).
