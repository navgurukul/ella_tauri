import { useId } from "react";
import { MouthDrawing, type EllaState } from "./EllaMascot";
import { castName } from "../lib/presentation";
import { SMILE, VISEMES, type MouthShape } from "../lib/visemes";
import type { CastId } from "../types";

const STAGE = { width: 660, height: 450 };
const MOUTH = { x: 307, y: 103, width: 46, height: 18, line: 3.5 };
/** The plain open mouth a partner talks with. */
const TALKING_MOUTH = { ...VISEMES.ah, height: 0.32, smile: 0.2, tongue: 0 };

/** Complete, rounded silhouettes. Each body continues below the stage while
 * its face is composed around the microphone's central axis. */
const BODIES: Record<CastId, string> = {
  "stall-owner": `M35 470 C14 362 16 224 81 117
    C131 37 224 -8 330 -8 C436 -8 529 37 579 117
    C644 224 646 362 625 470 Z`,
  landlord: `M48 470 C23 368 31 246 96 202
    C124 183 153 182 177 177 C207 171 219 125 230 86
    C248 19 301 -8 355 5 C430 13 472 77 496 140
    C518 197 555 218 580 272 C617 323 625 397 610 470 Z`,
  doctor: `M43 470 C23 352 50 222 116 132 C159 74 214 46 262 26
    C242 10 238 1 254 -3 C268 -11 283 0 286 24
    C275 -6 279 -27 294 -28 C310 -28 317 3 309 32
    C380 20 518 35 580 138 C626 220 639 349 619 470 Z`,
  debater: `M44 470 C21 297 70 157 185 87 C213 63 245 34 270 12
    C279 -23 296 -32 310 -22 C323 -13 310 9 306 20
    C331 -7 349 -19 364 -10 C379 0 365 19 355 33
    C380 18 400 13 418 31 C501 99 636 229 616 470 Z`,
};

const SMILES: Record<CastId, MouthShape> = {
  "stall-owner": { ...SMILE, width: 1.12, smile: 0.85 },
  landlord: { ...SMILE, width: 0.9, smile: 0.08, skew: -0.025 },
  doctor: { ...SMILE, width: 0.9, smile: 0.55, skew: -0.08 },
  debater: { ...SMILE, width: 1.04, smile: 0.65, skew: -0.13 },
};

const BROWS: Record<CastId, readonly [string, string]> = {
  "stall-owner": ["M-13 -22 Q0 -28 13 -22", "M-13 -22 Q0 -28 13 -22"],
  landlord: ["M-17 -29 Q0 -28 17 -21", "M-17 -21 Q0 -28 17 -29"],
  doctor: ["M-14 -19 Q0 -24 14 -18", "M-14 -23 Q0 -28 14 -21"],
  debater: ["M-16 -23 L16 -29", "M-17 -27 Q0 -38 17 -27"],
};

/** How a partner holds their face in each state: listening, they lift and
 * lean in with their eyes open wide; thinking, they glance up and away with
 * their brows knitted. Each is a held pose, changed only with the state. */
interface Pose {
  y: number;
  turn: number;
  gazeX: number;
  gazeY: number;
  openness: number;
  browLift: number;
  browTilt: number;
  browAsymmetry: number;
}

const AT_REST: Pose = { y: 0, turn: 0, gazeX: 0, gazeY: 0, openness: 1, browLift: 0, browTilt: 0, browAsymmetry: 0 };
const POSES: Record<EllaState, Pose> = {
  resting: AT_REST,
  speaking: AT_REST,
  listening: { y: -3, turn: 0.55, gazeX: 0, gazeY: 0, openness: 1.09, browLift: -2.5, browTilt: -2, browAsymmetry: 0 },
  thinking: { y: -1, turn: -0.7, gazeX: -2.6, gazeY: -3.2, openness: 0.88, browLift: -1, browTilt: 5, browAsymmetry: 2 },
};

/** Listening and thinking move the same eyes instead of replacing the face. */
function eyeOutline(id: CastId, right: boolean): string {
  if (id === "landlord") {
    return right
      ? "M-17 -3 L17 -12 C20 7 13 20 0 20 C-12 20 -20 12 -17 -3 Z"
      : "M-17 -12 L17 -3 C20 12 12 20 0 20 C-13 20 -20 7 -17 -12 Z";
  }
  if (id === "doctor") return "M-16 -5 Q0 -11 16 -7 C19 9 11 19 0 19 C-12 19 -19 8 -16 -5 Z";
  if (id === "stall-owner") return "M-16 1 C-16 -16 16 -16 16 1 C16 14 -16 14 -16 1 Z";
  return "M-17 0 C-17 -21 17 -21 17 0 C17 21 -17 21 -17 0 Z";
}

export function PartnerConversation({ id, state }: { id: CastId; state: EllaState }) {
  const uid = useId().replace(/[^\w-]/g, "");
  const pose = POSES[state];
  const speaking = state === "speaking";

  function eye(right: boolean) {
    const x = right ? 374 : 286;
    const clip = `${uid}-eye-${right ? "right" : "left"}`;
    const outline = eyeOutline(id, right);
    const brow = right
      ? `translate(0 ${pose.browLift - pose.browAsymmetry}) rotate(${-pose.browTilt})`
      : `translate(0 ${pose.browLift}) rotate(${pose.browTilt})`;
    return (
      <g transform={`translate(${x} 70)`}>
        <g className="figure__eye-aperture" transform={`scale(1 ${pose.openness})`}>
          <defs><clipPath id={clip}><path d={outline} /></clipPath></defs>
          <path d={outline} fill="#fffdf8" />
          <g clipPath={`url(#${clip})`}>
            <g className="figure__pupil" transform={`translate(${pose.gazeX} ${pose.gazeY})`}>
              <ellipse cy="3" rx="9" ry="11" fill="var(--ink)" />
              <circle cx="-3" cy="-1" r="2.6" fill="#fffdf8" />
            </g>
          </g>
        </g>
        <g className="figure__brow figure__line" transform={brow}>
          <path d={BROWS[id][right ? 1 : 0]} />
        </g>
      </g>
    );
  }

  return (
    <div className={`figure figure--${id} figure--conversation figure--${state}`}
      data-character={id} data-state={state} role="img"
      aria-label={`${castName(id)} is ${state === "resting" ? "ready" : state}`}>
      <div className="figure__motion" style={{ transform: `translateY(${pose.y}px) rotate(${pose.turn}deg)` }}>
        <svg className="figure__portrait" viewBox="0 0 660 450" fill="none" aria-hidden="true" focusable="false">
          <defs>
            <linearGradient id={`${uid}-light`} x1="180" y1="0" x2="410" y2="350" gradientUnits="userSpaceOnUse">
              <stop stopColor="#fff" stopOpacity="0.13" />
              <stop offset="0.7" stopColor="#fff" stopOpacity="0" />
            </linearGradient>
          </defs>
          <path fill="currentColor" d={BODIES[id]} />
          <path fill={`url(#${uid}-light)`} d={BODIES[id]} />
          {id === "stall-owner" && (
            <g fill="#ffbb8c">
              <ellipse cx="249" cy="98" rx="15" ry="9" />
              <ellipse cx="411" cy="98" rx="15" ry="9" />
            </g>
          )}
          {eye(false)}
          {eye(true)}
        </svg>
        <MouthDrawing shape={speaking ? TALKING_MOUTH : SMILES[id]} open={speaking} box={MOUTH} stage={STAGE} />
      </div>
    </div>
  );
}
