#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage:
  scripts/check-inference-engine.sh llamacpp /path/to/model.gguf
  HOLOGRAM_TOKENIZER_PATH=/path/to/tokenizer.json scripts/check-inference-engine.sh candle /path/to/model.gguf
  HOLOGRAM_TOKENIZER_PATH=/path/to/tokenizer.model HOLOGRAM_MODEL_ARCHITECTURE=llama3.2-1b scripts/check-inference-engine.sh burn /path/to/model.mpk
  VLLM_ENDPOINT=http://127.0.0.1:8000 scripts/check-inference-engine.sh vllm [model-id]

Optional environment:
  HOLOGRAM_BIN                 prebuilt hologram binary
  HOLOGRAM_TEST_GGUF           GGUF path when the second argument is omitted
  HOLOGRAM_LLAMA_FEATURES      Cargo features (default: llamacpp)
  HOLOGRAM_LLAMA_N_CTX         llama.cpp context size (default: 2048)
  HOLOGRAM_TOKENIZER_PATH      tokenizer file for Candle or Burn
  HOLOGRAM_MODEL_ARCHITECTURE  candle: llama; burn: supported Llama 3 variant
  INFERENCE_SMOKE_TIMEOUT_SECS request/startup timeout (default: 120)
  VLLM_API_KEY                 bearer token passed through to the vLLM adapter
EOF
  exit 2
}

ENGINE=${1:-}
SUBJECT=${2:-}
case "$ENGINE" in
  llamacpp | candle | burn | vllm) ;;
  *) usage ;;
esac

for command in cargo curl jq; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "error: $command is required" >&2
    exit 1
  fi
done

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TIMEOUT=${INFERENCE_SMOKE_TIMEOUT_SECS:-120}
N_CTX=${HOLOGRAM_LLAMA_N_CTX:-2048}
PORT=${HOLOGRAM_SMOKE_PORT:-$((24000 + ($$ % 16000)))}
LISTEN="127.0.0.1:$PORT"
BASE="http://$LISTEN"

json_string() {
  jq -Rn --arg value "$1" '$value'
}

vllm_get() {
  if [ -n "${VLLM_API_KEY:-}" ]; then
    curl -H "authorization: Bearer $VLLM_API_KEY" "$@"
  else
    curl "$@"
  fi
}

case "$ENGINE" in
  llamacpp)
    MODEL_PATH=${SUBJECT:-${HOLOGRAM_TEST_GGUF:-}}
    if [ -z "$MODEL_PATH" ] || [ ! -f "$MODEL_PATH" ]; then
      echo "error: pass a readable GGUF model path or set HOLOGRAM_TEST_GGUF" >&2
      exit 1
    fi
    MODEL_PATH=$(CDPATH= cd -- "$(dirname -- "$MODEL_PATH")" && pwd)/$(basename -- "$MODEL_PATH")
    MODEL_ID=$MODEL_PATH
    VLLM_ENDPOINT_VALUE=http://127.0.0.1:8000
    FEATURES=${HOLOGRAM_LLAMA_FEATURES:-llamacpp}
    TOKENIZER_PATH=
    MODEL_ARCHITECTURE=
    STREAM_KIND=native
    ;;
  candle | burn)
    MODEL_PATH=$SUBJECT
    TOKENIZER_PATH=${HOLOGRAM_TOKENIZER_PATH:-}
    MODEL_ARCHITECTURE=${HOLOGRAM_MODEL_ARCHITECTURE:-}
    if [ -z "$MODEL_PATH" ] || [ ! -f "$MODEL_PATH" ]; then
      echo "error: pass a readable $ENGINE model path" >&2
      exit 1
    fi
    if [ -z "$TOKENIZER_PATH" ] || [ ! -f "$TOKENIZER_PATH" ]; then
      echo "error: set HOLOGRAM_TOKENIZER_PATH to a readable tokenizer file" >&2
      exit 1
    fi
    if [ -z "$MODEL_ARCHITECTURE" ]; then
      echo "error: set HOLOGRAM_MODEL_ARCHITECTURE for $ENGINE" >&2
      exit 1
    fi
    MODEL_PATH=$(CDPATH= cd -- "$(dirname -- "$MODEL_PATH")" && pwd)/$(basename -- "$MODEL_PATH")
    TOKENIZER_PATH=$(CDPATH= cd -- "$(dirname -- "$TOKENIZER_PATH")" && pwd)/$(basename -- "$TOKENIZER_PATH")
    MODEL_ID=$MODEL_PATH
    VLLM_ENDPOINT_VALUE=http://127.0.0.1:8000
    FEATURES=$ENGINE
    if [ "$ENGINE" = candle ]; then
      STREAM_KIND=native
    else
      STREAM_KIND=emulated
    fi
    ;;
  vllm)
    MODEL_PATH=
    TOKENIZER_PATH=
    MODEL_ARCHITECTURE=
    VLLM_ENDPOINT_VALUE=${VLLM_ENDPOINT:-http://127.0.0.1:8000}
    MODEL_ID=$SUBJECT
    if [ -z "$MODEL_ID" ]; then
      MODEL_ID=$(vllm_get -fsS "${VLLM_ENDPOINT_VALUE%/}/v1/models" | jq -er '.data[0].id')
    fi
    FEATURES=
    STREAM_KIND=native
    ;;
esac

if [ -n "${HOLOGRAM_BIN:-}" ]; then
  BIN=$HOLOGRAM_BIN
else
  if [ -n "$FEATURES" ]; then
    cargo build --manifest-path "$ROOT/Cargo.toml" --locked --bin hologram --features "$FEATURES"
  else
    cargo build --manifest-path "$ROOT/Cargo.toml" --locked --bin hologram
  fi
  TARGET_DIR=$(cargo metadata --manifest-path "$ROOT/Cargo.toml" --no-deps --format-version 1 | jq -er '.target_directory')
  BIN="$TARGET_DIR/debug/hologram"
fi

if [ ! -x "$BIN" ]; then
  echo "error: hologram binary is not executable: $BIN" >&2
  exit 1
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/hologram-inference-smoke.XXXXXX")
HOME_DIR="$WORK/home"
CONFIG="$WORK/live.toml"
LOG="$WORK/hologram.log"
mkdir -p "$HOME_DIR"

cleanup() {
  if [ -n "${SERVER_PID:-}" ]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

ENGINE_JSON=$(json_string "$ENGINE")
MODEL_JSON=$(json_string "$MODEL_ID")
MODEL_PATH_JSON=$(json_string "$MODEL_PATH")
TOKENIZER_PATH_JSON=$(json_string "$TOKENIZER_PATH")
MODEL_ARCHITECTURE_JSON=$(json_string "$MODEL_ARCHITECTURE")
VLLM_ENDPOINT_JSON=$(json_string "$VLLM_ENDPOINT_VALUE")

cat >"$CONFIG" <<EOF
schema_version = 2

[server]
listen = "${LISTEN}"

[inference]
engine = ${ENGINE_JSON}
default_model = ${MODEL_JSON}
vllm_endpoint = ${VLLM_ENDPOINT_JSON}
vllm_token_env = "VLLM_API_KEY"
model_path = ${MODEL_PATH_JSON}
tokenizer_path = ${TOKENIZER_PATH_JSON}
model_architecture = ${MODEL_ARCHITECTURE_JSON}
n_ctx = ${N_CTX}
llamacpp_max_concurrent_requests = 1
request_timeout_secs = ${TIMEOUT}
EOF

HOME="$HOME_DIR" "$BIN" --config "$CONFIG" serve >"$LOG" 2>&1 &
SERVER_PID=$!

attempt=0
while ! curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    echo "error: hologram stopped before becoming ready" >&2
    tail -100 "$LOG" >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  if [ "$attempt" -ge "$TIMEOUT" ]; then
    echo "error: hologram did not become ready within ${TIMEOUT}s" >&2
    tail -100 "$LOG" >&2
    exit 1
  fi
  sleep 1
done

MODELS=$(curl -fsS "$BASE/v1/models")
printf '%s\n' "$MODELS" | jq -e --arg model "$MODEL_ID" '
  .object == "list" and any(.data[]; .id == $model)
' >/dev/null

OPENAI_REQUEST=$(jq -cn --arg model "$MODEL_ID" '{
  model: $model,
  messages: [{role: "user", content: "Reply with one short word."}],
  max_tokens: 8,
  temperature: 0
}')
OPENAI_HEADERS="$WORK/openai.headers"
OPENAI_RESPONSE=$(curl -fsS --max-time "$TIMEOUT" -D "$OPENAI_HEADERS" \
  -H 'content-type: application/json' --data-binary "$OPENAI_REQUEST" \
  "$BASE/v1/chat/completions")
grep -iq "^x-hologram-stream: ${STREAM_KIND}" "$OPENAI_HEADERS"
printf '%s\n' "$OPENAI_RESPONSE" | jq -e '
  .object == "chat.completion" and
  (.choices[0].message.content | type == "string") and
  .usage.prompt_tokens > 0 and
  .usage.completion_tokens <= 8
' >/dev/null

OPENAI_STREAM_REQUEST=$(printf '%s\n' "$OPENAI_REQUEST" | jq -c '. + {
  stream: true,
  stream_options: {include_usage: true}
}')
OPENAI_STREAM="$WORK/openai.sse"
curl -N -fsS --max-time "$TIMEOUT" -H 'content-type: application/json' \
  --data-binary "$OPENAI_STREAM_REQUEST" "$BASE/v1/chat/completions" >"$OPENAI_STREAM"
grep -q '"object":"chat.completion.chunk"' "$OPENAI_STREAM"
grep -Fq 'data: [DONE]' "$OPENAI_STREAM"

OLLAMA_REQUEST=$(jq -cn --arg model "$MODEL_ID" '{
  model: $model,
  prompt: "Reply with one short word.",
  stream: false,
  options: {num_predict: 8, temperature: 0}
}')
OLLAMA_RESPONSE=$(curl -fsS --max-time "$TIMEOUT" -H 'content-type: application/json' \
  --data-binary "$OLLAMA_REQUEST" "$BASE/api/generate")
printf '%s\n' "$OLLAMA_RESPONSE" | jq -e '
  .done == true and
  (.response | type == "string") and
  .prompt_eval_count > 0 and
  .eval_count <= 8
' >/dev/null

OLLAMA_STREAM_REQUEST=$(printf '%s\n' "$OLLAMA_REQUEST" | jq -c '.stream = true')
OLLAMA_STREAM="$WORK/ollama.ndjson"
curl -N -fsS --max-time "$TIMEOUT" -H 'content-type: application/json' \
  --data-binary "$OLLAMA_STREAM_REQUEST" "$BASE/api/generate" >"$OLLAMA_STREAM"
grep -q '"done":true' "$OLLAMA_STREAM"

SHOW_REQUEST=$(jq -cn --arg model "$MODEL_ID" '{model: $model}')
SHOW_RESPONSE=$(curl -fsS --max-time "$TIMEOUT" -H 'content-type: application/json' \
  --data-binary "$SHOW_REQUEST" "$BASE/api/show")
printf '%s\n' "$SHOW_RESPONSE" | jq -e --arg engine "$ENGINE" '
  .details.family == $engine
' >/dev/null

if [ "$ENGINE" = "llamacpp" ] || [ "$ENGINE" = "candle" ] || [ "$ENGINE" = "burn" ]; then
  OVERFLOW_REQUEST=$(printf '%s\n' "$OPENAI_REQUEST" | jq -c --argjson n_ctx "$N_CTX" '.max_tokens = $n_ctx')
  OVERFLOW_STATUS=$(curl -sS --max-time "$TIMEOUT" -o "$WORK/overflow.json" -w '%{http_code}' \
    -H 'content-type: application/json' --data-binary "$OVERFLOW_REQUEST" \
    "$BASE/v1/chat/completions")
  if [ "$OVERFLOW_STATUS" -lt 400 ]; then
    echo "error: llama.cpp context overflow unexpectedly returned HTTP $OVERFLOW_STATUS" >&2
    cat "$WORK/overflow.json" >&2
    exit 1
  fi
fi

CANCEL_REQUEST=$(printf '%s\n' "$OPENAI_REQUEST" | jq -c '.stream = true | .max_tokens = 256')
curl -N -sS --max-time 0.05 -H 'content-type: application/json' \
  --data-binary "$CANCEL_REQUEST" "$BASE/v1/chat/completions" >/dev/null 2>&1 || true
curl -fsS "$BASE/healthz" >/dev/null

echo "$ENGINE inference acceptance passed for $MODEL_ID"
