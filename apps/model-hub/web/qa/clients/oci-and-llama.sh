#!/usr/bin/env bash
# Client proofs that need no account: OCI tools and llama.cpp against the live hub. Each runs in a capped container,
# one after another. Output is a transcript: the command, what the client printed, and a check of the bytes.
#   oci-and-llama.sh [hub]      (default https://gethologram.ai)
set -uo pipefail
HUB=${1:-https://gethologram.ai}; HOST=${HUB#https://}
OUT=$(mktemp -d); trap 'rm -rf "$OUT"' EXIT; chmod 777 "$OUT"
CAP="--rm --memory 3g --cpus 2"
MODEL=hexgrad/kokoro-82m; SUMS="$HUB/hexgrad/Kokoro-82M/resolve/main/SHA256SUMS"
say() { printf '\n===== %s\n' "$*"; }
verify() { ( cd "$1" && n=$(find . -type f | wc -l) && curl -s "$SUMS" | sha256sum -c --quiet 2>/dev/null; echo "  files: $n, sha256sum -c exit: $?" ); }

say "crane (go-containerregistry): manifest, then one blob through the redirect, digest checked by hand"
docker run $CAP gcr.io/go-containerregistry/crane:latest manifest "$HOST/$MODEL:latest" > "$OUT/m.json" 2> "$OUT/crane.err"; tail -2 "$OUT/crane.err"
python3 - "$OUT/m.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1])); print("  artifactType:", m.get("artifactType"), "| layers:", len(m["layers"]))
l = next(x for x in m["layers"] if x["annotations"]["org.cncf.model.filepath"] == "config.json"); open(sys.argv[1] + ".digest", "w").write(l["digest"])
PY
D=$(cat "$OUT/m.json.digest"); got=$(docker run $CAP gcr.io/go-containerregistry/crane:latest blob "$HOST/$MODEL@$D" 2>/dev/null | sha256sum | cut -d' ' -f1)
echo "  blob $D -> sha256 of what arrived: $got -> $([ "sha256:$got" = "$D" ] && echo MATCH || echo MISMATCH)"

say "skopeo: copy the whole artifact to a directory"
mkdir -p "$OUT/skopeo"; chmod 777 "$OUT/skopeo"
timeout 600 docker run $CAP -v "$OUT/skopeo:/out" quay.io/skopeo/stable:latest copy "docker://$HOST/$MODEL:latest" dir:/out 2>&1 | tail -3 | cut -c1-200
echo "  blobs on disk: $(ls "$OUT/skopeo" 2>/dev/null | wc -l)"
python3 - "$OUT/skopeo" <<'PY'
import hashlib, os, sys
d = sys.argv[1]; bad = n = 0
for f in os.listdir(d):
    if len(f) == 64:
        n += 1; bad += hashlib.sha256(open(os.path.join(d, f), "rb").read()).hexdigest() != f
print(f"  {n} blobs named by digest, {bad} whose bytes do not hash to their name")
PY

say "modctl (CNCF ModelPack reference tool): pull, then extract"
mkdir -p "$OUT/modctl"; chmod 777 "$OUT/modctl"
timeout 900 docker run $CAP -v "$OUT/modctl:/out" debian:bookworm-slim sh -c '
  apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq curl ca-certificates jq >/dev/null 2>&1
  url=$(curl -s https://api.github.com/repos/modelpack/modctl/releases/latest | jq -r ".assets[] | select(.name | test(\"linux.*(amd64|x86_64).*tar.gz$\"; \"i\")) | .browser_download_url" | head -1)
  echo "  release: $url"; curl -fsSL "$url" | tar xz -C /usr/local/bin 2>/dev/null || { mkdir /t && curl -fsSL "$url" | tar xz -C /t && cp $(find /t -type f -name modctl | head -1) /usr/local/bin/; }
  modctl version 2>&1 | head -2
  modctl pull '"$HOST/$MODEL"':latest 2>&1 | tail -3
  modctl extract '"$HOST/$MODEL"':latest --output /out 2>&1 | tail -2' | cut -c1-220
verify "$OUT/modctl"

say "llama.cpp -hf through MODEL_ENDPOINT (download, then eight tokens on the CPU)"
timeout 900 docker run $CAP -e MODEL_ENDPOINT="$HUB/" -e LLAMA_CACHE=/tmp/cache ghcr.io/ggml-org/llama.cpp:light \
  -hf bartowski/MiniCPM5-2B-GGUF:IQ2_M -p "Reply with one word: hello" -n 8 -no-cnv 2>&1 | grep -iE "curl|http|download|error|fail|hello|load_tensors: loading|sampler seed|eval time" | tail -12 | cut -c1-220

say "what the hub logged for these clients"
docker logs --since 40m hub-resolve-1 2>&1 | grep -c '"dialect":"oci"' | sed 's/^/  oci blob redirects: /'
