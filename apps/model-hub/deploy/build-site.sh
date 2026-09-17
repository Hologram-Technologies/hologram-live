#!/usr/bin/env bash
# Rebuild the Model Hub site from the public hologram-live repository and swap it in atomically.
# Runs daily from cron on the VPS. No git credentials: the repository is public.
set -euo pipefail

HUB=/root/hub
SRC="$HUB/src"
LOG="$HUB/logs/build-site.log"
mkdir -p "$HUB/logs"
exec >>"$LOG" 2>&1
echo "== $(date -u +%FT%TZ) build start"

if [ ! -d "$SRC/.git" ]; then
  git clone --quiet --depth 1 --filter=blob:none --sparse https://github.com/Hologram-Technologies/hologram-live.git "$SRC"
  git -C "$SRC" sparse-checkout set apps/model-hub/web
else
  git -C "$SRC" fetch --quiet --depth 1 origin main
  git -C "$SRC" reset --quiet --hard origin/main
fi
echo "source $(git -C "$SRC" rev-parse --short HEAD)"

# Build inside node:22-alpine so the host's Node version never matters.
docker run --rm \
  -v "$SRC/apps/model-hub/web:/web" -w /web \
  --env-file "$HUB/build.env" \
  -e BASE=/ \
  node:22-alpine sh -c 'npm ci --no-fund --no-audit >/dev/null && node scripts/data.mjs --limit 500 && node scripts/lint-tokens.mjs && node build.mjs'

NEW="$SRC/apps/model-hub/web/dist"
test -f "$NEW/index.html"
test "$(find "$NEW/models" -name index.html | wc -l)" -ge 400

rm -rf "$HUB/site.next"
cp -a "$NEW" "$HUB/site.next"
# The site container mounts ./site; replace its contents in one rename.
rm -rf "$HUB/site.prev"
if [ -d "$HUB/site" ]; then mv "$HUB/site" "$HUB/site.prev"; fi
mv "$HUB/site.next" "$HUB/site"
docker compose -f "$HUB/docker-compose.yml" restart site >/dev/null
rm -rf "$HUB/site.prev"
echo "== $(date -u +%FT%TZ) build ok: $(find "$HUB/site/models" -name index.html | wc -l) model pages"
