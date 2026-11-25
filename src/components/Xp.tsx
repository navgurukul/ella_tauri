import { useId, useRef } from "react";
import { createPortal } from "react-dom";
import { useSheet } from "./Badges";
import type { XpView } from "../types";

/** A four-point spark, XP's mark, as Ella Mobile's pill draws it. */
export function SparkGlyph({ size = 16 }: { size?: number }) {
  return (
    <svg className="spark" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      <path d="M12 2.5c.5 4.6 2.4 6.9 7.5 9.5-5.1 2.6-7 4.9-7.5 9.5-.5-4.6-2.4-6.9-7.5-9.5 5.1-2.6 7-4.9 7.5-9.5z" />
    </svg>
  );
}

/** The learner's XP beside their streak. Opens the sheet that says how it is earned. */
export function XpPill({ xp, onOpen }: { xp: XpView; onOpen: () => void }) {
  return (
    <button className="xp-pill" onClick={onOpen} aria-haspopup="dialog">
      <SparkGlyph />
      <span>
        <b>{xp.total.toLocaleString()}</b> XP
      </span>
    </button>
  );
}

/** "every 10 seconds", from the backend's rate. */
function per(msPerXp: number): string {
  const seconds = Math.round(msPerXp / 1000);
  return seconds === 1 ? "every second" : `every ${seconds} seconds`;
}

/**
 * What XP is and how it is earned, over the window, as Ella Mobile's XP
 * sheet: their total, what today added, and the three rules a learner needs.
 * The rate it names is the backend's (`xp.rs`).
 */
export function XpSheet({ xp, onClose }: { xp: XpView; onClose: () => void }) {
  const title = useId();
  const card = useRef<HTMLDivElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const closeNow = useRef(onClose);
  closeNow.current = onClose;
  useSheet(card, close, closeNow);

  return createPortal(
    <div className="badge-sheet xp-sheet">
      <div className="badge-sheet__scrim" onClick={onClose} aria-hidden="true" />
      <div ref={card} className="badge-sheet__card" role="dialog" aria-modal="true" aria-labelledby={title}>
        <button ref={close} className="badge-sheet__close" onClick={onClose} aria-label="Close">
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
        <div className="badge-sheet__head">
          <p id={title} className="mono badge-sheet__label xp-sheet__label">
            Your XP
          </p>
          <p className="display xp-sheet__total">
            <SparkGlyph size={34} />
            {xp.total.toLocaleString()}
          </p>
          {xp.today > 0 && <span className="badge-sheet__pill is-earned">+{xp.today.toLocaleString()} today</span>}
        </div>
        <p className="mono badge-sheet__label">How you earn XP</p>
        <ul className="xp-rules">
          <li>
            <strong>+1 XP</strong> {per(xp.ms_per_xp)} you speak
          </li>
          <li>Every talk counts: lessons, talk partners and Word of the day</li>
          <li className="is-muted">Typed answers don’t count</li>
        </ul>
        <button className="btn btn--violet xp-sheet__done" onClick={onClose}>
          Got it
        </button>
      </div>
    </div>,
    document.body,
  );
}
