import { useEffect, useLayoutEffect, useRef } from "react";
import type { CSSProperties, MouseEvent, ReactNode, RefObject } from "react";

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
 */
export type EllaVariant = "welcome" | "corner" | "age" | "conversation" | "home" | "mentor";

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
};

/** The placements whose smile widens into a grin now and then while resting. */
const GRINS = new Set<EllaVariant>(["welcome", "corner", "age", "home"]);

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

interface EllaMascotProps {
  state?: EllaState;
  variant?: EllaVariant;
  /** 1 renders the variant at its designed size. */
  scale?: number;
  ears?: EllaEars;
  /** Fly in from off-screen when she mounts. */
  entrance?: boolean;
  /** Squish when clicked. */
  pokeable?: boolean;
  /** Decorative instances defer announcements to a dedicated status region. */
  decorative?: boolean;
  className?: string;
  style?: CSSProperties;
  /** Controls that sit on her, like the first talk's microphone. They stay in
   * the accessibility tree even when she is decorative. */
  children?: ReactNode;
}

/**
 * Ella is drawn, not illustrated: a purple blob with two capsule ears, two eyes
 * that follow the cursor and blink, and a mouth that opens while she talks.
 */
export function EllaMascot({
  state = "resting",
  variant = "corner",
  scale = 1,
  ears = "rest",
  entrance = true,
  pokeable = false,
  decorative = false,
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

  useEntrance(entry, entrance);

  useEffect(() => () => window.clearTimeout(pokeTimer.current), []);

  useEffect(() => {
    // Thinking closes her eyes into lines, so there is nothing to follow.
    if (prefersReducedMotion() || state === "thinking") return;

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
  }, [state, variant]);

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
          </div>
          {children && <div className="ella__slot">{children}</div>}
        </div>
      </div>
    </div>
  );
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
