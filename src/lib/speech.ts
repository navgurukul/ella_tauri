import type { AudioPayload } from "../types";
import { llog, llogAbsolute } from "./latency";

interface SpeechRecognitionAlternativeLike {
  transcript: string;
}

interface SpeechRecognitionResultLike {
  isFinal: boolean;
  0: SpeechRecognitionAlternativeLike;
}

interface SpeechRecognitionEventLike extends Event {
  results: ArrayLike<SpeechRecognitionResultLike>;
}

interface SpeechRecognitionLike {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  onresult: ((event: SpeechRecognitionEventLike) => void) | null;
  onerror: (() => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
  abort(): void;
}

declare global {
  interface Window {
    SpeechRecognition?: new () => SpeechRecognitionLike;
    webkitSpeechRecognition?: new () => SpeechRecognitionLike;
    __TAURI_INTERNALS__?: unknown;
  }
}

export interface VoiceCaptureResult {
  samples: number[];
  /** Samples not yet emitted through the live streaming callback. */
  streamTailSamples: number[];
  sampleRate: number;
  transcript: string;
}

export interface VoiceCapture {
  supported: boolean;
  start(
    onTranscript: (text: string) => void,
    onLevel: (level: number) => void,
    onChunk?: (samples: number[], sampleRate: number) => void,
  ): Promise<void>;
  stop(): Promise<VoiceCaptureResult>;
  cancel(): Promise<void>;
}

export interface SpeechPlaybackCallbacks {
  onStart?: () => void;
  onEnd?: () => void;
  onError?: (reason: unknown) => void;
}

/** How often accumulated audio is downsampled and handed to onChunk. */
const CHUNK_PUSH_INTERVAL_MS = 1000;

export function createVoiceCapture(): VoiceCapture {
  let stream: MediaStream | null = null;
  let context: AudioContext | null = null;
  let processor: ScriptProcessorNode | null = null;
  let source: MediaStreamAudioSourceNode | null = null;
  let recognition: SpeechRecognitionLike | null = null;
  let transcript = "";
  let chunks: Float32Array[] = [];
  let streamChunks: Float32Array[] = [];
  let inputRate = 48_000;
  let pushTimer: number | null = null;

  const recognitionCtor = window.SpeechRecognition ?? window.webkitSpeechRecognition;

  // Downsample everything accumulated so far and hand it to onChunk. A few
  // input samples stay behind so each flush consumes a whole number of
  // output windows (keeps windows aligned across flushes).
  function flushChunks(onChunk: (samples: number[], sampleRate: number) => void) {
    const joined = joinFloat32(streamChunks);
    const ratio = inputRate / 16_000;
    const outputLength = Math.floor(joined.length / ratio);
    if (outputLength === 0) return;
    const consumed = Math.floor(outputLength * ratio);
    const samples = downsampleToPcm16(joined.subarray(0, consumed), inputRate, 16_000);
    streamChunks = joined.length > consumed ? [joined.subarray(consumed).slice()] : [];
    if (samples.length) onChunk(samples, 16_000);
  }

  async function cleanup() {
    if (pushTimer !== null) window.clearInterval(pushTimer);
    pushTimer = null;
    try {
      recognition?.stop();
    } catch {
      // Some engines throw when stop follows an abort or a failed start.
    }
    recognition = null;
    try {
      processor?.disconnect();
    } catch {
      // The audio graph may only have been partially connected.
    }
    try {
      source?.disconnect();
    } catch {
      // The audio graph may only have been partially connected.
    }
    stream?.getTracks().forEach((track) => track.stop());
    if (context && context.state !== "closed") {
      try {
        await context.close();
      } catch {
        // Tracks still need to be released even if the context rejects close.
      }
    }
    stream = null;
    context = null;
    processor = null;
    source = null;
  }

  return {
    supported: Boolean(navigator.mediaDevices?.getUserMedia),

    async start(onTranscript, onLevel, onChunk) {
      chunks = [];
      streamChunks = [];
      transcript = "";
      if (!navigator.mediaDevices?.getUserMedia) {
        throw new Error("Microphone access is not available on this device.");
      }
      const micStarted = performance.now();
      llogAbsolute("mic:start", "requesting getUserMedia");
      stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      llogAbsolute(
        "mic:granted",
        `getUserMedia took ${(performance.now() - micStarted).toFixed(1)}ms`,
      );
      context = new AudioContext();
      inputRate = context.sampleRate;
      source = context.createMediaStreamSource(stream);
      // ScriptProcessor is deliberately used for the small POC. Replacing it
      // with an AudioWorklet does not change the Rust/domain boundary.
      processor = context.createScriptProcessor(2048, 1, 1);
      processor.onaudioprocess = (event) => {
        const input = event.inputBuffer.getChannelData(0);
        const frame = new Float32Array(input);
        chunks.push(frame);
        if (onChunk) streamChunks.push(frame);
        let sum = 0;
        for (const sample of input) sum += sample * sample;
        onLevel(Math.min(1, Math.sqrt(sum / input.length) * 6));
      };
      source.connect(processor);
      processor.connect(context.destination);
      llogAbsolute(
        "mic:recording",
        `audio graph live after ${(performance.now() - micStarted).toFixed(1)}ms (input rate ${inputRate} Hz)`,
      );

      if (onChunk) {
        pushTimer = window.setInterval(() => flushChunks(onChunk), CHUNK_PUSH_INTERVAL_MS);
        llogAbsolute(
          "mic:streaming",
          `live STT streaming on: pushing audio to Rust every ${CHUNK_PUSH_INTERVAL_MS}ms`,
        );
      }

      if (recognitionCtor) {
        recognition = new recognitionCtor();
        recognition.continuous = true;
        recognition.interimResults = true;
        recognition.lang = "en-IN";
        recognition.onresult = (event) => {
          let all = "";
          for (let index = 0; index < event.results.length; index += 1) {
            all += `${event.results[index][0].transcript} `;
          }
          transcript = all.trim();
          onTranscript(transcript);
        };
        recognition.onerror = () => undefined;
        recognition.start();
      }
    },

    async stop() {
      const stopStarted = performance.now();
      const joined = joinFloat32(chunks);
      const samples = downsampleToPcm16(joined, inputRate, 16_000);
      const streamTailSamples = downsampleToPcm16(joinFloat32(streamChunks), inputRate, 16_000);
      llog(
        "capture:downsampled",
        `join+downsample ${inputRate}->16000 Hz took ${(performance.now() - stopStarted).toFixed(1)}ms ` +
          `(${joined.length} -> ${samples.length} samples, ~${(samples.length / 16).toFixed(0)}ms of audio)`,
      );
      const finalTranscript = transcript.trim();
      const cleanupStarted = performance.now();
      await cleanup();
      llog(
        "capture:cleanup",
        `mic/AudioContext teardown took ${(performance.now() - cleanupStarted).toFixed(1)}ms`,
      );
      return { samples, streamTailSamples, sampleRate: 16_000, transcript: finalTranscript };
    },

    async cancel() {
      chunks = [];
      streamChunks = [];
      transcript = "";
      try {
        recognition?.abort();
      } catch {
        // Abort can throw if recognition never reached the running state.
      }
      await cleanup();
    },
  };
}

function joinFloat32(chunks: Float32Array[]): Float32Array {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const joined = new Float32Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.length;
  }
  return joined;
}

export function downsampleToPcm16(
  input: Float32Array,
  inputRate: number,
  outputRate: number,
): number[] {
  if (!input.length || outputRate <= 0 || inputRate < outputRate) return [];
  const ratio = inputRate / outputRate;
  const outputLength = Math.floor(input.length / ratio);
  const output = new Array<number>(outputLength);
  for (let index = 0; index < outputLength; index += 1) {
    const start = Math.floor(index * ratio);
    const end = Math.max(start + 1, Math.floor((index + 1) * ratio));
    let sum = 0;
    for (let cursor = start; cursor < end && cursor < input.length; cursor += 1) {
      sum += input[cursor];
    }
    const value = Math.max(-1, Math.min(1, sum / (end - start)));
    output[index] = value < 0 ? Math.round(value * 32768) : Math.round(value * 32767);
  }
  return output;
}

export function speakText(
  text: string,
  audio?: AudioPayload | null,
  callbacks: SpeechPlaybackCallbacks = {},
): () => void {
  if (audio) {
    const playbackRequested = performance.now();
    llog("playback:decode-start", `creating Audio element from ${audio.base64.length} base64 chars`);
    const player = new Audio(`data:${audio.mime_type};base64,${audio.base64}`);
    let cancelled = false;
    let settled = false;

    const onPlaying = () => {
      if (cancelled) return;
      llog(
        "playback:playing",
        `audio is audible; element setup->playing took ${(performance.now() - playbackRequested).toFixed(1)}ms`,
      );
      callbacks.onStart?.();
    };
    const onEnded = () => {
      if (cancelled || settled) return;
      settled = true;
      cleanup();
      llog("playback:ended", "Ella finished speaking");
      callbacks.onEnd?.();
    };
    const onError = (reason: unknown) => {
      if (cancelled || settled) return;
      settled = true;
      cleanup();
      llog("playback:error", reason instanceof Error ? reason.message : "audio playback failed");
      callbacks.onError?.(reason);
    };
    const cleanup = () => {
      player.removeEventListener("playing", onPlaying);
      player.removeEventListener("ended", onEnded);
      player.removeEventListener("error", onError);
    };

    player.addEventListener("playing", onPlaying, { once: true });
    player.addEventListener("ended", onEnded, { once: true });
    player.addEventListener("error", onError, { once: true });
    try {
      const playback = player.play();
      if (playback) void playback.catch(onError);
    } catch (reason) {
      onError(reason);
    }
    return () => {
      cancelled = true;
      cleanup();
      player.pause();
    };
  }

  if (!("speechSynthesis" in window)) {
    queueMicrotask(() => callbacks.onError?.(new Error("Speech playback is unavailable")));
    return () => undefined;
  }
  llog("playback:system-tts", `no Piper audio; using browser speechSynthesis (${text.length} chars)`);
  let cancelled = false;
  let settled = false;
  let utterance: SpeechSynthesisUtterance;
  try {
    window.speechSynthesis.cancel();
    utterance = new SpeechSynthesisUtterance(text);
  } catch (reason) {
    queueMicrotask(() => {
      if (!cancelled) callbacks.onError?.(reason);
    });
    return () => {
      cancelled = true;
    };
  }
  utterance.lang = "en-IN";
  utterance.rate = 0.92;
  utterance.pitch = 1.05;
  utterance.onstart = () => {
    if (!cancelled) callbacks.onStart?.();
  };
  utterance.onend = () => {
    if (cancelled || settled) return;
    settled = true;
    callbacks.onEnd?.();
  };
  utterance.onerror = (event) => {
    if (cancelled || settled) return;
    settled = true;
    callbacks.onError?.(event);
  };
  try {
    window.speechSynthesis.speak(utterance);
  } catch (reason) {
    queueMicrotask(() => {
      if (!cancelled && !settled) {
        settled = true;
        callbacks.onError?.(reason);
      }
    });
  }
  return () => {
    cancelled = true;
    window.speechSynthesis.cancel();
  };
}
