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
  rm -rf -- "$demo_dir"
}
trap cleanup EXIT

docker version --format '{{.Server.Version}}' >/dev/null
cargo build --release --locked --package hologram-live --bin hologram --manifest-path "$repo_root/Cargo.toml"

target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
binary="$target_dir/release/hologram"
manifest="$repo_root/examples/python-numpy-pandas/hologram.json"
request="$repo_root/examples/python-numpy-pandas/request.json"
isolated_env=(
  env
  "HOLOGRAM_CONFIG_DIR=$demo_dir/config"
  "HOLOGRAM_DATA_DIR=$demo_dir/data"
  "HOLOGRAM_STATE_DIR=$demo_dir/state"
  "HOLOGRAM_CACHE_DIR=$demo_dir/cache"
)

"${isolated_env[@]}" "$binary" --json compile "$manifest" --check >/dev/null
compile_json=$("${isolated_env[@]}" "$binary" --json compile "$manifest" --output "$archive")
run_json=$("${isolated_env[@]}" "$binary" --json run "$archive" --input "$request")
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
if source["builder"].get("archive_format") != "normalized-docker-archive-v1" or source["builder"].get("source_date_epoch") != 0:
    print(f"rootfs provenance is missing normalization evidence: {source!r}", file=sys.stderr)
    raise SystemExit(1)
base_image = source["base_image"]
resolved_base = base_image.get("resolved_reference", "")
if base_image.get("reference") != "python:3.12-slim" or not resolved_base.startswith("python@sha256:"):
    print(f"rootfs provenance did not bind the requested base to a digest: {base_image!r}", file=sys.stderr)
    raise SystemExit(1)
if len(resolved_base.removeprefix("python@sha256:")) != 64:
    print(f"rootfs provenance contains an invalid resolved base: {base_image!r}", file=sys.stderr)
    raise SystemExit(1)
if source["reproducibility"] != {"reproducible": True}:
    print(f"completed rootfs provenance does not claim the proven build contract: {source!r}", file=sys.stderr)
    raise SystemExit(1)
build_output = source.get("output", {})
if not build_output.get("layer_kappa", "").startswith("blake3:"):
    print(f"rootfs provenance is missing layer identity: {source!r}", file=sys.stderr)
    raise SystemExit(1)
if build_output.get("bundle_schema_version") != 3 or build_output.get("provider") != "normalized-docker-archive-zstd-v1":
    print(f"rootfs provenance has an unexpected bundle contract: {source!r}", file=sys.stderr)
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
        "base_reference": base_image["reference"],
        "resolved_base_reference": resolved_base,
        "image_id": build_output["image_id"],
        "layer_kappa": build_output["layer_kappa"],
        "reproducible": source["reproducibility"]["reproducible"],
    },
}
json.dump(summary, sys.stdout, separators=(",", ":"))
sys.stdout.write("\n")
' "$archive_persisted" "$archive" "$compile_json" <<<"$run_json"
