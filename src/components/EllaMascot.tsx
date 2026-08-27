import { useEffect, useRef } from "react";
import type { CSSProperties } from "react";

export type EllaState = "resting" | "listening" | "thinking" | "speaking";
export type EllaReaction = "success" | "error" | null;

/**
 * Ella is drawn, not illustrated: a purple blob with two capsule ears, two eyes
 * that follow the cursor, and a mouth that swaps shape while she talks.
 *
 * Four visual roles share one component. `peek` is the compact corner mascot;
 * `conversation` is the calm placement-flow face used throughout voice calls;
 * `hero` is the wider onboarding close-up; and `celebration` is the larger,
 * logo-derived expression used when feedback states need to feel unmistakable.
 */
export type EllaVariant = "peek" | "conversation" | "hero" | "celebration";

const BLOB: Record<EllaVariant, { width: number; height: number }> = {
  peek: { width: 660, height: 450 },
  conversation: { width: 660, height: 450 },
  hero: { width: 920, height: 560 },
  celebration: { width: 660, height: 450 },
};

interface EllaMascotProps {
  state?: EllaState;
  variant?: EllaVariant;
  /** 1 renders the variant at its intrinsic size. */
  scale?: number;
  /** Extra rotation, in degrees, for the corner-peeking variants. */
  rotate?: number;
  /** Transient feedback that can overlap the current operational state. */
  reaction?: EllaReaction;
  /** Normalised microphone activity for voice-responsive variants. */
  activity?: number;
  /** Decorative instances defer announcements to a dedicated status region. */
  decorative?: boolean;
  className?: string;
  style?: CSSProperties;
  children?: React.ReactNode;
}

export function EllaMascot({
  state = "resting",
  variant = "peek",
  scale = 1,
  rotate = 0,
  reaction = null,
  activity = 0,
  decorative = false,
  className = "",
  style,
  children,
}: EllaMascotProps) {
  const leftEye = useRef<HTMLDivElement>(null);
  const rightEye = useRef<HTMLDivElement>(null);
  const mouth = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    if (reduceMotion || state !== "resting" || variant === "celebration") return;

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

    const onPointerMove = (event: MouseEvent) => {
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
    scheduleGrin();

    return () => {
      window.removeEventListener("mousemove", onPointerMove);
      window.clearTimeout(blinkTimer);
      window.clearTimeout(blinkCloseTimer);
      window.clearTimeout(grinTimer);
      window.clearTimeout(grinResetTimer);
    };
  }, [state, variant]);

  const label = {
    resting: "Ella is ready",
    listening: "Ella is listening",
    thinking: "Ella is thinking",
    speaking: "Ella is speaking",
  }[state];
  const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  const motionActivity = reduceMotion || state !== "listening" ? 0 : Math.max(0, Math.min(1, activity));

  return (
    <div
      className={`ella ella--${variant} ella--${state} ${reaction ? `ella--reaction-${reaction}` : ""} ${className}`.trim()}
      style={
        {
          "--ella-w": `${BLOB[variant].width}px`,
          "--ella-h": `${BLOB[variant].height}px`,
          "--ella-scale": scale,
          "--ella-rotate": `${rotate}deg`,
          "--ella-activity": motionActivity,
          ...style,
        } as CSSProperties
      }
      role={decorative ? undefined : "img"}
      aria-label={decorative ? undefined : label}
      aria-hidden={decorative || undefined}
    >
      <div className="ella__stage">
        <span className="ella__ear ella__ear--left" />
        <span className="ella__ear ella__ear--right" />
        <span className="ella__body" />
        <div ref={leftEye} className="ella__eye ella__eye--left">
          <span className="ella__glint" />
        </div>
        <div ref={rightEye} className="ella__eye ella__eye--right">
          <span className="ella__glint" />
        </div>
        <span className="ella__eye-line ella__eye-line--left" />
        <span className="ella__eye-line ella__eye-line--right" />
        <span className="ella__brow ella__brow--left" />
        <span className="ella__brow ella__brow--right" />
        <span ref={mouth} className="ella__mouth ella__mouth--smile" />
        <span className="ella__mouth ella__mouth--open" />
        <span className="ella__mouth ella__mouth--worry" />
        <span className="ella__cheek ella__cheek--left" />
        <span className="ella__cheek ella__cheek--right" />
        <span className="ella__sparkle ella__sparkle--left" />
        <span className="ella__sparkle ella__sparkle--right" />
        {children && <div className="ella__slot">{children}</div>}
      </div>
    </div>
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
