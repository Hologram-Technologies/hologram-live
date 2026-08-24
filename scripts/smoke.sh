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
sed -i.bak 's/schema_version = 2/schema_version = 1/' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak '/dev.hologram.live.chat/d' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak "s/127\.0\.0\.1:11435/127.0.0.1:$PORT/g" "$CONFIG"
rm -f "$CONFIG.bak"

HOME="$HOME_DIR" "$BIN" start
grep -q 'schema_version = 2' "$CONFIG"
grep -q 'dev.hologram.live.chat' "$CONFIG"
HOME="$HOME_DIR" "$BIN" status >/dev/null
curl -fsS "http://127.0.0.1:$PORT/docs" | grep -q 'Scalar.createApiReference'
curl -fsS "http://127.0.0.1:$PORT/docs/scalar.js" >/dev/null
HOME="$HOME_DIR" "$BIN" modules list >/dev/null
printf 'stored by hologram\n' >"$TMP/upload.txt"
FILE=$(HOME="$HOME_DIR" "$BIN" --json files put "$TMP/upload.txt" --media-type text/plain)
FILE_ID=$(printf '%s\n' "$FILE" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\(blake3:[0-9a-f][0-9a-f]*\)".*/\1/p' | head -n 1)
if [ -z "$FILE_ID" ]; then
  echo "error: failed to parse stored file ID" >&2
  echo "$FILE" >&2
  exit 1
fi
FILES=$(HOME="$HOME_DIR" "$BIN" files list)
case "$FILES" in
  *upload.txt*) ;;
  *)
    echo "error: stored file missing from file listing" >&2
    echo "$FILES" >&2
    exit 1
    ;;
esac
HOME="$HOME_DIR" "$BIN" files rename "$FILE_ID" renamed.txt >/dev/null
RENAMED_FILES=$(HOME="$HOME_DIR" "$BIN" files list)
case "$RENAMED_FILES" in
  *renamed.txt*) ;;
  *)
    echo "error: renamed file missing from file listing" >&2
    echo "$RENAMED_FILES" >&2
    exit 1
    ;;
esac
HOME="$HOME_DIR" "$BIN" files get "$FILE_ID" --output "$TMP/download.txt" >/dev/null
cmp "$TMP/upload.txt" "$TMP/download.txt"
HTTP_OBJECT=$(curl -fsS -X POST -H 'content-type: text/plain' -H 'x-hologram-kind: smoke' -H 'x-hologram-filename: http.txt' --data-binary @"$TMP/upload.txt" "http://127.0.0.1:$PORT/api/v1/objects")
HTTP_ID=$(printf '%s\n' "$HTTP_OBJECT" | sed -n 's/.*"id":"\(blake3:[0-9a-f][0-9a-f]*\)".*/\1/p')
if [ -z "$HTTP_ID" ]; then
  echo "error: failed to parse HTTP object ID" >&2
  echo "$HTTP_OBJECT" >&2
  exit 1
fi
curl -fsS "http://127.0.0.1:$PORT/api/v1/objects/$HTTP_ID" >"$TMP/http-download.txt"
cmp "$TMP/upload.txt" "$TMP/http-download.txt"
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
THREAD=$(HOME="$HOME_DIR" "$BIN" --json history new smoke)
THREAD_ID=$(printf '%s\n' "$THREAD" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\(blake3:[0-9a-f][0-9a-f]*\)".*/\1/p' | head -n 1)
if [ -z "$THREAD_ID" ]; then
  echo "error: failed to parse conversation ID" >&2
  echo "$THREAD" >&2
  exit 1
fi
CHAT=$(HOME="$HOME_DIR" "$BIN" --json chat send "$THREAD_ID" "echo smoke")
case "$CHAT" in
  *'"role": "user"'*'"content": "echo smoke"'*'"role": "assistant"'*'"content": "echo smoke"'*) ;;
  *)
    echo "error: chat exchange was not recorded" >&2
    echo "$CHAT" >&2
    exit 1
    ;;
esac
HOME="$HOME_DIR" "$BIN" history list >/dev/null
HOME="$HOME_DIR" "$BIN" openapi --output "$TMP/openapi.json" >/dev/null
test -s "$TMP/openapi.json"
HOME="$HOME_DIR" "$BIN" stop >/dev/null

echo "hologram smoke test passed"
