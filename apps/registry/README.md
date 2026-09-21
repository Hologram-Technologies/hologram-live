# Hologram Registry

A drop-in replacement for the Docker Registry image (`registry:3`), built on the Kappa Registry store.

Take the compose file from the registry's own documentation and change only the image name. Clients, commands,
port 5000, the `REGISTRY_*` settings, TLS, htpasswd login, delete-off-by-default and `garbage-collect` behave the same.
Existing data migrates with one command.

**Status: under construction, not released.** The `/v2/` API is built behind the cargo feature `oci` and passes
the OCI Distribution conformance suite (74 passed, 0 failed); unmodified `docker` and `skopeo` work through it.
Login, TLS, the `REGISTRY_*` settings, the image and the operator commands are not built yet, and it is not yet
equal to `registry:3` everywhere: the differences left are counted by gate B on every change.

## What it will look like at 1.0

```bash
docker run -d -p 5000:5000 --restart=always --name registry ghcr.io/hologram-technologies/registry:1
docker tag ubuntu localhost:5000/ubuntu && docker push localhost:5000/ubuntu
docker pull localhost:5000/ubuntu
```

Moving in from an existing registry, and back out, is a copy:

```bash
skopeo sync --src docker --dest docker old:5000 new:5000      # or: hologram oci import /var/lib/registry
docker exec registry hologram oci verify                      # re-hash everything held
```

Data on disk is Kappa's layout, so an existing `/var/lib/registry` volume does not open in place. That is the one
step that differs from the reference: drop-in for clients and configuration; existing data migrates with one command.

## How it is built

This directory is the product. The registry itself is a server module, like the other ten.

| Part | Where |
|---|---|
| The `/v2/` API, module id `dev.hologram.live.oci` | `src/modules/oci/` |
| The store adapter, the only code that names a Kappa type | `src/oci_store/` |
| `REGISTRY_*` and `config.yml` compatibility | `src/registry_compat/` |
| `hologram oci verify`, `import`, `garbage-collect` | `src/cli/oci.rs` |
| Image, default config, Helm chart, docs, the gates | here |

```
apps/registry/
  Dockerfile                built from the repository root:  docker build -f apps/registry/Dockerfile .
  Dockerfile.dockerignore
  config.yml                the image's default, equal to the reference's
  DIFFERENCES.md            every kept difference from registry:3
  chart/                    Helm chart
  docs/                     "Coming from Docker Registry", migration, the settings table
  gates/                    differential/ · clients/ · corpus/ · compose/ · release/
```

The registry code sits behind the cargo feature `oci`, **off by default**. A stock build of Hologram Live pulls no
Kappa crate and behaves as it does today; `scripts/check-product-boundaries.sh` holds that line. The image and the
release binaries build with `--features oci`.

## How equivalence is proven

Eight gates, in CI. A release is refused unless all are green. A to E prove that it is the same registry:

| Gate | Proves |
|---|---|
| A | OCI Distribution 1.1 conformance suite, every category on |
| B | The same scripted session against a pinned `registry:3` and against this, with every response matching. Kept differences are listed in `DIFFERENCES.md`; an unlisted one fails, and so does a listed one that no longer happens |
| C | Unchanged tools: docker, buildx, containerd, a Kubernetes pull, oras, crane, skopeo, helm, cosign, ollama |
| D | A fixed corpus copied peer → this → peer with identical digests: Zot, Harbor, GitHub, Docker Hub, the clouds |
| E | Harbor replication and Zot sync pulling from this |

F, G and H hold what any server owes its operator: the image's form factor, operations (health, metrics, a clean
stop, certificate reload), and a valid OpenAPI document.

## Not in v1

Cloud storage back ends, token auth, pull-through mirror mode, webhooks, Let's Encrypt, a hosted service. A setting
that asks for one of these stops the start and names the key; it is never ignored.

## Plan

The dependency decision is ADR 025 (`specs/adrs/025-kappa-crates-in-the-graph.md`); its evidence is the spike
verdict, `docs/superpowers/specs/2026-09-21-registry-p0-verdict.md`. The specification and the phased plan follow
in their own pull request.
