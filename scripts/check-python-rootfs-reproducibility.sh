#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_count=2
report_path=""

usage() {
  printf 'usage: %s [--build-count 1|2] [--report PATH]\n' "$0"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-count)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      build_count=$2
      shift 2
      ;;
    --report)
      [[ $# -ge 2 && -n "$2" ]] || { usage >&2; exit 2; }
      report_path=$2
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$build_count" != 1 && "$build_count" != 2 ]]; then
  usage >&2
  exit 2
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-rootfs-repro.XXXXXX")
temporary_files=()
cleanup() {
  if [[ ${#temporary_files[@]} -gt 0 ]]; then
    rm -f -- "${temporary_files[@]}"
  fi
  rmdir "$work_dir"
}
trap cleanup EXIT

docker version --format '{{.Server.Version}}' >/dev/null
cargo build --release --locked --package hologram-live --bin hologram --manifest-path "$repo_root/Cargo.toml"

target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
binary="$target_dir/release/hologram"
manifest="$repo_root/examples/python-numpy-pandas/hologram.json"
reports=()
build=1
while [[ $build -le $build_count ]]; do
  archive="$work_dir/build-$build.holo"
  report="$work_dir/build-$build.json"
  temporary_files+=("$archive" "$report")
  "$binary" --json compile "$manifest" \
    --no-build-cache \
    --output "$archive" >"$report"
  reports+=("$report")
  build=$((build + 1))
done

if [[ -n "$report_path" ]]; then
  mkdir -p "$(dirname -- "$report_path")"
fi

python3 - "$report_path" "$manifest" "$build_count" "${reports[@]}" <<'PY'
import json
import pathlib
import sys

destination = sys.argv[1]
manifest = sys.argv[2]
build_count = int(sys.argv[3])
report_paths = sys.argv[4:]
reports = [json.loads(pathlib.Path(path).read_text()) for path in report_paths]

def identity(report):
    source = report["build_provenance"]["layers"][0]["source"]
    output = source["output"]
    if source["builder"].get("cache_disabled") is not True:
        raise ValueError("compile report does not prove that the builder cache was disabled")
    if source.get("reproducibility") != {"reproducible": True}:
        raise ValueError("compile report does not claim the proven rootfs build contract")
    return {
        "image_id": output["image_id"],
        "rootfs_layer_kappa": output["layer_kappa"],
        "rootfs_byte_length": output["byte_length"],
        "application_kappa": report["application_kappa"],
        "archive_kappa": report["archive_kappa"],
        "archive_fingerprint": report["archive_fingerprint"],
        "archive_byte_length": report["byte_length"],
    }

identities = [identity(report) for report in reports]
first_source = reports[0]["build_provenance"]["layers"][0]["source"]
equal = all(item == identities[0] for item in identities[1:])
result = {
    "schema_version": 1,
    "status": "ok" if equal else "mismatch",
    "manifest": manifest,
    "target_platform": first_source["target_platform"],
    "build_host": first_source["build_host"],
    "builder": first_source["builder"],
    "requested_base": first_source["base_image"]["reference"],
    "resolved_base": first_source["base_image"]["resolved_reference"],
    "no_build_cache": True,
    "build_count": build_count,
    "equal": equal,
    "identities": identities[0] if equal else None,
    "builds": [
        {"sequence": index + 1, "identities": value}
        for index, value in enumerate(identities)
    ],
}
payload = json.dumps(result, separators=(",", ":")) + "\n"
if destination:
    pathlib.Path(destination).write_text(payload)
sys.stdout.write(payload)
if not equal:
    raise SystemExit(1)
PY
