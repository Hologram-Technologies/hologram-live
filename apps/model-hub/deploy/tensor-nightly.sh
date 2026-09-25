#!/bin/sh
# The tensor index, kept current with no human step. Runs on the hub host from a checkout of this repo.
#
#   ./tensor-nightly.sh                 one night: discover -> index (budgeted) -> gate -> seal -> car -> promote
#   install (Ilya approves; live config):
#     echo '17 3 * * * root /root/hub/src/apps/model-hub/deploy/tensor-nightly.sh >> /root/hub/state/tensor-nightly.log 2>&1' > /etc/cron.d/hologram-tensors
#
# Work happens in tensors.work (the pipeline's own lock stops a second run). Promotion copies the work state to
# tensors.next, runs the site gate on it, and swaps it in atomically, keeping the previous state for rollback
# (tensor-sync.sh --rollback). A night that fails at any step leaves the live state untouched.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
LIVE=${TENSOR_LIVE:-/root/hub/state/resolve}
WORK=${TENSOR_WORK:-/root/hub/state/tensors.work}
BUDGET=${TENSOR_BUDGET_GB:-150}
mkdir -p "$WORK"
export TENSOR_STATE="$WORK"
cd "$HERE/../tensors"
node pipeline.mjs nightly --budget-gb "$BUDGET"
rm -rf "$LIVE/tensors.next"
rsync -a --exclude lock --exclude queue.json --exclude car/ --exclude failures.json "$WORK/" "$LIVE/tensors.next/"
node "$HERE/../web/qa/tensor-index.mjs" "$LIVE/tensors.next"
cd "$LIVE"; rm -rf tensors.prev; [ -d tensors ] && mv tensors tensors.prev; mv tensors.next tensors
TENSOR_STATE="$WORK" node "$HERE/../tensors/pipeline.mjs" status
