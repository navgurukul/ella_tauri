/**
 * How Ella carries herself, ported from Ella Mobile (`keyframes.dart`,
 * `expression_spec.dart` and `gaze.dart`) so she moves the same on a laptop
 * as on a phone.
 *
 * Her body runs one loop at a time, about the bottom centre of her box: her
 * pose's, or an expression's where Ella Mobile wears one in its place, which
 * is while she listens and on Home. Ella Mobile leaves out the design's
 * walk-on whenever a screen opens and its swoop on every listening turn as
 * too much motion to sit through again and again, and so does this: she
 * arrives where she stands, and the loops, her ears and her face carry the
 * life instead.
 */

/** One stop of a keyframe track: how far through the loop, and the value there. */
export type Stop = readonly [at: number, value: number];

/** A CSS `cubic-bezier` timing function, solved for its progress. */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const at = (a: number, b: number, s: number) => 3 * a * s * (1 - s) ** 2 + 3 * b * s * s * (1 - s) + s ** 3;
  const slope = (a: number, b: number, s: number) =>
    3 * a * (1 - s) ** 2 + 6 * (b - a) * s * (1 - s) + 3 * (1 - b) * s * s;
  return (t) => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    let s = t;
    for (let step = 0; step < 8; step += 1) {
      const d = slope(x1, x2, s);
      if (Math.abs(d) < 1e-6) break;
      s = Math.min(1, Math.max(0, s - (at(x1, x2, s) - t) / d));
    }
    return at(y1, y2, s);
  };
}

/** Flutter's `Curves.easeOut` and `Curves.easeInOut`, which are CSS's
 * `ease-out` and `ease-in-out`. */
export const easeOut = cubicBezier(0, 0, 0.58, 1);
export const easeInOut = cubicBezier(0.42, 0, 0.58, 1);

/**
 * A keyframe track at `t`, eased between each pair of stops as CSS eases
 * every interval of an animation — with `ease-in-out`, for every loop here.
 */
export function kf(stops: readonly Stop[], t: number, curve: (t: number) => number = easeInOut): number {
  if (stops.length === 1) return stops[0][1];
  const clamped = Math.min(1, Math.max(0, t));
  for (let index = 0; index < stops.length - 1; index += 1) {
    const [fromAt, fromValue] = stops[index];
    const [toAt, toValue] = stops[index + 1];
    if (clamped <= toAt || index === stops.length - 2) {
      const span = toAt - fromAt;
      const local = span <= 0 ? 1 : Math.min(1, Math.max(0, (clamped - fromAt) / span));
      return fromValue + (toValue - fromValue) * curve(local);
    }
  }
  return stops[stops.length - 1][1];
}

/** Her body at one moment, about the bottom centre of her box: `y` in her
 * variant's design pixels, `rotation` in degrees. */
export interface BodyPose {
  y: number;
  rotation: number;
  scaleX: number;
  scaleY: number;
}

/** One of her loops: how long a cycle takes, and where it has her `t` of
 * the way through one. */
export interface BodyLoop {
  period: number;
  at: (t: number) => BodyPose;
}

/** Her pose's loop, or the expression playing in its place. */
export type BodyLoopName = "idle" | "speaking" | "thinking" | "listening" | "homeEager" | "homeHappy";

const flat: Stop[] = [
  [0, 0],
  [1, 0],
];
const unit: Stop[] = [
  [0, 1],
  [1, 1],
];

function tracks(period: number, y: Stop[], rotation: Stop[], scaleX: Stop[], scaleY: Stop[]): BodyLoop {
  return {
    period,
    at: (t) => ({ y: kf(y, t), rotation: kf(rotation, t), scaleX: kf(scaleX, t), scaleY: kf(scaleY, t) }),
  };
}

/** The design's body loops, in her variant's design pixels. */
const POSE_LOOPS = {
  /** `bobIdle 4.2s`: a slow float, up 8px and a touch larger at the top. */
  idle: tracks(
    4200,
    [
      [0, 0],
      [0.5, -8],
      [1, 0],
    ],
    flat,
    [
      [0, 1],
      [0.5, 1.008],
      [1, 1],
    ],
    [
      [0, 1],
      [0.5, 1.008],
      [1, 1],
    ],
  ),
  /** `bounceSpeak 0.85s`: up 13px at 30%, a 2px dip past rest at 60%. */
  speaking: tracks(
    850,
    [
      [0, 0],
      [0.3, -13],
      [0.6, 2],
      [1, 0],
    ],
    flat,
    [
      [0, 1],
      [0.3, 1.02],
      [0.6, 0.99],
      [1, 1],
    ],
    [
      [0, 1],
      [0.3, 0.985],
      [0.6, 1.012],
      [1, 1],
    ],
  ),
  /** `thinkSway 2.2s`: rocking from -1.4 to 1.4 degrees, lifting 3px between. */
  thinking: tracks(
    2200,
    [
      [0, 0],
      [0.5, -3],
      [1, 0],
    ],
    [
      [0, -1.4],
      [0.5, 1.4],
      [1, -1.4],
    ],
    unit,
    unit,
  ),
} satisfies Record<string, BodyLoop>;

/** From rest, up to 1 halfway through the loop, and back. */
const wave = (t: number) => (1 - Math.cos(t * 2 * Math.PI)) / 2;

/**
 * Ella Mobile's expressions that take over from her pose's loop. Their
 * lifts are in widths of her mouth, which gives them the same weight in
 * every placement; `mouth` turns them into her variant's pixels.
 */
const EXPRESSION_LOOPS: Record<"listening" | "homeEager" | "homeHappy", (mouth: number) => BodyLoop> = {
  /** "Your turn. She is all ears": a little lift and lean towards the
   * learner, and every few seconds a small nod. */
  listening: (mouth) => ({
    period: 3600,
    at: (t) => ({
      y: (-0.2 - 0.045 * wave(t)) * mouth,
      rotation: 2 + 0.8 * Math.sin(Math.PI * t) ** 8,
      scaleX: 1,
      scaleY: 1,
    }),
  }),
  /** "Ready for our next conversation?": a light bounce, swaying either way. */
  homeEager: (mouth) => ({
    period: 2200,
    at: (t) => ({
      y: -0.23 * wave(t) * mouth,
      rotation: 1.4 * Math.sin(t * 2 * Math.PI),
      scaleX: 1,
      scaleY: 1 + 0.016 * wave(t),
    }),
  }),
  /** "Lovely to see you again", after a talk: a slower, smaller sway. */
  homeHappy: (mouth) => ({
    period: 3200,
    at: (t) => ({
      y: -0.1 * wave(t) * mouth,
      rotation: 1.2 * Math.sin(t * 2 * Math.PI),
      scaleX: 1,
      scaleY: 1,
    }),
  }),
};

export function bodyLoop(name: BodyLoopName, mouth: number): BodyLoop {
  return name === "idle" || name === "speaking" || name === "thinking" ? POSE_LOOPS[name] : EXPRESSION_LOOPS[name](mouth);
}

const round = (value: number) => Number(value.toFixed(4));

/** A pose as a CSS transform, in the order Ella Mobile composes it. */
export function bodyTransform({ y, rotation, scaleX, scaleY }: BodyPose): string {
  return `translateY(${round(y)}px) rotate(${round(rotation)}deg) scale(${round(scaleX)}, ${round(scaleY)})`;
}

/** A loop as keyframes for `Element.animate`: sampled finely enough that
 * joining the samples straight reads as the curve itself. */
export function bodyKeyframes(loop: BodyLoop, samples = 60): Keyframe[] {
  return Array.from({ length: samples + 1 }, (_, index) => ({
    offset: index / samples,
    transform: bodyTransform(loop.at(index / samples)),
  }));
}

/** How long a change of loop takes to ease her from the one to the other:
 * her face's `transition: opacity 0.16s ease`, as on Ella Mobile. */
export const LOOP_HANDOFF_MS = 160;

/**
 * `pokeSquish`, as Ella Mobile plays it: its stops joined straight, under a
 * single `cubic-bezier(0.3, 1.2, 0.4, 1)` across the whole squish, about her
 * base.
 */
export const POKE = {
  duration: 600,
  easing: "cubic-bezier(0.3, 1.2, 0.4, 1)",
  keyframes: [
    { offset: 0, transform: "translateY(0px) scale(1, 1)" },
    { offset: 0.25, transform: "translateY(6px) scale(1.06, 0.9)" },
    { offset: 0.55, transform: "translateY(-16px) scale(0.95, 1.07)" },
    { offset: 0.8, transform: "translateY(2px) scale(1.03, 0.97)" },
    { offset: 1, transform: "translateY(0px) scale(1, 1)" },
  ] satisfies Keyframe[],
};

/** When every Ella on screen blinks and grins. */
export interface FaceClock {
  readonly blinking: boolean;
  readonly grinning: boolean;
  /** Runs `listener` at each blink and grin, and their ends; the clock only
   * ticks while something is listening. */
  subscribe: (listener: () => void) => () => void;
}

/**
 * One clock for all of them, so they blink and grin together, as on Ella
 * Mobile: a 140 ms blink every 2.6 to 6 seconds, and a 650 ms grin every 5
 * to 9.
 */
export function createFaceClock(random: () => number = () => Math.random()): FaceClock {
  const listeners = new Set<() => void>();
  const timers = new Set<number>();
  let blinking = false;
  let grinning = false;

  const notify = () => listeners.forEach((listener) => listener());
  const after = (ms: number, then: () => void) => {
    const timer = window.setTimeout(() => {
      timers.delete(timer);
      then();
    }, ms);
    timers.add(timer);
  };
  const scheduleBlink = () =>
    after(2600 + random() * 3400, () => {
      blinking = true;
      notify();
      after(140, () => {
        blinking = false;
        notify();
        scheduleBlink();
      });
    });
  const scheduleGrin = () =>
    after(5000 + random() * 4000, () => {
      grinning = true;
      notify();
      after(650, () => {
        grinning = false;
        notify();
        scheduleGrin();
      });
    });

  return {
    get blinking() {
      return blinking;
    },
    get grinning() {
      return grinning;
    },
    subscribe(listener) {
      listeners.add(listener);
      if (listeners.size === 1) {
        scheduleBlink();
        scheduleGrin();
      }
      return () => {
        listeners.delete(listener);
        if (listeners.size > 0) return;
        timers.forEach((timer) => window.clearTimeout(timer));
        timers.clear();
        blinking = false;
        grinning = false;
      };
    },
  };
}

export const faceClock = createFaceClock();
