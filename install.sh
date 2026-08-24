#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PREFIX=${HOLOGRAM_INSTALL_PREFIX:-"$HOME/.local"}
CARGO_BIN=${CARGO:-cargo}
DEST="$PREFIX/bin/hologram"

if ! command -v "$CARGO_BIN" >/dev/null 2>&1; then
  echo "error: cargo is required to build this source distribution" >&2
  echo "install Rust from https://rustup.rs, then run ./install.sh again" >&2
  exit 1
fi

"$CARGO_BIN" build --manifest-path "$ROOT/Cargo.toml" --release --locked
mkdir -p "$PREFIX/bin"
if command -v install >/dev/null 2>&1; then
  install -m 0755 "$ROOT/target/release/hologram" "$DEST"
else
  cp "$ROOT/target/release/hologram" "$DEST"
  chmod 0755 "$DEST"
fi

"$DEST" init >/dev/null 2>&1 || true
printf 'installed hologram to %s\n' "$DEST"
case ":${PATH:-}:" in
  *":$PREFIX/bin:"*) ;;
  *) printf 'add %s/bin to PATH\n' "$PREFIX" ;;
esac
