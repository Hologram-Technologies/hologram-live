#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output_path=""
case "${1:-}" in
  "") ;;
  --output)
    if [[ $# -ne 2 || -z "${2:-}" ]]; then
      printf 'usage: %s [--output PATH]\n' "$0" >&2
      exit 2
    fi
    output_path=$2
    ;;
  --help|-h)
    printf 'usage: %s [--output PATH]\n' "$0"
    exit 0
    ;;
  *)
    printf 'usage: %s [--output PATH]\n' "$0" >&2
    exit 2
    ;;
esac

demo_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-python-demo.XXXXXX")
archive_persisted=false
if [[ -n "$output_path" ]]; then
  archive=$output_path
  archive_persisted=true
  mkdir -p "$(dirname -- "$archive")"
else
  archive="$demo_dir/numpy-pandas.holo"
fi
cleanup() {
  if [[ "$archive_persisted" == false ]]; then
    rm -f "$archive"
  fi
  rmdir "$demo_dir"
}
trap cleanup EXIT

docker version --format '{{.Server.Version}}' >/dev/null
cargo build --release --locked --package hologram-live --bin hologram --manifest-path "$repo_root/Cargo.toml"

binary="$repo_root/target/release/hologram"
manifest="$repo_root/examples/python-numpy-pandas/hologram.json"
request="$repo_root/examples/python-numpy-pandas/request.json"

"$binary" --json compile "$manifest" --check >/dev/null
compile_json=$("$binary" --json compile "$manifest" --output "$archive")
run_json=$("$binary" --json run "$archive" --input "$request")
python3 -c '
import json
import sys

run = json.load(sys.stdin)
compile_report = json.loads(sys.argv[3])
provenance = compile_report["build_provenance"]
source = provenance["layers"][0]["source"]
if provenance["canonical"] or source["profile"] != "rootfs":
    print(f"unexpected rootfs provenance: {provenance!r}", file=sys.stderr)
    raise SystemExit(1)
if not source["builder"].get("client_version") or not source["builder"].get("server_version"):
    print(f"rootfs provenance is missing Docker versions: {source!r}", file=sys.stderr)
    raise SystemExit(1)
build_output = source.get("output", {})
if not build_output.get("layer_kappa", "").startswith("blake3:"):
    print(f"rootfs provenance is missing layer identity: {source!r}", file=sys.stderr)
    raise SystemExit(1)
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
    "archive": sys.argv[2] if sys.argv[1] == "true" else None,
    "archive_persisted": sys.argv[1] == "true",
    "build": {
        "target_platform": source["target_platform"],
        "image_id": build_output["image_id"],
        "layer_kappa": build_output["layer_kappa"],
        "reproducible": source["reproducibility"]["reproducible"],
    },
}
json.dump(summary, sys.stdout, separators=(",", ":"))
sys.stdout.write("\n")
' "$archive_persisted" "$archive" "$compile_json" <<<"$run_json"
