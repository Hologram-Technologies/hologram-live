#!/usr/bin/env sh
set -eu

BIN=${1:-./target/release/hologram}
case "$BIN" in
  /*) ;;
  *) BIN=$(CDPATH= cd -- "$(dirname -- "$BIN")" && pwd)/$(basename -- "$BIN") ;;
esac

if [ ! -x "$BIN" ]; then
  echo "error: executable not found: $BIN" >&2
  exit 1
fi

TMP=${TMPDIR:-/tmp}/hologram-smoke-$$
HOME_DIR="$TMP/home"
PORT=$((20000 + ($$ % 20000)))
CONFIG="$HOME_DIR/.config/hologram/live.toml"
mkdir -p "$HOME_DIR"

cleanup() {
  HOME="$HOME_DIR" "$BIN" stop >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

HOME="$HOME_DIR" "$BIN" init
sed -i.bak "s/127\.0\.0\.1:11435/127.0.0.1:$PORT/g" "$CONFIG"
rm -f "$CONFIG.bak"

HOME="$HOME_DIR" "$BIN" start
HOME="$HOME_DIR" "$BIN" status >/dev/null
curl -fsS "http://127.0.0.1:$PORT/docs" | grep -q 'Scalar.createApiReference'
curl -fsS "http://127.0.0.1:$PORT/docs/scalar.js" >/dev/null
HOME="$HOME_DIR" "$BIN" modules list >/dev/null
HOME="$HOME_DIR" "$BIN" holo fixture "$TMP/fixture.holo"
IMPORT=$(HOME="$HOME_DIR" "$BIN" --json holo import "$TMP/fixture.holo")
KAPPA=$(printf '%s\n' "$IMPORT" | sed -n 's/.*"kappa"[[:space:]]*:[[:space:]]*"\(blake3:[0-9a-f][0-9a-f]*\)".*/\1/p' | head -n 1)
if [ -z "$KAPPA" ]; then
  echo "error: failed to parse imported kappa" >&2
  echo "$IMPORT" >&2
  exit 1
fi
HOME="$HOME_DIR" "$BIN" holo inspect "$KAPPA" >/dev/null
HOME="$HOME_DIR" "$BIN" holo verify "$KAPPA" >/dev/null
HOME="$HOME_DIR" "$BIN" history new smoke >/dev/null
HOME="$HOME_DIR" "$BIN" history list >/dev/null
HOME="$HOME_DIR" "$BIN" openapi --output "$TMP/openapi.json" >/dev/null
test -s "$TMP/openapi.json"
HOME="$HOME_DIR" "$BIN" stop >/dev/null

echo "hologram smoke test passed"
