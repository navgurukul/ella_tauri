// Console latency instrumentation for the WebView side of the conversation
// flow. Every line is prefixed with [LATENCY] so the devtools console can be
// filtered to just the timing trail. Times are relative to the current turn.
import type { TurnTimings } from "../types";

let turnStartedAt = 0;
let turnLabel = "";

/** Mark the start of a user-visible turn (mic stop pressed / text submitted). */
export function markTurnStart(label: string): number {
  turnStartedAt = performance.now();
  turnLabel = label;
  console.log(
    `[LATENCY] ================= ${label} started (${new Date().toISOString()}) =================`,
  );
  return turnStartedAt;
}

/** Log one stage with the elapsed ms since the turn started. */
export function llog(stage: string, detail?: string): void {
  const elapsed = turnStartedAt ? performance.now() - turnStartedAt : 0;
  console.log(
    `[LATENCY] +${elapsed.toFixed(1).padStart(8)}ms  ${stage.padEnd(26)} ${detail ?? ""}`,
  );
}

/** Log a stage that is not tied to a turn (e.g. mic warm-up before speaking). */
export function llogAbsolute(stage: string, detail?: string): void {
  console.log(`[LATENCY] ${stage.padEnd(26)} ${detail ?? ""}`);
}

/** Pretty-print the Rust-side timing breakdown returned with a TurnResult. */
export function logServerTimings(timings?: TurnTimings | null): void {
  if (!timings) {
    llog("server:timings", "no timings returned (demo/browser mode)");
    return;
  }
  llog("server:timings", `interaction_id=${timings.interaction_id} (${timings.kind} turn)`);
  console.table({
    "audio input (ms)": timings.audio_input_ms ?? "-",
    "audio after VAD (ms)": timings.audio_after_vad_ms ?? "-",
    "VAD (ms)": timings.vad_ms ?? "-",
    "STT (ms)": timings.stt_ms ?? "-",
    "STT engine": timings.stt_engine ?? "-",
    "STT backend": timings.stt_backend ?? "-",
    "STT fallback from": timings.stt_fallback_from ?? "-",
    "STT mel (ms)": timings.stt_mel_ms ?? "-",
    "STT encode (ms)": timings.stt_encode_ms ?? "-",
    "STT decode (ms)": timings.stt_decode_ms ?? "-",
    "LLM ttft (ms)": timings.llm_ttft_ms ?? "-",
    "LLM completion (ms)": timings.llm_completion_ms ?? "-",
    "TTS first audio (ms)": timings.tts_first_audio_ms ?? "-",
    "TTS completion (ms)": timings.tts_completion_ms ?? "-",
    "Rust total (ms)": timings.total_ms,
  });
}

/** Elapsed ms since the turn started (for summary lines). */
export function turnElapsed(): number {
  return turnStartedAt ? performance.now() - turnStartedAt : 0;
}

export function currentTurnLabel(): string {
  return turnLabel;
}
