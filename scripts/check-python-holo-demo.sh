#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
demo_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-python-demo.XXXXXX")
archive="$demo_dir/numpy-pandas.holo"
cleanup() {
  rm -f "$archive"
  rmdir "$demo_dir"
}
trap cleanup EXIT

docker version --format '{{.Server.Version}}' >/dev/null
cargo build --locked --bin hologram --manifest-path "$repo_root/Cargo.toml"

binary="$repo_root/target/debug/hologram"
manifest="$repo_root/examples/python-numpy-pandas/hologram.json"
request="$repo_root/examples/python-numpy-pandas/request.json"

"$binary" --json compile "$manifest" --check >/dev/null
"$binary" --json compile "$manifest" --output "$archive" >/dev/null
run_json=$("$binary" --json run "$archive" --input "$request")
actual=$(python3 -c 'import json,sys; print(bytes(json.load(sys.stdin)["outputs"][0]).decode())' <<<"$run_json")
expected='{"columns":["label","value"],"mean":20.0,"rows":3,"sum":60.0}'

if [[ "$actual" != "$expected" ]]; then
  printf 'unexpected Python .holo output\nexpected: %s\nactual:   %s\n' "$expected" "$actual" >&2
  exit 1
fi

printf 'Python .holo demo passed: %s\n' "$actual"
