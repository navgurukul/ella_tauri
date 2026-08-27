import { EllaMascot } from "./EllaMascot";
import type { SessionSummary } from "../types";

/**
 * Also outside the v5 design file. It reuses the Home hero as a celebration
 * panel, the green stat block from the right rail, and the growth-reveal line
 * the Garden screen already specifies.
 */
export function SummaryScreen({
  summary,
  onGarden,
  onHome,
}: {
  summary: SessionSummary;
  onGarden: () => void;
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
          <div>
            <dt className="display display--sm">{summary.best_evidence ? 1 : 0}</dt>
            <dd>skill watered</dd>
          </div>
          <div>
            <dt className="display display--sm">{summary.garden.total_conversations}</dt>
            <dd>talks in all</dd>
          </div>
        </dl>

        <div className="summary__actions">
          <button className="btn btn--light" onClick={onGarden}>
            See my garden
          </button>
          <button className="btn btn--ghost-light" onClick={onHome}>
            Back home
          </button>
        </div>
      </div>

      <EllaMascot className="ella--corner-summary" scale={0.62} rotate={-4} />
    </div>
  );
}
