import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const app = vi.hoisted(() => ({ version: vi.fn(async () => "0.1.21") }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: app.version }));

import notesByVersion from "../../shared/whats-new.json";
import { markWhatsNewSeen, whatsNewAfterUpdate, whatsNewFrom, whatsNewIn } from "./whatsNew";

function memory(seen?: string) {
  const kept = new Map<string, string>(seen ? [["ella-whats-new-seen", seen]] : []);
  return {
    getItem: (key: string) => kept.get(key) ?? null,
    setItem: (key: string, value: string) => void kept.set(key, value),
    seen: () => kept.get("ella-whats-new-seen"),
  };
}

describe("what a version brought", () => {
  it("reads the bullets under a release's What's new heading, and nothing else", () => {
    const body = [
      "## What's new in Ella 0.1.21",
      "",
      "- Ella starts answering a little sooner.",
      "- Long answers are not lost any more.",
      "",
      "## Installing",
      "- Windows: choose **More info** then **Run anyway**.",
    ].join("\r\n");
    expect(whatsNewFrom(body)).toEqual([
      "Ella starts answering a little sooner.",
      "Long answers are not lost any more.",
    ]);
    expect(whatsNewFrom("Windows installer (`-setup.exe`) and Mac apps.")).toEqual([]);
    expect(whatsNewFrom(undefined)).toEqual([]);
  });

  it("has words for the learner for every version it names, short enough for a card", () => {
    for (const [version, notes] of Object.entries(notesByVersion as Record<string, string[]>)) {
      expect(notes.length, version).toBeGreaterThan(0);
      for (const note of notes) expect(note.length, note).toBeLessThan(140);
    }
    expect(whatsNewIn("0.1.21").length).toBeGreaterThan(0);
    expect(whatsNewIn("0.0.1")).toEqual([]);
  });
});

describe("the card after an update", () => {
  beforeEach(() => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    app.version.mockResolvedValue("0.1.21");
  });
  afterEach(() => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("shows a returning learner what the new version brought, once", async () => {
    const storage = memory("0.1.20");
    const shown = await whatsNewAfterUpdate(true, storage);
    expect(shown).toEqual({ version: "0.1.21", notes: whatsNewIn("0.1.21") });
    markWhatsNewSeen("0.1.21", storage);
    expect(await whatsNewAfterUpdate(true, storage)).toBeNull();
  });

  it("shows a learner whose old version never kept anything, but not a new install", async () => {
    expect(await whatsNewAfterUpdate(true, memory())).not.toBeNull();
    const fresh = memory();
    expect(await whatsNewAfterUpdate(false, fresh)).toBeNull();
    expect(fresh.seen()).toBe("0.1.21");
  });

  it("stays out of the way outside the app, or when the version cannot be read", async () => {
    app.version.mockRejectedValueOnce(new Error("no IPC"));
    expect(await whatsNewAfterUpdate(true, memory("0.1.20"))).toBeNull();
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    expect(await whatsNewAfterUpdate(true, memory("0.1.20"))).toBeNull();
  });
});
