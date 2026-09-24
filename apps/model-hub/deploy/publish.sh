#!/usr/bin/env bash
# Daily, after build-site.sh and snapshot.sh: publish the day's models, sources and catalog THROUGH the Hologram Server
# (POST /api/v1/objects over the internal docker network), then point the descriptor at the new catalog.
# The descriptor's home is state/model-hub.json; the site serves a copy at /.well-known/model-hub.json.
# Safe to run again: unchanged models and an unchanged catalog publish nothing.
set -euo pipefail
HUB=/root/hub
PUB=$HUB/pub
DATA=$HUB/src/apps/model-hub/web/data
mkdir -p "$HUB/logs" "$HUB/state"
exec >>"$HUB/logs/publish.log" 2>&1
echo "== $(date -u +%FT%TZ) publish"
[ -f "$DATA/models.json" ] || { echo "no catalog data at $DATA"; exit 1; }

# Work from a private copy: a build started by hand resets and rewrites the data directory while it runs
# (seen 2026-09-18: a file vanished mid-publish). From cron the steps are sequential and the copy is merely cheap.
rm -rf "$PUB/data" && cp -a "$DATA" "$PUB/data"

# pub.env (mode 600) carries the server token to the container; it never appears on a command line.
docker run --rm --network twenty_default --env-file "$HUB/pub.env" \
	-v "$PUB:/pub" -v "$PUB/data:/data:ro" -v "$HUB/pins.json:/pins.json:ro" -v "$HUB/state:/state" -v "$HUB/site:/site" \
	-w /pub node:22-alpine sh -c \
	'[ -d node_modules/hash-wasm ] || npm i --no-save --silent hash-wasm@4.12.0 >/dev/null; node hub.mjs publish /data --pins /pins.json --site /site'
echo "== $(date -u +%FT%TZ) publish ok"
