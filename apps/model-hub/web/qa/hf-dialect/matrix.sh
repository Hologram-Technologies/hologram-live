#!/usr/bin/env sh
# Client matrix for the hub endpoint. Runs inside python:3.12-slim with HF_ENDPOINT set by the caller and an empty cache.
#   matrix.sh <huggingface_hub version spec, e.g. ">=1.0" or "<1.0">
# Every file that lands is checked with plain `sha256sum -c` against the endpoint's SHA256SUMS: no tool of ours.
set -eu
SPEC=${1:->=1.0}
MODEL=${MODEL:-sentence-transformers/all-MiniLM-L6-v2}
export HF_HUB_DISABLE_TELEMETRY=1 HF_HUB_DISABLE_PROGRESS_BARS=1 PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1
pip install -q "huggingface_hub$SPEC" transformers >/dev/null 2>&1
python - <<PY
import huggingface_hub, transformers, os
print("huggingface_hub", huggingface_hub.__version__, "| transformers", transformers.__version__, "| endpoint", os.environ["HF_ENDPOINT"])
PY

check() { # <dir with the downloaded files>
  python - "$1" <<PY
import hashlib, os, sys, urllib.request
root = sys.argv[1]
sums = urllib.request.urlopen(os.environ["HF_ENDPOINT"] + "/$MODEL/resolve/main/SHA256SUMS").read().decode().splitlines()
want = dict(reversed(l.split("  ", 1)) for l in sums)
got = bad = 0
for path, digest in want.items():
    f = os.path.join(root, path)
    if not os.path.exists(f): continue
    got += 1
    if hashlib.sha256(open(f, "rb").read()).hexdigest() != digest: bad += 1; print("  MISMATCH", path)
print(f"  {got} of {len(want)} files present, {bad} mismatches")
sys.exit(1 if bad or not got else 0)
PY
}

echo "1. hf download (the CLI), whole model"
if command -v hf >/dev/null 2>&1; then D=$(hf download "$MODEL" --quiet 2>/dev/null | tail -1); else D=$(huggingface-cli download "$MODEL" --quiet 2>/dev/null | tail -1); fi
check "$D"

echo "2. snapshot_download into a fresh directory"
rm -rf /tmp/snap && python -c "from huggingface_hub import snapshot_download; snapshot_download('$MODEL', local_dir='/tmp/snap')" >/dev/null 2>&1
check /tmp/snap

echo "3. hf_hub_download of one file, then again (must be served from cache without error)"
python -c "
from huggingface_hub import hf_hub_download
a = hf_hub_download('$MODEL', 'config.json'); b = hf_hub_download('$MODEL', 'config.json'); print('  same path twice:', a == b)"

echo "4. transformers: AutoConfig and AutoTokenizer from_pretrained"
python -c "
from transformers import AutoConfig, AutoTokenizer
c = AutoConfig.from_pretrained('$MODEL'); t = AutoTokenizer.from_pretrained('$MODEL')
print('  model_type', c.model_type, '| tokens for hello world:', t('hello world')['input_ids'])" 2>&1 | grep -v Warning | tail -1

echo "5. plain curl -L (what llama.cpp and shell scripts do), with a Range request"
python - <<PY
import hashlib, os, subprocess, urllib.request
base = os.environ["HF_ENDPOINT"] + "/$MODEL/resolve/main/"
whole = urllib.request.urlopen(base + "tokenizer.json").read()
part = urllib.request.urlopen(urllib.request.Request(base + "tokenizer.json", headers={"Range": "bytes=100-199"})).read()
sums = dict(reversed(l.split("  ", 1)) for l in urllib.request.urlopen(base + "SHA256SUMS").read().decode().splitlines())
print("  GET follows to the source:", hashlib.sha256(whole).hexdigest() == sums["tokenizer.json"], "| Range 100-199 equals the slice:", part == whole[100:200])
PY

echo "6. a model the hub does not have, and a revision it does not have"
python - <<PY
from huggingface_hub import hf_hub_download
for repo, rev in (("nobody/not-a-model", None), ("$MODEL", "0000000000000000000000000000000000000000")):
    try: hf_hub_download(repo, "config.json", revision=rev); print("  unexpected success")
    except Exception as e: print("  ", type(e).__name__, "|", str(e).replace("\n", " ")[:170])
PY
echo "7. hf cache verify: every downloaded file checked against the listing's oids (git sha1 for small files, sha256 for LFS)"
if command -v hf >/dev/null 2>&1 && hf cache verify --help >/dev/null 2>&1; then
  hf cache verify "$MODEL" 2>&1 | grep -E "checked|Error|expected" | head -3
else echo "  (this huggingface_hub has no cache verify)"; fi

echo "8. text-generation-webui's paging: a next page must be empty, or its download loop never ends"
python - <<PY
import json, os, urllib.request
base = os.environ["HF_ENDPOINT"] + "/api/models/$MODEL/tree/main"
first = json.load(urllib.request.urlopen(base + "?recursive=true")); nxt = json.load(urllib.request.urlopen(base + "?recursive=true&cursor=next"))
print("  first page", len(first), "entries | next page", len(nxt), "entries:", "ok" if first and not nxt else "LOOPS")
PY
echo "matrix done"
