#!/usr/bin/env bash
# Build kappa-server for the hub VPS inside Debian bookworm, so the binary's glibc matches the runtime image.
# Same revision and patches as hologram-live scripts/check-kappa-registry.sh.
set -euo pipefail

REV="2af86560a177fc9651b6c0e92e7974140ed77dd5"
WORK="$HOME/hub-kappa-build"
OUT="$HOME/hub-kappa-out"
mkdir -p "$WORK" "$OUT"

if [ ! -d "$WORK/src/.git" ]; then
  git init -q "$WORK/src"
  git -C "$WORK/src" remote add origin https://github.com/UOR-Foundation/kappa-registry.git
fi
git -C "$WORK/src" fetch -q --depth 1 origin "$REV"
git -C "$WORK/src" checkout -q -f FETCH_HEAD

python3 - "$WORK/src/Cargo.toml" <<'PY'
import sys
p = sys.argv[1]
t = open(p).read()
open(p, "w").write(t.replace('[[test]]\nname = "bdd"\nharness = false\n', ''))
PY
rm -f "$WORK/src/Cargo.lock"

docker run --rm \
  -v "$WORK/src:/src" -v "$WORK/cargo-registry:/usr/local/cargo/registry" -v "$WORK/target:/target" \
  -e CARGO_TARGET_DIR=/target -w /src rust:1.97-bookworm \
  bash -c 'cargo generate-lockfile >/dev/null && cargo build --release --package kappa-server -j 4'

cp "$WORK/target/release/kappa-server" "$OUT/kappa-server"
sha256sum "$OUT/kappa-server"
ls -la "$OUT/kappa-server"
