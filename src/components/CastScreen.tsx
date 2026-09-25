import { useRef } from "react";
import { EllaMascot, useEntrance } from "./EllaMascot";
import { MicGlyph } from "./HomeScreen";
import { castFor } from "../lib/presentation";
import type { CastGoal, CastId, Learner } from "../types";

/**
 * Talk partners: Ella as the learner's mentor, then the cast they practise
 * with. Every partner has a goal for the learner; pressing one starts that
 * conversation.
 */
export function CastScreen({
  learner,
  busy,
  onStartGoal,
}: {
  learner: Learner | null | undefined;
  busy: boolean;
  onStartGoal: (goal: CastGoal) => void;
}) {
  const cast = castFor(learner?.age);

  return (
    <div className="screen screen--scroll screen--cast" data-screen="cast">
      <header className="page-head">
        <h1 className="display">Talk partners</h1>
        <p className="page-head__sub">
          Practise with the cast. Ella learns from every talk and teaches you what she spots.
        </p>
      </header>

      <section className="mentor">
        <div className="mentor__text">
          <p className="eyebrow eyebrow--violet">Your mentor</p>
          <h2 className="display mentor__name">Ella</h2>
          <p className="mentor__tagline">Learns from every talk you have, and teaches from it.</p>
          {/* Nothing reviews a finished talk yet, so there is never a lesson to
              offer; this is the design's "nothing to fix" state. */}
          <p className="mentor__note">
            Nothing to fix yet. When Ella spots a pattern, she pops up on Home with a short lesson.
          </p>
        </div>
        <EllaMascot variant="mentor" className="ella--mentor" pokeable decorative />
      </section>

      <div className="section-head">
        <h3>Role-play</h3>
        <span>Each one has a goal for you</span>
      </div>

      <div className="cast-grid">
        {cast.map((member) => (
          <article key={member.id} className="partner" data-character={member.id}>
            <div className="partner__stage">
              <span className="partner__shadow" aria-hidden="true" />
              <PartnerFigure id={member.id} />
            </div>
            <div className="partner__about">
              <h4 className="display">{member.name}</h4>
              <p>{member.blurb}</p>
            </div>
            <div className="partner__goals">
              {member.goals.map((goal) => {
                const soon = goal.start.kind === "soon";
                return (
                  <button
                    key={goal.id}
                    className="goal"
                    disabled={busy || soon}
                    onClick={() => onStartGoal(goal)}
                  >
                    <span className="goal__text">
                      <strong>{goal.title}</strong>
                      <span>{goal.goal}</span>
                      <span className="mono goal__meta">
                        {goal.track} · {soon ? "COMING SOON" : `~${goal.minutes} MIN`}
                      </span>
                    </span>
                    <span className="goal__mic" aria-hidden="true">
                      <MicGlyph size={14} />
                    </span>
                  </button>
                );
              })}
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}

/** The parts of each partner's face, drawn and placed by `.figure--{id}` in the
 * stylesheet. */
const FIGURE_PARTS: Record<Exclude<CastId, "doctor" | "debater" | "landlord">, string[]> = {
  "stall-owner": ["body", "eye eye--left", "eye eye--right", "cheek cheek--left", "cheek cheek--right", "mouth"],
};

/** A talk partner, bobbing on their card after flying in with the screen. */
function PartnerFigure({ id }: { id: CastId }) {
  const entry = useRef<HTMLDivElement>(null);
  useEntrance(entry);
  return (
    <div ref={entry} className="partner__entry" aria-hidden="true">
      <div className={`figure figure--${id}`}>
        {id === "doctor" ? (
          <svg className="figure__portrait" viewBox="0 0 240 190" fill="none" focusable="false">
            {/* One continuous silhouette keeps the two soft tufts seamless. */}
            <path
              fill="currentColor"
              d="M35 202 C44 174 48 147 49 119 C50 94 53 77 63 62
                 C49 58 46 48 53 40 C59 33 68 36 77 43
                 C67 27 70 13 81 12 C92 10 99 24 105 36
                 C145 30 169 43 198 60 C218 72 232 80 251 85
                 L251 202 Z"
            />
            <g stroke="#211910" strokeWidth="4.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M83 119 Q87 104 101 107" />
              <path d="M115 99 Q122 86 134 91" />
              <path d="M118 132 Q132 138 133 124" />
            </g>
          </svg>
        ) : id === "debater" ? (
          <svg className="figure__portrait" viewBox="0 0 240 190" fill="none" focusable="false">
            <path
              fill="currentColor"
              d="M45 202 C25 172 38 120 80 71
                 C102 45 134 17 146 16 C156 15 159 25 154 36
                 C170 21 186 9 191 14 C197 21 188 42 181 53
                 C196 38 207 29 216 37 C234 54 248 80 253 99
                 L253 202 Z"
            />
            <g fill="#fffdf8">
              <ellipse cx="120" cy="101" rx="10" ry="11" />
              <ellipse cx="168" cy="101" rx="10" ry="11" />
            </g>
            <g fill="#211910">
              <circle cx="124" cy="105" r="7.5" />
              <circle cx="172" cy="105" r="7.5" />
            </g>
            <g stroke="#211910" strokeWidth="4.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M109 84 L128 78" />
              <path d="M160 73 Q173 64 185 73" />
              <path d="M135 135 C151 141 173 137 184 121" />
              <path d="M178 116 Q184 125 194 123" />
            </g>
          </svg>
        ) : id === "landlord" ? (
          <svg className="figure__portrait" viewBox="0 0 240 190" fill="none" focusable="false">
            {/* A low shoulder and a tall, rounded crown make Grumble a soft bean. */}
            <path
              fill="currentColor"
              d="M28 202 C15 184 11 164 19 146 C27 129 42 119 62 122
                 C84 126 96 112 109 86 C127 50 147 24 171 24
                 C204 23 220 49 223 79 C226 99 236 111 252 120
                 L252 202 Z"
            />
            <g fill="#fffdf8">
              <path d="M129 103 L153 112 C154 125 148 133 140 132 C131 131 127 119 129 103 Z" />
              <path d="M165 112 L188 103 C191 118 187 131 178 132 C170 134 164 125 165 112 Z" />
            </g>
            <g fill="#211910">
              <path d="M137 108 L150 113 C151 123 138 126 137 116 Z" />
              <path d="M168 113 L181 108 L182 116 C182 126 168 125 168 113 Z" />
            </g>
            <g stroke="#211910" strokeWidth="4" strokeLinecap="round" strokeLinejoin="round">
              <path d="M127 93 Q138 95 151 102" />
              <path d="M167 102 Q179 95 190 93" />
              <path d="M147 151 Q152 146 157 151 Q161 155 165 151 Q169 147 174 152" />
            </g>
          </svg>
        ) : (
          FIGURE_PARTS[id].map((part) => (
            <i key={part} className={part.split(" ").map((name) => `figure__${name}`).join(" ")} />
          ))
        )}
      </div>
    </div>
  );
}
