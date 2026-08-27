import type { AudioPayload, WordSpan } from "../types";
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

export interface SpeechQueueCallbacks extends SpeechPlaybackCallbacks {
  /** Called with the words that now have timings, in speaking order. The
   * screen draws the reply from its own text; this only says that there is a
   * schedule to follow, so the spoken word can be marked. */
  onWords?: (words: string[]) => void;
  /** Index into those words of the one being spoken now, or -1 for none. */
  onSpokenWord?: (index: number) => void;
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

/** How far ahead of `currentTime` a segment is scheduled. Enough that the
 * decode-to-schedule hop never lands in the past, short enough to be inaudible. */
const SCHEDULE_LEAD_SECONDS = 0.03;
/** How long to wait for segments the turn said were sent but that have not
 * arrived. Beyond this the queue finishes with what it has rather than hang. */
const MISSING_SEGMENT_GRACE_MS = 1500;

export interface SpeechQueue {
  /** Queue one sentence, with the timings that say when each of its words is
   * spoken. Segments play in push order regardless of decode order. */
  push(audio: AudioPayload, words?: WordSpan[]): void;
  /** How many segments have been queued so far. */
  readonly received: number;
  /** Every word queued so far, in speaking order. */
  readonly words: string[];
  /** No more segments are coming beyond `expected` in total. */
  finish(expected: number): void;
  /** Stop immediately and drop anything still queued. */
  cancel(): void;
}

/**
 * Plays a reply sentence by sentence as it is synthesized.
 *
 * Sentences are scheduled back-to-back on the AudioContext clock rather than
 * played through an <audio> element each: swapping an element's `src` leaves an
 * audible gap between sentences, which reads as Ella hesitating mid-thought.
 */
export function createSpeechQueue(callbacks: SpeechQueueCallbacks = {}): SpeechQueue {
  const openedAt = performance.now();
  let context: AudioContext | null = null;
  let cursor = 0;
  let received = 0;
  let played = 0;
  let expected: number | null = null;
  let started = false;
  let settled = false;
  let graceTimer: number | null = null;
  // Decoding is async, so without a serial chain a short sentence could be
  // scheduled ahead of the longer one before it.
  let chain: Promise<void> = Promise.resolve();
  const sources = new Set<AudioBufferSourceNode>();
  // Absolute AudioContext times for every word queued so far, so the highlight
  // reads one clock rather than tracking each sentence separately.
  const schedule: { start: number; end: number }[] = [];
  const words: string[] = [];
  let spoken = -1;
  let frame: number | null = null;

  /** Follow the audio clock and report the word being spoken as it changes. */
  const track = () => {
    frame = null;
    if (settled || !context) return;
    const now = context.currentTime;
    let index = -1;
    for (let cursor = 0; cursor < schedule.length; cursor += 1) {
      if (now >= schedule[cursor].start && now < schedule[cursor].end) {
        index = cursor;
        break;
      }
      // Past this word but before the next: hold the last one rather than
      // flickering to nothing between spans.
      if (now >= schedule[cursor].end) index = cursor;
    }
    if (index !== spoken) {
      spoken = index;
      callbacks.onSpokenWord?.(index);
    }
    if (started) frame = requestAnimationFrame(track);
  };

  const settle = (fail?: unknown) => {
    if (settled) return;
    settled = true;
    if (graceTimer !== null) window.clearTimeout(graceTimer);
    graceTimer = null;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    if (spoken !== -1) {
      spoken = -1;
      callbacks.onSpokenWord?.(-1);
    }
    if (fail !== undefined) {
      llog("stream-playback:error", fail instanceof Error ? fail.message : "playback failed");
      callbacks.onError?.(fail);
      return;
    }
    llog("stream-playback:ended", `${played} sentence(s) played`);
    callbacks.onEnd?.();
  };

  const maybeSettle = () => {
    if (expected !== null && played >= Math.min(expected, received)) settle();
  };

  const push = (audio: AudioPayload, wordSpans: WordSpan[] = []) => {
    if (settled) return;
    const index = received;
    received += 1;
    if (wordSpans.length > 0) {
      words.push(...wordSpans.map((span) => span.text));
      callbacks.onWords?.([...words]);
    }
    if (graceTimer !== null) {
      window.clearTimeout(graceTimer);
      graceTimer = null;
    }
    chain = chain
      .then(async () => {
        if (settled) return;
        if (!context) {
          context = new AudioContext();
          cursor = context.currentTime;
        }
        if (context.state === "suspended") await context.resume();
        const buffer = await context.decodeAudioData(base64ToArrayBuffer(audio.base64));
        if (settled || !context) return;
        const source = context.createBufferSource();
        source.buffer = buffer;
        source.connect(context.destination);
        const at = Math.max(context.currentTime + SCHEDULE_LEAD_SECONDS, cursor);
        source.onended = () => {
          sources.delete(source);
          played += 1;
          maybeSettle();
        };
        sources.add(source);
        source.start(at);
        // Word times are relative to this sentence's own clip; the schedule is
        // in AudioContext time, so each is offset by where the clip starts.
        for (const span of wordSpans) {
          schedule.push({ start: at + span.start_ms / 1000, end: at + span.end_ms / 1000 });
        }
        cursor = at + buffer.duration;
        if (!started) {
          started = true;
          if (frame === null) frame = requestAnimationFrame(track);
          llog(
            "stream-playback:first-sentence",
            `first sentence audible ${(performance.now() - openedAt).toFixed(1)}ms after the turn opened`,
          );
          callbacks.onStart?.();
        }
      })
      .catch((reason: unknown) => {
        // One unreadable sentence should not silence the rest of the reply.
        llog("stream-playback:segment-failed", `sentence ${index} could not be played`);
        // The highlight reads `schedule` by the same index it reads the word
        // list by, so a sentence that never scheduled still has to occupy its
        // slots — otherwise every later word highlights the wrong one.
        for (const _ of wordSpans) {
          schedule.push({ start: cursor, end: cursor });
        }
        played += 1;
        maybeSettle();
        if (!started && expected !== null) settle(reason);
      });
  };

  return {
    push,
    get received() {
      return received;
    },
    get words() {
      return [...words];
    },
    finish(total: number) {
      expected = total;
      if (received >= total) {
        // Everything is queued; settle once the last one finishes playing.
        void chain.then(maybeSettle);
        return;
      }
      // The turn says more segments were sent than have arrived. Give the
      // window a moment to deliver them, then finish with what is here.
      graceTimer = window.setTimeout(() => {
        graceTimer = null;
        expected = received;
        void chain.then(maybeSettle);
      }, MISSING_SEGMENT_GRACE_MS);
    },
    cancel() {
      if (settled) return;
      settled = true;
      if (graceTimer !== null) window.clearTimeout(graceTimer);
      graceTimer = null;
      if (frame !== null) cancelAnimationFrame(frame);
      frame = null;
      for (const source of sources) {
        try {
          source.stop();
        } catch {
          // Already finished; nothing to stop.
        }
      }
      sources.clear();
      void context?.close().catch(() => undefined);
      context = null;
    },
  };
}

function base64ToArrayBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
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
