#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
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

python_bin=python3
if ! command -v "$python_bin" >/dev/null 2>&1; then
  python_bin=python
fi
command -v "$python_bin" >/dev/null 2>&1 || {
  printf 'python3 or python is required to produce the JSON report\n' >&2
  exit 1
}
command -v uvx >/dev/null 2>&1 || {
  printf 'uvx is required to compile the pinned Python Component\n' >&2
  exit 1
}

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-component-repro.XXXXXX")
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

printf 'building the optimized hologram compiler\n' >&2
cargo build --release --locked --package hologram-live --bin hologram \
  --manifest-path "$repo_root/Cargo.toml" >&2

target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
binary="$target_dir/release/hologram"
if [[ ! -x "$binary" && -x "$binary.exe" ]]; then
  binary="$binary.exe"
fi
[[ -x "$binary" ]] || {
  printf 'compiled hologram binary is missing at %s\n' "$binary" >&2
  exit 1
}

manifest="$repo_root/examples/python-component-dependency/hologram.json"
reports=()
archives=()
runs=()
build=1
while [[ $build -le $build_count ]]; do
  build_dir="$work_dir/build-$build"
  archive="$build_dir/application.holo"
  compile_report="$build_dir/compile.json"
  run_report="$build_dir/run.json"
  mkdir -p "$build_dir"
  printf 'clean component build %s of %s\n' "$build" "$build_count" >&2
  env \
    "HOLOGRAM_CONFIG_DIR=$build_dir/config" \
    "HOLOGRAM_DATA_DIR=$build_dir/data" \
    "HOLOGRAM_STATE_DIR=$build_dir/state" \
    "HOLOGRAM_CACHE_DIR=$build_dir/hologram-cache" \
    "UV_CACHE_DIR=$build_dir/uv-cache" \
    "$binary" --json compile "$manifest" --output "$archive" >"$compile_report"
  env \
    "HOLOGRAM_CONFIG_DIR=$build_dir/config" \
    "HOLOGRAM_DATA_DIR=$build_dir/data" \
    "HOLOGRAM_STATE_DIR=$build_dir/state" \
    "HOLOGRAM_CACHE_DIR=$build_dir/hologram-cache" \
    "$binary" --json run "$archive" --input-text Ada --output-format json >"$run_report"
  reports+=("$compile_report")
  archives+=("$archive")
  runs+=("$run_report")
  build=$((build + 1))
done

if [[ -n "$report_path" ]]; then
  mkdir -p "$(dirname -- "$report_path")"
fi

"$python_bin" - "$report_path" "$manifest" "$build_count" \
  "${reports[@]}" --archives "${archives[@]}" --runs "${runs[@]}" <<'PY'
import hashlib
import json
import pathlib
import sys

destination = sys.argv[1]
manifest = sys.argv[2]
build_count = int(sys.argv[3])
separator_archives = sys.argv.index("--archives")
separator_runs = sys.argv.index("--runs")
report_paths = sys.argv[4:separator_archives]
archive_paths = sys.argv[separator_archives + 1:separator_runs]
run_paths = sys.argv[separator_runs + 1:]

if not (len(report_paths) == len(archive_paths) == len(run_paths) == build_count):
    raise ValueError("component report inputs do not match the requested build count")

reports = [json.loads(pathlib.Path(path).read_text()) for path in report_paths]
runs = [json.loads(pathlib.Path(path).read_text()) for path in run_paths]

def identity(report, archive_path):
    provenance = report["build_provenance"]
    source = provenance["layers"][0]["source"]
    output = source["output"]
    patch_set = source["componentizer"]["patch_set"]
    distribution = source["componentizer"]["distribution"]
    if provenance.get("canonical") is not False:
        raise ValueError("build provenance must remain non-canonical")
    if source.get("profile") != "wasi-component":
        raise ValueError("compile report is not for the Python Component profile")
    if patch_set.get("determinism_contract") != "hologram:componentizer/preinitialization-determinism@1":
        raise ValueError("compile report does not contain the deterministic componentizer contract")
    if patch_set.get("release_tag") != "componentizer-v0.25.0-hologram.1":
        raise ValueError("compile report does not contain the pinned componentizer release")
    if len(distribution.get("sha256", "")) != 64:
        raise ValueError("compile report does not contain the componentizer wheel digest")
    archive = pathlib.Path(archive_path).read_bytes()
    return {
        "component_layer_kappa": output["layer_kappa"],
        "component_byte_length": output["byte_length"],
        "capabilities_kappa": report["capabilities_kappa"],
        "application_kappa": report["application_kappa"],
        "archive_kappa": report["archive_kappa"],
        "archive_fingerprint": report["archive_fingerprint"],
        "archive_byte_length": report["byte_length"],
        "archive_sha256": hashlib.sha256(archive).hexdigest(),
    }

def execution(run):
    expected = {
        "dependency": "six-1.17.0",
        "message": "Hello, Ada!",
        "name": "Ada",
        "runtime": "python-component",
    }
    if run != expected:
        raise ValueError(f"unexpected component response: {run!r}")
    return {"status": "ok", "response": run}

identities = [
    identity(report, archive)
    for report, archive in zip(reports, archive_paths, strict=True)
]
executions = [execution(run) for run in runs]
first_source = reports[0]["build_provenance"]["layers"][0]["source"]
build_host = first_source["build_host"]
target_host = f'{build_host["os"]}/{build_host["arch"]}'
build_contract = {
    "compiler": first_source["compiler"],
    "runtime": first_source["runtime"],
    "componentizer": first_source["componentizer"],
    "componentizer_runner": first_source["componentizer_runner"],
    "dependency_installer": first_source["dependency_installer"],
    "guest_contract": first_source["guest_contract"],
    "target_abi": first_source["target_abi"],
}
equal = all(item == identities[0] for item in identities[1:])
result = {
    "schema_version": 1,
    "status": "ok" if equal else "mismatch",
    "manifest": manifest,
    "target_host": target_host,
    "build_host": build_host,
    "build_count": build_count,
    "isolated_uv_cache": True,
    "equal": equal,
    "provenance_reproducible": first_source["reproducibility"]["reproducible"],
    "build_contract": build_contract,
    "identities": identities[0] if equal else None,
    "builds": [
        {
            "sequence": index + 1,
            "identities": value,
            "execution": executions[index],
        }
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
