import { describe, expect, it, vi } from "vitest";
import { createSpeechQueue, downsampleToPcm16, sentenceGroups, speakText } from "./speech";

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
  /** Let the highlight's animation-frame loop run once. */
  const frame = () =>
    new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));

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

  /** The reply has to appear as it is spoken, not when the turn returns — the
   * whole point of streaming is that the turn returns seconds later. */
  it("reports each sentence's words as it is queued", async () => {
    install();
    const seen: string[][][] = [];
    const queue = createSpeechQueue({ onSentences: (sentences) => seen.push(sentences) });
    queue.push(audio(2), [
      { text: "Oh,", start_ms: 0, end_ms: 300 },
      { text: "yummy!", start_ms: 300, end_ms: 900 },
    ]);
    expect(seen.at(-1)).toEqual([["Oh,", "yummy!"]]);
    queue.push(audio(2), [{ text: "Really?", start_ms: 0, end_ms: 500 }]);
    // Grouped by sentence, so a new one cannot re-wrap the one on screen.
    expect(seen.at(-1)).toEqual([["Oh,", "yummy!"], ["Really?"]]);
    expect(queue.words).toEqual(["Oh,", "yummy!", "Really?"]);
    await settle();
  });

  it("highlights the word being spoken, and only that word", async () => {
    install();
    const spoken: number[] = [];
    const queue = createSpeechQueue({ onSpokenWord: (index) => spoken.push(index) });
    // 2s of audio: word 0 from 0.1-0.8s, word 1 from 0.8-1.9s.
    queue.push(audio(2), [
      { text: "Hello", start_ms: 100, end_ms: 800 },
      { text: "there.", start_ms: 800, end_ms: 1900 },
    ]);
    await settle();
    const base = FakeAudioContext.started[0].at;

    // Inside the leading silence: nothing is being said yet.
    context.currentTime = base + 0.05;
    await frame();
    expect(spoken.at(-1) ?? -1).toBe(-1);

    context.currentTime = base + 0.4;
    await frame();
    expect(spoken.at(-1)).toBe(0);

    context.currentTime = base + 1.2;
    await frame();
    expect(spoken.at(-1)).toBe(1);
  });

  /** Between the last word and the end of the clip there is trailing silence.
   * Dropping the highlight there reads as a flicker, so it holds. */
  it("holds the last word through the trailing silence", async () => {
    install();
    const spoken: number[] = [];
    const queue = createSpeechQueue({ onSpokenWord: (index) => spoken.push(index) });
    queue.push(audio(2), [{ text: "Hello", start_ms: 100, end_ms: 900 }]);
    await settle();
    context.currentTime = FakeAudioContext.started[0].at + 1.5;
    await frame();
    expect(spoken.at(-1)).toBe(0);
  });

  /** The highlight indexes the word list and the schedule by the same number,
   * so a sentence that fails to decode must still take up its slots. */
  it("keeps the highlight aligned when a sentence cannot be decoded", async () => {
    install();
    const spoken: number[] = [];
    const queue = createSpeechQueue({ onSpokenWord: (index) => spoken.push(index) });
    // First sentence is undecodable; its two words must still hold slots 0-1.
    queue.push({ mime_type: "audio/wav", base64: "!!!!not base64!!!!" }, [
      { text: "Broken", start_ms: 0, end_ms: 200 },
      { text: "sentence.", start_ms: 200, end_ms: 500 },
    ]);
    queue.push(audio(2), [
      { text: "Good", start_ms: 100, end_ms: 600 },
      { text: "sentence.", start_ms: 600, end_ms: 1800 },
    ]);
    await settle();

    expect(queue.words).toEqual(["Broken", "sentence.", "Good", "sentence."]);
    const base = FakeAudioContext.started[0].at;
    context.currentTime = base + 0.3;
    await frame();
    // Slot 2 is "Good" — the first word of the sentence that did play.
    expect(spoken.at(-1)).toBe(2);
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

/** These cases mirror `speech_stream_tests` in
 * src-tauri/src/infrastructure/engines.rs. When the two splitters disagree the
 * reply restacks the moment its audio catches up, so they are kept in step. */
describe("sentence grouping for the subtitles", () => {
  const grouped = (text: string) => sentenceGroups(text).map((words) => words.join(" "));

  it("splits a reply the way Piper cuts it", () => {
    expect(grouped("That sounds delicious! What did it taste like?")).toEqual([
      "That sounds delicious!",
      "What did it taste like?",
    ]);
    expect(grouped("Hi Souvik! Tell me about the tastiest thing you ate. Where did you find it?"))
      .toEqual([
        "Hi Souvik!",
        "Tell me about the tastiest thing you ate.",
        "Where did you find it?",
      ]);
  });

  it("keeps an abbreviation and a decimal inside their sentence", () => {
    expect(grouped("My price is Rs. 250 for this cloth.")).toEqual([
      "My price is Rs. 250 for this cloth.",
    ]);
    expect(grouped("The cloth is 2.5 metres wide and very soft.")).toEqual([
      "The cloth is 2.5 metres wide and very soft.",
    ]);
  });

  it("still ends a sentence that finishes on a figure", () => {
    expect(grouped("My final price is 250. Do we have a deal?")).toEqual([
      "My final price is 250.",
      "Do we have a deal?",
    ]);
  });

  it("merges a fragment too short to be its own line", () => {
    expect(grouped("Ok. I understand what you mean now.")).toEqual([
      "Ok. I understand what you mean now.",
    ]);
  });

  it("treats an ellipsis as one sentence", () => {
    expect(grouped("Well... that is a very low offer.")).toEqual([
      "Well... that is a very low offer.",
    ]);
  });

  it("loses no words, whatever the shape", () => {
    for (const text of [
      "Sounds nice. What did you like about it?",
      "Rs 250 is my last price. Take it or leave it, my friend!",
      "One sentence with no ending punctuation",
      "",
    ]) {
      expect(sentenceGroups(text).flat().join(" ")).toBe(text.trim().replace(/\s+/g, " "));
    }
  });
});
