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
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$HOME_DIR"

cleanup() {
  HOME="$HOME_DIR" "$BIN" stop >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

HOME="$HOME_DIR" "$BIN" init
# Emulate a genuine schema-v1 config: strip every module added since v1 so the
# enabled set matches the v1 defaults and the migration path triggers.
sed -i.bak 's/schema_version = 2/schema_version = 1/' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak '/dev.hologram.live.chat/d' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak '/dev.hologram.live.inference/d' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak '/dev.hologram.live.openai-compat/d' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak '/dev.hologram.live.ollama-compat/d' "$CONFIG"
rm -f "$CONFIG.bak"
sed -i.bak "s/127\.0\.0\.1:11435/127.0.0.1:$PORT/g" "$CONFIG"
rm -f "$CONFIG.bak"

HOME="$HOME_DIR" "$BIN" start
grep -q 'schema_version = 2' "$CONFIG"
grep -q 'dev.hologram.live.chat' "$CONFIG"
grep -q 'dev.hologram.live.inference' "$CONFIG"
# Migration restores chat + inference only; re-enable the compatibility modules
# and restart so their endpoints are exercised below.
sed -i.bak '/dev.hologram.live.inference/a\
    "dev.hologram.live.openai-compat",\
    "dev.hologram.live.ollama-compat",' "$CONFIG"
rm -f "$CONFIG.bak"
HOME="$HOME_DIR" "$BIN" restart >/dev/null
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
HOME="$HOME_DIR" "$BIN" compile "$ROOT/features/fixtures/wasm-app/hologram.json" -o "$TMP/wasm-app.holo" >/dev/null
WASM_IMPORT=$(HOME="$HOME_DIR" "$BIN" --json holo import "$TMP/wasm-app.holo")
WASM_KAPPA=$(printf '%s\n' "$WASM_IMPORT" | sed -n 's/.*"kappa"[[:space:]]*:[[:space:]]*"\(blake3:[0-9a-f][0-9a-f]*\)".*/\1/p' | head -n 1)
if [ -z "$WASM_KAPPA" ]; then
  echo "error: failed to parse imported wasm kappa" >&2
  echo "$WASM_IMPORT" >&2
  exit 1
fi
HOME="$HOME_DIR" "$BIN" holo load "$WASM_KAPPA" >/dev/null
printf 'hello hologram' >"$TMP/holo-input.txt"
RUN=$(HOME="$HOME_DIR" "$BIN" --json run "$WASM_KAPPA" --input "$TMP/holo-input.txt")
RUN_FLAT=$(printf '%s\n' "$RUN" | tr -d ' \n')
case "$RUN_FLAT" in
  *'[72,69,76,76,79,32,72,79,76,79,71,82,65,77]'*) ;;
  *)
    echo "error: wasm run did not upper-case the input" >&2
    echo "$RUN" >&2
    exit 1
    ;;
esac
RESIDENT=$(HOME="$HOME_DIR" "$BIN" --json holo resident)
case "$RESIDENT" in
  *"$WASM_KAPPA"*) ;;
  *)
    echo "error: loaded archive missing from resident listing" >&2
    echo "$RESIDENT" >&2
    exit 1
    ;;
esac
HOME="$HOME_DIR" "$BIN" holo unload "$WASM_KAPPA" >/dev/null
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
HOME="$HOME_DIR" "$BIN" models list >/dev/null
MODELS_JSON=$(curl -fsS "http://127.0.0.1:$PORT/v1/models")
case "$MODELS_JSON" in
  *'"object":"list"'*) ;;
  *)
    echo "error: OpenAI model listing missing list object" >&2
    echo "$MODELS_JSON" >&2
    exit 1
    ;;
esac
COMPLETION=$(curl -fsS -X POST -H 'content-type: application/json' --data '{"model":"echo","messages":[{"role":"user","content":"smoke"}]}' "http://127.0.0.1:$PORT/v1/chat/completions")
case "$COMPLETION" in
  *'"object":"chat.completion"'*) ;;
  *)
    echo "error: OpenAI chat completion missing completion object" >&2
    echo "$COMPLETION" >&2
    exit 1
    ;;
esac
GENERATE=$(curl -fsS -X POST -H 'content-type: application/json' --data '{"model":"echo","prompt":"smoke","stream":false}' "http://127.0.0.1:$PORT/api/generate")
case "$GENERATE" in
  *'"done":true'*) ;;
  *)
    echo "error: Ollama generate did not finish" >&2
    echo "$GENERATE" >&2
    exit 1
    ;;
esac
curl -fsS "http://127.0.0.1:$PORT/api/tags" | grep -q '"models"'
HOME="$HOME_DIR" "$BIN" openapi --output "$TMP/openapi.json" >/dev/null
test -s "$TMP/openapi.json"
HOME="$HOME_DIR" "$BIN" stop >/dev/null

echo "hologram smoke test passed"
