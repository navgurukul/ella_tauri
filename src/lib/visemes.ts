/**
 * Ella's mouth while she talks, ported from Ella Mobile's lip sync
 * (`speech_viseme.dart`): an English palette of 21 speaking poses and silence,
 * the timed cues Piper's own token durations make of them (see `alignment.ts`),
 * and a sampler that turns a position in the audio into a mouth.
 */

/** Mouth geometry in multiples of the placement's resting mouth width. Every
 * pose shares one path, so a timed cue morphs into the next without a cut. */
export interface MouthShape {
  width: number;
  height: number;
  round: number;
  teeth: number;
  lowerTeeth: number;
  tongue: number;
  smile: number;
  skew: number;
}

const mouth = (fields: Partial<MouthShape>): MouthShape => ({
  width: 1,
  height: 0,
  round: 0.35,
  teeth: 0,
  lowerTeeth: 0,
  tongue: 0,
  smile: 0,
  skew: 0,
  ...fields,
});

export function lerpMouth(a: MouthShape, b: MouthShape, t: number): MouthShape {
  const at = (from: number, to: number) => from + (to - from) * t;
  return {
    width: at(a.width, b.width),
    height: at(a.height, b.height),
    round: at(a.round, b.round),
    teeth: at(a.teeth, b.teeth),
    lowerTeeth: at(a.lowerTeeth, b.lowerTeeth),
    tongue: at(a.tongue, b.tongue),
    smile: at(a.smile, b.smile),
    skew: at(a.skew, b.skew),
  };
}

export type Viseme =
  | "rest"
  | "ah"
  | "aa"
  | "oh"
  | "eh"
  | "er"
  | "ee"
  | "oo"
  | "ow"
  | "aw"
  | "oy"
  | "ay"
  | "h"
  | "r"
  | "l"
  | "sz"
  | "sh"
  | "th"
  | "fv"
  | "dtn"
  | "kg"
  | "pbm";

/**
 * Each pose, as Ella Mobile draws it. Heights follow the jaw: open vowels stay
 * near the design's speaking "o" (.6 of the width deep), and consonants made
 * by the tongue alone sit below the vowels around them, so the jaw does not
 * flap open on t, k or l. Diphthongs are anchor poses; their glide is a
 * second timed cue.
 */
export const VISEMES: Record<Viseme, MouthShape> = {
  rest: mouth({ width: 0.76 }),
  ah: mouth({ width: 1, height: 0.48, tongue: 0.15 }), // cat, about, cup
  aa: mouth({ width: 0.94, height: 0.72, round: 0.55, tongue: 0.23 }), // father
  oh: mouth({ width: 0.72, height: 0.62, round: 0.92 }), // thought
  eh: mouth({ width: 1.12, height: 0.4, teeth: 0.2 }), // bed, day
  er: mouth({ width: 0.78, height: 0.34, round: 0.72, tongue: 0.12 }), // bird
  ee: mouth({ width: 1.22, height: 0.22, teeth: 0.34, lowerTeeth: 0.15 }), // yes, see, sit
  oo: mouth({ width: 0.44, height: 0.38, round: 1 }), // we, blue, book
  ow: mouth({ width: 0.6, height: 0.48, round: 0.96 }), // go
  aw: mouth({ width: 1, height: 0.64, round: 0.45, tongue: 0.22 }), // now
  oy: mouth({ width: 0.74, height: 0.58, round: 0.84, tongue: 0.12 }), // boy
  ay: mouth({ width: 1.1, height: 0.6, teeth: 0.14, tongue: 0.2 }), // my
  h: mouth({ width: 0.95, height: 0.3, round: 0.4 }), // hello
  r: mouth({ width: 0.66, height: 0.32, round: 0.83, teeth: 0.13 }), // red
  l: mouth({ width: 0.96, height: 0.32, teeth: 0.16, tongue: 0.42 }), // love
  sz: mouth({ width: 1.06, height: 0.19, teeth: 0.42, lowerTeeth: 0.32 }), // sun, zoo
  sh: mouth({ width: 0.72, height: 0.3, round: 0.78, teeth: 0.32, lowerTeeth: 0.17 }), // she, chair, jam
  // TH and D/T/N differ by how much tongue shows between the teeth.
  th: mouth({ width: 0.98, height: 0.26, teeth: 0.26, tongue: 0.74 }), // think, this
  // The lower lip tucks under the upper teeth: the upper lip arches over them
  // (a negative smile) while the lower one stays flat, which keeps F apart
  // from the S and EE slits.
  fv: mouth({ width: 0.95, height: 0.24, teeth: 0.6, round: 0.18, smile: -0.25 }), // fun, very
  dtn: mouth({ width: 1, height: 0.22, teeth: 0.3, lowerTeeth: 0.12, tongue: 0.15 }), // day, tea, no
  kg: mouth({ width: 0.96, height: 0.26, teeth: 0.22, lowerTeeth: 0.08, tongue: 0.12 }), // key, go, sing
  // Level and only a little longer than rest, so a seal does not rock the mouth.
  pbm: mouth({ width: 0.84 }), // pen, bee, me
};

/** Her U smile in the same path family, so speech grows out of it and hands
 * back to it as one mouth. Its centre sits a little higher to reach the U's
 * arms (see `SMILE_LIFT`). */
export const SMILE: MouthShape = mouth({ width: 0.88, smile: 0.82 });

/** How far above the mouth box's centre the smile sits, in resting widths. */
export const SMILE_LIFT = 0.065;

const VOWEL_VISEMES = new Set<Viseme>(["ah", "aa", "oh", "eh", "er", "ee", "oo", "ow", "aw", "oy", "ay"]);

/** Piper's English IPA, sound by sound. Unknown symbols close safely. */
const IPA: Record<string, Viseme> = {
  æ: "ah",
  ə: "ah",
  ʌ: "ah",
  ɑ: "aa",
  a: "aa",
  ɐ: "ah",
  ɒ: "oh",
  e: "eh",
  o: "ow",
  ɜ: "er",
  ɾ: "dtn",
  // A glottal stop has no lip or jaw target. In en-US it comes before a
  // syllabic n (button), so the tongue is already on the ridge.
  ʔ: "dtn",
  ɑː: "aa",
  ɔ: "oh",
  ɔː: "oh",
  ɛ: "eh",
  // Lax but rounded: book, and the glide of aʊ and oʊ.
  ʊ: "oo",
  eɪ: "eh",
  ɝ: "er",
  ɚ: "er",
  ɜː: "er",
  j: "ee",
  i: "ee",
  iː: "ee",
  ɪ: "ee",
  // The reduced vowel of roses and wanted.
  ᵻ: "ee",
  w: "oo",
  u: "oo",
  uː: "oo",
  oʊ: "ow",
  əʊ: "ow",
  aʊ: "aw",
  ɔɪ: "oy",
  aɪ: "ay",
  h: "h",
  ɹ: "r",
  r: "r",
  l: "l",
  s: "sz",
  z: "sz",
  ʃ: "sh",
  tʃ: "sh",
  dʒ: "sh",
  ʒ: "sh",
  θ: "th",
  ð: "th",
  f: "fv",
  v: "fv",
  d: "dtn",
  t: "dtn",
  n: "dtn",
  k: "kg",
  ɡ: "kg",
  g: "kg",
  ŋ: "kg",
  p: "pbm",
  b: "pbm",
  m: "pbm",
  // Sounds English lacks, from names and loan words (Bach, loch), take the
  // nearest English shape rather than holding the sound before.
  x: "kg",
  ɣ: "kg",
  χ: "kg",
  ʁ: "kg",
  q: "kg",
  ɬ: "l",
  ɫ: "l",
  ɭ: "l",
  ʎ: "l",
  ʍ: "oo",
  ɥ: "oo",
  ʈ: "dtn",
  ɖ: "dtn",
  ɳ: "dtn",
  ɲ: "dtn",
  c: "dtn",
  ɟ: "dtn",
  ɽ: "r",
  ɻ: "r",
  ɺ: "r",
  ʋ: "fv",
  ɸ: "fv",
  β: "fv",
  ɱ: "fv",
  ç: "h",
  ɦ: "h",
  ɕ: "sh",
  ʑ: "sh",
  ʂ: "sh",
  ʐ: "sh",
};

/** The pose for one IPA sound, or a pair Piper times as two (aɪ, tʃ). */
export function visemeFor(phoneme: string): Viseme {
  return IPA[phoneme.trim()] ?? "rest";
}

export interface VisemeCue {
  /** Milliseconds from the start of the audio. */
  start: number;
  end: number;
  viseme: Viseme;
  /** How far a vowel opens toward its pose, which is drawn stressed: 1 for
   * primary stress, less when unstressed or for a closer approximant. */
  emphasis?: number;
  /** Lip rounding toward the pose from a neutral width and curvature. */
  rounding?: number;
  /** Tells a vowel from the j or w that share its pose. */
  vowel?: boolean;
}

export function isVowelCue(cue: VisemeCue): boolean {
  return cue.vowel ?? VOWEL_VISEMES.has(cue.viseme);
}

/** The furthest any sound reaches, a vowel's, in ms. */
const REACH = 90;
/** Silence let in before the first cue and after the last, in ms. */
const SILENCE = 200;
/** A rest this long (ms) is a pause the mouth closes for. A shorter one, like
 * most commas, only dips toward rest between two words. */
const PAUSE_LENGTH = 100;
/** Two frames at 60 fps: the shortest p, b or m that still reads as shut. */
const SHORTEST_SEAL = 34;
/** Minimum seal ramp in ms, limiting its slope. A closure beside a very short
 * vowel can still land within one 60 fps frame. */
const SHORTEST_RAMP = 20;

/**
 * The mouth at any position in a line, from its cues.
 *
 * Sounds shape their neighbours: each articulator is a weighted average of the
 * poses within reach. A short sound leans toward its pose; a held vowel gently
 * relaxes. Lip seals and pauses apply after the average, so it can never
 * smooth a closure away. The mouth depends on the position alone; nothing
 * carries over from one frame to the next, so pausing, seeking and replaying
 * stay exact.
 */
export class VisemeTrack {
  readonly cues: readonly VisemeCue[];
  private readonly segments: Segment[];

  /** Cues must be ordered and must not overlap. */
  constructor(cues: readonly VisemeCue[]) {
    this.cues = cues;
    this.segments = segment(cues);
  }

  /** Where the last cue ends, in ms. */
  get end(): number {
    return this.cues.length ? this.cues[this.cues.length - 1].end : 0;
  }

  sample(position: number, speaking = true): MouthShape {
    const rest = VISEMES.rest;
    const segments = this.segments;
    if (!speaking || position < 0 || !segments.length) return rest;
    const t = position;
    let low = 0;
    let high = segments.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (segments[mid].end + REACH < t) low = mid + 1;
      else high = mid;
    }
    let jaw = 0;
    let lips = 0;
    let teeth = 0;
    let tongue = 0;
    let height = 0;
    let width = 0;
    let round = 0;
    let smile = 0;
    let skew = 0;
    let upper = 0;
    let lower = 0;
    let tip = 0;
    let seal = 0;
    let relax = 0;
    let count = 0;
    let alone: MouthShape | null = null;
    for (let index = low; index < segments.length; index += 1) {
      const current = segments[index];
      if (current.start - REACH > t) break;
      // Every seal lies within reach of a segment it belongs to.
      seal = Math.max(seal, current.seal?.at(t) ?? 0);
      relax = Math.max(relax, current.relax?.at(t) ?? 0);
      const k = current.weight(t);
      if (k === 0) continue;
      const d = current.dominance;
      const x = current.pose(t);
      alone = count === 0 ? x : null;
      count += 1;
      const j = d.jaw * k;
      const l = d.lips * k;
      const e = d.teeth * k;
      const g = d.tongue * k;
      jaw += j;
      height += j * x.height;
      lips += l;
      width += l * x.width;
      round += l * x.round;
      smile += l * x.smile;
      skew += l * x.skew;
      teeth += e;
      upper += e * x.teeth;
      lower += e * x.lowerTeeth;
      tongue += g;
      tip += g * x.tongue;
    }
    if (count === 0 || relax === 1) return rest;
    // A sound heard alone keeps its pose, with only long vowels settling.
    let shape: MouthShape = alone ?? {
      width: width / lips,
      height: height / jaw,
      round: round / lips,
      teeth: upper / teeth,
      lowerTeeth: lower / teeth,
      tongue: tip / tongue,
      smile: smile / lips,
      skew: skew / lips,
    };
    if (seal > 0) shape = { ...shape, height: shape.height * (1 - seal) };
    return relax === 0 ? shape : lerpMouth(shape, rest, relax);
  }
}

/** One sound, or the silence around and between them, in ms of playback. */
class Segment {
  constructor(
    readonly start: number,
    readonly end: number,
    readonly target: MouthShape,
    readonly dominance: Dominance,
    /** Closes the lips (height only). */
    readonly seal: Ramp | null,
    /** Relaxes every field to rest. */
    readonly relax: Ramp | null,
    readonly settle: boolean,
  ) {}

  pose(t: number): MouthShape {
    if (!this.settle || t <= this.start + 150) return this.target;
    // Held vowels gently release the jaw, including their outgoing falloff.
    // The clock alone drives this, so seeking cannot restart the hold.
    const x = clamp((t - this.start - 150) / (this.end - this.start - 150), 0, 1);
    return { ...this.target, height: this.target.height * (1 - 0.1 * x * x * (3 - 2 * x)) };
  }

  /** 1 at the edges, rising to 3 inside, so a long sound reaches its pose
   * while a short one blends; outside, a compact falloff to zero. */
  weight(t: number): number {
    if (t < this.start) return falloff((this.start - t) / this.dominance.before);
    if (t > this.end) return falloff((t - this.end) / this.dominance.after);
    const x = Math.min(1, Math.min(t - this.start, this.end - t) / 20);
    return 1 + 2 * x * x * (3 - 2 * x);
  }
}

function falloff(x: number): number {
  if (x >= 1) return 0;
  const y = 1 - x * x;
  return y * y;
}

/** How firmly a sound holds the jaw, lips, teeth and tongue, and how far (ms)
 * it reaches before and after itself. */
interface Dominance {
  jaw: number;
  lips: number;
  teeth: number;
  tongue: number;
  before: number;
  after: number;
}

const dominance = (jaw: number, lips: number, teeth: number, tongue: number, before = 40, after = 40): Dominance => ({
  jaw,
  lips,
  teeth,
  tongue,
  before,
  after,
});

/** 1 over [from, to], easing to 0 over `rampIn` ms before and `rampOut` after. */
class Ramp {
  constructor(
    readonly from: number,
    readonly to: number,
    readonly rampIn: number,
    readonly rampOut: number,
  ) {}

  at(t: number): number {
    const x = t < this.from ? (this.from - t) / this.rampIn : t > this.to ? (t - this.to) / this.rampOut : 0;
    if (x >= 1) return 0;
    const u = 1 - x;
    return u * u * (3 - 2 * u);
  }

  join(next: Ramp): Ramp {
    return new Ramp(
      Math.min(this.from, next.from),
      Math.max(this.to, next.to),
      this.from <= next.from ? this.rampIn : next.rampIn,
      this.to >= next.to ? this.rampOut : next.rampOut,
    );
  }
}

type Span = [start: number, end: number, cue: VisemeCue | null];

function segment(cues: readonly VisemeCue[]): Segment[] {
  // Silence is let in before, between and after the cues, so a track starts
  // and ends on rest and a hole between cues closes the mouth. Before the
  // first cue it runs back past zero, so even a cue at zero opens from rest.
  const spans: Span[] = [];
  let reached = 0;
  for (const cue of cues) {
    if (cue.start < reached || cue.end <= cue.start) {
      throw new RangeError("Viseme cues must be ordered, positive and non-overlapping");
    }
    if (!spans.length) spans.push([Math.min(0, cue.start - SILENCE), cue.start, null]);
    else if (cue.start > reached) spans.push([reached, cue.start, null]);
    spans.push([cue.start, cue.end, cue]);
    reached = cue.end;
  }
  if (!spans.length) return [];
  spans.push([reached, reached + SILENCE, null]);

  const seals: (Ramp | null)[] = spans.map(() => null);
  const relaxes: (Ramp | null)[] = spans.map(() => null);
  const half = (span: Span) => (span[1] - span[0]) / 2;
  spans.forEach(([start, end, cue], index) => {
    // Closing and opening take about half the sound either side, so a short
    // vowel beside a p or a pause still opens. The floor limits speed at the
    // cost of some short-vowel opening.
    const rampIn = index > 0 ? clamp(half(spans[index - 1]), SHORTEST_RAMP, 40) : 40;
    const rampOut = index + 1 < spans.length ? clamp(half(spans[index + 1]), SHORTEST_RAMP, 45) : 45;
    if (!cue) {
      // Silence closes by the end of the cue before it, as the sound does.
      seals[index] = relaxes[index] = new Ramp(start, end, rampIn, rampOut);
    } else if (cue.viseme === "rest" && end - start >= PAUSE_LENGTH) {
      // The mouth settles inside the pause, not before it.
      const middle = (start + end) / 2;
      seals[index] = relaxes[index] = new Ramp(Math.min(middle, start + 30), Math.max(middle, end - 30), 30, 30);
    } else if (cue.viseme === "pbm") {
      // The lips meet for the whole sound and never shorter than two frames,
      // however much the average around it would open them.
      const spread = Math.max(0, SHORTEST_SEAL - (end - start)) / 2;
      seals[index] = new Ramp(start - spread, end + spread, rampIn, rampOut);
    }
  });
  join(seals);
  join(relaxes);
  return spans.map(
    (span, index) =>
      new Segment(
        span[0],
        span[1],
        target(span[2]),
        dominanceOf(span),
        seals[index],
        relaxes[index],
        span[2] !== null && isVowelCue(span[2]) && span[1] - span[0] >= 220,
      ),
  );
}

function target(cue: VisemeCue | null): MouthShape {
  if (!cue) return VISEMES.rest;
  const shape = VISEMES[cue.viseme];
  const emphasis = cue.emphasis ?? 1;
  const rounding = cue.rounding ?? 1;
  if (emphasis === 1 && rounding === 1) return shape;
  return {
    ...shape,
    width: 1 + (shape.width - 1) * rounding,
    height: shape.height * emphasis,
    round: 0.35 + (shape.round - 0.35) * rounding,
  };
}

/** Ramps on neighbouring segments become one, so a pause straight into a p,
 * or two bilabials in a row, stays shut instead of blipping open. */
function join(ramps: (Ramp | null)[]) {
  for (let index = 1; index < ramps.length; index += 1) {
    const before = ramps[index - 1];
    const ramp = ramps[index];
    if (before && ramp) ramps[index] = before.join(ramp);
  }
  for (let index = ramps.length - 2; index >= 0; index -= 1) {
    if (ramps[index] && ramps[index + 1]) ramps[index] = ramps[index + 1];
  }
}

function dominanceOf([start, end, cue]: Span): Dominance {
  const pause = dominance(1.5, 1, 1, 1, 30, 30);
  if (!cue) return pause;
  const viseme = cue.viseme;
  if (viseme === "rest") return end - start >= PAUSE_LENGTH ? pause : dominance(1.5, 1, 1, 1);
  // Vowels carry the jaw and reach furthest. Rounded ones hold the lips across
  // the consonants around them, as in 'two' or 'soon'.
  if (VOWEL_VISEMES.has(viseme)) return dominance(1, VISEMES[viseme].round >= 0.8 ? 2 : 1, 1, 1, 90, 90);
  switch (viseme) {
    // The seal closes the lips; the pose itself barely pulls on the average.
    case "pbm":
      return dominance(0.5, 0.6, 1, 0.3, 50, 35);
    // Lip on teeth: the jaw has to follow.
    case "fv":
      return dominance(2.5, 1, 3, 0.5, 50, 40);
    case "sz":
      return dominance(1.2, 0.6, 2, 0.5, 50, 40);
    // Rounded early, and held into the vowel after.
    case "sh":
      return dominance(1.2, 2, 2, 0.5, 80, 50);
    case "r":
      return dominance(0.8, 2, 1, 0.5, 80, 50);
    case "th":
      return dominance(0.8, 0.5, 1.5, 3);
    // Made by the tongue alone, so jaw and lips come from the neighbours.
    case "dtn":
    case "l":
      return dominance(0.7, 0.3, 1.2, 1.5);
    default:
      // k, g, ng and h.
      return dominance(0.5, 0.2, 0.3, 0.3);
  }
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}
