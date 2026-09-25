#!/usr/bin/env bash
# Daily Model Hub index snapshot → gethologram.ai registry as model-hub/index:<YYYY-MM-DD>.
# Runs after build-site.sh (same data). Uses the official hologram CLI: compile a thin library .holo whose layers are
# the day's files by BLAKE3, then `hologram push`. Pushes go to the registry over the internal docker network; the
# registry token never leaves this server.
#
# The registry keeps every day (decision 2026-09-18, measured): 96.7 % of a day's bytes are already stored, so history
# costs about 0.5 MB a day. The hub's catalog and model objects (publish.sh, state/model-hub.json) live in the same store
# and link to earlier versions, so the store must never be rebuilt. IPFS (archive.sh) stays the off-site copy.
set -euo pipefail

HUB=/root/hub
TM=$HUB/tm
LOG=$HUB/logs/snapshot.log
DATE=${1:-$(date -u +%F)}
REF="model-hub/index:$DATE"
DATA=$HUB/src/apps/model-hub/web/data
WORK=$TM/work/$DATE
mkdir -p "$HUB/logs" "$TM/work"
exec >>"$LOG" 2>&1
echo "== $(date -u +%FT%TZ) snapshot $DATE start"

test -f "$DATA/models.json"
SOURCE=$(git -C "$HUB/src" rev-parse --short HEAD)

run() {
  docker run --rm --network twenty_default \
    -e HOME=/tm/home -e HOLOGRAM_CONFIG=/tm/live.toml \
    -v "$TM:/tm" -v "$HUB/bin/hologram:/usr/local/bin/hologram:ro" -v /etc/ssl/certs:/etc/ssl/certs:ro \
    -w /tm debian:bookworm-slim "$@"
}

# The registry answers anonymously inside the network, but carry the token anyway so the config stays correct if
# registry-side auth is enabled later.
if [ ! -f "$TM/live.toml" ]; then
  # Start from `hologram init`'s template, move state under /tm, and point [registry] at the hub registry.
  awk -v token="$(cat "$HUB/registry-token")" '
    /^\[/ { section = $0 }
    section == "[registry]" && /^provider/ { print "provider = \"kappa\""; next }
    section == "[registry]" && /^endpoint/ { print "endpoint = \"http://hub-kappa:5000\""; next }
    section == "[registry]" && /^namespace/ { print "namespace = \"model-hub\""; next }
    section == "[registry]" && /^token/ { print "token = \"" token "\""; next }
    section == "[registry]" && /^request_timeout_secs/ { print "request_timeout_secs = 120"; next }
    { print }
  ' "$TM/home/.config/hologram/live.toml"     | sed 's#/tm/home/.local/share/hologram#/tm/data#; s#/tm/home/.local/state/hologram#/tm/state#; s#/tm/home/.cache/hologram#/tm/cache#'     > "$TM/live.toml"
  chmod 600 "$TM/live.toml"
fi

# Already published? Tags must never move; an identical day is a no-op.
if run hologram --json pull "$REF" >/dev/null 2>&1; then
  echo "already published: $REF"
  exit 0
fi

rm -rf "$WORK" && mkdir -p "$WORK"
docker run --rm -v "$DATA:/data:ro" -v "$WORK:/out" -v "$HUB/bin/snapshot.mjs:/app/snapshot.mjs:ro" -w /app node:22-alpine \
  sh -c "npm init -y >/dev/null && npm i --no-fund --no-audit --loglevel=error hash-wasm@4.12.0 >/dev/null && node snapshot.mjs /data /out $DATE $SOURCE"

# Stage the day's layer bytes in the CLI's object store so push finds them locally (the current day only).
rm -rf "$TM/data/registry/blobs/blake3" && mkdir -p "$TM/data/registry/blobs/blake3" "$TM/data/registry/metadata"
cp "$WORK/store/blobs/blake3/"* "$TM/data/registry/blobs/blake3/"

run hologram --json compile "work/$DATE/hologram.json" --thin --output "work/$DATE/index.holo" | tail -1

run hologram --json push "work/$DATE/index.holo" "$REF" | tail -1

# Keep the current day's working copy only; archive.sh reads it next, then IPFS is the durable home.
ls -1d "$TM/work"/* 2>/dev/null | sort | head -n -1 | xargs -r rm -rf
echo "== $(date -u +%FT%TZ) snapshot $DATE ok"
