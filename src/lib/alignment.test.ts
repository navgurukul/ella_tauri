import { describe, expect, it } from "vitest";
import fixtures from "../test/voice_alignment.json";
import type { PhonemeSpan } from "../types";
import { cuesFromPhonemes } from "./alignment";
import { isVowelCue, VISEMES, VisemeTrack, type MouthShape, type VisemeCue } from "./visemes";

/** One clip, each token 10 ms long unless `ms` says. */
function clip(tokens: string[], ms?: number[]): PhonemeSpan[] {
  let at = 0;
  return tokens.map((phoneme, index) => {
    const start = at;
    at += ms?.[index] ?? 10;
    return { phoneme, start_ms: start, end_ms: at };
  });
}

const align = (tokens: string[], ms?: number[]) => cuesFromPhonemes(clip(tokens, ms));

const said = (cues: VisemeCue[]) => cues.map((cue) => `${cue.viseme}[${cue.start},${cue.end})`).join(" ");

const PAUSES = new Set(["^", "$", ".", "!", "?", ",", ";", ":"]);

/** Real Piper timings: the NavGurukul voice through the resident daemon, as
 * Ella Mobile's own fixture sentences. */
const voices = fixtures.map((fixture) => ({
  ...fixture,
  track: new VisemeTrack(cuesFromPhonemes(fixture.phonemes)),
}));

/** Positions `step` ms apart from `from` until the mouth has settled after
 * the last cue. */
function across(track: VisemeTrack, step: number, from = 0): number[] {
  const positions: number[] = [];
  for (let at = from; at <= track.end + 250; at += step) positions.push(at);
  return positions;
}

/** How far `position` is from the nearest of `spans`. */
function distance(position: number, spans: [number, number][]): number {
  return Math.min(
    ...spans.map(([start, end]) => (position < start ? start - position : position > end ? position - end : 0)),
  );
}

const fields = (shape: MouthShape) => [
  shape.width,
  shape.height,
  shape.round,
  shape.teeth,
  shape.lowerTeeth,
  shape.tongue,
  shape.smile,
  shape.skew,
];

describe("Piper's timings as mouth cues", () => {
  it("lets blanks, word spaces and stress marks lead into the next sound", () => {
    const cues = align(["m", "_", "ˈ", "_", "ɛ", "_", " ", "_", "p"]);
    expect(said(cues)).toBe("pbm[0,10) eh[10,50) pbm[50,90)");
    expect(cues.map((cue) => cue.emphasis)).toEqual([1, 1, 1]);
  });

  it("holds a sound through its length marks, diacritics and poseless symbols", () => {
    expect(said(align(["u", "_", "ː"]))).toBe("oo[0,30)");
    // 'button' ends ʌʔn̩: the syllabic mark must not close the mouth, and the
    // glottal stop must not open it.
    expect(said(align(["ʌ", "ʔ", "n", "̩"]))).toBe("ah[0,10) dtn[10,20) dtn[20,40)");
    expect(said(align(["ɑ", '"', "t"]))).toBe("aa[0,20) dtn[20,30)");
  });

  it("pairs sounds only inside a word", () => {
    expect(said(align(["t", "_", "ʃ"]))).toBe("sh[0,30)");
    expect(said(align(["t", "_", " ", "_", "ʃ"]))).toBe("dtn[0,10) sh[10,50)");
    expect(said(align(["a", "_", "ɪ"]))).toBe("ay[0,10) ee[10,30)");
    expect(said(align(["a", "_", " ", "ɪ"]))).toBe("aa[0,10) ee[10,40)");
  });

  it("rounds a j before a rounded vowel already", () => {
    expect(said(align(["j", "_", "u"]))).toBe("oo[0,10) oo[10,30)");
    expect(said(align(["j", "_", "ɔ"]))).toBe("oo[0,10) oh[10,30)");
    expect(said(align(["j", "_", "ɛ"]))).toBe("ee[0,10) eh[10,30)");
    expect(said(align(["j", "_", " ", "_", "u"]))).toBe("ee[0,10) oo[10,50)");
    expect(align(["j", "_", "u"]).map((cue) => cue.emphasis)).toEqual([0.7, 0.9]);
    expect(align(["j", "_", " ", "_", "u"])[0].emphasis).toBe(1);
  });

  it("gives rounded approximants a closer jaw than their vowels", () => {
    for (const sound of ["w", "ʍ", "ɥ"]) {
      const cues = align([sound, "_", "ɛ"]);
      expect(cues.map((cue) => cue.viseme)).toEqual(["oo", "eh"]);
      expect(cues.map((cue) => cue.emphasis)).toEqual([0.7, 0.9]);
      expect(cues.map(isVowelCue)).toEqual([false, true]);
    }
    expect(align(["j", "_", "u"]).map(isVowelCue)).toEqual([false, true]);
    expect(isVowelCue(align(["j", "_", "ɛ"])[0])).toBe(false);
  });

  it("softens a lax vowel's pucker, but a diphthong's glide keeps it", () => {
    const [book] = align(["ʊ"], [200]);
    expect(book.viseme).toBe("oo");
    const mouth = new VisemeTrack([book]).sample(100);
    expect(mouth.height).toBeCloseTo(VISEMES.oo.height * 0.9, 10);
    expect(mouth.width).toBeCloseTo(0.552, 10);
    expect(mouth.round).toBeCloseTo(0.87, 10);
    for (const first of ["a", "o", "ə"]) {
      expect(align([first, "_", "ʊ"]).at(-1)?.rounding).toBe(1);
      expect(align([first, "_", " ", "_", "ʊ"]).at(-1)?.rounding).toBe(0.8);
    }
  });

  it("shuts a p after a vowel on the silent last token of the vowel", () => {
    const apple = ["_", "ˈ", "_", "æ", "_", "p"];
    const ms = [10, 20, 10, 30, 10, 20];
    expect(said(align(apple, ms))).toBe("ah[0,40) pbm[40,100)");
    // A b or m is voiced as the lips close, so it keeps its own timing.
    expect(said(align([...apple.slice(0, 5), "b"], ms))).toBe("ah[0,70) pbm[70,100)");
    // Length marks are the vowel's last token too (sheep).
    expect(said(align(["i", "_", "ː", "_", "p"], [40, 10, 20, 10, 20]))).toBe("ee[0,50) pbm[50,100)");
    // Not across a word, and never leaving the vowel under 40 ms.
    expect(said(align(["_", "æ", "_", " ", "_", "p"], [10, 50, 10, 10, 10, 20]))).toBe("ah[0,60) pbm[60,110)");
    expect(said(align(["_", "æ", "_", "p"], [10, 30, 10, 20]))).toBe("ah[0,40) pbm[40,70)");
  });

  it("parts the lips 12 ms into a long p or b, as it bursts", () => {
    // The closure is the blank before; the rest of the token is the vowel.
    expect(said(align(["_", "p", "_", "ˈ", "_", "ɛ"], [30, 40, 10, 10, 10, 40]))).toBe("pbm[0,42) eh[42,140)");
    expect(said(align(["_", "b", "_", "l"], [30, 20, 10, 20]))).toBe("pbm[0,42) l[42,80)");
    // With the closure moved onto the silent end of the vowel before: apple.
    expect(
      said(align(["_", "ˈ", "_", "æ", "_", "p", "_", "ə"], [10, 20, 10, 30, 10, 20, 10, 20])),
    ).toBe("ah[0,40) pbm[40,92) ah[92,130)");
    // A one-frame token, an m, and a p before a word, a pause or a stop keep
    // their own timing.
    expect(said(align(["_", "p", "_", "ɛ"], [30, 10, 10, 40]))).toBe("pbm[0,40) eh[40,90)");
    expect(said(align(["_", "m", "_", "ɛ"], [30, 40, 10, 40]))).toBe("pbm[0,70) eh[70,120)");
    expect(said(align(["_", "p", "_", " ", "_", "ɛ"], [30, 40, 10, 10, 10, 40]))).toBe("pbm[0,70) eh[70,140)");
    expect(said(align(["_", "p", "_", "."], [30, 40, 10, 20]))).toBe("pbm[0,70) rest[70,100)");
    expect(said(align(["_", "p", "t"], [30, 40, 10]))).toBe("pbm[0,70) dtn[70,80)");
  });

  it("carries each vowel's stress, copies it onto a glide, and merges pauses", () => {
    const cues = align([
      "ˌ", "a", "_", "ɪ", "_", "ə", "_", ",", "_", " ", "_", "ˈ", "_", "ɑ", "_", ".", "_", "$",
    ]);
    expect(said(cues)).toBe("ay[0,20) ee[20,40) ah[40,60) rest[60,80) aa[80,140) rest[140,180)");
    expect(cues.map((cue) => cue.emphasis)).toEqual([0.95, 0.95, 0.9, 1, 1, 1]);
  });

  it("refuses spans that overlap or leave a hole", () => {
    expect(() =>
      cuesFromPhonemes([
        { phoneme: "m", start_ms: 0, end_ms: 15 },
        { phoneme: "m", start_ms: 10, end_ms: 20 },
      ]),
    ).toThrow(RangeError);
    expect(() =>
      cuesFromPhonemes([
        { phoneme: "m", start_ms: 0, end_ms: 10 },
        { phoneme: "a", start_ms: 15, end_ms: 20 },
      ]),
    ).toThrow(RangeError);
  });
});

describe("her mouth in real speech", () => {
  it("makes ordered cues that shut, round and spread, and closes between sentences", () => {
    for (const voice of voices) {
      const cues = voice.track.cues;
      expect(cues.length).toBeGreaterThan(25);
      for (const viseme of ["pbm", "oo", "ee"] as const) {
        expect(cues.some((cue) => cue.viseme === viseme), `${voice.text}: ${viseme}`).toBe(true);
      }
      // Only punctuation closes the mouth, never a blank between two sounds.
      const pauses = voice.phonemes.filter((span) => PAUSES.has(span.phoneme));
      for (const cue of cues.filter((cue) => cue.viseme === "rest")) {
        expect(
          pauses.some((span) => span.start_ms < cue.end && cue.start < span.end_ms),
          `${voice.text}: rest at ${cue.start}`,
        ).toBe(true);
      }
      // Each fixture is two sentences, and she closes her mouth between them.
      const join = voice.phonemes.findIndex((span, index) => span.phoneme === "$" && index < voice.phonemes.length - 1);
      const between = voice.phonemes[join];
      expect(voice.track.sample((between.start_ms + between.end_ms) / 2).height).toBe(0);
    }
  });

  it("shuts only for a p, b, m or a pause", () => {
    for (const { track } of voices) {
      const cues = track.cues;
      const closures: [number, number][] = [
        ...cues.filter((cue) => cue.viseme === "pbm" || cue.viseme === "rest").map((cue): [number, number] => [cue.start, cue.end]),
        // Silence between cues and after the line has no cue of its own.
        ...cues.slice(1).flatMap((cue, index): [number, number][] =>
          cues[index].end < cue.start ? [[cues[index].end, cue.start]] : [],
        ),
        [track.end, track.end + 1000],
      ];
      for (const at of across(track, 1)) {
        if (track.sample(at).height >= 0.05) continue;
        // The lips start closing before a p, and a short one is held longer
        // than it sounds.
        expect(distance(at, closures), `shut at ${at}`).toBeLessThanOrEqual(20);
      }
    }
  });

  it("only snaps the jaw at a lip closure", () => {
    const frame = 1000 / 60;
    for (const { track } of voices) {
      const seals = track.cues.filter((cue) => cue.viseme === "pbm").map((cue): [number, number] => [cue.start, cue.end]);
      for (let phase = 0; phase < 4; phase += 1) {
        const frames = across(track, frame, (frame * phase) / 4);
        for (let index = 1; index < frames.length; index += 1) {
          const [before, after] = [frames[index - 1], frames[index]];
          if (distance(before, seals) <= 50 || distance(after, seals) <= 50) continue;
          expect(Math.abs(track.sample(after).height - track.sample(before).height), `jump at ${after}`).toBeLessThanOrEqual(0.25);
        }
      }
    }
  });

  it("does not carry rounding into the spread i of sheep", () => {
    const sheep = voices.flatMap(({ track }) =>
      track.cues
        .map((cue, index, cues) =>
          index > 0 && cues[index - 1].viseme === "sh" && cue.viseme === "ee" && cues[index + 1]?.viseme === "pbm"
            ? track.sample(cue.start + (cue.end - cue.start) / 2)
            : null,
        )
        .filter((shape): shape is MouthShape => shape !== null),
    );
    expect(sheep).toHaveLength(1);
    expect(sheep[0].width).toBeGreaterThanOrEqual(1.1);
  });

  it("depends on the position alone, in any order", () => {
    for (const { track } of voices) {
      const positions = across(track, 0.997);
      const inOrder = positions.map((at) => fields(track.sample(at)));
      const shuffled = positions.map((_, index) => index).sort((a, b) => ((a * 7919) % 101) - ((b * 7919) % 101));
      for (const index of shuffled) expect(fields(track.sample(positions[index]))).toEqual(inOrder[index]);
    }
  });
});
