/**
 * Mouth shapes, from Ella Mobile's palette (`speech_viseme.dart`): her smile
 * and the speaking poses a talk partner's open mouth is drawn from. Each is
 * drawn still, once, by `mouth.ts`; nothing here runs on the audio clock.
 */

/** Mouth geometry in multiples of the placement's resting mouth width. Every
 * pose shares one path. */
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
 * flap open on t, k or l.
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
