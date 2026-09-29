import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, MouseEvent, ReactNode, RefObject } from "react";
import { mouthGeometry, type Box } from "../lib/mouth";
import type { SpeechMouth } from "../lib/speech";
import { lerpMouth, SMILE, SMILE_LIFT, VISEMES, type MouthShape } from "../lib/visemes";

export type EllaState = "resting" | "listening" | "thinking" | "speaking";

/**
 * Where Ella appears. The design draws her afresh for each placement, at its
 * own size and with its own ear, eye and mouth positions rather than one blob
 * under a scale, so each placement is a variant and `.ella--{variant}` in the
 * stylesheet carries its face.
 *
 *   welcome       the giant close-up behind "Hi buddy!"
 *   corner        peeking in from the bottom right: onboarding, boot, summary
 *   age           rising behind the age question once an age is typed
 *   conversation  the talk stage, where she listens, thinks and speaks
 *   home          peeking over the top of the "Today's talk" card
 *   mentor        bobbing in the corner of the mentor card on Talk partners
 *   profile       the learner's own Ella, in their colour, rising out of the
 *                 profile card and their level's page
 */
export type EllaVariant = "welcome" | "corner" | "age" | "conversation" | "home" | "mentor" | "profile";

/** How her ears stand: at rest, pricked up while she listens, or looming
 * after she has dived back in from above. */
export type EllaEars = "rest" | "listen" | "loom";

const BLOB: Record<EllaVariant, { width: number; height: number }> = {
  welcome: { width: 1100, height: 570 },
  corner: { width: 440, height: 320 },
  age: { width: 760, height: 420 },
  conversation: { width: 660, height: 450 },
  home: { width: 300, height: 210 },
  mentor: { width: 380, height: 200 },
  profile: { width: 190, height: 135 },
};

/** The placements whose smile widens into a grin now and then while resting. */
const GRINS = new Set<EllaVariant>(["welcome", "corner", "age", "home"]);

/** Where lip sync draws her mouth: the box `.ella--conversation` gives her
 * smile in the stylesheet, and its line. Only the talk stage speaks. */
const SPEECH_MOUTH: Partial<Record<EllaVariant, Box & { line: number }>> = {
  conversation: { x: 314, y: 86, width: 30, height: 13, line: 3 },
};

/** Speech takes her mouth over from the smile fast, since Piper can sound a
 * line's first phoneme 12 ms in, and gives it back over the face's fade. */
const HANDOFF_IN_MS = 60;
const HANDOFF_OUT_MS = 160;

/** Where a mascot flies in from when its screen opens, as a direction off-screen. */
const ENTRANCE_FROM: Array<[number, number]> = [
  [-1, 0.25],
  [1, 0.25],
  [0, 1],
  [-0.9, 0.9],
  [0.9, 0.9],
  [-0.8, -0.7],
  [0.8, -0.7],
  [0, -1],
];

export function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/**
 * Sends an element in from a random direction off-screen, the way every mascot
 * and talk partner arrives when a screen opens. It is put out of place before
 * the first paint, then released on the frame after.
 */
export function useEntrance(ref: RefObject<HTMLElement | null>, enabled = true) {
  useLayoutEffect(() => {
    const element = ref.current;
    if (!enabled || !element || prefersReducedMotion() || typeof requestAnimationFrame !== "function") {
      return;
    }
    const [x, y] = ENTRANCE_FROM[Math.floor(Math.random() * ENTRANCE_FROM.length)];
    const rotation = (Math.random() * 26 - 13).toFixed(1);
    element.style.transition = "none";
    element.style.transform = `translate(${x * 1000}px, ${y * 800}px) rotate(${rotation}deg)`;
    let release = 0;
    const arm = requestAnimationFrame(() => {
      release = requestAnimationFrame(() => {
        element.style.transition = "transform 0.82s cubic-bezier(0.22, 1.08, 0.36, 1)";
        element.style.transform = "translate(0px, 0px) rotate(0deg)";
      });
    });
    return () => {
      cancelAnimationFrame(arm);
      cancelAnimationFrame(release);
      element.style.transition = "";
      element.style.transform = "";
    };
  }, [ref, enabled]);
}

/** `cheer` closes her eyes into happy arcs and opens her mouth in a grin, as
 * the recap draws her. Only the conversation face has it. */
export type EllaMood = "calm" | "cheer";

interface EllaMascotProps {
  state?: EllaState;
  mood?: EllaMood;
  variant?: EllaVariant;
  /** Her body's colour, when she is the learner's own; purple otherwise. */
  color?: string;
  /** 1 renders the variant at its designed size. */
  scale?: number;
  ears?: EllaEars;
  /** Fly in from off-screen when she mounts. */
  entrance?: boolean;
  /** Squish when clicked. */
  pokeable?: boolean;
  /** Decorative instances defer announcements to a dedicated status region. */
  decorative?: boolean;
  /** What she is saying, for her mouth to follow. Only the talk stage has a
   * mouth that moves with her voice; elsewhere this is ignored. */
  speech?: SpeechMouth;
  className?: string;
  style?: CSSProperties;
  /** Controls that sit on her, like the first talk's microphone. They stay in
   * the accessibility tree even when she is decorative. */
  children?: ReactNode;
}

/**
 * Ella is drawn, not illustrated: a purple blob with two capsule ears, two eyes
 * that follow the cursor and blink, and a mouth that opens while she talks —
 * on the talk stage, into the shape of each sound she makes.
 */
export function EllaMascot({
  state = "resting",
  mood = "calm",
  variant = "corner",
  color,
  scale = 1,
  ears = "rest",
  entrance = true,
  pokeable = false,
  decorative = false,
  speech,
  className = "",
  style,
  children,
}: EllaMascotProps) {
  const entry = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLDivElement>(null);
  const leftEye = useRef<HTMLDivElement>(null);
  const rightEye = useRef<HTMLDivElement>(null);
  const mouth = useRef<HTMLSpanElement>(null);
  const pokeTimer = useRef(0);
  const speechBox = speech ? SPEECH_MOUTH[variant] : undefined;
  // Audio without Piper's timings, which her drawn mouth cannot follow.
  const [untimed, setUntimed] = useState(false);

  useEntrance(entry, entrance);

  useEffect(() => () => window.clearTimeout(pokeTimer.current), []);

  useEffect(() => {
    // Thinking closes her eyes into lines, and cheering into arcs, so there
    // is nothing to follow.
    if (prefersReducedMotion() || state === "thinking" || mood === "cheer") return;

    const eyes = [leftEye, rightEye];
    let pointerX: number | null = null;
    let pointerY: number | null = null;
    let blinking = false;

    const applyEyes = () => {
      for (const eye of eyes) {
        const element = eye.current;
        if (!element) continue;
        let dx = 0;
        let dy = 0;
        if (pointerX !== null && pointerY !== null) {
          const box = element.getBoundingClientRect();
          const cx = box.left + box.width / 2;
          const cy = box.top + box.height / 2;
          const distance = Math.min(5, Math.hypot(pointerX - cx, pointerY - cy) / 40);
          const angle = Math.atan2(pointerY - cy, pointerX - cx);
          dx = Math.cos(angle) * distance;
          dy = Math.sin(angle) * distance;
        }
        element.style.transform = `translate(${dx}px, ${dy}px) scaleY(${blinking ? 0.12 : 1})`;
      }
    };

    const onPointerMove = (event: globalThis.MouseEvent) => {
      pointerX = event.clientX;
      pointerY = event.clientY;
      applyEyes();
    };
    window.addEventListener("mousemove", onPointerMove);

    let blinkTimer = 0;
    let blinkCloseTimer = 0;
    const scheduleBlink = () => {
      blinkTimer = window.setTimeout(() => {
        blinking = true;
        applyEyes();
        blinkCloseTimer = window.setTimeout(() => {
          blinking = false;
          applyEyes();
          scheduleBlink();
        }, 140);
      }, 2600 + Math.random() * 3400);
    };
    scheduleBlink();

    let grinTimer = 0;
    let grinResetTimer = 0;
    const scheduleGrin = () => {
      grinTimer = window.setTimeout(() => {
        const element = mouth.current;
        if (element) {
          element.style.transform = "scale(1.4)";
          grinResetTimer = window.setTimeout(() => {
            element.style.transform = "";
          }, 650);
        }
        scheduleGrin();
      }, 5000 + Math.random() * 4000);
    };
    if (GRINS.has(variant) && state === "resting") scheduleGrin();

    return () => {
      window.removeEventListener("mousemove", onPointerMove);
      window.clearTimeout(blinkTimer);
      window.clearTimeout(blinkCloseTimer);
      window.clearTimeout(grinTimer);
      window.clearTimeout(grinResetTimer);
    };
  }, [state, variant, mood]);

  function poke(event: MouseEvent<HTMLDivElement>) {
    // A click on a control sitting on her, like the first talk's mic, is not a poke.
    if ((event.target as Element).closest("button")) return;
    const element = inner.current;
    if (!element || prefersReducedMotion()) return;
    // Dropping the animation and reading layout restarts it from the top, so a
    // second poke mid-squish squishes again.
    element.style.animation = "none";
    void element.offsetWidth;
    element.style.animation = "pokeSquish 0.6s cubic-bezier(0.3, 1.2, 0.4, 1)";
    window.clearTimeout(pokeTimer.current);
    pokeTimer.current = window.setTimeout(() => {
      element.style.animation = "";
    }, 640);
  }

  const label = {
    resting: "Ella is ready",
    listening: "Ella is listening",
    thinking: "Ella is thinking",
    speaking: "Ella is speaking",
  }[state];
  const classes = [
    "ella",
    `ella--${variant}`,
    `ella--${state}`,
    ears !== "rest" ? `ella--ears-${ears}` : "",
    mood === "cheer" ? "ella--cheer" : "",
    speechBox ? "ella--lipsync" : "",
    speechBox && untimed ? "is-untimed" : "",
    pokeable ? "is-pokeable" : "",
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className={classes}
      style={
        {
          "--ella-w": `${BLOB[variant].width}px`,
          "--ella-h": `${BLOB[variant].height}px`,
          "--ella-scale": scale,
          ...(color ? { "--ella-color": color } : {}),
          ...style,
        } as CSSProperties
      }
      onClick={pokeable ? poke : undefined}
    >
      <div ref={entry} className="ella__entry">
        <div className="ella__stage">
          <div
            ref={inner}
            className="ella__inner"
            role={decorative ? undefined : "img"}
            aria-label={decorative ? undefined : label}
            aria-hidden={decorative || undefined}
          >
            <span className="ella__ear ella__ear--left">
              <i />
            </span>
            <span className="ella__ear ella__ear--right">
              <i />
            </span>
            <span className="ella__body" />
            <div ref={leftEye} className="ella__eye ella__eye--left">
              <span className="ella__glint" />
            </div>
            <div ref={rightEye} className="ella__eye ella__eye--right">
              <span className="ella__glint" />
            </div>
            <span className="ella__eye-line ella__eye-line--left" />
            <span className="ella__eye-line ella__eye-line--right" />
            <span ref={mouth} className="ella__mouth ella__mouth--smile" />
            <span className="ella__mouth ella__mouth--open" />
            {mood === "cheer" && (
              <>
                <span className="ella__happy-eye ella__happy-eye--left" />
                <span className="ella__happy-eye ella__happy-eye--right" />
                <span className="ella__grin">
                  <i />
                </span>
              </>
            )}
            {speech && speechBox && (
              <SpeechMouthDrawing
                speech={speech}
                box={speechBox}
                stage={BLOB[variant]}
                state={state}
                onUntimed={setUntimed}
              />
            )}
          </div>
          {children && <div className="ella__slot">{children}</div>}
        </div>
      </div>
    </div>
  );
}

/**
 * Her mouth as one path, drawn the way Ella Mobile draws it: her U smile at
 * rest and, while a timed reply plays, the shape of each sound she is making,
 * read off the audio clock every frame. Speech grows out of the smile and
 * hands back to it, so the two never cut.
 */
function SpeechMouthDrawing({
  speech,
  box,
  stage,
  state,
  onUntimed,
}: {
  speech: SpeechMouth;
  box: Box & { line: number };
  /** The variant's own box, which the drawing covers at design size. */
  stage: { width: number; height: number };
  state: EllaState;
  onUntimed: (untimed: boolean) => void;
}) {
  const clip = useId().replace(/[^\w-]/g, "");
  const group = useRef<SVGGElement>(null);
  const lips = useRef<SVGPathElement>(null);
  const lipsClip = useRef<SVGPathElement>(null);
  const lowerTeeth = useRef<SVGRectElement>(null);
  const tongue = useRef<SVGEllipseElement>(null);
  const teethClip = useRef<SVGRectElement>(null);
  const teeth = useRef<SVGRectElement>(null);
  const speaking = useRef(state === "speaking");
  speaking.current = state === "speaking";
  const wake = useRef<() => void>(() => undefined);

  useLayoutEffect(() => {
    const reduce = prefersReducedMotion();
    let frame = 0;
    let last = 0;
    // How far the handoff has got, linearly in time, and which curve it runs
    // on. A curve keeps its direction until it settles, so a line cut off
    // while the mouth comes in turns back without a jump.
    let progress = 0;
    let rising = true;
    let held: MouthShape | null = null;
    let untimed = false;

    const draw = (shape: MouthShape, weight: number) => {
      const geometry = mouthGeometry(shape, box.width, box.line);
      const x = box.x + box.width / 2;
      // The smile sits a little higher, to reach the U's arms.
      const y = box.y + box.height / 2 - SMILE_LIFT * box.width * (1 - weight);
      const turn = (geometry.skew * 180) / Math.PI;
      group.current?.setAttribute("transform", `translate(${x} ${y.toFixed(3)}) rotate(${turn.toFixed(3)})`);
      lips.current?.setAttribute("d", geometry.path);
      lips.current?.setAttribute("stroke-width", `${geometry.outline}`);
      lipsClip.current?.setAttribute("d", geometry.path);
      place(lowerTeeth.current, geometry.lowerTeeth);
      place(teethClip.current, geometry.teeth?.clip ?? null);
      place(teeth.current, geometry.teeth);
      teeth.current?.setAttribute("rx", `${geometry.teeth?.radius ?? 0}`);
      const tip = tongue.current;
      if (!tip) return;
      if (!geometry.tongue) {
        tip.setAttribute("display", "none");
        return;
      }
      tip.removeAttribute("display");
      tip.setAttribute("cx", `${geometry.tongue.cx}`);
      tip.setAttribute("cy", `${geometry.tongue.cy}`);
      tip.setAttribute("rx", `${geometry.tongue.rx}`);
      tip.setAttribute("ry", `${geometry.tongue.ry}`);
    };

    const tick = (now: number) => {
      frame = 0;
      const elapsed = last ? now - last : 0;
      last = now;
      const talking = speaking.current;
      const speaks = talking && speech.started;
      const target = speaks ? 1 : 0;
      if (reduce) {
        progress = target;
      } else if (progress !== target) {
        if (progress === 0 || progress === 1) rising = target === 1;
        progress =
          target === 1
            ? Math.min(1, progress + elapsed / HANDOFF_IN_MS)
            : Math.max(0, progress - elapsed / HANDOFF_OUT_MS);
      }
      const weight = rising ? easeOut(progress) : easeInOut(progress);
      if (speech.started) held = speech.shape();
      if (weight > 0 || speaks) {
        // Back into her smile as the weight falls, so a line cut off mid-vowel
        // closes rather than ghosting over it.
        draw(lerpMouth(SMILE, held ?? VISEMES.rest, weight), weight);
      } else {
        held = null;
        draw(SMILE, 0);
      }
      const nowUntimed = talking && speech.timed === false;
      if (nowUntimed !== untimed) {
        untimed = nowUntimed;
        onUntimed(untimed);
      }
      if (talking || progress > 0) frame = requestAnimationFrame(tick);
      else last = 0;
    };

    wake.current = () => {
      if (!frame) frame = requestAnimationFrame(tick);
    };
    draw(SMILE, 0);
    wake.current();
    return () => {
      cancelAnimationFrame(frame);
      wake.current = () => undefined;
    };
  }, [speech, box, onUntimed]);

  // Talking starts the frames; they stop by themselves once she is back to
  // her smile.
  useEffect(() => wake.current(), [state]);

  return (
    <svg
      className="ella__speech"
      width={stage.width}
      height={stage.height}
      viewBox={`0 0 ${stage.width} ${stage.height}`}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <clipPath id={`${clip}-lips`}>
          <path ref={lipsClip} />
        </clipPath>
        <clipPath id={`${clip}-teeth`}>
          <rect ref={teethClip} />
        </clipPath>
      </defs>
      <g ref={group}>
        <path ref={lips} className="ella__lips" />
        {/* Upper teeth go last: the tongue sits behind them. */}
        <g clipPath={`url(#${clip}-lips)`}>
          <rect ref={lowerTeeth} className="ella__teeth" />
          <ellipse ref={tongue} className="ella__tongue" />
          <g clipPath={`url(#${clip}-teeth)`}>
            <rect ref={teeth} className="ella__teeth" />
          </g>
        </g>
      </g>
    </svg>
  );
}

function place(element: SVGRectElement | null, box: Box | null) {
  if (!element) return;
  if (!box) {
    element.setAttribute("display", "none");
    return;
  }
  element.removeAttribute("display");
  element.setAttribute("x", `${box.x}`);
  element.setAttribute("y", `${box.y}`);
  element.setAttribute("width", `${box.width}`);
  element.setAttribute("height", `${box.height}`);
}

/** A CSS `cubic-bezier` timing function, solved for its progress. */
function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
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

/** Flutter's `Curves.easeOut` and `Curves.easeInOut`, as Ella Mobile hands over. */
const easeOut = cubicBezier(0, 0, 0.58, 1);
const easeInOut = cubicBezier(0.42, 0, 0.58, 1);

/** The learner's own little blob, in the colour they picked on their profile. */
export function LearnerAvatar({ color, size = "sm" }: { color: string; size?: "sm" | "lg" }) {
  return (
    <span
      className={`learner-avatar learner-avatar--${size}`}
      style={{ "--avatar": color } as CSSProperties}
      aria-hidden="true"
    >
      <span className="learner-avatar__blob">
        <i className="learner-avatar__ear learner-avatar__ear--left" />
        <i className="learner-avatar__ear learner-avatar__ear--right" />
        <i className="learner-avatar__body" />
        <i className="learner-avatar__eye learner-avatar__eye--left" />
        <i className="learner-avatar__eye learner-avatar__eye--right" />
        {size === "lg" && <i className="learner-avatar__mouth" />}
      </span>
    </span>
  );
}

/** Static brand mark used beside the Ella wordmark. */
export function EllaGlyph({ size = 40 }: { size?: number }) {
  return (
    <span className="ella-glyph" style={{ "--glyph": `${size}px` } as CSSProperties} aria-hidden="true">
      <img className="ella-glyph__image" src="/assets/ella/ella-logo.svg" alt="" draggable={false} />
    </span>
  );
}

/** Five bars that rise and fall while Ella speaks. */
export function SpeakingWave() {
  return (
    <div className="wave" aria-hidden="true">
      {[0, 1, 2, 3, 4].map((index) => (
        <span key={index} style={{ animationDelay: `${index * 0.15}s` }} />
      ))}
    </div>
  );
}

/** Three dots that blink while Ella thinks. */
export function ThinkingDots() {
  return (
    <div className="dots" aria-hidden="true">
      {[0, 1, 2].map((index) => (
        <span key={index} style={{ animationDelay: `${index * 0.2}s` }} />
      ))}
    </div>
  );
}

/** Live input level, shown under the mic while the learner is speaking. */
export function VoiceMeter({ level }: { level: number }) {
  return (
    <div className="voice-meter" aria-hidden="true">
      {[0.25, 0.5, 0.8, 0.45, 0.65].map((weight, index) => (
        <span
          key={weight}
          style={
            {
              "--voice-scale": Math.max(0.22, Math.min(1, level * 1.8 + weight * 0.2)),
              "--voice-delay": `${index * 45}ms`,
            } as CSSProperties
          }
        />
      ))}
    </div>
  );
}
