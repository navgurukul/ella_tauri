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
  sampleRate: number;
  transcript: string;
}

export interface VoiceCapture {
  supported: boolean;
  start(onTranscript: (text: string) => void, onLevel: (level: number) => void): Promise<void>;
  stop(): Promise<VoiceCaptureResult>;
  cancel(): Promise<void>;
}

export function createVoiceCapture(): VoiceCapture {
  let stream: MediaStream | null = null;
  let context: AudioContext | null = null;
  let processor: ScriptProcessorNode | null = null;
  let source: MediaStreamAudioSourceNode | null = null;
  let recognition: SpeechRecognitionLike | null = null;
  let transcript = "";
  let chunks: Float32Array[] = [];
  let inputRate = 48_000;

  const recognitionCtor = window.SpeechRecognition ?? window.webkitSpeechRecognition;

  async function cleanup() {
    recognition?.stop();
    recognition = null;
    processor?.disconnect();
    source?.disconnect();
    stream?.getTracks().forEach((track) => track.stop());
    if (context && context.state !== "closed") await context.close();
    stream = null;
    context = null;
    processor = null;
    source = null;
  }

  return {
    supported: Boolean(navigator.mediaDevices?.getUserMedia),

    async start(onTranscript, onLevel) {
      chunks = [];
      transcript = "";
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
        chunks.push(new Float32Array(input));
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
      return { samples, sampleRate: 16_000, transcript: finalTranscript };
    },

    async cancel() {
      chunks = [];
      transcript = "";
      recognition?.abort();
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

export function speakText(text: string, audio?: AudioPayload | null): () => void {
  if (audio) {
    const playbackRequested = performance.now();
    llog("playback:decode-start", `creating Audio element from ${audio.base64.length} base64 chars`);
    const player = new Audio(`data:${audio.mime_type};base64,${audio.base64}`);
    player.addEventListener(
      "playing",
      () => {
        llog(
          "playback:playing",
          `audio is audible; element setup->playing took ${(performance.now() - playbackRequested).toFixed(1)}ms`,
        );
      },
      { once: true },
    );
    player.addEventListener(
      "ended",
      () => llog("playback:ended", `Zoe finished speaking`),
      { once: true },
    );
    void player.play();
    return () => player.pause();
  }

  if (!("speechSynthesis" in window)) return () => undefined;
  llog("playback:system-tts", `no Piper audio; using browser speechSynthesis (${text.length} chars)`);
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.lang = "en-IN";
  utterance.rate = 0.92;
  utterance.pitch = 1.05;
  window.speechSynthesis.speak(utterance);
  return () => window.speechSynthesis.cancel();
}
