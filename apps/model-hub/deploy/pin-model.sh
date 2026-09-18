#!/usr/bin/env bash
# Pin one hosted model on IPFS through Filebase and record it in /root/hub/pins.json.
#
#   pin-model.sh <owner/name>
#
# Bytes come from this hub's own registry (hologram pull, every chunk verified), are rebuilt into files and checked
# against both whole-file addresses (verify-pull.py), packed into a CAR locally with ipfs-car, uploaded to the Filebase
# IPFS bucket, and accepted only if the CID Filebase reports equals the CID computed here.
set -euo pipefail

ID=${1:?usage: pin-model.sh owner/name}
HUB=/root/hub
TM=$HUB/tm
BUCKET=hologram-model-hub
PINS=$HUB/pins.json
LOG=$HUB/logs/pin.log
mkdir -p "$HUB/logs"
exec > >(tee -a "$LOG") 2>&1
echo "== $(date -u +%FT%TZ) pin $ID"

NAME=$(echo "$ID" | tr 'A-Z' 'a-z')
REV=$(curl -fsS "https://hub.uor.foundation/v2/models/$NAME/tags/list" | python3 -c 'import json,sys; t=json.load(sys.stdin)["tags"]; print(t[-1])')
REF="hub.uor.foundation/models/$NAME:$REV"
if [ -f "$PINS" ] && python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); m=p.get("models",{}).get(sys.argv[2]); sys.exit(0 if m and m["revision"]==sys.argv[3] else 1)' "$PINS" "$ID" "$REV"; then
  echo "already pinned: $ID@$REV"; exit 0
fi

WORK=$TM/pin/$(echo "$ID" | tr '/' '_')
rm -rf "$WORK" && mkdir -p "$WORK/model"
sed "s#http://hub-kappa:5000#https://hub.uor.foundation#; s#^token = .*#token = \"\"#; s#/tm/data#/w/data#; s#/tm/state#/w/state#; s#/tm/cache#/w/cache#" "$TM/live.toml" > "$WORK/live.toml"
docker run --rm -e HOME=/w -e HOLOGRAM_CONFIG=/w/live.toml -v "$WORK:/w" -v "$HUB/bin/hologram:/usr/local/bin/hologram:ro" \
  -v /etc/ssl/certs:/etc/ssl/certs:ro debian:bookworm-slim hologram --json pull "$REF" >/dev/null
python3 "$HUB/bin/verify-pull.py" "$WORK/data/registry" "$ID" "$WORK/model"
rm -rf "$WORK/data"

ROOT=$(docker run --rm -v "$WORK:/w" -w /w node:22-alpine sh -c "npx --yes ipfs-car@3.1.0 pack model --output model.car 2>/dev/null" | tail -1)
echo "local root: $ROOT"

set -a; . "$HUB/filebase.env"; set +a
rc() {
  docker run --rm -v "$WORK:/w" -e RCLONE_CONFIG_FB_TYPE=s3 -e RCLONE_CONFIG_FB_PROVIDER=Other \
    -e RCLONE_CONFIG_FB_ENDPOINT=https://s3.filebase.io -e RCLONE_CONFIG_FB_REGION=auto \
    -e RCLONE_CONFIG_FB_ACCESS_KEY_ID="$FILEBASE_KEY" -e RCLONE_CONFIG_FB_SECRET_ACCESS_KEY="$FILEBASE_SECRET" \
    rclone/rclone:1.71 "$@" 2>&1 | grep -v NOTICE || true
}
KEY="models/$NAME@$REV.car"
rc copyto /w/model.car "fb:$BUCKET/$KEY" --header-upload "x-amz-meta-import: car" -q
REMOTE=$(rc lsjson -M "fb:$BUCKET/$KEY" | tr -d '\n' | grep -o '"cid":"[^"]*"' | cut -d'"' -f4)
if [ "$REMOTE" != "$ROOT" ]; then
  echo "refused: Filebase reports $REMOTE, local root is $ROOT"; exit 7
fi

# Per-file CIDs are not needed: the gateway resolves <root>/<path>, and the browser checks each file's SHA-256.
python3 - "$PINS" "$ID" "$REV" "$ROOT" <<'PY'
import json, os, sys, time
path, model, rev, root = sys.argv[1:]
pins = json.load(open(path)) if os.path.exists(path) else {"format": "hologram.model-hub.pins/v1", "gateway": "https://ipfs.filebase.io/ipfs/", "models": {}}
pins["models"][model] = {"revision": rev, "root": root, "pinned": time.strftime("%Y-%m-%d", time.gmtime())}
tmp = path + ".tmp"
json.dump(pins, open(tmp, "w"), indent=1, sort_keys=True)
os.replace(tmp, path)
PY
cp "$PINS" "$HUB/site/pins.json"
rm -rf "$WORK"
echo "== $(date -u +%FT%TZ) pinned $ID@$REV root $ROOT"
