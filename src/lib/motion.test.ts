import { afterEach, describe, expect, it, vi } from "vitest";
import { bodyKeyframes, bodyLoop, bodyTransform, createFaceClock, kf, type BodyLoopName } from "./motion";

const LOOPS: BodyLoopName[] = ["idle", "speaking", "thinking", "listening", "homeEager", "homeHappy"];

describe("Ella's body loops, as Ella Mobile plays them", () => {
  it("eases each interval of a track as CSS does", () => {
    const track = [
      [0, 0],
      [0.5, -8],
      [1, 0],
    ] as const;
    expect(kf(track, 0)).toBe(0);
    expect(kf(track, 0.25)).toBeCloseTo(-4, 6);
    expect(kf(track, 0.5)).toBe(-8);
    // Ease-in-out: slow off each stop.
    expect(kf(track, 0.05)).toBeGreaterThan(-0.4);
  });

  it("keeps the design's pose loops", () => {
    expect(bodyLoop("idle", 30).period).toBe(4200);
    expect(bodyLoop("idle", 30).at(0.5)).toEqual({ y: -8, rotation: 0, scaleX: 1.008, scaleY: 1.008 });
    expect(bodyLoop("speaking", 30).period).toBe(850);
    expect(bodyLoop("speaking", 30).at(0.3)).toEqual({ y: -13, rotation: 0, scaleX: 1.02, scaleY: 0.985 });
    expect(bodyLoop("speaking", 30).at(0.6)).toEqual({ y: 2, rotation: 0, scaleX: 0.99, scaleY: 1.012 });
    expect(bodyLoop("thinking", 30).period).toBe(2200);
    expect(bodyLoop("thinking", 30).at(0)).toEqual({ y: 0, rotation: -1.4, scaleX: 1, scaleY: 1 });
    expect(bodyLoop("thinking", 30).at(0.5)).toEqual({ y: -3, rotation: 1.4, scaleX: 1, scaleY: 1 });
  });

  it("leans in to listen by a few pixels, with a small nod, not the design's swoop", () => {
    const listening = bodyLoop("listening", 30);
    expect(listening.period).toBe(3600);
    const start = listening.at(0);
    expect(start.y).toBeCloseTo(-6, 6);
    expect(start.rotation).toBeCloseTo(2, 6);
    const nod = listening.at(0.5);
    expect(nod.y).toBeCloseTo(-7.35, 6);
    expect(nod.rotation).toBeCloseTo(2.8, 6);
    // Most of the loop she just holds the lean: the nod is brief.
    expect(listening.at(0.2).rotation).toBeLessThan(2.05);
  });

  it("measures an expression's lifts in widths of her mouth", () => {
    expect(bodyLoop("homeEager", 24).period).toBe(2200);
    const eager = bodyLoop("homeEager", 24).at(0.25);
    expect(eager.y).toBeCloseTo(-0.23 * 0.5 * 24, 6);
    expect(eager.rotation).toBeCloseTo(1.4, 6);
    expect(eager.scaleY).toBeCloseTo(1.008, 6);
    expect(bodyLoop("homeHappy", 24).period).toBe(3200);
    expect(bodyLoop("homeHappy", 24).at(0.5).y).toBeCloseTo(-2.4, 6);
    expect(bodyLoop("homeHappy", 48).at(0.5).y).toBeCloseTo(-4.8, 6);
  });

  it("writes a pose in the order Ella Mobile composes it", () => {
    expect(bodyTransform({ y: -7.351234, rotation: 2.8, scaleX: 1, scaleY: 1.008 })).toBe(
      "translateY(-7.3512px) rotate(2.8deg) scale(1, 1.008)",
    );
  });

  it("samples every loop into keyframes that close on themselves", () => {
    for (const name of LOOPS) {
      const frames = bodyKeyframes(bodyLoop(name, 30));
      expect(frames).toHaveLength(61);
      expect(frames[0].offset).toBe(0);
      expect(frames[60].offset).toBe(1);
      expect(frames[60].transform).toBe(frames[0].transform);
    }
  });
});

describe("the face clock", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("blinks and grins every Ella together, and stops once nobody is watching", () => {
    vi.useFakeTimers();
    const clock = createFaceClock(() => 0);
    const seen: string[] = [];
    const first = clock.subscribe(() => seen.push(`${clock.blinking ? "blink" : "open"} ${clock.grinning ? "grin" : "smile"}`));
    const second = clock.subscribe(() => undefined);

    vi.advanceTimersByTime(2599);
    expect(clock.blinking).toBe(false);
    vi.advanceTimersByTime(1);
    expect(clock.blinking).toBe(true);
    vi.advanceTimersByTime(140);
    expect(clock.blinking).toBe(false);
    vi.advanceTimersByTime(5000 - 2740);
    expect(clock.grinning).toBe(true);
    vi.advanceTimersByTime(650);
    expect(clock.grinning).toBe(false);
    expect(seen).toEqual(["blink smile", "open smile", "open grin", "blink grin", "open grin", "open smile"]);

    first();
    second();
    seen.length = 0;
    vi.advanceTimersByTime(60_000);
    expect(seen).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });
});
