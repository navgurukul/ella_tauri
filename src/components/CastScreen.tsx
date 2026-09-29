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

/** The parts of each partner, drawn and placed by `.figure--{id}` in the
 * stylesheet, as the Ella Desktop design draws the cast: a soft body and a
 * face in CSS shapes. */
function FigureParts({ id }: { id: CastId }) {
  switch (id) {
    case "stall-owner":
      return (
        <>
          <i className="figure__body" />
          <i className="figure__eye figure__eye--left" />
          <i className="figure__eye figure__eye--right" />
          <i className="figure__cheek figure__cheek--left" />
          <i className="figure__cheek figure__cheek--right" />
          <i className="figure__mouth" />
        </>
      );
    case "landlord":
      return (
        <>
          <i className="figure__body" />
          <i className="figure__brow figure__brow--left" />
          <i className="figure__brow figure__brow--right" />
          <i className="figure__eye figure__eye--left">
            <i className="figure__pupil" />
          </i>
          <i className="figure__eye figure__eye--right">
            <i className="figure__pupil" />
          </i>
          <i className="figure__mouth" />
        </>
      );
    case "doctor":
      return (
        <>
          <i className="figure__body" />
          <i className="figure__eye figure__eye--left" />
          <i className="figure__eye figure__eye--right" />
          <i className="figure__mouth" />
        </>
      );
    case "debater":
      return (
        <>
          <i className="figure__body" />
          <i className="figure__lobe figure__lobe--left" />
          <i className="figure__lobe figure__lobe--middle" />
          <i className="figure__lobe figure__lobe--right" />
          <i className="figure__brow figure__brow--left" />
          <i className="figure__brow figure__brow--right" />
          <i className="figure__eye figure__eye--left">
            <i className="figure__pupil" />
          </i>
          <i className="figure__eye figure__eye--right">
            <i className="figure__pupil" />
          </i>
          <i className="figure__mouth" />
        </>
      );
  }
}

/** A talk partner, bobbing on their card after flying in with the screen. */
export function PartnerFigure({ id, entrance = true }: { id: CastId; entrance?: boolean }) {
  const entry = useRef<HTMLDivElement>(null);
  useEntrance(entry, entrance);
  return (
    <div ref={entry} className="partner__entry" aria-hidden="true">
      <div className={`figure figure--${id}`}>
        <FigureParts id={id} />
      </div>
    </div>
  );
}
