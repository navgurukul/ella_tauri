import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, MouseEvent, ReactNode, RefObject } from "react";
import {
  bodyKeyframes,
  bodyLoop,
  bodyTransform,
  easeInOut,
  easeOut,
  faceClock,
  LOOP_HANDOFF_MS,
  POKE,
  type BodyLoopName,
} from "../lib/motion";
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

/** How her ears stand: at rest, or lifted a little and twitching now and
 * then while she listens on the talk stage. */
export type EllaEars = "rest" | "listen";

/**
 * Ella Mobile's expressions that change how she holds herself, in place of
 * her pose's loop. `listening` comes by itself whenever she listens; Home
 * picks `homeEager`, or `homeHappy` once a talk has been finished.
 */
export type EllaExpression = "listening" | "homeEager" | "homeHappy";

/** Each variant's design size, and the width of her mouth there, which an
 * expression's lifts are measured in. */
const BLOB: Record<EllaVariant, { width: number; height: number; mouth: number }> = {
  welcome: { width: 1100, height: 570, mouth: 64 },
  corner: { width: 440, height: 320, mouth: 32 },
  age: { width: 760, height: 420, mouth: 52 },
  conversation: { width: 660, height: 450, mouth: 30 },
  home: { width: 300, height: 210, mouth: 24 },
  mentor: { width: 380, height: 200, mouth: 26 },
  profile: { width: 190, height: 135, mouth: 35 },
};

/** Her pose's loop: `bobIdle`, `bounceSpeak` and `thinkSway`. Listening
 * always wears its expression instead. */
const POSE_LOOP: Record<EllaState, BodyLoopName> = {
  resting: "idle",
  listening: "listening",
  thinking: "thinking",
  speaking: "speaking",
};

/** The learner's own Ella on the profile keeps her body still, as Ella
 * Mobile's avatar does; only her eyes move. */
const STILL = new Set<EllaVariant>(["profile"]);

/** The placements whose smile widens into a grin now and then, as on Ella
 * Mobile. Home wears an expression instead, which never grins, and the
 * learner's own Ella only blinks. */
const GRINS = new Set<EllaVariant>(["welcome", "corner", "age", "conversation", "mentor"]);

/** Where lip sync draws her mouth: the box `.ella--conversation` gives her
 * smile in the stylesheet, and its line. Only the talk stage speaks. */
const SPEECH_MOUTH: Partial<Record<EllaVariant, Box & { line: number }>> = {
  conversation: { x: 314, y: 86, width: 30, height: 13, line: 3 },
};

/** Speech takes her mouth over from the smile fast, since Piper can sound a
 * line's first phoneme 12 ms in, and gives it back over the face's fade. */
const HANDOFF_IN_MS = 60;
const HANDOFF_OUT_MS = 160;

export function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/**
 * Plays her body's loop on `target`, easing her over from wherever the loop
 * before had got to, as Ella Mobile does: without it, being cut off into
 * listening would lift and tilt her in a single frame. A loop starts from its
 * own first frame, so the handoff aims for where the new loop will be once it
 * is over. With reduced motion she holds the loop's first frame, which for
 * listening is the lean itself.
 */
function useBodyMotion(target: RefObject<HTMLElement | null>, name: BodyLoopName | null, mouth: number) {
  const moving = useRef(false);

  useLayoutEffect(() => {
    const element = target.current;
    if (!element) return;
    const playing = typeof element.getAnimations === "function" ? element.getAnimations() : [];
    // Read before cancelling: the loop being replaced still holds her pose.
    const from = moving.current && playing.length > 0 ? getComputedStyle(element).transform : "none";
    playing.forEach((animation) => animation.cancel());
    moving.current = false;
    if (!name) {
      element.style.transform = "";
      return;
    }
    const loop = bodyLoop(name, mouth);
    if (prefersReducedMotion() || typeof element.animate !== "function") {
      element.style.transform = bodyTransform(loop.at(0));
      return;
    }
    element.style.transform = "";
    element.animate(bodyKeyframes(loop), { duration: loop.period, iterations: Infinity });
    // Made after the loop, so it plays over it until the two meet.
    if (from && from !== "none") {
      element.animate(
        [{ transform: from }, { transform: bodyTransform(loop.at(LOOP_HANDOFF_MS / loop.period)) }],
        { duration: LOOP_HANDOFF_MS, easing: "ease" },
      );
    }
    moving.current = true;
  }, [target, name, mouth]);

  useEffect(() => {
    const element = target.current;
    return () => element?.getAnimations?.().forEach((animation) => animation.cancel());
  }, [target]);
}

/**
 * Her eyes follow the pointer, and she blinks and grins with every other
 * Ella on screen (`faceClock`). While she listens she looks straight out at
 * the learner instead, as on Ella Mobile.
 */
function useFaceLife({
  root,
  eyes,
  lively,
  eyesShown,
  gazes,
  grins,
}: {
  root: RefObject<HTMLElement | null>;
  eyes: Array<RefObject<HTMLElement | null>>;
  lively: boolean;
  eyesShown: boolean;
  gazes: boolean;
  grins: boolean;
}) {
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const apply = useRef<() => void>(() => undefined);
  apply.current = () => {
    if (eyesShown) {
      for (const eye of eyes) {
        const element = eye.current;
        if (!element) continue;
        let dx = 0;
        let dy = 0;
        const at = pointer.current;
        if (gazes && at) {
          const box = element.getBoundingClientRect();
          const cx = box.left + box.width / 2;
          const cy = box.top + box.height / 2;
          const distance = Math.min(5, Math.hypot(at.x - cx, at.y - cy) / 40);
          const angle = Math.atan2(at.y - cy, at.x - cx);
          dx = Math.cos(angle) * distance;
          dy = Math.sin(angle) * distance;
        }
        element.style.transform = `translate(${dx}px, ${dy}px) scaleY(${faceClock.blinking ? 0.12 : 1})`;
      }
    }
    // A data attribute rather than a class, which React would drop the next
    // time it writes her classes.
    const element = root.current;
    if (!element) return;
    if (grins && faceClock.grinning) element.dataset.grinning = "";
    else delete element.dataset.grinning;
  };

  useLayoutEffect(() => apply.current(), [eyesShown, gazes, grins]);

  useEffect(() => {
    if (!lively) return;
    const onPointerMove = (event: globalThis.MouseEvent) => {
      pointer.current = { x: event.clientX, y: event.clientY };
      apply.current();
    };
    window.addEventListener("mousemove", onPointerMove);
    const unsubscribe = faceClock.subscribe(() => apply.current());
    return () => {
      window.removeEventListener("mousemove", onPointerMove);
      unsubscribe();
    };
  }, [lively]);
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
  /** How she holds herself in place of her pose's loop. Listening needs no
   * asking. */
  expression?: EllaExpression;
  /** Whether her face follows what she is doing: bars for eyes while she
   * thinks, and the design's "o" for a voice her mouth cannot follow. The
   * first talk keeps her smile and open eyes throughout, as Ella Mobile's
   * does. */
  faceFollowsState?: boolean;
  /** False where what holds her moves her instead, as the recap's rise and
   * hops do: no loop, blink, gaze, grin or squish of her own. */
  animate?: boolean;
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
 *
 * She moves as Ella Mobile moves her, nested as the design nests her: the
 * corner peek outermost, then her variant's scale, the body loop for her pose
 * or expression (`useBodyMotion`), and the poke's squish inside that, so a
 * prod never stops her bobbing.
 */
export function EllaMascot({
  state = "resting",
  mood = "calm",
  variant = "corner",
  color,
  scale = 1,
  ears = "rest",
  expression,
  faceFollowsState = true,
  animate = true,
  pokeable = false,
  decorative = false,
  speech,
  className = "",
  style,
  children,
}: EllaMascotProps) {
  const root = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLDivElement>(null);
  const squish = useRef<HTMLDivElement>(null);
  const squishing = useRef<Animation | null>(null);
  const leftEye = useRef<HTMLDivElement>(null);
  const rightEye = useRef<HTMLDivElement>(null);
  const speechBox = speech ? SPEECH_MOUTH[variant] : undefined;
  // Audio without Piper's timings, which her drawn mouth cannot follow.
  const [untimed, setUntimed] = useState(false);

  // The face she wears, which in the first talk stays her resting one.
  const face: EllaState = faceFollowsState ? state : "resting";
  const shown = expression ?? (state === "listening" ? "listening" : null);
  const loop = !animate || STILL.has(variant) ? null : (shown ?? POSE_LOOP[state]);
  useBodyMotion(inner, loop, BLOB[variant].mouth);

  const lively = animate && !prefersReducedMotion();
  // Thinking closes her eyes into lines, and cheering into arcs, so there is
  // nothing to follow.
  const eyesShown = lively && face !== "thinking" && mood !== "cheer";
  useFaceLife({
    root,
    eyes: [leftEye, rightEye],
    lively,
    eyesShown,
    gazes: eyesShown && shown !== "listening",
    // Only her own smile grins: not while her voice has her mouth, nor while
    // an expression holds her face.
    grins: eyesShown && GRINS.has(variant) && face === "resting" && state !== "speaking" && !shown,
  });

  useEffect(() => () => squishing.current?.cancel(), []);

  function poke(event: MouseEvent<HTMLDivElement>) {
    // A click on a control sitting on her, like the first talk's mic, is not a poke.
    if ((event.target as Element).closest("button")) return;
    const element = squish.current;
    if (!element || !lively || typeof element.animate !== "function") return;
    // From the top again, so a second poke mid-squish squishes again.
    squishing.current?.cancel();
    squishing.current = element.animate(POKE.keyframes, { duration: POKE.duration, easing: POKE.easing });
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
    `ella--${face}`,
    ears !== "rest" ? `ella--ears-${ears}` : "",
    shown ? `ella--expression-${shown}` : "",
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
      ref={root}
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
      <div className="ella__peek">
        <div className="ella__stage">
          <div
            ref={inner}
            className="ella__inner"
            role={decorative ? undefined : "img"}
            aria-label={decorative ? undefined : label}
            aria-hidden={decorative || undefined}
          >
            <div ref={squish} className="ella__squish">
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
              <span className="ella__mouth ella__mouth--smile" />
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
      {/* A grin widens her smile about the middle of its box, as the
          stylesheet's smile grows on every other face. */}
      <g
        className="ella__speech-grin"
        style={{ transformOrigin: `${box.x + box.width / 2}px ${box.y + box.height / 2}px` }}
      >
        <g ref={group} className="ella__speech-mouth">
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
