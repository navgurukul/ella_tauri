import { useId } from "react";
import type { CSSProperties, ReactNode } from "react";
import { mouthGeometry, type Box } from "../lib/mouth";
import { SMILE_LIFT, type MouthShape } from "../lib/visemes";

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
 *   mentor        in the corner of the mentor card on Talk partners
 *   profile       the learner's own Ella, in their colour, rising out of the
 *                 profile card and their level's page
 */
export type EllaVariant = "welcome" | "corner" | "age" | "conversation" | "home" | "mentor" | "profile";

/** How her ears stand: at rest, or lifted a little while she listens on the
 * talk stage. */
export type EllaEars = "rest" | "listen";

/**
 * Ella Mobile's expressions that change how she holds her ears. `listening`
 * comes by itself whenever she listens; Home picks `homeEager`, or
 * `homeHappy` once a talk has been finished.
 */
export type EllaExpression = "listening" | "homeEager" | "homeHappy";

/** Each variant's design size. */
const BLOB: Record<EllaVariant, { width: number; height: number }> = {
  welcome: { width: 1100, height: 570 },
  corner: { width: 440, height: 320 },
  age: { width: 760, height: 420 },
  conversation: { width: 660, height: 450 },
  home: { width: 300, height: 210 },
  mentor: { width: 380, height: 200 },
  profile: { width: 190, height: 135 },
};

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
  /** How she holds her ears. Listening needs no asking. */
  expression?: EllaExpression;
  /** Whether her face follows what she is doing: bars for eyes while she
   * thinks, and the design's "o" while she speaks. The first talk keeps her
   * smile and open eyes throughout, as Ella Mobile's does. */
  faceFollowsState?: boolean;
  /** Decorative instances defer announcements to a dedicated status region. */
  decorative?: boolean;
  className?: string;
  style?: CSSProperties;
  /** Controls that sit on her, like the first talk's microphone. They stay in
   * the accessibility tree even when she is decorative. */
  children?: ReactNode;
}

/**
 * Ella is drawn, not illustrated: a purple blob with two capsule ears, two
 * eyes, and a mouth that opens into the design's "o" while she talks.
 *
 * She holds still. Every pose is a class the stylesheet draws, and changes
 * only when what she is doing does: nothing of hers runs on a clock, so on a
 * laptop whose CPU is also running the model, the conversation gets all of
 * it.
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
  decorative = false,
  className = "",
  style,
  children,
}: EllaMascotProps) {
  // The face she wears, which in the first talk stays her resting one.
  const face: EllaState = faceFollowsState ? state : "resting";
  const shown = expression ?? (state === "listening" ? "listening" : null);

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
    >
      <div className="ella__peek">
        <div className="ella__stage">
          <div
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
            <div className="ella__eye ella__eye--left">
              <span className="ella__glint" />
            </div>
            <div className="ella__eye ella__eye--right">
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
          </div>
          {children && <div className="ella__slot">{children}</div>}
        </div>
      </div>
    </div>
  );
}

/**
 * One mouth, drawn once in the path Ella Mobile draws every mouth with: a talk
 * partner's resting smile, or the plain open mouth they talk with. It is drawn
 * again only when the shape changes, never frame by frame.
 */
export function MouthDrawing({
  shape,
  open,
  box,
  stage,
}: {
  shape: MouthShape;
  /** An open mouth sits on its box's centre; a smile a little higher, to
   * reach the U's arms. */
  open: boolean;
  box: Box & { line: number };
  /** The drawing's own box, which the mouth covers at design size. */
  stage: { width: number; height: number };
}) {
  const clip = useId().replace(/[^\w-]/g, "");
  const geometry = mouthGeometry(shape, box.width, box.line);
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2 - (open ? 0 : SMILE_LIFT * box.width);
  const turn = (geometry.skew * 180) / Math.PI;
  const { lowerTeeth, tongue, teeth } = geometry;
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
          <path d={geometry.path} />
        </clipPath>
        {teeth && (
          <clipPath id={`${clip}-teeth`}>
            <rect x={teeth.clip.x} y={teeth.clip.y} width={teeth.clip.width} height={teeth.clip.height} />
          </clipPath>
        )}
      </defs>
      <g className="ella__speech-mouth" transform={`translate(${x} ${y.toFixed(3)}) rotate(${turn.toFixed(3)})`}>
        <path className="ella__lips" d={geometry.path} strokeWidth={geometry.outline} />
        {/* Upper teeth go last: the tongue sits behind them. */}
        <g clipPath={`url(#${clip}-lips)`}>
          {lowerTeeth && (
            <rect
              className="ella__teeth"
              x={lowerTeeth.x}
              y={lowerTeeth.y}
              width={lowerTeeth.width}
              height={lowerTeeth.height}
            />
          )}
          {tongue && <ellipse className="ella__tongue" cx={tongue.cx} cy={tongue.cy} rx={tongue.rx} ry={tongue.ry} />}
          {teeth && (
            <g clipPath={`url(#${clip}-teeth)`}>
              <rect
                className="ella__teeth"
                x={teeth.x}
                y={teeth.y}
                width={teeth.width}
                height={teeth.height}
                rx={teeth.radius}
              />
            </g>
          )}
        </g>
      </g>
    </svg>
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

/** Five bars, a sound mark, shown while Ella speaks. */
export function SpeakingWave() {
  return (
    <div className="wave" aria-hidden="true">
      {[0, 1, 2, 3, 4].map((index) => (
        <span key={index} />
      ))}
    </div>
  );
}

/** Three dots shown while Ella thinks. */
export function ThinkingDots() {
  return (
    <div className="dots" aria-hidden="true">
      {[0, 1, 2].map((index) => (
        <span key={index} />
      ))}
    </div>
  );
}

/** Live input level, shown under the mic while the learner checks it. */
export function VoiceMeter({ level }: { level: number }) {
  return (
    <div className="voice-meter" aria-hidden="true">
      {[0.25, 0.5, 0.8, 0.45, 0.65].map((weight) => (
        <span
          key={weight}
          style={{ "--voice-scale": Math.max(0.22, Math.min(1, level * 1.8 + weight * 0.2)) } as CSSProperties}
        />
      ))}
    </div>
  );
}
