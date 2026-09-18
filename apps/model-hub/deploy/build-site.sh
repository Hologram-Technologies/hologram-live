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
# build.env may set HF_TOKEN (catalog refresh) and HUB_BRANCH (a branch to publish ahead of its merge; default main).
[ -f "$HUB/build.env" ] && . "$HUB/build.env"
BRANCH=${HUB_BRANCH:-main}

if [ ! -d "$SRC/.git" ]; then
  git clone --quiet --depth 1 --filter=blob:none --sparse https://github.com/Hologram-Technologies/hologram-live.git "$SRC"
  git -C "$SRC" sparse-checkout set apps/model-hub/web
fi
# A branch published ahead of its merge disappears when it merges: fall back to main rather than stop the daily refresh.
git -C "$SRC" fetch --quiet --depth 1 origin "$BRANCH" || { echo "branch $BRANCH is gone, building main"; BRANCH=main; git -C "$SRC" fetch --quiet --depth 1 origin main; }
git -C "$SRC" reset --quiet --hard FETCH_HEAD
echo "source $BRANCH $(git -C "$SRC" rev-parse --short HEAD)"

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
# pins.json is written by pin-model.sh and must survive the swap: data.mjs reads it from the public URL.
[ -f "$HUB/pins.json" ] && cp -f "$HUB/pins.json" "$HUB/site/pins.json"
[ -f "$HUB/archive.json" ] && cp -f "$HUB/archive.json" "$HUB/site/archive.json"
[ -f "$HUB/archive.cid" ] && cp -f "$HUB/archive.cid" "$HUB/site/archive.cid"
# The site container mounts ./archive on /srv/archive; the mount point must exist inside the read-only site.
mkdir -p "$HUB/site/archive"
docker compose -f "$HUB/docker-compose.yml" restart site >/dev/null
rm -rf "$HUB/site.prev"
echo "== $(date -u +%FT%TZ) build ok: $(find "$HUB/site/models" -name index.html | wc -l) model pages"
