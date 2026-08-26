import "@testing-library/jest-dom/vitest";

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
