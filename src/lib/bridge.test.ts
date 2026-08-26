import { describe, expect, it } from "vitest";
import { createBrowserBridge, type StorageLike } from "./bridge";

function memoryStorage(): StorageLike {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

describe("browser proof-of-concept bridge", () => {
  it("runs a complete learner journey and persists garden progress", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    expect((await bridge.bootstrap()).learner).toBeUndefined();

    await bridge.saveLearner("Asha");
    const session = await bridge.startSession("school-life");
    expect(session.messages[0].speaker).toBe("zoe");

    for (const text of [
      "I played football with my best friend",
      "We went to the field after class",
      "I felt happy because our team played well",
    ]) {
      await bridge.sendTextTurn(session.id, text);
    }

    const summary = await bridge.completeSession(session.id);
    expect(summary.turns).toBe(3);
    expect(summary.garden.total_conversations).toBe(1);
    expect(summary.garden.skills.every((skill) => skill.stage === 1)).toBe(true);

    const restored = await bridge.bootstrap();
    expect(restored.learner?.name).toBe("Asha");
    expect(restored.recent_sessions[0].status).toBe("complete");
  });

  it("requires an actual transcript in demo voice mode", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await bridge.saveLearner("Riya");
    const session = await bridge.startSession("my-dreams");

    await expect(
      bridge.sendVoiceTurn({
        sessionId: session.id,
        samples: [1, 2, 3],
        sampleRate: 16_000,
      }),
    ).rejects.toThrow(/speech recognition is unavailable/i);
  });

  it("does not allow turns after a conversation ends", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await bridge.saveLearner("Neha");
    const session = await bridge.startSession("food-i-love");
    await bridge.completeSession(session.id);
    await expect(bridge.sendTextTurn(session.id, "I also like rice")).rejects.toThrow(
      /already ended/i,
    );
  });
});
