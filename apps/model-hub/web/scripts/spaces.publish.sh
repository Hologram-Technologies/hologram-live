#!/usr/bin/env bash
# Publish the sealed Spaces to a Hologram Registry as OCI artifacts — one repository per Space, `spaces/<id>`,
# tags `latest` and the first 16 hex of κ. The manifest bytes are the ones spaces.build.mjs wrote
# (<id>/manifest.json): they are pushed verbatim, so the registry's Docker-Content-Digest is the lock's root, κ.
# Layers are every file the manifest names, including the shared runtime (public/spaces/runtime/…), so the
# artifact is the whole Space. Reads need no token; writes carry HUB_REGISTRY_TOKEN. Idempotent: a blob already
# present is skipped by digest.
#
#   HUB_REGISTRY_TOKEN=… scripts/spaces.publish.sh [https://hub.uor.foundation] [space-id …]
#   scripts/spaces.publish.sh http://127.0.0.1:5055        # the rehearsal registry needs no token
set -euo pipefail
HUB="${1:-https://hub.uor.foundation}"; shift || true
HERE="$(cd "$(dirname "$0")/.." && pwd)"
SPACES="$HERE/public/spaces"
# Git Bash hands python a /c/… path it cannot open; give it the Windows spelling when cygpath exists.
PY_SPACES="$SPACES"; command -v cygpath >/dev/null 2>&1 && PY_SPACES="$(cygpath -m "$SPACES")"
py() { python "$@" | tr -d '\r'; }   # python on Windows writes CRLF; a digest with a CR is not a digest
IDS=("$@"); [ ${#IDS[@]} -gt 0 ] || IDS=($(py -c "import json,io; d=json.load(io.open('$PY_SPACES/spaces.json',encoding='utf-8')); print(' '.join(s['id'] for s in d['spaces']))"))
AUTH=(); [ -n "${HUB_REGISTRY_TOKEN:-}" ] && AUTH=(-H "Authorization: Bearer $HUB_REGISTRY_TOKEN")

put_blob() { # repo file digest
  local repo="$1" file="$2" digest="$3"
  if curl -sf -o /dev/null "${AUTH[@]}" -I "$HUB/v2/$repo/blobs/$digest" 2>/dev/null; then echo "  = ${digest:7:12} $(basename "$file")"; return; fi
  local loc; loc=$(curl -sfi "${AUTH[@]}" -X POST "$HUB/v2/$repo/blobs/uploads/" | tr -d '\r' | awk 'tolower($1)=="location:"{print $2}')
  [ -n "$loc" ] || { echo "upload start failed for $file" >&2; exit 1; }
  case "$loc" in http*) ;; *) loc="$HUB$loc";; esac
  local sep="?"; case "$loc" in *\?*) sep="&";; esac
  curl -sf -o /dev/null "${AUTH[@]}" -X PUT -H "Content-Type: application/octet-stream" --data-binary "@$file" "${loc}${sep}digest=$digest"
  echo "  + ${digest:7:12} $(basename "$file") ($(wc -c < "$file" | tr -d ' ') B)"
}

t0=$(date +%s)
for id in "${IDS[@]}"; do
  dir="$SPACES/$id"; repo="spaces/$id"; PY_DIR="$PY_SPACES/$id"
  [ -f "$dir/holospace.lock.json" ] && [ -f "$dir/manifest.json" ] || { echo "no lock/manifest for $id — run scripts/spaces.build.mjs" >&2; exit 1; }
  root=$(py -c "import json,io; print(json.load(io.open('$PY_DIR/holospace.lock.json',encoding='utf-8'))['root'])")
  local_digest="sha256:$(py -c "import hashlib,io; print(hashlib.sha256(io.open('$PY_DIR/manifest.json','rb').read()).hexdigest())")"
  [ "$root" = "$local_digest" ] || { echo "$id: lock root $root != sha256(manifest.json) $local_digest — rebuild" >&2; exit 1; }
  echo "$repo  κ $root"
  # blobs: the config, then every layer (Space files + the shared runtime), in the manifest's order
  list=$(mktemp)
  py -c "
import json,io
d,spaces='$PY_DIR','$PY_SPACES'
m=json.load(io.open(d+'/manifest.json',encoding='utf-8'))
print(d+'/holospace.json\t'+m['config']['digest'])
for l in m['layers']:
    t=l['annotations']['org.opencontainers.image.title']
    print(((spaces+'/'+t) if t.startswith('runtime/') else (d+'/'+t))+'\t'+l['digest'])
" > "$list"
  while IFS=$'\t' read -r file digest; do [ -n "$file" ] && put_blob "$repo" "$file" "$digest"; done < "$list"
  rm -f "$list"
  short="${root:7:16}"
  for tag in latest "$short"; do
    got=$(curl -sfi "${AUTH[@]}" -X PUT -H "Content-Type: application/vnd.oci.image.manifest.v1+json" --data-binary "@$dir/manifest.json" "$HUB/v2/$repo/manifests/$tag" | tr -d '\r' | awk 'tolower($1)=="docker-content-digest:"{print $2}')
    [ "$got" = "$root" ] || { echo "$id: registry digest '$got' != κ $root" >&2; exit 1; }
    echo "  → $repo:$tag  $got"
  done
done
echo "published ${#IDS[@]} space(s) to $HUB in $(( $(date +%s) - t0 )) s"
echo "verify: curl -sI $HUB/v2/spaces/${IDS[0]}/manifests/latest -H 'Accept: application/vnd.oci.image.manifest.v1+json' | grep -i docker-content-digest"
