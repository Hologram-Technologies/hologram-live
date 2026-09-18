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
| `build-site.sh` | Daily: sparse clone of this repository, build the site in `node:22-alpine`, atomic swap. Optional `build.env` holds `HF_TOKEN` and `HUB_BRANCH` (publish a branch ahead of its merge; falls back to `main` once the branch is gone) |
| `snapshot.sh` | Daily after the site build: `hologram compile --thin` and `hologram push model-hub/index:<YYYY-MM-DD>`. Never moves an existing tag |
| `push-model.sh <owner/name>` | Publishes one Hugging Face model as `models/<owner>/<name>:<revision>`: permissive licence allowlist, 15 GB hub budget, download at the revision the address index pins, SHA-256 checked against the index, 64 MiB chunks, `hologram compile --thin` + `push` |
| `bin/pack-model.mjs` | Chunks and verifies one model; writes `model.json` (`hologram.model-hub.model/v1`: per file path, SHA-256, BLAKE3, size, ordered chunks) as layer 0 |
| `bin/verify-pull.py <store> <owner/name> [out]` | After `hologram pull`, rebuilds each file from its chunks and requires both whole-file addresses to match |
| `pin-model.sh <owner/name>` | Pins a hosted model on IPFS through Filebase: pulls it back from the public registry, rebuilds and verifies the files, packs a CAR locally (`ipfs-car`), uploads it to the IPFS bucket, and accepts the pin only if the CID Filebase reports equals the CID computed here. Records `pins.json`, which the site reads to turn the IPFS column green. Runs automatically after `push-model.sh` |
| `archive.sh [date]` | Daily after `snapshot.sh`: rebuilds the day's directory from the snapshot, packs a CAR (`ipfs-car`), uploads it to the Filebase IPFS bucket, accepts it only if the read-back CID equals the local root, appends the day to the hash-chained ledger `archive.json` (pinned; its CID kept in `archive.cid`), keeps the directory under `archive/<date>` as the fast mirror the site container serves at `/archive/<date>/`, and warms the gateway. Schema in `../README.md` |
| `backup.sh` | Nightly (03:15 UTC): `rclone sync` of the content-addressed store to the private Filebase S3 bucket, object-count check, disk-pressure warning at 80%. Restore: sync it back and restart |
| `filebase.env` | `FILEBASE_KEY` and `FILEBASE_SECRET`, mode 600. Never committed |
| `health.sh` | Every 5 minutes: probes both services from inside the Caddy container and restarts one that stops answering |
| `registry-token` | Generated with `openssl rand -hex 32`, mode 600. Never committed |

Both binaries are built in `rust:1.97-bookworm` so their glibc matches the runtime image. Build on a workstation, not on the host: the builds need several GB of memory.

## Install

```bash
mkdir -p /root/hub/{bin,store,logs,site} && cd /root/hub
cp <this directory>/{docker-compose.yml,build-site.sh,health.sh,snapshot.sh,archive.sh} .
cp <this directory>/snapshot.mjs bin/ && cp <built>/kappa-server <built>/hologram bin/
umask 077 && openssl rand -hex 32 > registry-token && touch build.env
docker compose up -d
./build-site.sh
crontab -e   # 45 9 * * * /root/hub/build-site.sh && /root/hub/snapshot.sh && /root/hub/archive.sh
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
- **Archive:** `archive.json` on the site is the ledger; `logs/archive.log` records each day's CID and the read-back check. The Filebase gateway answers a cold CID in tens of seconds and rate-limits parallel reads (429 above a few at once), which is why the mirror exists; the mount point `site/archive` must exist inside the read-only site (build-site.sh creates it) or the site container will not start.
- **Verify a published day from anywhere:** `hologram pull hub.uor.foundation/model-hub/index:<YYYY-MM-DD>`.
- **IPFS:** measured 2026-09-18 with Kokoro-82M: Filebase read-back CID equals the local CID; the 327 MB weights fetched from `ipfs.filebase.io` in 33 s at 10 MB/s match Hugging Face SHA-256; the gateway sends `Access-Control-Allow-Origin: *`, so browser Verify reports "Identical bytes from Hugging Face, ModelScope and IPFS". Public gateways `ipfs.io` and `dweb.link` rate-limited the same CID (429), so the site uses the Filebase gateway.
- **Get a hosted model:** `hologram pull hub.uor.foundation/models/hexgrad/kokoro-82m:f3ff3571791e39611d31c381e3a41a3af07b4987`, then `verify-pull.py` to rebuild the files. Measured: 363 MB in 7.4 s, 72 of 72 files match Hugging Face SHA-256; a byte flipped in a stored chunk on the server is refused with `does not match its bytes`.
