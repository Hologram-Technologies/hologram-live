#!/usr/bin/env bash
# Publish the sealed Spaces to the Hologram Registry as OCI artifacts — one repository per Space,
# `spaces/<id>`, tag `latest`, config = holospace.json, one layer per file, the sealed root in an annotation.
# Reads need no token (the Spaces page probes /v2/spaces/<id>/manifests/latest anonymously); writes carry the
# registry token Caddy checks. Idempotent: a blob already present is skipped by digest.
#
#   HUB_REGISTRY_TOKEN=… scripts/spaces.publish.sh [https://gethologram.ai] [space-id …]
set -euo pipefail
HUB="${1:-https://gethologram.ai}"; shift || true
: "${HUB_REGISTRY_TOKEN:?set HUB_REGISTRY_TOKEN (the registry write token)}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
SPACES="$HERE/public/spaces"
IDS=("$@"); [ ${#IDS[@]} -gt 0 ] || IDS=($(python -c "import json,io,sys; d=json.load(io.open('$SPACES/spaces.json',encoding='utf-8')); print(' '.join(s['id'] for s in d['spaces']))"))
AUTH=(-H "Authorization: Bearer $HUB_REGISTRY_TOKEN")

put_blob() { # repo file digest size
  local repo="$1" file="$2" digest="$3"
  if curl -sfI "${AUTH[@]}" "$HUB/v2/$repo/blobs/$digest" >/dev/null 2>&1; then echo "  = $digest ($(basename "$file"))"; return; fi
  local loc; loc=$(curl -sfi "${AUTH[@]}" -X POST "$HUB/v2/$repo/blobs/uploads/" | tr -d '\r' | awk 'tolower($1)=="location:"{print $2}')
  [ -n "$loc" ] || { echo "upload start failed for $file" >&2; exit 1; }
  case "$loc" in http*) ;; *) loc="$HUB$loc";; esac
  local sep="?"; case "$loc" in *\?*) sep="&";; esac
  curl -sf "${AUTH[@]}" -X PUT -H "Content-Type: application/octet-stream" --data-binary "@$file" "${loc}${sep}digest=$digest" >/dev/null
  echo "  + $digest ($(basename "$file"))"
}

for id in "${IDS[@]}"; do
  dir="$SPACES/$id"; repo="spaces/$id"
  [ -f "$dir/holospace.lock.json" ] || { echo "no lock for $id" >&2; exit 1; }
  echo "$repo"
  manifest=$(python - "$dir" "$repo" <<'EOF'
import json,io,sys,os,hashlib
d,repo=sys.argv[1],sys.argv[2]
lock=json.load(io.open(os.path.join(d,'holospace.lock.json'),encoding='utf-8'))
man=json.load(io.open(os.path.join(d,'holospace.json'),encoding='utf-8'))
cfg=io.open(os.path.join(d,'holospace.json'),'rb').read()
layers=[]
for path,f in sorted(lock['files'].items()):
    if path=='holospace.json': continue
    mt='text/html' if path.endswith('.html') else 'text/javascript' if path.endswith(('.js','.mjs')) else 'text/css' if path.endswith('.css') else 'image/svg+xml' if path.endswith('.svg') else 'application/json' if path.endswith('.json') else 'application/octet-stream'
    layers.append({"mediaType":mt,"digest":"sha256:"+f['sha256'],"size":f['bytes'],"annotations":{"org.opencontainers.image.title":path}})
m={"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","artifactType":"application/vnd.hologram.space.v1+json",
   "config":{"mediaType":"application/vnd.hologram.space.config.v1+json","digest":"sha256:"+hashlib.sha256(cfg).hexdigest(),"size":len(cfg)},
   "layers":layers,
   "annotations":{"org.opencontainers.image.title":man['name'],"org.opencontainers.image.description":man.get('tagline',''),
                  "org.opencontainers.image.source":man['space']['source'],"foundation.uor.space.root":lock['root'],
                  "foundation.uor.space.models":",".join(man['space']['models']),"foundation.uor.space.entry":man['entry']}}
print(json.dumps(m,separators=(',',':')))
EOF
)
  # blobs: the config, then every file the lock names
  cfg_digest="sha256:$(python -c "import hashlib,io; print(hashlib.sha256(io.open('$dir/holospace.json','rb').read()).hexdigest())")"
  put_blob "$repo" "$dir/holospace.json" "$cfg_digest"
  while IFS=$'\t' read -r path digest; do put_blob "$repo" "$dir/$path" "sha256:$digest"; done < <(python -c "
import json,io; l=json.load(io.open('$dir/holospace.lock.json',encoding='utf-8'))
for p,f in sorted(l['files'].items()):
    if p!='holospace.json': print(p+'\t'+f['sha256'])")
  root_tag=$(python -c "import json,io; print(json.load(io.open('$dir/holospace.lock.json',encoding='utf-8'))['root'].split(':')[1][:16])")
  for tag in latest "$root_tag"; do
    curl -sf "${AUTH[@]}" -X PUT -H "Content-Type: application/vnd.oci.image.manifest.v1+json" --data-binary "$manifest" "$HUB/v2/$repo/manifests/$tag" >/dev/null
    echo "  → $repo:$tag"
  done
done
echo "verify: curl -sI $HUB/v2/spaces/${IDS[0]}/manifests/latest | grep -i docker-content-digest"
