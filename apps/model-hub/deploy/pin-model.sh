#!/usr/bin/env bash
# Pin one model on IPFS through Filebase and record it in /root/hub/pins.json.
#
#   pin-model.sh <owner/name>          FORCE=1 pins again even when this revision is already pinned
#
# Bytes come from Hugging Face at the revision the Hologram address index pins and are refused unless their SHA-256
# matches the index. The files are packed into a CAR locally with ipfs-car, uploaded to the Filebase IPFS bucket, and
# accepted only if the CID Filebase reports equals the CID computed here. Nothing stays on this host afterwards:
# IPFS is the only copy of the weights the hub offers.
set -euo pipefail

ID=${1:?usage: pin-model.sh owner/name}
HUB=/root/hub
TM=$HUB/tm
API=https://humuhumu33.github.io/hologram-api
BUCKET=hologram-model-hub
PINS=$HUB/pins.json
LOG=$HUB/logs/pin.log
mkdir -p "$HUB/logs"
exec > >(tee -a "$LOG") 2>&1
echo "== $(date -u +%FT%TZ) pin $ID"

WORK=$TM/pin/$(echo "$ID" | tr '/' '_')
rm -rf "$WORK" && mkdir -p "$WORK/model"
curl -fsS "$API/v1/huggingface.co/$ID/latest.json" -o "$WORK/index.json"
REV=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["revision"])' "$WORK/index.json")
if [ -z "${FORCE:-}" ] && [ -f "$PINS" ] && python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); m=p.get("models",{}).get(sys.argv[2]); sys.exit(0 if m and m["revision"]==sys.argv[3] else 1)' "$PINS" "$ID" "$REV"; then
  echo "already pinned: $ID@$REV"; rm -rf "$WORK"; exit 0
fi
LICENSE=$(curl -fsS "https://huggingface.co/api/models/$ID" | python3 -c 'import json,sys; d=json.load(sys.stdin); print((d.get("cardData") or {}).get("license") or "")')
case "$LICENSE" in
  apache-2.0|mit|bsd-3-clause|bsd-2-clause|cc-by-4.0) ;;
  *) echo "refused: license '$LICENSE' is not on the re-hosting allowlist"; rm -rf "$WORK"; exit 4 ;;
esac

# Download at the pinned revision, then refuse the model if any file's SHA-256 differs from the index.
python3 - "$WORK/index.json" <<'PY' > "$WORK/urls.txt"
import json, sys
for f in json.load(open(sys.argv[1]))["files"]:
    print(f["path"] + "\t" + f["url"])
PY
while IFS=$'\t' read -r path url; do
  mkdir -p "$WORK/model/$(dirname "$path")"
  curl -fsSL --retry 3 -o "$WORK/model/$path" "$url"
done < "$WORK/urls.txt"
python3 - "$WORK/index.json" "$WORK/model" <<'PY'
import hashlib, json, os, sys
index_path, root = sys.argv[1:]
index = json.load(open(index_path))
for f in index["files"]:
    h = hashlib.sha256()
    with open(os.path.join(root, f["path"]), "rb") as fh:
        for piece in iter(lambda: fh.read(8 << 20), b""):
            h.update(piece)
    got = "sha256:" + h.hexdigest()
    if got != f["address"]:
        print(f"MISMATCH {f['path']}: got {got}, index says {f['address']}")
        sys.exit(3)
print(f"verified {len(index['files'])} files against the index at {index['revision'][:12]}")
PY

ROOT=$(docker run --rm -v "$WORK:/w" -w /w node:22-alpine sh -c "npx --yes ipfs-car@3.1.0 pack model --hidden --output model.car 2>/dev/null" | tail -1)
echo "local root: $ROOT"

set -a; . "$HUB/filebase.env"; set +a
rc() {
  docker run --rm -v "$WORK:/w" -e RCLONE_CONFIG_FB_TYPE=s3 -e RCLONE_CONFIG_FB_PROVIDER=Other \
    -e RCLONE_CONFIG_FB_ENDPOINT=https://s3.filebase.io -e RCLONE_CONFIG_FB_REGION=auto \
    -e RCLONE_CONFIG_FB_ACCESS_KEY_ID="$FILEBASE_KEY" -e RCLONE_CONFIG_FB_SECRET_ACCESS_KEY="$FILEBASE_SECRET" \
    rclone/rclone:1.71 "$@" 2>&1 | grep -v NOTICE || true
}
NAME=$(echo "$ID" | tr 'A-Z' 'a-z')
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
# hidden: the archive includes dotfiles (.gitattributes and the like), so it holds every file of the repository
pins["models"][model] = {"revision": rev, "root": root, "pinned": time.strftime("%Y-%m-%d", time.gmtime()), "hidden": True}
tmp = path + ".tmp"
json.dump(pins, open(tmp, "w"), indent=1, sort_keys=True)
os.replace(tmp, path)
PY
cp "$PINS" "$HUB/site/pins.json"
rm -rf "$WORK"
echo "== $(date -u +%FT%TZ) pinned $ID@$REV root $ROOT"
