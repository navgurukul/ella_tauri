#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

"$SCRIPT_DIR/start-local-engines.sh" &
ENGINE_LAUNCHER_PID=$!

cleanup() {
  kill "$ENGINE_LAUNCHER_PID" 2>/dev/null || true
  wait "$ENGINE_LAUNCHER_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for attempt in {1..90}; do
  if curl --fail --silent http://127.0.0.1:39091/health >/dev/null \
    && curl --fail --silent http://127.0.0.1:39092/health >/dev/null; then
    break
  fi
  if ! kill -0 "$ENGINE_LAUNCHER_PID" 2>/dev/null; then
    echo "A local engine stopped before becoming ready." >&2
    exit 1
  fi
  if [[ "$attempt" == "90" ]]; then
    echo "Local engines did not become ready within 90 seconds." >&2
    exit 1
  fi
  sleep 1
done

cd "$PROJECT_DIR"
# The launcher above owns llama-server. Naming it here stops the app starting
# a second one against the same model, which it otherwise does when installed.
ELLA_ENGINE_MODE=local ELLA_LLM_BASE_URL=http://127.0.0.1:39091/v1 npm run desktop:dev
