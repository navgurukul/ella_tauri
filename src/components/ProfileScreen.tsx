import { useState } from "react";
import type { FormEvent, KeyboardEvent, ReactNode } from "react";
import { BadgeDisc, BadgeRow, BadgeSheet } from "./Badges";
import { EllaMascot } from "./EllaMascot";
import { Glyph } from "./Glyphs";
import { LevelTrackCard } from "./LevelCard";
import { AVATAR_COLORS, avatarTint } from "../lib/avatar";
import { dayKey } from "../lib/days";
import {
  earnedBadges,
  learnerBadges,
  openBadges,
  shortDate,
  spokenTime,
  spokenTimeKnown,
  streak,
  talkTotals,
} from "../lib/presentation";
import type { AppSnapshot, BadgeStart, LearnerBadge } from "../types";

/** How many earned badges the profile's row has room for, latest first. */
const BADGE_ROW = 5;
/** And how many of the rest it offers next. */
const NEXT_UP = 3;

/**
 * My profile, as the Ella Desktop design lays it out. On the left, the learner
 * in the colour they gave their Ella, with her rising out of the corner; their
 * counts; and the settings. On the right, MY LEVEL with a track of every
 * level, and the badges: those earned, latest first, and the ones to earn
 * next. A badge opens its sheet, and "Where to earn more" the level map.
 */
export function ProfileScreen({
  snapshot,
  avatarColor,
  busy,
  onSave,
  onAvatarColor,
  onMicCheck,
  onLevels,
  onLogOut,
  onBadgeStart,
}: {
  snapshot: AppSnapshot;
  avatarColor: string;
  busy: boolean;
  /** Saves the learner; resolves false when the backend refused the change. */
  onSave: (name: string, age: number | null) => Promise<boolean>;
  onAvatarColor: (color: string) => void;
  onMicCheck: () => void;
  onLevels: () => void;
  onLogOut: () => void;
  /** A badge sheet's mic: the scenes, talk or placement that earn it. */
  onBadgeStart: (start: BadgeStart) => void;
}) {
  const learner = snapshot.learner;
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const [age, setAge] = useState("");
  const [sheet, setSheet] = useState<string | null>(null);

  const today = new Date();
  const run = streak(snapshot.progress, today);
  const totals = talkTotals(snapshot.progress);
  const badges = learnerBadges(snapshot, today);
  const earned = earnedBadges(badges);
  const next = openBadges(badges).slice(0, NEXT_UP);
  const shown = badges.find((badge) => badge.id === sheet) ?? null;

  // The same checks as onboarding. An age cannot be taken away once given —
  // sent without one, the backend keeps the old — so an emptied field is not
  // something Done can save.
  const parsedAge = Number(age);
  const ageUsable = age === "" ? learner?.age == null : parsedAge >= 3 && parsedAge <= 120;
  const canSave = name.trim().length >= 2 && ageUsable && !busy;

  function startEditing() {
    setName(learner?.name ?? "");
    setAge(learner?.age != null ? String(learner.age) : "");
    setEditing(true);
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!canSave) return;
    // Anything the backend still refuses explains itself in the toast, and
    // editing stays open for a fix.
    if (await onSave(name, age ? parsedAge : null)) setEditing(false);
  }

  /** Escape puts the name and age back as they are saved. The colour is
   * saved the moment it is picked, so it stays. */
  function cancelOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") setEditing(false);
  }

  function open(badge: LearnerBadge) {
    setSheet(badge.id);
  }

  return (
    <div className="screen screen--scroll screen--profile" data-screen="profile">
      <h1 className="display page-title">My profile</h1>

      <div className="profile-grid">
        <div className="profile-col">
          <section
            className="profile-hero"
            style={{ background: avatarTint(avatarColor) }}
            aria-label="About you"
          >
            <div className="profile-hero__text">
              {editing ? (
                <form className="profile-hero__form" onSubmit={(event) => void save(event)} onKeyDown={cancelOnEscape}>
                  <input
                    className="profile-hero__name-field"
                    aria-label="Your name"
                    value={name}
                    maxLength={40}
                    autoFocus
                    placeholder="Your name"
                    onChange={(event) => setName(event.target.value)}
                  />
                  <div className="profile-hero__age-row">
                    <input
                      className="profile-hero__age-field"
                      aria-label="Your age"
                      value={age}
                      inputMode="numeric"
                      maxLength={3}
                      placeholder="Age"
                      onChange={(event) => setAge(event.target.value.replace(/[^0-9]/g, ""))}
                    />
                    <span>years old</span>
                  </div>
                  <div className="swatches" role="group" aria-label="Avatar colour">
                    {AVATAR_COLORS.map((color) => (
                      <button
                        key={color.value}
                        type="button"
                        className={`swatch ${color.value === avatarColor ? "is-picked" : ""}`.trim()}
                        style={{ background: color.value }}
                        aria-label={color.name}
                        aria-pressed={color.value === avatarColor}
                        onClick={() => onAvatarColor(color.value)}
                      />
                    ))}
                  </div>
                  <button type="submit" className="profile-hero__done" disabled={!canSave}>
                    Done
                  </button>
                </form>
              ) : (
                <>
                  <h2 className="display profile-hero__name">{learner?.name ?? "friend"}</h2>
                  <p className="profile-hero__about">
                    {learner?.age != null && <span>{learner.age} years old</span>}
                    {learner?.created_at && <span>Talking since {joined(learner.created_at, today)}</span>}
                  </p>
                  <button type="button" className="profile-hero__edit" onClick={startEditing}>
                    <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true">
                      <path d="M15.2 4.3a2.2 2.2 0 013.1 0l1.4 1.4a2.2 2.2 0 010 3.1L9.1 19.4l-4.8 1.2c-.6.1-1-.3-.9-.9l1.2-4.8z" />
                    </svg>
                    Edit profile
                  </button>
                </>
              )}
            </div>
            <EllaMascot
              variant="profile"
              color={avatarColor}
              className="ella--profile-peek"
              decorative
            />
          </section>

          <div className="profile-stats">
            <Stat icon={<Glyph glyph="flame" size={20} color="#FF7A00" />} value={String(run.days)} label="DAY STREAK" />
            <Stat icon={<Glyph glyph="mic" size={20} color="#9347DD" />} value={String(totals.talks)} label="TALKS DONE" />
            <Stat
              icon={<ClockIcon />}
              // Answers from before Ella kept their length leave nothing to
              // add up, which is not the same as having said nothing.
              value={spokenTimeKnown(snapshot.progress) ? spokenTime(totals.spokenMs) : "—"}
              label="SPOKEN"
            />
          </div>

          <section className="panel profile-settings">
            <h3 className="profile-panel__title">Settings</h3>
            <div className="settings">
              <button className="settings__row" onClick={onMicCheck}>
                Mic check
                <Chevron />
              </button>
              {/* English is the only language the app speaks, so there is
                  nothing to choose yet; the row only reports it. */}
              <div className="settings__row">
                App language
                <span className="settings__value">English</span>
              </div>
              {/* Signs out and nothing more: the talks stay on this laptop
                  for the next time this learner logs in. */}
              <button className="settings__row settings__row--danger" onClick={onLogOut} disabled={busy}>
                Log out
              </button>
            </div>
          </section>
        </div>

        <div className="profile-col">
          {snapshot.standing && <LevelTrackCard standing={snapshot.standing} onOpen={onLevels} />}

          <section className="panel profile-badges" aria-labelledby="profile-badges-title">
            <div className="profile-badges__head">
              <h3 id="profile-badges-title" className="profile-panel__title">
                Badges
              </h3>
              <span className="profile-badges__count">
                {earned.length} of {badges.length}
              </span>
              <button className="profile-badges__more" onClick={onLevels}>
                Where to earn more
              </button>
            </div>
            {earned.length > 0 ? (
              <ul className="profile-badges__row">
                {earned.slice(0, BADGE_ROW).map((badge) => (
                  <li key={badge.id}>
                    <button className="profile-badge" onClick={() => open(badge)}>
                      <BadgeDisc badge={badge} size={64} glyph={28} ring={0} />
                      <span>{badge.name}</span>
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="profile-badges__none">Finish a talk with Ella to earn your first badge.</p>
            )}
            {next.length > 0 && (
              <div className="profile-badges__next">
                <p className="mono profile-badges__label">NEXT UP</p>
                <ul className="badge-list">
                  {next.map((badge) => (
                    <BadgeRow key={badge.id} badge={badge} today={today} onOpen={open} />
                  ))}
                </ul>
              </div>
            )}
          </section>
        </div>
      </div>

      {shown && (
        <BadgeSheet
          badge={shown}
          today={today}
          busy={busy}
          onStart={(start) => {
            setSheet(null);
            onBadgeStart(start);
          }}
          onClose={() => setSheet(null)}
        />
      )}
    </div>
  );
}

/** "21 Aug", the day the learner first told Ella their name. */
function joined(createdAt: string, today: Date): string {
  const at = new Date(createdAt);
  return Number.isNaN(at.getTime()) ? "" : shortDate(dayKey(at), today);
}

function Stat({ icon, value, label }: { icon: ReactNode; value: string; label: string }) {
  return (
    <div className="profile-stat">
      {icon}
      <strong className="display">{value}</strong>
      <span className="mono">{label}</span>
    </div>
  );
}

function ClockIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="20"
      height="20"
      fill="none"
      stroke="#68B506"
      strokeWidth="2.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </svg>
  );
}

function Chevron() {
  return (
    <svg className="settings__chevron" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
      <path d="M9 5l7 7-7 7" />
    </svg>
  );
}
