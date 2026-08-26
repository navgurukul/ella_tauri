import type { CSSProperties } from "react";

export type ZoeState = "resting" | "listening" | "thinking" | "speaking";

interface ZoeMascotProps {
  state?: ZoeState;
  size?: "small" | "medium" | "large";
}

export function ZoeMascot({ state = "resting", size = "medium" }: ZoeMascotProps) {
  const label = {
    resting: "Zoe is ready",
    listening: "Zoe is listening",
    thinking: "Zoe is thinking",
    speaking: "Zoe is speaking",
  }[state];

  return (
    <div className={`zoe-mascot zoe-mascot--${size} zoe-mascot--${state}`} aria-label={label}>
      <div className="zoe-glow" />
      <img
        className="zoe-frame zoe-frame--closed"
        src="/assets/zoe/mascot-mouthclose.svg"
        alt=""
        draggable={false}
      />
      <img
        className="zoe-frame zoe-frame--open"
        src="/assets/zoe/mascot-open.svg"
        alt=""
        draggable={false}
      />
      {state === "listening" && (
        <div className="listening-rings" aria-hidden="true">
          <span />
          <span />
          <span />
        </div>
      )}
    </div>
  );
}

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
