import { describe, expect, it, vi } from "vitest";
import { createSpeechQueue, downsampleToPcm16, speakText } from "./speech";

describe("audio conversion", () => {
  it("downsamples float microphone frames to bounded PCM16", () => {
    const input = new Float32Array(48_000);
    for (let index = 0; index < input.length; index += 1) {
      input[index] = Math.sin((index / 48_000) * Math.PI * 2 * 440);
    }
    const output = downsampleToPcm16(input, 48_000, 16_000);
    expect(output).toHaveLength(16_000);
    expect(Math.max(...output)).toBeLessThanOrEqual(32_767);
    expect(Math.min(...output)).toBeGreaterThanOrEqual(-32_768);
  });

  it("rejects an unsupported upsample request", () => {
    expect(downsampleToPcm16(new Float32Array([0.5]), 8_000, 16_000)).toEqual([]);
  });

  it("tracks browser speech from its real playback lifecycle", () => {
    const original = window.speechSynthesis;
    const onStart = vi.fn();
    const onEnd = vi.fn();
    const cancel = vi.fn();
    Object.defineProperty(window, "speechSynthesis", {
      configurable: true,
      value: {
        cancel,
        speak: (utterance: SpeechSynthesisUtterance) => {
          utterance.onstart?.({} as SpeechSynthesisEvent);
          utterance.onend?.({} as SpeechSynthesisEvent);
        },
      },
    });

    try {
      const stop = speakText("Hello", null, { onStart, onEnd });
      expect(onStart).toHaveBeenCalledOnce();
      expect(onEnd).toHaveBeenCalledOnce();
      stop();
      expect(cancel).toHaveBeenCalledTimes(2);
    } finally {
      Object.defineProperty(window, "speechSynthesis", { configurable: true, value: original });
    }
  });
});

/** Minimal AudioContext good enough to observe scheduling order and timing. */
class FakeAudioContext {
  static started: { at: number; duration: number }[] = [];
  state = "running";
  currentTime = 0;
  destination = {};
  private ended: (() => void)[] = [];

  async resume() {
    this.state = "running";
  }
  async close() {
    this.state = "closed";
  }
  /** One second of audio per byte of payload, so durations are predictable. */
  async decodeAudioData(buffer: ArrayBuffer) {
    return { duration: buffer.byteLength } as AudioBuffer;
  }
  createBufferSource() {
    const context = this;
    return {
      buffer: null as AudioBuffer | null,
      onended: null as (() => void) | null,
      connect() {},
      stop() {},
      start(at: number) {
        FakeAudioContext.started.push({ at, duration: this.buffer?.duration ?? 0 });
        context.ended.push(() => this.onended?.());
      },
    };
  }
  /** Pretend every scheduled sentence finished playing. */
  drain() {
    const pending = this.ended.splice(0);
    for (const end of pending) end();
  }
}

describe("streamed sentence playback", () => {
  const audio = (bytes: number) => ({
    mime_type: "audio/wav",
    base64: btoa("x".repeat(bytes)),
  });
  let context: FakeAudioContext;

  const install = () => {
    FakeAudioContext.started = [];
    Object.defineProperty(window, "AudioContext", {
      configurable: true,
      value: class extends FakeAudioContext {
        constructor() {
          super();
          context = this;
        }
      },
    });
  };
  /** Let the queue's decode chain settle. */
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

  it("plays sentences back to back with no gap between them", async () => {
    install();
    const queue = createSpeechQueue();
    queue.push(audio(2));
    queue.push(audio(3));
    await settle();

    const [first, second] = FakeAudioContext.started;
    expect(FakeAudioContext.started).toHaveLength(2);
    // The second sentence starts exactly where the first one ends.
    expect(second.at).toBeCloseTo(first.at + first.duration, 5);
  });

  /** Decoding is async, so a short sentence must not overtake a long one. */
  it("keeps sentences in the order they were queued", async () => {
    install();
    const queue = createSpeechQueue();
    queue.push(audio(9));
    queue.push(audio(1));
    await settle();

    expect(FakeAudioContext.started.map((source) => source.duration)).toEqual([9, 1]);
  });

  it("reports the turn finished only once the last sentence has played", async () => {
    install();
    const onStart = vi.fn();
    const onEnd = vi.fn();
    const queue = createSpeechQueue({ onStart, onEnd });
    queue.push(audio(1));
    queue.push(audio(1));
    await settle();

    expect(onStart).toHaveBeenCalledOnce();
    queue.finish(2);
    await settle();
    expect(onEnd).not.toHaveBeenCalled();

    context.drain();
    await settle();
    expect(onEnd).toHaveBeenCalledOnce();
  });

  /** A learner who taps the mic mid-reply must not be spoken over. */
  it("drops everything still queued when cancelled", async () => {
    install();
    const onEnd = vi.fn();
    const queue = createSpeechQueue({ onEnd });
    queue.push(audio(1));
    await settle();
    queue.cancel();
    queue.push(audio(1));
    await settle();

    expect(FakeAudioContext.started).toHaveLength(1);
    expect(onEnd).not.toHaveBeenCalled();
  });
});
