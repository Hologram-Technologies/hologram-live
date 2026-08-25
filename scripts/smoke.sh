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
if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required to validate the CLI JSON contract" >&2
  exit 1
fi

json_output() {
  output=$("$@") || {
    status=$?
    echo "error: JSON command failed ($status): $*" >&2
    printf '%s\n' "$output" >&2
    return "$status"
  }
  if ! printf '%s\n' "$output" | jq -e . >/dev/null; then
    echo "error: command emitted invalid JSON: $*" >&2
    printf '%s\n' "$output" >&2
    return 1
  fi
  printf '%s\n' "$output"
}

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

HOME="$HOME_DIR" json_output "$BIN" --json init >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json config path >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json config validate >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json config show >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json doctor >/dev/null
mkdir -p "$TMP/generated-app"
HOME="$HOME_DIR" json_output "$BIN" --json app init "$TMP/generated-app" --yes >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json compile --check "$ROOT/features/fixtures/wasm-app/hologram.json" >/dev/null
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

HOME="$HOME_DIR" json_output "$BIN" --json start >/dev/null
grep -q 'schema_version = 2' "$CONFIG"
grep -q 'dev.hologram.live.chat' "$CONFIG"
grep -q 'dev.hologram.live.inference' "$CONFIG"
# Migration restores chat + inference only; re-enable the compatibility modules
# and restart so their endpoints are exercised below.
sed -i.bak '/dev.hologram.live.inference/a\
    "dev.hologram.live.openai-compat",\
    "dev.hologram.live.ollama-compat",' "$CONFIG"
rm -f "$CONFIG.bak"
HOME="$HOME_DIR" json_output "$BIN" --json restart >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json status >/dev/null
curl -fsS "http://127.0.0.1:$PORT/docs" | grep -q 'Scalar.createApiReference'
curl -fsS "http://127.0.0.1:$PORT/docs/scalar.js" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json modules list >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json modules graph >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json route system.health >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json route holo.plan | jq -e '.operation == "holo.plan"' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json registry list >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json nodes list >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json nodes heartbeat smoke-node >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json plugins list >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json tracing show >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json tracing set info >/dev/null
printf 'stored by hologram\n' >"$TMP/upload.txt"
FILE=$(HOME="$HOME_DIR" json_output "$BIN" --json files put "$TMP/upload.txt" --media-type text/plain)
FILE_ID=$(printf '%s\n' "$FILE" | jq -er '.id')
if [ -z "$FILE_ID" ]; then
  echo "error: failed to parse stored file ID" >&2
  echo "$FILE" >&2
  exit 1
fi
FILES=$(HOME="$HOME_DIR" json_output "$BIN" --json files list)
printf '%s\n' "$FILES" | jq -e 'any(.[]; .filename == "upload.txt")' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json files rename "$FILE_ID" renamed.txt >/dev/null
RENAMED_FILES=$(HOME="$HOME_DIR" json_output "$BIN" --json files list)
printf '%s\n' "$RENAMED_FILES" | jq -e 'any(.[]; .filename == "renamed.txt")' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json files get "$FILE_ID" --output "$TMP/download.txt" >/dev/null
cmp "$TMP/upload.txt" "$TMP/download.txt"
REGISTRY_OBJECT=$(HOME="$HOME_DIR" json_output "$BIN" --json registry put "$TMP/upload.txt" --kind smoke --media-type text/plain)
REGISTRY_ID=$(printf '%s\n' "$REGISTRY_OBJECT" | jq -er '.id')
HOME="$HOME_DIR" json_output "$BIN" --json registry get "$REGISTRY_ID" --output "$TMP/registry-download.txt" >/dev/null
cmp "$TMP/upload.txt" "$TMP/registry-download.txt"
HTTP_OBJECT=$(curl -fsS -X POST -H 'content-type: text/plain' -H 'x-hologram-kind: smoke' -H 'x-hologram-filename: http.txt' --data-binary @"$TMP/upload.txt" "http://127.0.0.1:$PORT/api/v1/objects")
HTTP_ID=$(printf '%s\n' "$HTTP_OBJECT" | sed -n 's/.*"id":"\(blake3:[0-9a-f][0-9a-f]*\)".*/\1/p')
if [ -z "$HTTP_ID" ]; then
  echo "error: failed to parse HTTP object ID" >&2
  echo "$HTTP_OBJECT" >&2
  exit 1
fi
curl -fsS "http://127.0.0.1:$PORT/api/v1/objects/$HTTP_ID" >"$TMP/http-download.txt"
cmp "$TMP/upload.txt" "$TMP/http-download.txt"
HOME="$HOME_DIR" json_output "$BIN" --json holo fixture "$TMP/fixture.holo" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json holo inspect "$TMP/fixture.holo" >/dev/null
set +e
AI_ERROR=$(HOME="$HOME_DIR" "$BIN" --json ai inspect "$TMP/fixture.holo" 2>/dev/null)
AI_STATUS=$?
set -e
if [ "$AI_STATUS" -eq 0 ]; then
  echo "error: model inspection unexpectedly accepted a model-free archive" >&2
  exit 1
fi
printf '%s\n' "$AI_ERROR" | jq -e '.code == "LIVE_HOLO_INVALID"' >/dev/null
IMPORT=$(HOME="$HOME_DIR" json_output "$BIN" --json holo import "$TMP/fixture.holo")
KAPPA=$(printf '%s\n' "$IMPORT" | jq -er '.kappa')
if [ -z "$KAPPA" ]; then
  echo "error: failed to parse imported kappa" >&2
  echo "$IMPORT" >&2
  exit 1
fi
HOME="$HOME_DIR" json_output "$BIN" --json holo inspect "$KAPPA" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json holo verify "$KAPPA" >/dev/null
COMPILE=$(HOME="$HOME_DIR" json_output "$BIN" --json compile "$ROOT/features/fixtures/wasm-app/hologram.json" -o "$TMP/wasm-app.holo")
printf '%s\n' "$COMPILE" | jq -e '
  (.archive_kappa | startswith("blake3:")) and
  (.archive_fingerprint | length > 0) and
  (.application_kappa | startswith("blake3:"))
' >/dev/null
INSPECTION=$(HOME="$HOME_DIR" json_output "$BIN" --json holo inspect "$TMP/wasm-app.holo")
printf '%s\n' "$INSPECTION" | jq -e --arg application_kappa "$(printf '%s\n' "$COMPILE" | jq -r '.application_kappa')" '
  (.kappa | startswith("blake3:")) and
  .application_kappa == $application_kappa
' >/dev/null
DIRECT_PLAN=$(HOME="$HOME_DIR" json_output "$BIN" --json holo plan "$TMP/wasm-app.holo")
printf '%s\n' "$DIRECT_PLAN" | jq -e '
  .execution_target == "direct" and
  .packaging == "fat" and
  .runnable == true and
  .layers[0].provider.status == "available" and
  (.layers[0] | has("content") | not) and
  (.layers[0] | has("bytes") | not)
' >/dev/null
WASM_IMPORT=$(HOME="$HOME_DIR" json_output "$BIN" --json holo import "$TMP/wasm-app.holo")
WASM_KAPPA=$(printf '%s\n' "$WASM_IMPORT" | jq -er '.kappa')
if [ -z "$WASM_KAPPA" ]; then
  echo "error: failed to parse imported wasm kappa" >&2
  echo "$WASM_IMPORT" >&2
  exit 1
fi
RESIDENT_PLAN=$(HOME="$HOME_DIR" json_output "$BIN" --json holo plan "$WASM_KAPPA")
printf '%s\n' "$RESIDENT_PLAN" | jq -e --arg kappa "$WASM_KAPPA" '
  .archive_kappa == $kappa and
  .execution_target == "resident" and
  .runnable == true
' >/dev/null
curl -fsS "http://127.0.0.1:$PORT/api/v1/holo/$WASM_KAPPA/plan" |
  jq -e --arg kappa "$WASM_KAPPA" '.archive_kappa == $kappa and .runnable == true' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json holo load "$WASM_KAPPA" >/dev/null
printf 'hello hologram' >"$TMP/holo-input.txt"
RUN=$(HOME="$HOME_DIR" json_output "$BIN" --json run "$WASM_KAPPA" --input "$TMP/holo-input.txt")
printf '%s\n' "$RUN" | jq -e '.outputs[0] == [72,69,76,76,79,32,72,79,76,79,71,82,65,77]' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json run "$WASM_KAPPA" --input "$TMP/holo-input.txt" --output-format text >/dev/null
RESIDENT=$(HOME="$HOME_DIR" json_output "$BIN" --json holo resident)
printf '%s\n' "$RESIDENT" | jq -e --arg kappa "$WASM_KAPPA" 'any(.[]; .kappa == $kappa and .state == "running")' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json holo unload "$WASM_KAPPA" >/dev/null
THREAD=$(HOME="$HOME_DIR" json_output "$BIN" --json history new smoke)
THREAD_ID=$(printf '%s\n' "$THREAD" | jq -er '.id')
if [ -z "$THREAD_ID" ]; then
  echo "error: failed to parse conversation ID" >&2
  echo "$THREAD" >&2
  exit 1
fi
HOME="$HOME_DIR" json_output "$BIN" --json history append "$THREAD_ID" --role system "smoke system" >/dev/null
CHAT=$(HOME="$HOME_DIR" json_output "$BIN" --json chat send "$THREAD_ID" "echo smoke")
printf '%s\n' "$CHAT" | jq -e '[.messages[] | select(.content == "echo smoke") | .role] == ["user", "assistant"]' >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json history show "$THREAD_ID" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json history list >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json history archive "$THREAD_ID" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json history unarchive "$THREAD_ID" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json models list >/dev/null
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
HOME="$HOME_DIR" json_output "$BIN" --json openapi --output "$TMP/openapi.json" >/dev/null
test -s "$TMP/openapi.json"
HOME="$HOME_DIR" json_output "$BIN" --json openapi >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json holo remove "$WASM_KAPPA" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json history delete "$THREAD_ID" >/dev/null
HOME="$HOME_DIR" json_output "$BIN" --json stop >/dev/null

echo "hologram smoke test passed"
