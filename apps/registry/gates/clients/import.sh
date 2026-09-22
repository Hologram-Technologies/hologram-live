#!/usr/bin/env bash
# P8 T2, FR-012: a volume the reference itself wrote, imported. The reference
# fills a bind-mounted volume (two images sharing a base layer, one under a
# nested name; a multi-platform index; a referrer), stops, and
# `hologram oci import` copies it in. A second run adds nothing. Hologram
# Registry then serves the copy: every tag has its original digest, every
# platform of the index pulls whole, and the referrer is listed.
# Usage: import.sh <the hologram binary>
set -euo pipefail
source "$(dirname "$0")/lib.sh"
source "$(dirname "$0")/../reference.env"
hologram=$(realpath "$1")
SRC=127.0.0.1:5004
OURS=127.0.0.1:5005
work=$(mktemp -d)
mkdir -p "$work/source" "$work/ctx/one" "$work/ctx/two"
cleanup() {
  docker rm -f import-source > /dev/null 2>&1 || true
  [ -n "${server:-}" ] && kill "$server" 2> /dev/null || true
}
trap cleanup EXIT

wait_up() {
  for _ in $(seq 1 60); do
    curl -fsS "http://$1/v2/" > /dev/null 2>&1 && return 0
    sleep 1
  done
  printf 'FAIL: nothing answers on %s\n' "$1" >&2
  return 1
}
inspect() { skopeo inspect --raw --tls-verify=false "docker://$1" | sha256sum | cut -d' ' -f1 | sed 's/^/sha256:/'; }

# The reference, as its own user so the volume stays readable here.
docker run -d --name import-source --user "$(id -u):$(id -g)" -p "$SRC:5000" \
  -v "$work/source:/var/lib/registry" "${REGISTRY_REF/:3@/@}" > /dev/null
wait_up "$SRC"

printf 'a base layer both images share\n' > "$work/ctx/base.txt"
for name in one two; do
  cp "$work/ctx/base.txt" "$work/ctx/$name/base.txt"
  printf '%s\n' "$name" > "$work/ctx/$name/own.txt"
  printf 'FROM scratch\nCOPY base.txt /base.txt\nCOPY own.txt /own.txt\n' > "$work/ctx/$name/Dockerfile"
done
docker build -q -t "$SRC/imp/one:v1" "$work/ctx/one" > /dev/null
docker build -q -t "$SRC/imp/nested/two:v1" "$work/ctx/two" > /dev/null
docker push -q "$SRC/imp/one:v1" > /dev/null
docker push -q "$SRC/imp/nested/two:v1" > /dev/null
# A multi-platform index, every platform: the reference image itself, from
# the peer registry gate C already runs.
skopeo copy -q --all --preserve-digests --dest-tls-verify=false \
  "docker://${REGISTRY_REF/:3@/@}" "docker://$SRC/imp/registry:3"

# A referrer of imp/one:v1, pushed by hand: an empty config, one layer, and
# a subject.
upload() {
  local repo=$1 file=$2 digest location sep
  digest="sha256:$(sha256sum "$file" | cut -d' ' -f1)"
  location=$(curl -fsS -o /dev/null -D - -X POST "http://$SRC/v2/$repo/blobs/uploads/" |
    tr -d '\r' | awk 'tolower($1) == "location:" { print $2 }')
  case $location in http*) ;; *) location="http://$SRC$location" ;; esac
  sep='?'; [[ $location == *\?* ]] && sep='&'
  curl -fsS -o /dev/null -X PUT -H 'Content-Type: application/octet-stream' \
    --data-binary "@$file" "$location${sep}digest=$digest"
  printf '%s' "$digest"
}
printf '{}' > "$work/empty.json"
printf 'a signature\n' > "$work/sig.txt"
empty=$(upload imp/one "$work/empty.json")
sig=$(upload imp/one "$work/sig.txt")
headers=$(curl -fsS -I -H 'Accept: application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json' \
  "http://$SRC/v2/imp/one/manifests/v1" | tr -d '\r')
subject=$(awk 'tolower($1) == "docker-content-digest:" { print $2 }' <<< "$headers")
subject_type=$(awk 'tolower($1) == "content-type:" { print $2 }' <<< "$headers")
subject_size=$(awk 'tolower($1) == "content-length:" { print $2 }' <<< "$headers")
cat > "$work/referrer.json" <<EOF
{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","artifactType":"application/vnd.gate.signature","config":{"mediaType":"application/vnd.oci.empty.v1+json","digest":"$empty","size":2},"layers":[{"mediaType":"text/plain","digest":"$sig","size":12}],"subject":{"mediaType":"$subject_type","digest":"$subject","size":$subject_size}}
EOF
referrer="sha256:$(sha256sum "$work/referrer.json" | cut -d' ' -f1)"
curl -fsS -o /dev/null -X PUT -H 'Content-Type: application/vnd.oci.image.manifest.v1+json' \
  --data-binary "@$work/referrer.json" "http://$SRC/v2/imp/one/manifests/$referrer"

tags=(imp/one:v1 imp/nested/two:v1 imp/registry:3)
declare -A want
for tag in "${tags[@]}"; do want[$tag]=$(inspect "$SRC/$tag"); done
docker stop -t 10 import-source > /dev/null

# Import, and again.
"$hologram" oci import "$work/source" --into "$work/ours" | tee "$work/first.txt"
grep -q '^3 repositories: ' "$work/first.txt" || fail "the first import's report"
"$hologram" oci import "$work/source" --into "$work/ours" | tee "$work/second.txt"
grep -q ' 0 blobs copied (0 bytes), 0 linked from another repository, 0 manifests, 0 tags$' "$work/second.txt" ||
  fail "a second import added something"

# Serve the copy.
mkdir -p "$work/config"
cat > "$work/config/live.toml" <<EOF
schema_version = 2

[paths]
config_dir = "$work/config"
data_dir = "$work/ours"
state_dir = "$work/state"
cache_dir = "$work/cache"

[server]
listen = "$OURS"

[modules]
enabled = ["dev.hologram.live.system", "dev.hologram.live.oci"]
EOF
"$hologram" --config "$work/config/live.toml" serve > "$work/server.log" 2>&1 &
server=$!
wait_up "$OURS" || { cat "$work/server.log"; exit 1; }

for tag in "${tags[@]}"; do
  same "$(inspect "$OURS/$tag")" "${want[$tag]}" "$tag after import"
  # Every platform, every blob, hashed by skopeo on the way.
  rm -rf "$work/pulled"
  skopeo copy -q --all --src-tls-verify=false "docker://$OURS/$tag" "oci:$work/pulled:x" ||
    fail "pull $tag whole from the import"
done
curl -fsS "http://$OURS/v2/imp/one/referrers/$subject" | grep -q "\"$referrer\"" ||
  fail "the referrer of imp/one:v1 is not listed"
printf 'import: a volume the reference wrote, served with every digest unchanged: ok\n'
