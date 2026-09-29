import type { MouthShape } from "./visemes";

/** A rectangle in the mouth's own coordinates, centred on the mouth. */
export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * What to draw for one mouth, centred on 0,0: an ink outline, and the teeth
 * and tongue clipped inside it. Ported from Ella Mobile's `paintMouth`, so
 * her mouth has the same shapes on the laptop as on the phone.
 */
export interface MouthGeometry {
  /** The lips: four cubic curves around the opening, filled with ink. */
  path: string;
  /** The ink line stroked along `path`, with round caps and joins. It is
   * what a closed mouth is, and what keeps a sealed one a line. */
  outline: number;
  /** Radians. */
  skew: number;
  lowerTeeth: Box | null;
  tongue: { cx: number; cy: number; rx: number; ry: number } | null;
  /** The upper teeth: a rounded band clipped to `clip`, so a bent lip's
   * inner edge can never leak white into the U's arms. */
  teeth: (Box & { radius: number; clip: Box }) | null;
}

/**
 * The mouth for `shape` at `unit`, the placement's resting mouth width, with
 * its lip line `stroke` thick.
 */
export function mouthGeometry(shape: MouthShape, unit: number, stroke = unit * 0.1): MouthGeometry {
  const line = stroke;
  // Keep the lip line as they close, so every sealed mouth is still a line.
  const y = Math.max(0, (unit * shape.height) / 2 - line / 2);
  const x = (unit * shape.width) / 2 - Math.min(y, line / 2);
  // A round mouth is an ellipse (kappa .5523); a spread one is squarer.
  const k = lerp(0.72, 0.5523, clamp(shape.round, 0, 1));
  // A closed smile bends the line, corners up and centre down. As it opens,
  // the bend gives way to raised corners, so an open smile is the design's
  // D-shaped grin rather than a deep crescent.
  const grin = ease(clamp(y / (0.15 * unit), 0, 1));
  // An F/V tuck arches only after the lips part; a closed arch reads as a frown.
  const tuck = shape.smile < 0 ? clamp((2 * y) / line, 0, 1) : 1;
  const smile = shape.smile * tuck;
  const bend = smile * unit * (1 - grin);
  const corner = -0.14 * bend - 0.85 * smile * y * grin;
  const top = 0.2875 * bend - y;
  const bottom = 0.2875 * bend + y;
  // While the bend outweighs the opening, the handles are those of the
  // designed closed curve split at its centre, so closed smiles keep their
  // exact shape; open mouths use `k`.
  const depth = (0.14 + 0.2875) * bend;
  const curve = depth > 0 ? depth / (depth + y) : 0;
  const rise = lerp(k, 2 / 3, curve);
  const reach = lerp(k, 0.475, curve);
  const side = x * (1 - 0.05 * curve);
  const upper = corner + (top - corner) * rise;
  const lower = corner + (bottom - corner) * rise;
  const n = (value: number) => round(value);
  const path =
    `M${n(-x)} ${n(corner)}` +
    `C${n(-side)} ${n(upper)} ${n(-x * reach)} ${n(top)} 0 ${n(top)}` +
    `C${n(x * reach)} ${n(top)} ${n(side)} ${n(upper)} ${n(x)} ${n(corner)}` +
    `C${n(side)} ${n(lower)} ${n(x * reach)} ${n(bottom)} 0 ${n(bottom)}` +
    `C${n(-x * reach)} ${n(bottom)} ${n(-side)} ${n(lower)} ${n(-x)} ${n(corner)}Z`;

  const geometry: MouthGeometry = { path, outline: line, skew: shape.skew, lowerTeeth: null, tongue: null, teeth: null };
  // A closing mouth tucks teeth and tongue under the lip geometrically; fading
  // a full-sized white band over ink would turn the teeth grey.
  const reveal = ease(clamp((2 * y) / (0.06 * unit), 0, 1));
  if (reveal <= 0 || curve >= 1 || (shape.teeth <= 0 && shape.lowerTeeth <= 0 && shape.tongue <= 0)) {
    return geometry;
  }
  // The bands start at the inner lip and tuck beneath it with the bend.
  const height = unit * shape.height * (1 - curve);
  // Blending leaves traces of a neighbour's teeth and tongue, which would
  // flicker as hairlines. Tuck those by depth too; a pose's own teeth are
  // fully exposed from a third of the lip line and always stay white.
  const band = (amount: number) => {
    const deep = height * amount * reveal;
    return deep * ease(clamp(deep / (0.3 * line), 0, 1));
  };
  const lowerTeeth = band(shape.lowerTeeth);
  const tongue = band(shape.tongue);
  const teeth = band(shape.teeth);
  if (lowerTeeth > 0) geometry.lowerTeeth = box(-x * 0.86, bottom - lowerTeeth, x * 1.72, height + line);
  if (tongue > 0) {
    geometry.tongue = {
      cx: 0,
      cy: n(bottom - tongue + (height * 0.95) / 2),
      rx: n(x * 0.58),
      ry: n((height * 0.95) / 2),
    };
  }
  if (teeth > 0) {
    geometry.teeth = {
      ...box(-x * 0.86, top - line, x * 1.72, teeth + line),
      radius: n(unit * 0.04),
      clip: box(-x, top, x * 2, teeth),
    };
  }
  return geometry;
}

function box(x: number, y: number, width: number, height: number): Box {
  return { x: round(x), y: round(y), width: round(width), height: round(height) };
}

/** Ella Mobile's `CurvesForElla.ease`, a smoothstep. */
function ease(t: number): number {
  return t * t * (3 - 2 * t);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

/** Three decimals of a pixel: finer than any screen, and a short path string. */
function round(value: number): number {
  return Math.round(value * 1000) / 1000 + 0;
}
