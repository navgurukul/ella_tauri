import { EllaMascot } from "./EllaMascot";
import type { SessionSummary } from "../types";

/**
 * Also outside the v5 design file. It reuses the Home hero as a celebration
 * panel and the green stat block from the right rail. The skill-watered and
 * garden tiles left with the garden; how progress is shown is being rethought.
 */
export function SummaryScreen({
  summary,
  onHome,
}: {
  summary: SessionSummary;
  onHome: () => void;
}) {
  return (
    <div className="screen screen--summary" data-screen="summary">
      <div className="summary__panel">
        <p className="hero__eyebrow">Conversation complete</p>
        <h1 className="display display--lg">{summary.headline}</h1>
        <p className="summary__lede">{summary.encouragement}</p>

        <dl className="stats stats--light">
          <div>
            <dt className="display display--sm">{summary.turns}</dt>
            <dd>answers shared</dd>
          </div>
        </dl>

        <div className="summary__actions">
          <button className="btn btn--light" onClick={onHome}>
            Back home
          </button>
        </div>
      </div>

      <EllaMascot className="ella--corner-summary" scale={0.62} rotate={-4} />
    </div>
  );
}
