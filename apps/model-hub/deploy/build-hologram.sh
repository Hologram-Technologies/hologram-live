#!/usr/bin/env bash
# Build the hologram CLI from hologram-live origin/main inside Debian bookworm (glibc matches the VPS runtime image).
set -euo pipefail

WORK="$HOME/hub-hologram-build"
OUT="$HOME/hub-kappa-out"
mkdir -p "$WORK" "$OUT"

if [ ! -d "$WORK/src/.git" ]; then
  git clone --quiet --depth 1 https://github.com/Hologram-Technologies/hologram-live.git "$WORK/src"
else
  git -C "$WORK/src" fetch --quiet --depth 1 origin main
  git -C "$WORK/src" reset --quiet --hard origin/main
fi
git -C "$WORK/src" rev-parse --short HEAD

docker run --rm \
  -v "$WORK/src:/src" -v "$WORK/cargo-registry:/usr/local/cargo/registry" -v "$WORK/cargo-git:/usr/local/cargo/git" \
  -v "$WORK/target:/target" -e CARGO_TARGET_DIR=/target -w /src rust:1.97-bookworm \
  bash -c 'rustup show active-toolchain >/dev/null 2>&1 || true; cargo build --release --locked --bin hologram -j 6'

cp "$WORK/target/release/hologram" "$OUT/hologram"
sha256sum "$OUT/hologram"
ls -la "$OUT/hologram"
