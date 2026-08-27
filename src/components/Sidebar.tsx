import { useState } from "react";
import type { ReactElement } from "react";
import { RotateCcw, Settings2 } from "lucide-react";
import { EllaGlyph } from "./EllaMascot";
import { initials } from "../lib/presentation";
import type { EngineStatus, LevelInfo } from "../types";

export type NavKey = "home" | "talk" | "garden";

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
    key: "talk",
    label: "Talk",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          fillRule="evenodd"
          clipRule="evenodd"
          d="M12 2.9c5.3 0 9.6 3.5 9.6 7.8 0 4.3-4.3 7.8-9.6 7.8-.86 0-1.7-.09-2.5-.26l-3.9 1.86c-.9.43-1.86-.46-1.5-1.4l.94-2.5C3.4 14.9 2.4 13 2.4 10.7c0-4.3 4.3-7.8 9.6-7.8zM8.5 8.7a1 1 0 011 1v1.6a1 1 0 11-2 0V9.7a1 1 0 011-1zm3.5-1.8a1 1 0 011 1v5.2a1 1 0 11-2 0V7.9a1 1 0 011-1zm3.5 1.8a1 1 0 011 1v1.6a1 1 0 11-2 0V9.7a1 1 0 011-1z"
        />
      </svg>
    ),
  },
  {
    key: "garden",
    label: "Garden",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="4.9" r="3.5" />
        <circle cx="19.1" cy="12" r="3.5" />
        <circle cx="12" cy="19.1" r="3.5" />
        <circle cx="4.9" cy="12" r="3.5" />
        <circle cx="12" cy="12" r="2.7" />
      </svg>
    ),
  },
];

export function Sidebar({
  active,
  learnerName,
  level,
  engineStatus,
  onNavigate,
  onReset,
}: {
  active: NavKey;
  learnerName: string;
  level: LevelInfo;
  engineStatus: EngineStatus;
  onNavigate: (key: NavKey) => void;
  onReset: () => void;
}) {
  const [showEngines, setShowEngines] = useState(false);
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
        <button
          className="engine-chip"
          onClick={() => setShowEngines((open) => !open)}
          aria-expanded={showEngines}
        >
          <span className={`dot ${engineStatus.ready ? "is-ready" : "is-warning"}`} />
          <span className="engine-chip__text">
            <strong>{engineStatus.label}</strong>
            <small>{engineStatus.ready ? "Ready" : "Needs attention"}</small>
          </span>
          <Settings2 size={16} aria-hidden="true" />
        </button>

        {showEngines && (
          <div className="engine-popover" role="dialog" aria-label="Engine status">
            <strong>{engineStatus.mode === "demo" ? "Demo mode" : "Local engine mode"}</strong>
            {engineStatus.components.map((component) => (
              <div className="engine-popover__row" key={component.name}>
                <span className={`dot ${component.ready ? "is-ready" : "is-warning"}`} />
                <span>
                  <b>{component.name}</b>
                  <small>{component.detail}</small>
                </span>
              </div>
            ))}
            <button className="engine-popover__reset" onClick={onReset}>
              <RotateCcw size={14} aria-hidden="true" /> Reset demo data
            </button>
          </div>
        )}

        <div className="profile">
          <span className="profile__avatar">{initials(learnerName)}</span>
          <span className="profile__text">
            <strong>{learnerName}</strong>
            <small>Level {level.code}</small>
          </span>
        </div>
      </div>
    </aside>
  );
}
