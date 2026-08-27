import { describe, expect, it, vi } from "vitest";
import { downsampleToPcm16, speakText } from "./speech";

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
