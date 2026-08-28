#!/usr/bin/env bash
set -euo pipefail

componentizer_source=${1:?usage: prepare-componentizer-source.sh COMPONENTIZE_PY_SOURCE}
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH='' cd -- "${script_dir}/.." && pwd)

componentizer_revision=c0949b19d464f5d70bc1051195a3ae0e6a012df9
wasmtime_wasi_version=46.0.1
wasmtime_wasi_sha256=e9f65ef30a2c5478873cdb619085a7a649d3ce41cc3eaf298a7ce3dee96a8e11

actual_revision=$(git -C "${componentizer_source}" rev-parse HEAD)
if [[ "${actual_revision}" != "${componentizer_revision}" ]]; then
  echo "componentize-py revision mismatch: expected ${componentizer_revision}, got ${actual_revision}" >&2
  exit 1
fi
if ! git -C "${componentizer_source}" diff --quiet ||
  ! git -C "${componentizer_source}" diff --cached --quiet; then
  echo "componentize-py source must be clean before deterministic patching" >&2
  exit 1
fi

git -C "${componentizer_source}" apply --check \
  "${repository_root}/tools/componentize-py/deterministic-build-randomness.patch"
git -C "${componentizer_source}" apply \
  "${repository_root}/tools/componentize-py/deterministic-build-randomness.patch"
git -C "${componentizer_source}" apply --check \
  "${repository_root}/tools/componentize-py/deterministic-generation-order.patch"
git -C "${componentizer_source}" apply \
  "${repository_root}/tools/componentize-py/deterministic-generation-order.patch"

archive=$(mktemp "${TMPDIR:-/tmp}/wasmtime-wasi-${wasmtime_wasi_version}.XXXXXX")
trap 'rm -f "${archive}"' EXIT
curl --fail --location --silent --show-error \
  "https://static.crates.io/crates/wasmtime-wasi/wasmtime-wasi-${wasmtime_wasi_version}.crate" \
  --output "${archive}"
"${script_dir}/verify-sha256.sh" "${archive}" "${wasmtime_wasi_sha256}"

mkdir -p "${componentizer_source}/vendor"
tar -xzf "${archive}" -C "${componentizer_source}/vendor"
mv \
  "${componentizer_source}/vendor/wasmtime-wasi-${wasmtime_wasi_version}" \
  "${componentizer_source}/vendor/wasmtime-wasi"
git -C "${componentizer_source}" apply --directory=vendor/wasmtime-wasi --check \
  "${repository_root}/tools/componentize-py/wasmtime-wasi-deterministic-metadata.patch"
git -C "${componentizer_source}" apply --directory=vendor/wasmtime-wasi \
  "${repository_root}/tools/componentize-py/wasmtime-wasi-deterministic-metadata.patch"
git -C "${componentizer_source}" apply --directory=vendor/wasmtime-wasi --check \
  "${repository_root}/tools/componentize-py/wasmtime-wasi-deterministic-readdir.patch"
git -C "${componentizer_source}" apply --directory=vendor/wasmtime-wasi \
  "${repository_root}/tools/componentize-py/wasmtime-wasi-deterministic-readdir.patch"
git -C "${componentizer_source}" apply --check \
  "${repository_root}/tools/componentize-py/deterministic-metadata-wiring.patch"
git -C "${componentizer_source}" apply \
  "${repository_root}/tools/componentize-py/deterministic-metadata-wiring.patch"
git -C "${componentizer_source}" apply --check \
  "${repository_root}/tools/componentize-py/deterministic-metadata-preregistration.patch"
git -C "${componentizer_source}" apply \
  "${repository_root}/tools/componentize-py/deterministic-metadata-preregistration.patch"

echo "prepared deterministic componentize-py ${componentizer_revision}" >&2
