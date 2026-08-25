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
cargo build --release --locked --bin hologram --manifest-path "$repo_root/Cargo.toml"

binary="$repo_root/target/release/hologram"
manifest="$repo_root/examples/python-numpy-pandas/hologram.json"
request="$repo_root/examples/python-numpy-pandas/request.json"

"$binary" --json compile "$manifest" --check >/dev/null
"$binary" --json compile "$manifest" --output "$archive" >/dev/null
run_json=$("$binary" --json run "$archive" --input "$request")
python3 -c '
import json
import sys

run = json.load(sys.stdin)
output = json.loads(bytes(run["outputs"][0]).decode())
expected = {
    "columns": ["label", "value"],
    "mean": 20.0,
    "rows": 3,
    "sum": 60.0,
}
if output != expected:
    print(
        f"unexpected Python .holo output: expected {expected!r}, got {output!r}",
        file=sys.stderr,
    )
    raise SystemExit(1)

summary = {
    "schema_version": 1,
    "status": "ok",
    "application": "numpy-pandas-holo",
    "kappa": run["kappa"],
    "output": output,
    "elapsed_micros": run["elapsed_micros"],
    "archive_bytes": run["resident_bytes"],
}
json.dump(summary, sys.stdout, separators=(",", ":"))
sys.stdout.write("\n")
' <<<"$run_json"
