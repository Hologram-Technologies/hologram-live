# Model Hub deployment: hub.uor.foundation

Everything needed to rebuild `https://hub.uor.foundation` on one Linux host with Docker and an existing Caddy.

| Path | Serves |
|---|---|
| `/` | Model Hub static site (`apps/model-hub/web`, built with `BASE=/`) |
| `/v2/*`, `/_*` | Kappa Registry (OCI distribution). Reads are public; writes need the registry bearer token |

## Layout on the host (`/root/hub`)

| File | Purpose |
|---|---|
| `docker-compose.yml` | `kappa` (registry binary in `debian:bookworm-slim`, 1 GB memory limit) and `site` (Caddy file server). Joins the Caddy network `twenty_default` as `hub-kappa:5000` and `hub-site:8080` |
| `Caddyfile.hub` | The site block for the front Caddy. Replace `{$HUB_REGISTRY_TOKEN}` with the token, or pass it as an environment variable to Caddy |
| `bin/kappa-server` | Built by `build-kappa-server.sh`: kappa-registry at the revision `scripts/check-kappa-registry.sh` tests, with the same two build patches |
| `bin/hologram` | Built by `build-hologram.sh` from this repository's `main` |
| `bin/snapshot.mjs` | Turns one day's index into a library `.holo` manifest with one tensor layer per distinct file |
| `build-site.sh` | Daily: sparse clone of this repository, build the site in `node:22-alpine`, atomic swap. Optional `build.env` holds `HF_TOKEN` |
| `snapshot.sh` | Daily after the site build: `hologram compile --thin` and `hologram push model-hub/index:<YYYY-MM-DD>`. Never moves an existing tag |
| `push-model.sh <owner/name>` | Publishes one Hugging Face model as `models/<owner>/<name>:<revision>`: permissive licence allowlist, 15 GB hub budget, download at the revision the address index pins, SHA-256 checked against the index, 64 MiB chunks, `hologram compile --thin` + `push` |
| `bin/pack-model.mjs` | Chunks and verifies one model; writes `model.json` (`hologram.model-hub.model/v1`: per file path, SHA-256, BLAKE3, size, ordered chunks) as layer 0 |
| `bin/verify-pull.py <store> <owner/name> [out]` | After `hologram pull`, rebuilds each file from its chunks and requires both whole-file addresses to match |
| `health.sh` | Every 5 minutes: probes both services from inside the Caddy container and restarts one that stops answering |
| `registry-token` | Generated with `openssl rand -hex 32`, mode 600. Never committed |

Both binaries are built in `rust:1.97-bookworm` so their glibc matches the runtime image. Build on a workstation, not on the host: the builds need several GB of memory.

## Install

```bash
mkdir -p /root/hub/{bin,store,logs,site} && cd /root/hub
cp <this directory>/{docker-compose.yml,build-site.sh,health.sh,snapshot.sh} .
cp <this directory>/snapshot.mjs bin/ && cp <built>/kappa-server <built>/hologram bin/
umask 077 && openssl rand -hex 32 > registry-token && touch build.env
docker compose up -d
./build-site.sh
crontab -e   # 45 9 * * * /root/hub/build-site.sh && /root/hub/snapshot.sh
             # */5 * * * * /root/hub/health.sh
```

Append `Caddyfile.hub` (with the token) to the front Caddyfile, then `caddy reload`. Point an `A` record for the domain at the host, DNS only, and Caddy issues the certificate.

## Traps (measured)

- The registry builds an outbound HTTPS client at startup: mount `/etc/ssl/certs` or it panics.
- `KAPPA_AUTH_REQUIRED=true` also blocks anonymous reads, so write protection lives in Caddy.
- Kappa blob size defaults to 256 MiB (`KAPPA_MAX_BLOB_SIZE`). The registry holds an upload in memory at about 3x its size: a 327 MB blob OOM-killed it under a 1 GB limit. Model files are therefore published as 64 MiB chunks (peak registry memory measured at 219 MB).
- `hologram compile` rejects duplicate layer entries: give every tensor layer a unique `entry`.
- `hologram init`'s template already contains a `[registry]` table: edit it, do not append another.

## Operations

- **Uptime:** `.github/workflows/model-hub-uptime.yml` probes the public URLs every 15 minutes and opens an issue when they fail.
- **Logs:** `/root/hub/logs/{build-site,snapshot,health}.log`.
- **Verify a published day from anywhere:** `hologram pull hub.uor.foundation/model-hub/index:<YYYY-MM-DD>`.
- **Get a hosted model:** `hologram pull hub.uor.foundation/models/hexgrad/kokoro-82m:f3ff3571791e39611d31c381e3a41a3af07b4987`, then `verify-pull.py` to rebuild the files. Measured: 363 MB in 7.4 s, 72 of 72 files match Hugging Face SHA-256; a byte flipped in a stored chunk on the server is refused with `does not match its bytes`.
