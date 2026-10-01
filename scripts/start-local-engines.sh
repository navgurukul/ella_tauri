#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# engines live at <repo>/engines (symlink or staged tree)
ENGINE_ROOT="${ELLA_ENGINE_ROOT:-$PROJECT_DIR/engines}"
LLM_PORT="${ELLA_LLM_PORT:-39091}"
STT_PORT="${ELLA_STT_PORT:-39092}"

LLAMA_BIN="$ENGINE_ROOT/bin/llama/llama-server"
WHISPER_BIN="$ENGINE_ROOT/bin/whisper/whisper-server"
LLM_MODEL="$ENGINE_ROOT/models/llm/model.gguf"
STT_MODEL="$ENGINE_ROOT/models/stt/ggml-small.bin"
CANARY_MODEL="${ELLA_CANARY_MODEL:-$ENGINE_ROOT/models/stt/canary-180m-flash-Q8_0.gguf}"
# Where llama-server keeps talks' evaluated instructions, as the installed app
# does beside its model. Ella reads them too when told with ELLA_LLM_SLOT_DIR.
SLOT_DIR="${ELLA_LLM_SLOT_DIR:-$ENGINE_ROOT/models/llm-slots}"

for required in "$LLAMA_BIN" "$WHISPER_BIN" "$LLM_MODEL" "$STT_MODEL" "$CANARY_MODEL"; do
  if [[ ! -e "$required" ]]; then
    echo "Missing local engine asset: $required" >&2
    echo "Install STT models from $PROJECT_DIR with: npm run models:install" >&2
    exit 1
  fi
done

if [[ "$(head -c 4 "$CANARY_MODEL")" != "GGUF" ]]; then
  echo "Canary model is not a valid GGUF: $CANARY_MODEL" >&2
  echo "Repair it from $PROJECT_DIR with: npm run models:install" >&2
  exit 1
fi

mkdir -p "$SLOT_DIR"

export DYLD_LIBRARY_PATH="$ENGINE_ROOT/bin/llama:$ENGINE_ROOT/bin/whisper:${DYLD_LIBRARY_PATH:-}"

cleanup() {
  kill "${LLAMA_PID:-}" "${WHISPER_PID:-}" 2>/dev/null || true
  wait "${LLAMA_PID:-}" "${WHISPER_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

"$LLAMA_BIN" \
  --model "$LLM_MODEL" \
  --host 127.0.0.1 \
  --port "$LLM_PORT" \
  --ctx-size 4096 \
  --threads "${ELLA_LLAMA_THREADS:-4}" \
  --cors-origins "tauri://localhost,http://tauri.localhost" \
  --slot-save-path "$SLOT_DIR" \
  --parallel 1 &
LLAMA_PID=$!

WHISPER_ARGS=(
  --model "$STT_MODEL"
  --host 127.0.0.1
  --port "$STT_PORT"
  --threads "${ELLA_WHISPER_THREADS:-3}"
  --language en
)
if [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "x86_64" ]]; then
  WHISPER_ARGS+=(--no-gpu)
fi
"$WHISPER_BIN" "${WHISPER_ARGS[@]}" &
WHISPER_PID=$!

echo "Canary native STT will load from $CANARY_MODEL (Whisper remains on $STT_PORT as fallback)."
echo "Local Ella sidecars are starting on 127.0.0.1:$LLM_PORT and 127.0.0.1:$STT_PORT"
echo "In another terminal run: ELLA_ENGINE_MODE=local ELLA_LLM_BASE_URL=http://127.0.0.1:$LLM_PORT/v1 ELLA_LLM_SLOT_DIR=\"$SLOT_DIR\" npm run desktop:dev"
wait
