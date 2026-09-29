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
        <svg className="level-card__chevron" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
          <path d="M9 5l7 7-7 7" />
        </svg>
      </span>
    </button>
  );
}
