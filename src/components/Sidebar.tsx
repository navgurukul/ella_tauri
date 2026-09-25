import type { ReactElement } from "react";
import { EllaGlyph, LearnerAvatar } from "./EllaMascot";

export type NavKey = "home" | "cast";

const NAV: Array<{ key: NavKey; label: string; icon: ReactElement }> = [
  {
    key: "home",
    label: "Home",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          fillRule="evenodd"
          clipRule="evenodd"
          d="M11 3.2a1.6 1.6 0 012 0l7.6 6.2c.5.4.2 1.3-.6 1.3H19v6.1a3.2 3.2 0 01-3.2 3.2H8.2A3.2 3.2 0 015 16.8v-6.1H4c-.8 0-1.1-.9-.6-1.3L11 3.2zm-.6 16.8v-3.2a1.6 1.6 0 013.2 0V20h-3.2z"
        />
      </svg>
    ),
  },
  {
    key: "cast",
    label: "Talk partners",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="8.7" y="2.6" width="2.6" height="7" rx="1.3" transform="rotate(-14 10 6.1)" />
        <rect x="12.9" y="2.8" width="2.6" height="6.6" rx="1.3" transform="rotate(12 14.2 6.1)" />
        <path
          fillRule="evenodd"
          clipRule="evenodd"
          d="M3 21v-5.2C3 11.5 7 8.2 12 8.2s9 3.3 9 7.6V21H3zm6.6-7.9a1.3 1.3 0 100 2.6 1.3 1.3 0 000-2.6zm4.8 0a1.3 1.3 0 100 2.6 1.3 1.3 0 000-2.6z"
        />
      </svg>
    ),
  },
];

export function Sidebar({
  active,
  learnerName,
  streakDays,
  avatarColor,
  onNavigate,
  onProfile,
}: {
  /** Null on screens that are not in the nav, like the profile. */
  active: NavKey | null;
  learnerName: string;
  streakDays: number;
  avatarColor: string;
  onNavigate: (key: NavKey) => void;
  onProfile: () => void;
}) {
  return (
    <aside className="sidebar">
      <div className="wordmark">
        <EllaGlyph />
        <span>Ella</span>
      </div>

      <nav className="nav" aria-label="Main navigation">
        {NAV.map((item) => (
          <button
            key={item.key}
            className={`nav__item ${active === item.key ? "is-active" : ""}`}
            aria-current={active === item.key ? "page" : undefined}
            onClick={() => onNavigate(item.key)}
          >
            {item.icon}
            {item.label}
          </button>
        ))}
      </nav>

      <div className="sidebar__foot">
        <div className="streak-pill">
          <FlameGlyph />
          <span>
            <b>{streakDays}</b> day streak
          </span>
        </div>
        <button className="profile" onClick={onProfile}>
          <LearnerAvatar color={avatarColor} />
          <span className="profile__text">
            <strong>{learnerName}</strong>
            <small>View profile</small>
          </span>
        </button>
      </div>
    </aside>
  );
}

/** A two-drop flame, drawn in CSS so it can take any card's colours. */
export function FlameGlyph() {
  return (
    <span className="flame" aria-hidden="true">
      <i />
      <i />
    </span>
  );
}
