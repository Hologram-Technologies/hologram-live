#!/usr/bin/env bash
set -euo pipefail

# Build a pinned kappa-registry, run it on a scratch store, and execute the
# provider conformance suite against it.
#
# Two upstream defects at this revision stop a clean clone from building, so
# both are patched here rather than blocking CI on an upstream merge:
#   1. Cargo.toml declares [[test]] in a virtual workspace manifest, which
#      cargo refuses to parse.
#   2. Cargo.lock lists the package `inventory` twice.
# Remove the patches once upstream fixes them and the pin moves.

readonly REV="2af86560a177fc9651b6c0e92e7974140ed77dd5"
readonly REPO="https://github.com/uoR-Foundation/kappa-registry.git"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-kappa-registry.XXXXXX")
server_pid=""

cleanup() {
  if [[ -n "${server_pid}" ]]; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  rm -rf -- "${work_dir}"
}
trap cleanup EXIT

for command in cargo git curl python3; do
  command -v "${command}" >/dev/null || {
    printf 'error: %s is required\n' "${command}" >&2
    exit 1
  }
done

printf 'cloning kappa-registry at %s\n' "${REV}"
git init --quiet "${work_dir}/src"
git -C "${work_dir}/src" remote add origin "${REPO}"
git -C "${work_dir}/src" fetch --quiet --depth 1 origin "${REV}"
git -C "${work_dir}/src" checkout --quiet FETCH_HEAD

# Patch 1: strip the [[test]] section from the virtual manifest.
python3 - "${work_dir}/src/Cargo.toml" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
text = text.replace('[[test]]\nname = "bdd"\nharness = false\n', '')
open(path, 'w').write(text)
PY

# Patch 2: regenerate the lockfile, which contains a duplicate entry.
rm -f "${work_dir}/src/Cargo.lock"
cargo generate-lockfile --manifest-path "${work_dir}/src/Cargo.toml" >/dev/null

printf 'building kappa-server (first run takes several minutes)\n'
cargo build --quiet --manifest-path "${work_dir}/src/Cargo.toml" --package kappa-server

# An ephemeral port so a developer's own registry on 5000 is never disturbed
# and two runs cannot collide.
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
endpoint="http://127.0.0.1:${port}"

KAPPA_LISTEN_ADDR="127.0.0.1:${port}" \
KAPPA_STORE_ROOT="${work_dir}/data" \
  "${work_dir}/src/target/debug/kappa-server" >"${work_dir}/server.log" 2>&1 &
server_pid=$!

printf 'waiting for %s\n' "${endpoint}"
ready=0
for _ in $(seq 1 60); do
  if curl -fsS -o /dev/null "${endpoint}/v2/" 2>/dev/null; then
    ready=1
    break
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    printf 'error: kappa-server exited during startup\n' >&2
    cat "${work_dir}/server.log" >&2
    exit 1
  fi
  sleep 1
done
if [[ "${ready}" -ne 1 ]]; then
  printf 'error: kappa-server did not become ready within 60 seconds\n' >&2
  cat "${work_dir}/server.log" >&2
  exit 1
fi

printf 'running provider conformance against %s\n' "${endpoint}"
KAPPA_REGISTRY_ENDPOINT="${endpoint}" \
  cargo test --locked --manifest-path "${repo_root}/Cargo.toml" \
  --test registry_conformance -- --test-threads=1

printf 'kappa registry conformance passed\n'
