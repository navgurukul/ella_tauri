import { useEffect } from "react";
import { LoaderCircle } from "lucide-react";
import { EllaMascot } from "./EllaMascot";
import { levelTone } from "../lib/curriculum";
import type { Assessment, SessionSummary } from "../types";

/**
 * Outside the Ella Desktop design, so it is built from Home's parts: the white
 * "Today's talk" card with Ella peeking over its top edge, and its violet
 * button.
 *
 * It opens at once, while the backend works out what the talk did — which
 * skills it counted, and whether it finished a step or a level, which is
 * celebrated at the top of the card as Ella Mobile celebrates moving on. The
 * first answer takes a moment, while the model reads the talk; the app owns
 * that question, so leaving before the answer comes loses nothing.
 */
export function SummaryScreen({
  summary,
  assessment,
  assessError,
  onRetry,
  onCelebrated,
  onHome,
  onLevels,
}: {
  summary: SessionSummary;
  /** Null until the backend has answered. */
  assessment: Assessment | null;
  /** Why it could not answer; asking again is safe. */
  assessError: string | null;
  onRetry: () => void;
  /** The card has shown the step or level this talk finished, so nothing
   * else needs to. */
  onCelebrated: () => void;
  onHome: () => void;
  onLevels: () => void;
}) {
  useEffect(() => {
    if (assessment?.advanced) onCelebrated();
    // Once per answer; `onCelebrated` is a fresh closure every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [assessment?.session_id, assessment?.advanced]);

  const standing = assessment?.standing;
  const advanced = assessment?.advanced;

  return (
    <div className="screen screen--summary" data-screen="summary">
      <section className="summary">
        <EllaMascot variant="home" className="ella--summary-peek" pokeable decorative />
        <div className="summary__card">
          {advanced && standing && (
            <div className={`moved-on ladder-${levelTone(standing.level_number)}`} role="status">
              <span className="moved-on__badge" aria-hidden="true">
                {advanced === "level" ? standing.level_number : <Check />}
              </span>
              <div>
                <p className="display moved-on__title">{advanced === "level" ? "Level up!" : "Step complete!"}</p>
                <p className="moved-on__line">
                  {advanced === "level"
                    ? `You’ve reached ${standing.level_name}.`
                    : `On to Step ${standing.step} of ${standing.level_name}.`}
                </p>
              </div>
            </div>
          )}

          <p className="eyebrow">Conversation complete</p>
          <h1 className="display summary__headline">{summary.headline}</h1>
          <p className="summary__lede">{summary.encouragement}</p>

          <dl className="stats stats--ink">
            <div>
              <dt className="display">{summary.turns}</dt>
              <dd>answers shared</dd>
            </div>
          </dl>

          {assessError ? (
            <div className="summary-progress summary-progress--failed">
              <p className="inline-error" role="alert">
                {assessError}
              </p>
              <button className="btn btn--quiet btn--compact" onClick={onRetry}>
                Try again
              </button>
            </div>
          ) : !assessment || !standing ? (
            <p className="summary-progress summary-progress--waiting" aria-live="polite">
              <LoaderCircle className="spin" size={18} aria-hidden="true" />
              Ella is looking back over your talk…
            </p>
          ) : (
            <div className={`summary-progress ladder-${levelTone(standing.level_number)}`}>
              {assessment.skills.length > 0 && (
                <div className="grown">
                  <p className="eyebrow">Skills that grew</p>
                  <ul className="grown__list">
                    {assessment.skills.map((skill) => (
                      <li key={skill.label} className="grown__skill">
                        {skill.label}
                        <span className="grown__count">
                          {skill.count === 1 ? "first time" : `${skill.count} talks`}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <button className="summary-level" onClick={onLevels}>
                <span className="summary-level__text">
                  <span className="eyebrow">
                    {assessment.kind === "placement" && assessment.scored
                      ? "Your level"
                      : `Level ${standing.level_number}`}
                  </span>
                  <strong className="display">{standing.level_name}</strong>
                  <small>
                    Step {standing.step} of {standing.step_count} · {standing.step_title}
                  </small>
                </span>
                <span className="summary-level__meter">
                  <span className="summary-level__bar">
                    <span style={{ width: `${standing.percent}%` }} />
                  </span>
                  <small>{standing.percent}% · See all levels</small>
                </span>
              </button>
            </div>
          )}

          <button className="btn btn--violet summary__home" onClick={onHome}>
            Back home
          </button>
        </div>
      </section>
    </div>
  );
}

function Check() {
  return (
    <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
      <path d="M20 6L9 17L4 12" />
    </svg>
  );
}
