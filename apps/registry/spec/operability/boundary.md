# Boundary: what is Hologram Server, what is Hologram Registry

> **Superseded in part, 21 September 2026: one product.** Hologram Server and Hologram Registry are now one product, **Hologram Registry**. Read this file as the registry's operability requirements. "Hologram Server" is an engineering name for the binary. The `server` image is dropped from v1.0; the `registry` image is also published to Docker Hub. One board: org project 3, "Hologram Registry Roadmap". The decision is recorded in `apps/registry/README.md`.

2026-09-21. Every other file in this folder must agree with this page.

Marks: **[ran]**, **[read]** file and line or URL, **[docs]** fetched today, **[memory]**, **[assumption]**.

## The line, in one sentence

**Hologram Server is the `hologram` binary and everything a server owes its operator. Hologram Registry is that same binary, built with the `oci` feature and started in registry mode, packaged so it answers to the name `registry`.**

One codebase, one binary, one version number. Two images, because two kinds of user arrive by two doors.

The repository already says so: "This repository produces two independent products: Hologram Server, the standalone `hologram` binary … and Hologram Desktop" [read: `README.md:5-8`]. Registry becomes the first product **built on** Server, as Desktop is the first product built **around** it.

## Push back on the brief, stated once

"Equivalent to Distribution in feature set" cannot be a requirement on the Server. Distribution is a registry. Its registry features (the `/v2/` API, storage, garbage collection) are compared with **Hologram Registry**. What is compared with the **Server** is what Distribution is as a piece of server software: how it is installed, configured, secured, observed, upgraded and supported. The parity matrix therefore has an owner column, and every row belongs to exactly one side.

## What each name means

| Name | Is | Is not |
|---|---|---|
| **Hologram Server** | The `hologram` binary: CLI plus service. The module host. The listener, TLS, the authentication framework, configuration, logs, metrics, tracing, health, the OpenAPI document, packaging, release, upgrade, support | A registry. A desktop app |
| **Hologram Registry** | A distribution of Hologram Server: the `oci` module on, registry mode on, the reference's settings and command names understood, its own image and chart | A separate binary, a fork, or a second codebase |
| **Hologram Live** | The repository, `Hologram-Technologies/hologram-live`, and the Rust package name `hologram-live`. An engineering name | A product name. It does not appear in anything a user downloads or reads first |
| **Hologram Desktop** | The Tauri app in `apps/desktop` that runs the Server as a sidecar | In scope here |

## Which requirements belong where

| Concern | Server | Registry |
|---|---|---|
| Process: start, stop, signals, graceful drain, exit codes | owns | inherits |
| Listeners: plain, TLS, second listener for metrics and health | owns | sets values from `http.*` |
| Authentication framework: bearer token for the Server API; the hook that lets a module authenticate itself | owns | htpasswd (v1), token service (v1.1) |
| Administration surface, separate from the public one | owns | uses it for `verify`, shutdown |
| Configuration: TOML, `HOLOGRAM_*`, validation, refusing unknown keys | owns | `config.yml`, `REGISTRY_*`, the key table |
| Logs, metrics, tracing, health, readiness | owns the machinery and the endpoint shapes | names its own metrics; maps `log.*`, `http.debug.*`, `health.*` |
| OpenAPI document, `/docs`, the policy in `openapi-policy.md` | owns | contributes the `/v2/` paths |
| The module host and the other ten modules | owns | turns them off |
| `/v2/` behaviour, errors, grammars, garbage collection, import, verify, adopt, `DIFFERENCES.md` | | owns (all of `001`) |
| The Kappa store crates | | owns; the Server builds without them by default (`001` plan, section 0, rule 1) |
| Release: versioning, tags, binaries, checksums, signatures, SBOM, provenance, update channel | owns | adds its image, chart and the five gates as a release condition |
| Upgrade and rollback between our own versions | owns the policy | owns the data layout marker |
| Security policy, disclosure address, supported versions | owns | inherits |
| Documentation site | owns the site and the Server section | owns one section, "Registry" |
| Licence files | owns | inherits |

## What a user downloads, by name

| Arrives wanting | Downloads | Sees on first run |
|---|---|---|
| "A registry I can swap in for `registry:3`" | `ghcr.io/hologram-technologies/registry:1` | Nothing new: it listens on 5000, answers `/v2/`, logs one line. Entry point `registry`, command `serve /etc/distribution/config.yml`, as the reference [read: `dist:Dockerfile`, last 8 lines] |
| "The Hologram Server" (objects, files, `.holo` apps, models, chat, and the registry as one module among them) | `ghcr.io/hologram-technologies/server:1`, or the `hologram` binary for their system | `hologram` with no arguments prints help: what it is in two lines, the five commands to start (`init`, `serve`, `status`, `modules list`, `doctor`), and the address of `/docs` |
| "It on Kubernetes" | Helm chart `oci://ghcr.io/hologram-technologies/charts/registry` | A StatefulSet, one replica, a Service on 5000 |

Both images are built from one build stage and carry the same binary. They differ in entry point, default command, default configuration, exposed port and labels. Cost of the second image: half a day.

`hologram` with no arguments, and `registry` with no arguments, must each exit 0 and fit one screen. Today `hologram` prints clap's generated help [read: `src/cli/mod.rs:37-55`]; it lists 27 subcommands, which is not a first screen. Requirement FR-S01.

## One name per thing

| Thing | Name | Notes |
|---|---|---|
| Product, platform | Hologram Server | |
| Product, registry | Hologram Registry | |
| Binary | `hologram` | also reachable as `registry` by hard link inside the registry image |
| Rust package | `hologram-live` | unchanged; engineering only |
| Server image | `ghcr.io/hologram-technologies/server` | |
| Registry image | `ghcr.io/hologram-technologies/registry` | as `001` plan section 0 |
| Helm chart | `oci://ghcr.io/hologram-technologies/charts/registry` | chart name `registry` |
| Module id | `dev.hologram.live.oci` | the id `…kappa-registry` is taken by the objects API [read: `src/modules/mod.rs:32`] |
| CLI group for registry operations | `hologram oci …` | `hologram registry …` already exists and means the objects provider [read: `src/cli/mod.rs:86`] |
| Server configuration | `live.toml`, keys as today, environment `HOLOGRAM_*` | |
| Registry configuration | `config.yml`, environment `REGISTRY_*` | the reference's names |
| Hologram-only registry settings | TOML table `[oci]`, environment `HOLOGRAM_OCI_*` | never inside the `REGISTRY_*` namespace |
| Version | one number, from `Cargo.toml` | see conflict 1 |
| Docs sections | "Server", "Registry", "Desktop" | one site: `hologram-technologies.github.io/hologram-live` [read: `RELEASING.md:56`] |

## Conflicts with the Registry plan (`001`), and which side moves

| # | Conflict | Moves |
|---|---|---|
| 1 | **Tags.** `001` releases on `registry-v*` tags so the `server-v*` workflow stays quiet. But there is one binary and one version: a registry release **is** a server release. Two tag families for one artefact will drift | `001` moves. One tag, `server-v<version>`, publishes binaries, both images and the chart. The five gates become a condition of that workflow. **The maintainer's decision 2** |
| 2 | **Port 5001.** `001` puts the loopback administration listener on `127.0.0.1:5001`. The reference's own default configuration puts its debug and metrics listener on `:5001` [read: `dist:cmd/registry/config-dev.yml`] | `001` moves. Administration goes to a Unix socket at `<root>/live/state/admin.sock` (a named pipe on Windows). 5001 is left for `http.debug.addr` |
| 3 | **`http.debug.*` refused.** `001` refuses the metrics port as out of scope. The reference image turns it on by default, so an image whose default file equals the reference's would refuse its own default | `001` moves. Metrics and `/debug/health` are v1, owned by the Server |
| 4 | **"Delete is off by default."** True of the binary. The reference **image** ships a file with `storage.delete.enabled: true`, log level `debug`, and upload purging off [read: same file] | `001` keeps the binary default and copies the image default, and says both in its docs. No behaviour change; the sentence in its brief and spec is imprecise |
| 5 | **TLS, the self-authentication hook, the second listener, registry-mode paths** are planned as registry work | They are Server features. Same code, same days; they move to Server requirements and are written so another module can use them |
| 6 | **`001` trusted a recalled default config (research D2).** The real one differs (4 above) | `001` research D2 is corrected from the source read today |

`impact-on-001.md` lists every edit these imply. The `001` files were not touched.
