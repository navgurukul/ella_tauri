import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createBrowserBridge, type StorageLike } from "./bridge";
import type { EllaBridge } from "../types";

function memoryStorage(): StorageLike {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

/** One talk on a topic with these answers, ended by hand. */
async function talk(bridge: EllaBridge, answers: string[], topicId = "street-food") {
  const session = await bridge.startSession(topicId);
  for (const text of answers) await bridge.sendTextTurn(session.id, text);
  return bridge.completeSession(session.id);
}

describe("browser proof-of-concept bridge", () => {
  it("runs a complete learner journey and persists it", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    expect((await bridge.bootstrap()).learner).toBeNull();

    await bridge.saveLearner("Asha");
    const session = await bridge.startSession("street-food");
    expect(session.messages[0].speaker).toBe("ella");

    for (const text of [
      "I played football with my best friend",
      "We went to the field after class",
      "I felt happy because our team played well",
    ]) {
      await bridge.sendTextTurn(session.id, text);
    }

    const summary = await bridge.completeSession(session.id);
    expect(summary.turns).toBe(3);
    expect(summary.headline).not.toHaveLength(0);
    expect(summary.encouragement).toBe("That is 1 conversation finished. Come back and talk to Ella again.");

    const restored = await bridge.bootstrap();
    expect(restored.learner?.name).toBe("Asha");
    expect(restored.recent_sessions[0].status).toBe("complete");
    expect(restored.progress).toMatchObject({ talks_finished: 1, answers: 3, finished_topics: ["street-food"] });
    expect(restored.progress.days).toHaveLength(1);
    expect(restored.progress.days[0]).toMatchObject({ talks: 1, answers: 3 });
  });

  it("requires an actual transcript in demo voice mode", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await bridge.saveLearner("Riya");
    const session = await bridge.startSession("job-interview");

    await expect(
      bridge.sendVoiceTurn({
        sessionId: session.id,
        samples: [1, 2, 3],
        sampleRate: 16_000,
      }),
    ).rejects.toThrow(/speech recognition is unavailable/i);
  });

  it("starts a chore on its authored opening, once there is a learner", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await expect(bridge.startChore("deposit-refund")).rejects.toThrow(/tell ella your name/i);

    await bridge.saveLearner("Asha", 15);
    const session = await bridge.startChore("deposit-refund");
    expect(session.topic_label).toBe("Get a deposit refunded");
    expect(session.messages).toHaveLength(1);
    expect(session.messages[0].content).toMatch(/rather busy this morning/i);
    expect((await bridge.bootstrap()).recent_sessions[0]).toMatchObject({
      id: session.id,
      topic_id: "deposit-refund",
      status: "active",
    });

    await expect(bridge.startChore("haggle-with-a-dragon")).rejects.toThrow(/no chore called/i);
  });

  it("does not allow turns after a conversation ends", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await bridge.saveLearner("Neha");
    const session = await bridge.startSession("restaurant-order");
    await bridge.completeSession(session.id);
    await expect(bridge.sendTextTurn(session.id, "I also like rice")).rejects.toThrow(
      /already ended/i,
    );
  });
});

/** A stored message, for the older shapes a preview may have left behind. */
function storedMessage(id: string, speaker: "ella" | "learner", turn: number, created_at: string) {
  return { id, speaker, content: speaker === "ella" ? "Tell me more." : "I ate poha.", turn, created_at };
}

/** A finished stored talk with this many answers in it, on 2 September. */
function storedTalk(id: string, topic_id: string, answers: number, extra: Record<string, unknown> = {}) {
  const messages = [storedMessage(`${id}-ella-0`, "ella", 0, "2026-09-02T10:00:00.000Z")];
  for (let turn = 1; turn <= answers; turn += 1) {
    messages.push(
      storedMessage(`${id}-learner-${turn}`, "learner", turn, `2026-09-02T10:0${turn}:00.000Z`),
      storedMessage(`${id}-ella-${turn}`, "ella", turn, `2026-09-02T10:0${turn}:01.000Z`),
    );
  }
  return {
    id,
    topic_id,
    topic_label: topic_id,
    status: "complete",
    started_at: "2026-09-02T10:00:00.000Z",
    completed_at: "2026-09-02T10:09:00.000Z",
    messages,
    ...extra,
  };
}

/** What the several-learner development preview stored: Asha, then Ravi, who
 * was about more recently, each with a talk, and one talk from before either. */
function multiLearnerState(signedIn: number | null) {
  return {
    learners: [
      {
        id: 1,
        name: "Asha",
        age: 14,
        level_name: "Morning Meadow",
        created_at: "2026-09-01T10:00:00.000Z",
        avatar_color: "#FF7A00",
        last_active_at: "2026-09-10T10:00:00.000Z",
      },
      {
        id: 2,
        name: "Ravi",
        age: 20,
        level_name: "Morning Meadow",
        created_at: "2026-09-03T10:00:00.000Z",
        avatar_color: null,
        last_active_at: "2026-09-20T10:00:00.000Z",
      },
    ],
    signed_in: signedIn,
    sessions: [
      storedTalk("asha-talk", "market-bargaining", 2, { learner_id: 1 }),
      storedTalk("ravi-talk", "booking-a-cab", 1, { learner_id: 2 }),
      storedTalk("orphan-talk", "street-food", 3, { learner_id: null }),
    ],
  };
}

describe("the preview's learner", () => {
  // Only the clock is faked; nothing here waits on a timer.
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 8, 25, 10));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reads a v0.1.6 preview as it is, signed in with every talk", async () => {
    const storage = memoryStorage();
    storage.setItem(
      "ella-desktop-state",
      JSON.stringify({
        learner: { name: "Asha", age: 14, level_name: "Morning Meadow", created_at: "2026-09-01T10:00:00.000Z" },
        sessions: [storedTalk("old-talk", "market-bargaining", 2)],
        conversations: 1,
      }),
    );
    const bridge = createBrowserBridge(storage);

    const snapshot = await bridge.bootstrap();
    expect(snapshot.learner).toEqual({
      name: "Asha",
      age: 14,
      level_name: "Morning Meadow",
      created_at: "2026-09-01T10:00:00.000Z",
      avatar_color: null,
    });
    expect(snapshot.saved_learner).toEqual({ name: "Asha", avatar_color: null });
    expect(snapshot.recent_sessions.map((session) => session.id)).toEqual(["old-talk"]);
    expect(snapshot.progress).toMatchObject({
      talks_finished: 1,
      answers: 2,
      finished_topics: ["market-bargaining"],
    });

    // Logging out and in again works on it like on any other.
    await bridge.logOut();
    expect(await bridge.logIn()).toEqual(snapshot.learner);
    expect(await bridge.bootstrap()).toEqual(snapshot);
  });

  it("reads a preview that never had a learner as a laptop nobody has used", async () => {
    const storage = memoryStorage();
    storage.setItem("ella-desktop-state", JSON.stringify({ sessions: [], conversations: 0 }));
    const bridge = createBrowserBridge(storage);
    const snapshot = await bridge.bootstrap();
    expect(snapshot.learner).toBeNull();
    expect(snapshot.saved_learner).toBeNull();
    expect(snapshot.progress).toEqual({ days: [], talks_finished: 0, answers: 0, finished_topics: [] });
    await expect(bridge.logIn()).rejects.toThrow("Tell Ella your name first.");
  });

  it("converts the several-learner preview back to whoever was signed in, keeping every talk", async () => {
    const storage = memoryStorage();
    storage.setItem("ella-desktop-state", JSON.stringify(multiLearnerState(1)));
    const bridge = createBrowserBridge(storage);

    // Asha was signed in, so she stays, although Ravi was about more recently.
    const snapshot = await bridge.bootstrap();
    expect(snapshot.learner).toEqual({
      name: "Asha",
      age: 14,
      level_name: "Morning Meadow",
      created_at: "2026-09-01T10:00:00.000Z",
      avatar_color: "#FF7A00",
    });
    expect(snapshot.saved_learner).toEqual({ name: "Asha", avatar_color: "#FF7A00" });
    expect(snapshot.recent_sessions.map((session) => session.id)).toEqual([
      "orphan-talk",
      "ravi-talk",
      "asha-talk",
    ]);
    expect(snapshot.progress).toMatchObject({
      talks_finished: 3,
      answers: 6,
      finished_topics: ["booking-a-cab", "market-bargaining", "street-food"],
    });

    // Written back in the one-learner shape, so it is converted only once.
    const stored = JSON.parse(storage.getItem("ella-desktop-state") ?? "{}");
    expect(stored.learners).toBeUndefined();
    expect(stored.signed_in).toBeUndefined();
    expect(stored.learner).toMatchObject({ name: "Asha", signed_out: false });
    expect(stored.sessions).toHaveLength(3);
    expect(stored.sessions.some((session: Record<string, unknown>) => "learner_id" in session)).toBe(false);
    expect(await bridge.bootstrap()).toEqual(snapshot);
  });

  it("converts a signed-out several-learner preview to the learner about most recently, signed out", async () => {
    const storage = memoryStorage();
    storage.setItem("ella-desktop-state", JSON.stringify(multiLearnerState(null)));
    const bridge = createBrowserBridge(storage);

    const signedOut = await bridge.bootstrap();
    expect(signedOut.learner).toBeNull();
    expect(signedOut.saved_learner).toEqual({ name: "Ravi", avatar_color: null });
    expect(signedOut.recent_sessions).toEqual([]);

    expect(await bridge.logIn()).toMatchObject({ name: "Ravi", age: 20, created_at: "2026-09-03T10:00:00.000Z" });
    const back = await bridge.bootstrap();
    expect(back.recent_sessions).toHaveLength(3);
    expect(back.progress).toMatchObject({ talks_finished: 3, answers: 6 });
  });

  it("logs out without deleting anything, and logs back in to all of it", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    const aarav = await bridge.saveLearner("Aarav Kumar", 14);
    await bridge.saveAvatarColor("#FF7A00");
    await talk(bridge, ["I went to the market"]);

    const signedOut = await bridge.logOut();
    expect(signedOut.learner).toBeNull();
    expect(signedOut.recent_sessions).toEqual([]);
    expect(signedOut.progress).toEqual({ days: [], talks_finished: 0, answers: 0, finished_topics: [] });
    // Signed out, the laptop still knows who it keeps.
    expect(signedOut.saved_learner).toEqual({ name: "Aarav Kumar", avatar_color: "#FF7A00" });
    expect(await bridge.bootstrap()).toEqual(signedOut);
    await expect(bridge.startSession("street-food")).rejects.toThrow(/tell ella your name/i);
    await expect(bridge.saveAvatarColor("#68B506")).rejects.toThrow("Tell Ella your name first.");

    const back = await bridge.logIn();
    expect(back).toEqual({ ...aarav, avatar_color: "#FF7A00" });
    const restored = await bridge.bootstrap();
    expect(restored.learner).toEqual(back);
    expect(restored.recent_sessions).toHaveLength(1);
    expect(restored.progress).toMatchObject({ talks_finished: 1, answers: 1, finished_topics: ["street-food"] });
  });

  it("will not log in on a laptop nobody has been saved on", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await expect(bridge.logIn()).rejects.toThrow("Tell Ella your name first.");
    expect((await bridge.bootstrap()).learner).toBeNull();
  });

  it("treats saving after a log out as the same learner onboarding again", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    const asha = await bridge.saveLearner("Asha", 14);
    await bridge.saveAvatarColor("#68B506");
    await talk(bridge, ["I ate poha", "It was spicy"]);
    await bridge.logOut();

    // Signed in again, with the new name, the age kept and the history theirs.
    const again = await bridge.saveLearner("  Asha R ", null);
    expect(again).toEqual({ ...asha, name: "Asha R", avatar_color: "#68B506" });
    const snapshot = await bridge.bootstrap();
    expect(snapshot.learner).toEqual(again);
    expect(snapshot.saved_learner).toEqual({ name: "Asha R", avatar_color: "#68B506" });
    expect(snapshot.recent_sessions).toHaveLength(1);
    expect(snapshot.progress).toMatchObject({ talks_finished: 1, answers: 2 });
    // The next talk is counted on top of the old ones.
    const summary = await talk(bridge, ["I took a cab"], "booking-a-cab");
    expect(summary.encouragement).toBe("That is 2 conversations finished. Come back and talk to Ella again.");
  });

  it("checks a name and an age as the backend does", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await expect(bridge.saveLearner(" A ")).rejects.toThrow("Please enter at least two letters.");
    await expect(bridge.saveLearner("a".repeat(41))).rejects.toThrow("Please use a name shorter than 40 letters.");
    await expect(bridge.saveLearner("Asha", 2)).rejects.toThrow("Please enter an age between 3 and 120.");
    await expect(bridge.saveLearner("Asha", 121)).rejects.toThrow("Please enter an age between 3 and 120.");
    expect((await bridge.bootstrap()).saved_learner).toBeNull();
    expect(await bridge.saveLearner(" Asha \n", 3)).toMatchObject({ name: "Asha", age: 3 });
  });

  it("does not count a talk in which nothing was said", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await bridge.saveLearner("Kabir", 14);

    const silent = await talk(bridge, [], "market-bargaining");
    expect(silent.turns).toBe(0);
    expect(silent.encouragement).toBe(
      "Say a few words next time and it counts. Come back and talk to Ella again.",
    );
    let snapshot = await bridge.bootstrap();
    expect(snapshot.recent_sessions[0].status).toBe("complete");
    expect(snapshot.progress).toEqual({ days: [], talks_finished: 0, answers: 0, finished_topics: [] });

    // An answer in a talk left open counts as an answer and a talk day, but
    // not as a finished talk.
    const open = await bridge.startSession("street-food");
    await bridge.sendTextTurn(open.id, "I ate poha");
    snapshot = await bridge.bootstrap();
    expect(snapshot.progress).toMatchObject({ talks_finished: 0, answers: 1, finished_topics: [] });
    expect(snapshot.progress.days).toHaveLength(1);

    const chore = await bridge.startChore("market-cloth-price");
    await bridge.sendTextTurn(chore.id, "I want this shirt");
    const spoken = await bridge.completeSession(chore.id);
    expect(spoken.encouragement).toBe("That is 1 conversation finished. Come back and talk to Ella again.");
    expect((await bridge.bootstrap()).progress.finished_topics).toEqual(["market-cloth-price"]);
  });

  it("puts a talk that runs past midnight on the day of its first answer", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    vi.setSystemTime(new Date(2026, 8, 24, 23, 50));
    await bridge.saveLearner("Riya", 14);
    const session = await bridge.startSession("street-food");
    await bridge.sendTextTurn(session.id, "I ate vada pav");
    vi.setSystemTime(new Date(2026, 8, 25, 0, 10));
    await bridge.sendTextTurn(session.id, "It was late");
    await bridge.sendTextTurn(session.id, "Then I slept");
    await bridge.completeSession(session.id);

    expect((await bridge.bootstrap()).progress.days).toEqual([
      { day: "2026-09-25", talks: 0, answers: 2 },
      { day: "2026-09-24", talks: 1, answers: 1 },
    ]);
  });

  it("saves the avatar colour on the signed-in learner, and only a real colour", async () => {
    const bridge = createBrowserBridge(memoryStorage());
    await expect(bridge.saveAvatarColor("#FF7A00")).rejects.toThrow("Tell Ella your name first.");

    await bridge.saveLearner("Asha", 14);
    for (const refused of ["orange", "#FF7A0", "#FF7A00 ", "FF7A00", "#GG7A00"]) {
      await expect(bridge.saveAvatarColor(refused)).rejects.toThrow(
        "Please choose one of the colours on the profile screen.",
      );
    }
    expect((await bridge.bootstrap()).learner?.avatar_color).toBeNull();
    expect(await bridge.saveAvatarColor("#ff7a00")).toMatchObject({ name: "Asha", avatar_color: "#ff7a00" });

    // It is the learner's, so it waits for them through a log out.
    expect((await bridge.logOut()).saved_learner).toEqual({ name: "Asha", avatar_color: "#ff7a00" });
    expect((await bridge.logIn()).avatar_color).toBe("#ff7a00");
  });
});
