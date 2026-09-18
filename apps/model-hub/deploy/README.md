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
| `snapshot.sh` | Daily after the site build: `hologram compile --thin` and `hologram push model-hub/index:<YYYY-MM-DD>`. Tags never move and the registry keeps every day: its store also holds the hub's published objects, so it is never rebuilt |
| `publish.sh` + `pub/hub.mjs` | Daily after `snapshot.sh`: publishes the day's models, sources and catalog as objects through the Hologram Server (`POST /api/v1/objects` over the internal network), then points the descriptor `state/model-hub.json` (served at `/.well-known/model-hub.json`) at the new catalog. `pub/llms.txt` is the agent guide served at `/llms.txt`. `pub.env` (mode 600) carries the tokens |
| `bin/hub-resolve.mjs` | The `resolve` service (alias `hub-resolve:8090`, 96 MB, no dependencies): Hugging Face's dialect for `HF_ENDPOINT`, answered from `site/data/files` and, for models the site does not show, from the address index; every file request is a redirect to a healthy source. Health is one verified fetch per source every 30 s; `state/resolve/override.json` `{"down": ["huggingface.co"]}` marks a source down for a drill; `GET /api/hub/health` shows the table. `build-site.sh` restarts it after the site swap |
| `hologram/live.toml` | Configuration of the `hologram` service (`hologram serve`, alias `hub-server:11435`, 256 MB): the public read API behind the front Caddy (`/api/v1/objects/{id}`, `/api/v1/capabilities`, `/openapi.json`, `/docs`, `/healthz`); publishing needs the publisher token |
| `pin-model.sh <owner/name>` | Pins one model on IPFS through Filebase (dotfiles included: `ipfs-car pack --hidden`; `FORCE=1` pins an already pinned revision again): permissive-licence allowlist, download from Hugging Face at the revision the address index pins, every file's SHA-256 checked against the index, CAR packed locally (`ipfs-car`), uploaded to the IPFS bucket, accepted only if the CID Filebase reports equals the CID computed here. Records `pins.json`, which the site reads to turn the IPFS column green. Nothing stays on the host: IPFS is the only copy of weights the hub offers (decision 2026-09-18) |
| `archive.sh [date]` | Daily after `snapshot.sh`: rebuilds the day's directory from the snapshot, packs a CAR (`ipfs-car`), uploads it to the Filebase IPFS bucket, accepts it only if the read-back CID equals the local root, appends the day to the hash-chained ledger `archive.json` (pinned with `pins.json`; the ledger CID kept in `archive.cid`), keeps the current day's directory under `archive/<date>` as the fast mirror the site container serves at `/archive/<date>/` (older days removed), and warms the gateway. IPFS is the only backup of the index. Schema in `../README.md` |
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
crontab -e   # 45 9 * * * /root/hub/build-site.sh && /root/hub/snapshot.sh && { /root/hub/publish.sh; /root/hub/archive.sh; }
             # */5 * * * * /root/hub/health.sh
```

Append `Caddyfile.hub` (with the token) to the front Caddyfile, then `caddy reload`. Point an `A` record for the domain at the host, DNS only, and Caddy issues the certificate.

## Traps (measured)

- The registry builds an outbound HTTPS client at startup: mount `/etc/ssl/certs` or it panics.
- `KAPPA_AUTH_REQUIRED=true` also blocks anonymous reads, so write protection lives in Caddy.
- Kappa blob size defaults to 256 MiB (`KAPPA_MAX_BLOB_SIZE`). The registry holds an upload in memory at about 3x its size: a 327 MB blob OOM-killed it under a 1 GB limit. Anything larger than the daily index files must be chunked (64 MiB was measured at 219 MB peak) or, as decided, kept off the registry entirely.
- `hologram compile` rejects duplicate layer entries: give every tensor layer a unique `entry`.
- `hologram init`'s template already contains a `[registry]` table: edit it, do not append another.

## Operations

- **Uptime:** `.github/workflows/model-hub-uptime.yml` runs `uptime.mjs` every 15 minutes and keeps one issue open per outage. Beyond the public URLs it checks that the pointer names a catalog, that the catalog and one model hash to their addresses, that the snapshot is no older than 36 hours (the only check that notices a silently failing publish), and that the closed doors stay closed: unknown paths and gRPC 404, anonymous publish, list and search 401. Set `EXPECT_SEARCH=200` in the workflow when search opens. Run it by hand: `npm i --no-save hash-wasm@4.12.0 && node uptime.mjs`.
- **Logs:** `/root/hub/logs/{build-site,snapshot,publish,archive,health}.log`.
- **Rollback:** Data: write the current catalog's `prev` into `state/model-hub.json` and copy it to `site/.well-known/`. Server binary: keep `bin/hologram.prev`, swap, `docker compose restart hologram`. Caddy: keep a dated copy of the front Caddyfile before every edit, write it back in place (the file is a single-file bind mount) and reload.
- **Backup:** none on this host and no S3 copy: every day's index is a pinned CAR on IPFS and the ledger is pinned. Losing the host loses the registry's day tags, the published objects and the current-day mirror; the next daily run republishes today, and every past day's index remains on IPFS.
- **Archive:** `archive.json` on the site is the ledger; `logs/archive.log` records each day's CID and the read-back check. The Filebase gateway answers a cold CID in tens of seconds and rate-limits parallel reads (429 above a few at once), which is why the mirror exists; the mount point `site/archive` must exist inside the read-only site (build-site.sh creates it) or the site container will not start.
- **Verify a published day from anywhere:** `hologram pull hub.uor.foundation/model-hub/index:<YYYY-MM-DD>`.
- **IPFS:** measured 2026-09-18 with Kokoro-82M: Filebase read-back CID equals the local CID; the 327 MB weights fetched from `ipfs.filebase.io` in 33 s at 10 MB/s match Hugging Face SHA-256; the gateway sends `Access-Control-Allow-Origin: *`, so browser Verify reports "Identical bytes from Hugging Face, ModelScope and IPFS". Public gateways `ipfs.io` and `dweb.link` rate-limited the same CID (429), so the site uses the Filebase gateway.
- **Get a pinned model:** `pins.json` gives the IPFS root per model; `https://ipfs.filebase.io/ipfs/<root>/<path>` serves each file, and the browser checks it against the index SHA-256.
