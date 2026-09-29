import { describe, expect, it } from "vitest";
import { mouthGeometry } from "./mouth";
import { lerpMouth, SMILE, VISEMES, type MouthShape } from "./visemes";

/** The talk stage's mouth: 30 px wide, on a 3 px line. */
const UNIT = 30;
const LINE = 3;

const draw = (shape: MouthShape) => mouthGeometry(shape, UNIT, LINE);

/** Every point of the path, in order. */
function points(path: string): [number, number][] {
  const numbers = path.match(/-?\d+(\.\d+)?/g)!.map(Number);
  const pairs: [number, number][] = [];
  for (let index = 0; index < numbers.length; index += 2) pairs.push([numbers[index], numbers[index + 1]]);
  return pairs;
}

describe("her mouth", () => {
  it("closes to a flat, round-capped line", () => {
    const shut = draw(VISEMES.rest);
    expect(points(shut.path).every(([, y]) => y === 0)).toBe(true);
    expect(shut.outline).toBe(LINE);
    // Its ends sit on the pose's width; the round caps go past them.
    const xs = points(shut.path).map(([x]) => x);
    expect(Math.max(...xs) * 2).toBeCloseTo(UNIT * VISEMES.rest.width, 5);
    expect(shut).toMatchObject({ lowerTeeth: null, tongue: null, teeth: null });
  });

  it("smiles as the design's U, corners up and centre down, in its 30 by 13 box", () => {
    const smile = points(draw(SMILE).path);
    const corner = smile[0][1];
    const centres = smile.filter(([x]) => x === 0).map(([, y]) => y);
    expect(corner).toBeLessThan(0);
    expect(Math.min(...centres)).toBeGreaterThan(0);
    // Line included, as the stylesheet's border is.
    const width = Math.max(...smile.map(([x]) => x)) * 2 + LINE;
    const height = Math.max(...centres) - corner + LINE;
    expect(width).toBeGreaterThan(29);
    expect(width).toBeLessThanOrEqual(30);
    expect(height).toBeGreaterThan(13);
    expect(height).toBeLessThan(14);
  });

  it("opens an open vowel inside its lip line, as deep as the pose", () => {
    const open = points(draw(VISEMES.aa).path);
    const ys = open.map(([, y]) => y);
    expect(Math.max(...ys) - Math.min(...ys) + LINE).toBeCloseTo(UNIT * VISEMES.aa.height, 5);
  });

  it("draws a fully round mouth as an ellipse", () => {
    const round = draw({ ...VISEMES.oo, smile: 0 });
    const [[x], , [handle, top]] = points(round.path).map(([px, py]) => [-px, py]);
    // A quarter ellipse's handle is .5523 of its radius from the axis.
    expect(handle / x).toBeCloseTo(0.5523, 3);
    expect(top).toBeLessThan(0);
  });

  it("shows teeth only once the lips have parted", () => {
    expect(draw({ ...VISEMES.ee, height: 0.08 }).teeth).toBeNull();
    const ee = draw(VISEMES.ee);
    expect(ee.teeth?.height).toBeGreaterThan(LINE);
    expect(ee.lowerTeeth).not.toBeNull();
    // The upper teeth never reach past the inner edge of the upper lip.
    expect(ee.teeth!.clip.y).toBeCloseTo(Math.min(...points(ee.path).map(([, y]) => y)), 3);
  });

  it("keeps a trace of a neighbour's teeth tucked under the lip", () => {
    const shown = (shape: MouthShape) => {
      const geometry = draw(shape);
      const bottom = Math.max(...points(geometry.path).map(([, y]) => y));
      return geometry.lowerTeeth ? bottom - geometry.lowerTeeth.y : 0;
    };
    // Almost all AA with a little EE: EE's lower teeth would be a hairline.
    expect(shown(lerpMouth(VISEMES.aa, VISEMES.ee, 0.02))).toBeLessThan(0.01);
    // EE's own lower teeth show in full.
    expect(shown(VISEMES.ee)).toBeCloseTo(UNIT * VISEMES.ee.height * VISEMES.ee.lowerTeeth, 2);
  });

  it("keeps an F level until the lips part, then tucks the lower lip under arched teeth", () => {
    const onset = points(draw({ ...VISEMES.fv, height: 0 }).path);
    expect(onset.every(([, y]) => y === 0)).toBe(true);
    const fv = draw(VISEMES.fv);
    const [corner] = points(fv.path)[0].slice(1);
    const top = Math.min(...points(fv.path).map(([, y]) => y));
    // A negative smile arches the upper lip: its corners sit below its middle.
    expect(corner).toBeGreaterThan(top);
    expect(fv.teeth).not.toBeNull();
  });
});
