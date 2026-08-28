#!/usr/bin/env bash
set -euo pipefail

file=${1:?usage: verify-sha256.sh FILE EXPECTED_SHA256}
expected_sha256=${2:?usage: verify-sha256.sh FILE EXPECTED_SHA256}

if command -v sha256sum >/dev/null 2>&1; then
  actual_sha256=$(sha256sum "${file}" | awk '{print $1}')
else
  actual_sha256=$(shasum -a 256 "${file}" | awk '{print $1}')
fi

if [[ "${actual_sha256}" != "${expected_sha256}" ]]; then
  echo "SHA-256 mismatch for ${file}: expected ${expected_sha256}, got ${actual_sha256}" >&2
  exit 1
fi

echo "${file}: SHA-256 verified" >&2
