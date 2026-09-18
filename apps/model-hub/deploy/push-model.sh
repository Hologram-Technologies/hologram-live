#!/usr/bin/env bash
# Publish one Hugging Face model to hub.uor.foundation as models/<owner>/<name>:<revision>.
#
#   push-model.sh <owner/name>
#
# Bytes come from Hugging Face at the revision the Hologram address index pins, are refused unless their SHA-256
# matches the index, and are published with the official hologram CLI (compile --thin, push). Work files are removed
# afterwards; the registry is the only lasting copy on this host.
set -euo pipefail

ID=${1:?usage: push-model.sh owner/name}
HUB=/root/hub
TM=$HUB/tm
API=https://humuhumu33.github.io/hologram-api
BUDGET_BYTES=$((15 * 1024 * 1024 * 1024))
WORK=$TM/models/$(echo "$ID" | tr '/' '_')
LOG=$HUB/logs/push-model.log
mkdir -p "$HUB/logs"
exec > >(tee -a "$LOG") 2>&1
echo "== $(date -u +%FT%TZ) $ID"

rm -rf "$WORK" && mkdir -p "$WORK/files"
curl -fsS "$API/v1/huggingface.co/$ID/latest.json" -o "$WORK/index.json"
REV=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["revision"])' "$WORK/index.json")
LICENSE=$(curl -fsS "https://huggingface.co/api/models/$ID" | python3 -c 'import json,sys; d=json.load(sys.stdin); print((d.get("cardData") or {}).get("license") or "")')
case "$LICENSE" in
  apache-2.0|mit|bsd-3-clause|bsd-2-clause|cc-by-4.0) ;;
  *) echo "refused: license '$LICENSE' is not on the re-hosting allowlist"; exit 4 ;;
esac

NAME=$(echo "$ID" | tr 'A-Z' 'a-z')
REF="models/$NAME:$REV"
if docker run --rm --network twenty_default -e HOME=/tm/home -e HOLOGRAM_CONFIG=/tm/live.toml -v "$TM:/tm" \
     -v "$HUB/bin/hologram:/usr/local/bin/hologram:ro" -v /etc/ssl/certs:/etc/ssl/certs:ro debian:bookworm-slim hologram --json pull "$REF" >/dev/null 2>&1; then
  echo "already published: $REF"
  rm -rf "$WORK"; exit 0
fi

TOTAL=$(python3 -c 'import json,sys; print(sum(f.get("size") or 0 for f in json.load(open(sys.argv[1]))["files"]))' "$WORK/index.json")
USED=$(du -sb "$HUB/store" | cut -f1)
if [ $((USED + TOTAL)) -gt $((BUDGET_BYTES + 1024 * 1024 * 1024)) ]; then
  echo "refused: $TOTAL bytes would exceed the 15 GB hub budget (store is $USED bytes)"; rm -rf "$WORK"; exit 5
fi

# Download at the pinned revision.
python3 - "$WORK/index.json" <<'PY' > "$WORK/urls.txt"
import json, sys
for f in json.load(open(sys.argv[1]))["files"]:
    print(f["path"] + "\t" + f["url"])
PY
while IFS=$'\t' read -r path url; do
  mkdir -p "$WORK/files/$(dirname "$path")"
  curl -fsSL --retry 3 -o "$WORK/files/$path" "$url"
done < "$WORK/urls.txt"

# Verify every file against the index SHA-256, then stage blobs and write the library manifest.
docker run --rm -v "$WORK:/w" -v "$HUB/bin/pack-model.mjs:/app/pack-model.mjs:ro" -w /app node:22-alpine \
  sh -c "npm init -y >/dev/null && npm i --no-fund --no-audit --loglevel=error hash-wasm@4.12.0 >/dev/null && node pack-model.mjs /w/files /w /w/index.json"

# Raw downloads are no longer needed: every byte now lives in the verified chunks.
rm -rf "$WORK/files"

W=/tm/models/$(basename "$WORK")
run() {
  docker run --rm --network twenty_default -e HOME=/tm/home -e HOLOGRAM_CONFIG="$W/live.toml"     -v "$TM:/tm" -v "$HUB/bin/hologram:/usr/local/bin/hologram:ro" -v /etc/ssl/certs:/etc/ssl/certs:ro -w "$W" debian:bookworm-slim "$@"
}
# Compile first (layer paths point at the chunks), then hand the chunks to a per-model CLI store for push.
run hologram --json compile hologram.json --thin --output model.holo | tr -d '
' | cut -c1-300; echo
sed "s#/tm/data#$W/cli#" "$TM/live.toml" > "$WORK/live.toml"
mkdir -p "$WORK/cli/registry/blobs" "$WORK/cli/registry/metadata"
mv "$WORK/store/blobs/blake3" "$WORK/cli/registry/blobs/blake3"
status=0
run hologram --json push model.holo "$REF" >"$WORK/push.out" 2>"$WORK/push.err" || status=$?
tr -d '
' < "$WORK/push.out" | cut -c1-600; echo
if [ "$status" -ne 0 ] || grep -q '"code"' "$WORK/push.out"; then
  grep -v '^{"kappa' "$WORK/push.err" | tail -c 1500
  echo "push failed: $REF (exit $status); work kept at $WORK"
  exit 6
fi

rm -rf "$WORK"
echo "== $(date -u +%FT%TZ) published $REF ($(du -sh "$HUB/store" | cut -f1) store)"

# Every hosted model is also pinned on IPFS (pin-model.sh pulls it back from the public registry, so this proves the
# publish too). A pin failure is logged, never fatal: the registry copy is already live.
[ -f "$HUB/filebase.env" ] && "$HUB/pin-model.sh" "$ID" || echo "pin skipped or failed for $ID"
