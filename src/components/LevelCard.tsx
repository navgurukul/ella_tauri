import { Check } from "./Glyphs";
import type { Standing } from "../types";

/**
 * The learner's level at a glance: purple, as Ella Mobile v7 draws MY LEVEL,
 * with how far through it they are and where the next level is. The whole card
 * opens the level map. The level goes by its name and number, never its code.
 */
export function LevelCard({ standing, onOpen }: { standing: Standing; onOpen: () => void }) {
  const last = standing.level_number >= standing.level_count;
  return (
    <button className="card card--level" onClick={onOpen} aria-label={`My level: ${standing.level_name}. See all levels`}>
      <span className="card--level__orb" aria-hidden="true" />
      <p className="eyebrow">My level</p>
      <span className="level-card__head">
        <strong className="display level-card__number">{standing.level_number}</strong>
        <span className="level-card__who">
          <span className="display level-card__name">{standing.level_name}</span>
          <small>
            Step {standing.step} of {standing.step_count} · {standing.step_title}
          </small>
        </span>
      </span>
      <span
        className="level-card__bar"
        role="progressbar"
        aria-label="Through this level"
        aria-valuenow={standing.percent}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <span style={{ width: `${standing.percent}%` }} />
      </span>
      <span className="level-card__foot">
        {last ? `${standing.percent}% of the way through` : `${standing.percent}% to level ${standing.level_number + 1}`}
        <Chevron size={16} />
      </span>
    </button>
  );
}

/**
 * MY LEVEL on the profile, as the Ella Desktop design draws it: the level's
 * number and name, how far to the next, and a track of every level, done ones
 * ticked, the learner's own ringed and its stretch filled as far as they have
 * got. The whole card opens Levels and badges.
 */
export function LevelTrackCard({ standing, onOpen }: { standing: Standing; onOpen: () => void }) {
  const last = standing.level_number >= standing.level_count;
  const levels = Array.from({ length: standing.level_count }, (_, index) => index + 1);
  const toward = last ? "of the way" : `to level ${standing.level_number + 1}`;
  return (
    <button
      className="level-track-card"
      onClick={onOpen}
      aria-label={`My level: ${standing.level_name}, level ${standing.level_number}, ${standing.percent}% ${toward}. See all levels`}
    >
      <span className="level-track-card__top">
        <span className="eyebrow">My level</span>
        <span className="level-track-card__all">
          All levels
          <Chevron size={14} />
        </span>
      </span>
      <span className="level-track-card__head">
        <span className="level-track-card__who">
          <strong className="display level-track-card__number">{standing.level_number}</strong>
          <span className="display level-track-card__name">{standing.level_name}</span>
        </span>
        <span className="level-track-card__to">
          <strong className="display">{standing.percent}%</strong>
          <small>{toward}</small>
        </span>
      </span>
      <span className="level-track" aria-hidden="true">
        {levels.map((number) => {
          const here = number === standing.level_number;
          const state = number < standing.level_number ? "done" : here ? "current" : "ahead";
          const fill = number < standing.level_number ? 100 : here ? standing.percent : 0;
          return (
            <span key={number} className={`level-track__stop is-${state}`}>
              <span className="level-track__node">
                <span className="level-track__dot">{state === "done" && <Check size={13} />}</span>
                <span className="mono level-track__label">{number}</span>
              </span>
              {number < standing.level_count && (
                <span className="level-track__bar">
                  <span style={{ width: `${fill}%` }} />
                </span>
              )}
            </span>
          );
        })}
      </span>
    </button>
  );
}

function Chevron({ size }: { size: number }) {
  return (
    <svg className="level-card__chevron" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      <path d="M9 5l7 7-7 7" />
    </svg>
  );
}
