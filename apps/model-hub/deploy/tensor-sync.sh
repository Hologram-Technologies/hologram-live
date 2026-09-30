#!/bin/sh
# Publish one tensor-index build to the live hub. Run on the laptop by Ilya (the VPS write path needs his key);
# every step is idempotent and the previous state is kept for rollback.
#
#   ./tensor-sync.sh <tensor state dir>      publish (the directory pipeline.mjs writes: TENSOR_STATE)
#   ./tensor-sync.sh --rollback              previous tensor state back
#
# 1) refuse unless the gate passes on the local state, 2) rsync it to /root/hub/state/resolve/tensors.next,
# 3) swap it into place atomically, 4) prove one model: its manifest, and one weight file rebuilt from tensors
# by the live host, hashed here against the manifest digest. hub-resolve re-reads models.json within 30 s.
set -eu
HOST=${HUB_HOST:-root@217.216.66.62}
HUB=${HUB:-https://gethologram.ai}
STATE=/root/hub/state/resolve
if [ "${1:-}" = "--rollback" ]; then
  ssh "$HOST" "set -e; cd $STATE; [ -d tensors.prev ] || { echo 'nothing to roll back to'; exit 1; }; mv tensors tensors.rolledback.\$(date +%s); mv tensors.prev tensors; echo rolled back"
  exit 0
fi
SRC=${1:?tensor state dir}
HERE=$(cd "$(dirname "$0")" && pwd)
node "$HERE/../web/qa/tensor-index.mjs" "$SRC"
rsync -a --delete --exclude lock --exclude queue.json --exclude car/ --exclude failures.json "$SRC/" "$HOST:$STATE/tensors.next/"
ssh "$HOST" "set -e; cd $STATE; rm -rf tensors.prev; [ -d tensors ] && mv tensors tensors.prev; mv tensors.next tensors; echo swapped"
sleep 31
REPO=$(node -e "const m=JSON.parse(require('fs').readFileSync('$SRC/models.json','utf8')); const r=Object.entries(m).filter(([,v])=>v.gated).sort((a,b)=>a[1].weightBytes-b[1].weightBytes)[0][0]; console.log(r.toLowerCase())")
echo "proof: $REPO"
node "$HERE/../tensors/pull.mjs" "$REPO" --hub "$HUB" --format safetensors --verify-only
