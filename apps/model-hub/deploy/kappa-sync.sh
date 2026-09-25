#!/bin/sh
# Publish one κ mirror build to the live hub. Run on the laptop by Ilya (the VPS write paths need his credential);
# every step is idempotent and the previous mirror state is kept for rollback.
#
#   REGISTRY_AUTH="publisher:<password>" ./kappa-sync.sh <registry-kappa dir>        publish
#   ./kappa-sync.sh --rollback                                                          previous mirror state back
#
# What it does: 1) rsync out/kappa/ → /root/hub/state/resolve/kappa.next, 2) push held artifacts + the sealed
# index into the live Hologram Registry (publish.mjs, plain OCI calls), 3) swap kappa.next into place atomically,
# 4) prove one cold pull from the live host. Nothing is deleted: old digests stay pullable for ever.
set -eu
HOST=${HUB_HOST:-root@217.216.66.62}
STATE=/root/hub/state/resolve
if [ "${1:-}" = "--rollback" ]; then
  ssh "$HOST" "set -e; cd $STATE; [ -d kappa.prev ] || { echo 'nothing to roll back to'; exit 1; }; mv kappa kappa.rolledback.\$(date +%s); mv kappa.prev kappa; echo rolled back"
  exit 0
fi
SRC=${1:?registry-kappa dir}
: "${REGISTRY_AUTH:?publisher:<password> for hub.uor.foundation}"
test -f "$SRC/out/kappa/mirror.json"
rsync -a --delete "$SRC/out/kappa/" "$HOST:$STATE/kappa.next/"
( cd "$SRC" && REGISTRY=https://hub.uor.foundation REGISTRY_AUTH="$REGISTRY_AUTH" node publish.mjs )
ssh "$HOST" "set -e; cd $STATE; rm -rf kappa.prev; [ -d kappa ] && mv kappa kappa.prev; mv kappa.next kappa; echo swapped"
# hub-resolve re-reads mirror.json on its next request (30 s cache); no restart.
NAME=$(node -e "const m=JSON.parse(require('fs').readFileSync('$SRC/out/kappa/mirror.json','utf8')).repos; const n=Object.keys(m).find(k=>k.startsWith('docker.io/library/')); console.log(n+'@'+Object.values(m[n].tags)[0])")
echo "cold pull: $NAME"
crane pull "hub.uor.foundation/$NAME" /tmp/kappa-proof.tar && crane validate --remote "hub.uor.foundation/$NAME" && echo "live mirror verified"
