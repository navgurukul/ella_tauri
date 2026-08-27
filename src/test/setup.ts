import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Vitest runs without `globals`, so Testing Library never registers its own
// auto-cleanup; without this every render stacks up in the same document.
afterEach(cleanup);

// The garden scales its fixed-coordinate stage to the window, which jsdom has
// no ResizeObserver for.
class MockResizeObserver implements ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
Object.defineProperty(globalThis, "ResizeObserver", {
  configurable: true,
  value: MockResizeObserver,
});

const values = new Map<string, string>();
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(),
  },
});

Object.defineProperty(window, "speechSynthesis", {
  configurable: true,
  value: {
    cancel: () => undefined,
    speak: () => undefined,
  },
});

class MockSpeechSynthesisUtterance {
  lang = "";
  rate = 1;
  pitch = 1;

  constructor(public text: string) {}
}

Object.defineProperty(globalThis, "SpeechSynthesisUtterance", {
  configurable: true,
  value: MockSpeechSynthesisUtterance,
});
