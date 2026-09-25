import { EllaMascot } from "./EllaMascot";
import type { SessionSummary } from "../types";

/**
 * Outside the Ella Desktop design, so it is built from Home's parts: the white
 * "Today's talk" card with Ella peeking over its top edge, and its violet
 * button. How progress is shown after a talk is still being rethought.
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
      <section className="summary">
        <EllaMascot variant="home" className="ella--summary-peek" pokeable decorative />
        <div className="summary__card">
          <p className="eyebrow">Conversation complete</p>
          <h1 className="display summary__headline">{summary.headline}</h1>
          <p className="summary__lede">{summary.encouragement}</p>

          <dl className="stats stats--ink">
            <div>
              <dt className="display">{summary.turns}</dt>
              <dd>answers shared</dd>
            </div>
          </dl>

          <button className="btn btn--violet summary__home" onClick={onHome}>
            Back home
          </button>
        </div>
      </section>
    </div>
  );
}
