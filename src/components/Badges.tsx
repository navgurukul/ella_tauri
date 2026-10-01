import { useEffect, useId, useRef } from "react";
import type { CSSProperties } from "react";
import { createPortal } from "react-dom";
import { PartnerFigure } from "./CastScreen";
import { Check, Glyph } from "./Glyphs";
import { MicGlyph } from "./HomeScreen";
import { badgePercent, badgeStat, badgeStatus } from "../lib/presentation";
import type { BadgeGlyph, BadgeStart, CastId, LearnerBadge } from "../types";

/**
 * A badge's disc: filled in its colour once earned, a dashed ring in its
 * colour round its glyph until then. `ring` is the ring's width, which the
 * profile's row of earned badges goes without.
 */
export function BadgeDisc({
  badge,
  size,
  glyph,
  ring = 2,
  className = "",
  style,
}: {
  badge: LearnerBadge;
  size: number;
  glyph: number;
  ring?: number;
  className?: string;
  style?: CSSProperties;
}) {
  const border = ring > 0 ? `${ring}px ${badge.earned ? "solid" : "dashed"} ${badge.color}` : "none";
  return (
    <span
      className={`badge-disc ${badge.earned ? "is-earned" : ""} ${className}`.trim()}
      style={{ width: size, height: size, background: badge.earned ? badge.color : "var(--white)", border, ...style }}
      aria-hidden="true"
    >
      <Glyph glyph={badge.glyph} size={glyph} color={badge.earned ? "#ffffff" : badge.color} />
    </span>
  );
}

/** A badge in a list: its disc, name and where it is earned, a bar for a
 * count, and when it was earned or how it is going. Opens its sheet. */
export function BadgeRow({
  badge,
  today,
  onOpen,
}: {
  badge: LearnerBadge;
  today: Date;
  onOpen: (badge: LearnerBadge) => void;
}) {
  return (
    <li className="badge-row-item">
      <button className="badge-row" onClick={() => onOpen(badge)}>
        <BadgeDisc badge={badge} size={44} glyph={20} />
        <span className="badge-row__text">
          <strong>{badge.name}</strong>
          <span>{badge.where}</span>
          {!badge.earned && badge.goal && (
            <span className="badge-bar" aria-hidden="true">
              <span style={{ width: `${badgePercent(badge)}%`, background: badge.color }} />
            </span>
          )}
        </span>
        <span className={`badge-row__stat ${badge.earned ? "is-earned" : ""}`.trim()}>
          {badge.earned && <Check size={14} />}
          <span className="sr-only">{badge.earned ? "Earned " : ""}</span>
          {badgeStat(badge, today)}
        </span>
      </button>
    </li>
  );
}

/** Where each partner's scene is drawn, as the design tints them: the
 * thumbnail's wash and the row's paler one. */
const PARTNER_TINT: Record<CastId, [string, string]> = {
  "stall-owner": ["#FFE9D4", "#FFF6EC"],
  landlord: ["#E4ECFB", "#F1F5FD"],
  doctor: ["#FFE2DA", "#FFF0ED"],
  debater: ["#E7F4D6", "#F2F9E9"],
};

/** The same for a badge that is not a partner's, by its colour. */
const COLOR_TINT: Record<string, [string, string]> = {
  "#FF7A00": ["#FFE9D4", "#FFF6EC"],
  "#5B7DEF": ["#E4ECFB", "#F1F5FD"],
  "#FF3181": ["#FFE2EE", "#FFF0F6"],
  "#68B506": ["#E7F4D6", "#F2F9E9"],
  "#9347DD": ["#EDE2FA", "#F6F0FD"],
};

/**
 * One badge, over the window: how it is going, what earns it, and where —
 * with a mic that opens that partner's scenes on Talk partners, or starts the
 * talk that earns it. Escape, the scrim and the close button all close it.
 * While it is open the app behind it is inert and Tab stays inside it, and
 * focus goes back to whatever had it before.
 */
export function BadgeSheet({
  badge,
  today,
  busy,
  onStart,
  onClose,
}: {
  badge: LearnerBadge;
  today: Date;
  busy: boolean;
  onStart: (start: BadgeStart) => void;
  onClose: () => void;
}) {
  const title = useId();
  const card = useRef<HTMLDivElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const closeNow = useRef(onClose);
  closeNow.current = onClose;

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // The sheet is portalled beside the app, so the whole app can go inert.
    const app = document.getElementById("root");
    const wasInert = app?.hasAttribute("inert") ?? false;
    app?.setAttribute("inert", "");
    close.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeNow.current();
        return;
      }
      if (event.key !== "Tab" || !card.current) return;
      const stops = [...card.current.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      if (stops.length === 0) return;
      const first = stops[0];
      const last = stops[stops.length - 1];
      const at = document.activeElement;
      if (event.shiftKey && (at === first || !card.current.contains(at))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (at === last || !card.current.contains(at))) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (!wasInert) app?.removeAttribute("inert");
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  const [thumb, wash] = (badge.partner && PARTNER_TINT[badge.partner]) ?? COLOR_TINT[badge.color] ?? ["#EDEDEA", "#F5F5F2"];
  const canGo = badge.partner !== null || !badge.earned;
  const goLabel = badge.partner && badge.minutes ? `${badge.minutes} min` : "Talk";
  const line = [badge.line, badge.partner && badge.minutes ? `${badge.minutes} min` : null].filter(Boolean).join(" · ");

  return createPortal(
    <div className="badge-sheet">
      <div className="badge-sheet__scrim" onClick={onClose} aria-hidden="true" />
      <div ref={card} className="badge-sheet__card" role="dialog" aria-modal="true" aria-labelledby={title}>
        <button ref={close} className="badge-sheet__close" onClick={onClose} aria-label="Close">
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
        <div className="badge-sheet__head">
          <BadgeDisc
            badge={badge}
            size={104}
            glyph={48}
            ring={3}
            className={badge.earned ? "badge-disc--pop" : ""}
          />
          <h2 id={title} className="display badge-sheet__name">
            {badge.name}
          </h2>
          <span className={`badge-sheet__pill ${badge.earned ? "is-earned" : ""}`.trim()}>
            {badgeStatus(badge, today)}
          </span>
          <p className="badge-sheet__how">{badge.how}</p>
          {!badge.earned && badge.goal && (
            <span
              className="badge-bar badge-bar--sheet"
              role="progressbar"
              aria-label={`${badge.goal.have} of ${badge.goal.of} ${badge.goal.unit}`}
              aria-valuenow={badgePercent(badge)}
              aria-valuemin={0}
              aria-valuemax={100}
            >
              <span style={{ width: `${badgePercent(badge)}%`, background: badge.color }} />
            </span>
          )}
        </div>
        <p className="mono badge-sheet__label">{badge.earned ? "WHERE YOU EARNED IT" : "WHERE TO EARN IT"}</p>
        <div className="badge-place" style={{ background: wash }}>
          <span className="badge-place__thumb" style={{ background: thumb }} aria-hidden="true">
            {badge.partner ? (
              <span className="badge-place__figure">
                <PartnerFigure id={badge.partner} entrance={false} />
              </span>
            ) : (
              <span className="badge-place__glyph" style={{ background: badge.color }}>
                <Glyph glyph={placeGlyph(badge)} size={22} color="#ffffff" />
              </span>
            )}
          </span>
          <span className="badge-place__text">
            <strong className="display">{badge.place}</strong>
            <span>{line}</span>
          </span>
          {canGo && (
            <span className="badge-place__go">
              <button
                className="badge-place__mic"
                style={{ background: badge.color }}
                onClick={() => onStart(badge.start)}
                disabled={busy}
                aria-label={badge.start.kind === "partners" ? `Talk to ${badge.place}` : "Start this talk"}
              >
                <MicGlyph size={20} />
              </button>
              <span aria-hidden="true">{goLabel}</span>
            </span>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** The glyph a place without a partner is drawn with: Ella for the
 * placement, the flame for a streak, the mic for any other talk. */
function placeGlyph(badge: LearnerBadge): BadgeGlyph {
  if (badge.start.kind === "placement") return "ella";
  if (badge.glyph === "flame") return "flame";
  return "mic";
}
