#!/usr/bin/env bash
# Archive one day's index on IPFS and append it to the hash-chained ledger /root/hub/archive.json.
#
#   archive.sh [YYYY-MM-DD]        (default: today; needs the day's snapshot work dir from snapshot.sh)
#
# The day is rebuilt from the snapshot's own artifacts (index.json + layers), so the archive holds exactly the bytes
# the registry holds under model-hub/index:<date>. The directory is packed into a CAR locally, uploaded to the
# Filebase IPFS bucket, and accepted only if the CID Filebase reports equals the CID computed here. The ledger entry
# records the day's CID, the BLAKE3 of its index.json (what the browser verifies first), and the previous entry's
# CID, so history cannot be rewritten silently. The ledger itself is pinned; its CID is recorded by the next entry.
#
# The current day's directory is also kept under /root/hub/archive/<date>, served as
# https://hub.uor.foundation/archive/<date>/. That mirror is only a fast path for the current day: the browser verifies
# every file against the ledger whichever source answered, and falls back to the IPFS gateway when the mirror is
# silent. The hub serves the current index only (decision 2026-09-18); every past day lives on IPFS alone.
set -euo pipefail

HUB=/root/hub
TM=$HUB/tm
DATE=${1:-$(date -u +%F)}
WORK=$TM/work/$DATE
STAGE=$TM/archive/$DATE
LEDGER=$HUB/archive.json
BUCKET=hologram-model-hub
GATEWAY=https://ipfs.filebase.io/ipfs/
MIRROR=https://hub.uor.foundation/archive/
REGISTRY=hub.uor.foundation/model-hub/index
LOG=$HUB/logs/archive.log
mkdir -p "$HUB/logs" "$TM/archive"
exec > >(tee -a "$LOG") 2>&1
echo "== $(date -u +%FT%TZ) archive $DATE"

test -f "$WORK/index.json" || { echo "no snapshot for $DATE at $WORK"; exit 2; }
if [ -f "$LEDGER" ] && python3 -c 'import json,sys; sys.exit(0 if any(e["date"]==sys.argv[2] for e in json.load(open(sys.argv[1]))["days"]) else 1)' "$LEDGER" "$DATE"; then
  echo "already archived: $DATE"; exit 0
fi

# Rebuild the day's directory from the snapshot: every file by its BLAKE3 layer, plus the index that names them.
rm -rf "$STAGE" && mkdir -p "$STAGE"
python3 - "$WORK" "$STAGE" <<'PY'
import json, os, shutil, sys
work, stage = sys.argv[1:]
index = json.load(open(os.path.join(work, "index.json")))
shutil.copy2(os.path.join(work, "index.json"), os.path.join(stage, "index.json"))
for path, address, size in index["files"]:
    src = os.path.join(work, "layers", address.split(":", 1)[1])
    dst = os.path.join(stage, path)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(src, dst)
    assert os.path.getsize(dst) == size, path
print(f"staged {len(index['files'])} files")
PY

ROOT=$(docker run --rm -v "$STAGE:/stage" -v "$TM/archive:/out" -w /out node:22-alpine sh -c "npx --yes ipfs-car@3.1.0 pack /stage --output $DATE.car 2>/dev/null" | tail -1)
echo "local root: $ROOT"

set -a; . "$HUB/filebase.env"; set +a
rc() {
  docker run --rm -v "$TM/archive:/a" -v "$HUB:/hub" -e RCLONE_CONFIG_FB_TYPE=s3 -e RCLONE_CONFIG_FB_PROVIDER=Other \
    -e RCLONE_CONFIG_FB_ENDPOINT=https://s3.filebase.io -e RCLONE_CONFIG_FB_REGION=auto \
    -e RCLONE_CONFIG_FB_ACCESS_KEY_ID="$FILEBASE_KEY" -e RCLONE_CONFIG_FB_SECRET_ACCESS_KEY="$FILEBASE_SECRET" \
    rclone/rclone:1.71 "$@" 2>&1 | grep -v NOTICE || true
}
cid_of() { rc lsjson -M "fb:$BUCKET/$1" | tr -d '\n' | grep -o '"cid":"[^"]*"' | cut -d'"' -f4; }

rc copyto "/a/$DATE.car" "fb:$BUCKET/index/$DATE.car" --header-upload "x-amz-meta-import: car" -q
REMOTE=$(cid_of "index/$DATE.car")
[ "$REMOTE" = "$ROOT" ] || { echo "refused: Filebase reports '$REMOTE', local root is $ROOT"; exit 7; }

# The index's own BLAKE3 is the name of its blob in the snapshot store (the registry layer); no hashing tool needed.
INDEX_B3=""
for f in "$WORK"/store/blobs/blake3/*; do cmp -s "$f" "$WORK/index.json" && INDEX_B3="blake3:$(basename "$f")" && break; done
[ -n "$INDEX_B3" ] || { echo "index.json blob not found in the snapshot store"; exit 3; }

# The previous ledger's CID goes into this entry, then the new ledger is pinned and its CID kept beside it.
PREV_LEDGER=$(cat "$HUB/archive.cid" 2>/dev/null || true)
python3 - "$LEDGER" "$WORK/index.json" "$STAGE/models.json" "$DATE" "$ROOT" "$PREV_LEDGER" "$GATEWAY" "$INDEX_B3" "$MIRROR" "$REGISTRY" <<'PY'
import json, os, sys, time
ledger_path, index_path, models_path, date, root, prev_ledger, gateway, index_b3, mirror, registry = sys.argv[1:]
index = json.load(open(index_path))
models = json.load(open(models_path))
ledger = json.load(open(ledger_path)) if os.path.exists(ledger_path) else {"format": "hologram.model-hub.archive/v1", "gateway": gateway, "days": []}
prev = ledger["days"][-1]["cid"] if ledger["days"] else None
entry = {
    "date": date, "cid": root,
    "models": len(models["models"]), "addressed": sum(1 for m in models["models"] if m.get("state") == "addressed"),
    "files": len(index["files"]), "bytes": sum(f[2] for f in index["files"]),
    "source": index.get("source"), "prev": prev, "prev_ledger": prev_ledger or None,
    "archived": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
entry["index"] = index_b3
ledger["mirror"] = mirror          # current day only
ledger["registry"] = registry      # current day only: hologram pull <registry>:<date>
ledger["days"].append(entry)
ledger["days"].sort(key=lambda e: e["date"])
tmp = ledger_path + ".tmp"
json.dump(ledger, open(tmp, "w"), indent=1, sort_keys=True)
os.replace(tmp, ledger_path)
print(json.dumps({k: entry[k] for k in ("date", "cid", "files", "bytes", "prev")}))
PY

rc copyto /hub/archive.json "fb:$BUCKET/archive.json" -q
[ -f "$HUB/pins.json" ] && rc copyto /hub/pins.json "fb:$BUCKET/pins.json" -q
LEDGER_CID=$(cid_of archive.json)
[ -n "$LEDGER_CID" ] && echo "$LEDGER_CID" > "$HUB/archive.cid"
cp "$LEDGER" "$HUB/site/archive.json"; cp "$HUB/archive.cid" "$HUB/site/archive.cid" 2>/dev/null || true
# The mirror: the same bytes the CAR holds, served from this host for the current day only.
mkdir -p "$HUB/archive"; rm -rf "$HUB/archive/$DATE"; mv "$STAGE" "$HUB/archive/$DATE"
find "$HUB/archive" -mindepth 1 -maxdepth 1 -type d ! -name "$DATE" -exec rm -rf {} +
rm -f "$TM/archive/$DATE.car"
# Warm the gateway for the files the site reads first. A cold read can take tens of seconds; the gateway
# answers 429 above a few parallel requests, so this stays sequential.
for p in index.json models.json; do
  code=$(curl -s -o /dev/null -w "%{http_code}" --retry 4 --retry-delay 2 --max-time 120 "$GATEWAY$ROOT/$p")
  echo "gateway $p: $code"
done
echo "== $(date -u +%FT%TZ) archived $DATE root $ROOT ledger $LEDGER_CID"
