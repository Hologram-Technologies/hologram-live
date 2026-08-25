#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

# cargo-tree only needs rustc version discovery. Bypass optional compiler-cache
# wrappers so this structural check also works in restricted build sandboxes.
server_tree=$(RUSTC_WRAPPER= cargo tree \
  --manifest-path "$ROOT/Cargo.toml" \
  --package hologram-live \
  --edges normal \
  --prefix none)

case "$server_tree" in
  *"hologram-application-watch v"*|*"hologram-desktop v"*|*"tauri v"*)
    echo "error: standalone server dependency graph contains desktop code" >&2
    exit 1
    ;;
esac

if ! grep -q '^hologram-application-watch = ' "$ROOT/apps/desktop/src-tauri/Cargo.toml"; then
  echo "error: desktop adapter must consume the external application-watch crate" >&2
  exit 1
fi

printf 'product boundary gate passed (server graph excludes desktop dependencies)\n'
