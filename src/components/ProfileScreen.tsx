import { useState } from "react";
import type { FormEvent } from "react";
import { LearnerAvatar } from "./EllaMascot";
import { MicGlyph } from "./HomeScreen";
import { AVATAR_COLORS } from "../lib/avatar";
import { badges, streak, talkTotals } from "../lib/presentation";
import type { AppSnapshot, Badge } from "../types";

export function ProfileScreen({
  snapshot,
  avatarColor,
  busy,
  onSave,
  onAvatarColor,
  onMicCheck,
  onLogOut,
}: {
  snapshot: AppSnapshot;
  avatarColor: string;
  busy: boolean;
  /** Saves the learner; resolves false when the backend refused the change. */
  onSave: (name: string, age: number | null) => Promise<boolean>;
  onAvatarColor: (color: string) => void;
  onMicCheck: () => void;
  onLogOut: () => void;
}) {
  const learner = snapshot.learner;
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const [age, setAge] = useState("");

  const run = streak(snapshot.progress);
  const totals = talkTotals(snapshot.progress);
  const earned = badges(snapshot.progress, run);

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

  return (
    <div className="screen screen--scroll screen--profile" data-screen="profile">
      <h1 className="display page-title">My profile</h1>

      <div className="profile-grid">
        <div className="profile-col">
          <form className="panel profile-card" onSubmit={(event) => void save(event)}>
            <div className="profile-card__row">
              <LearnerAvatar color={avatarColor} size="lg" />
              {editing ? (
                <div className="profile-card__fields">
                  <input
                    className="profile-card__name-field"
                    aria-label="Your name"
                    value={name}
                    maxLength={40}
                    autoFocus
                    placeholder="Your name"
                    onChange={(event) => setName(event.target.value)}
                  />
                  <input
                    className="profile-card__age-field"
                    aria-label="Your age"
                    value={age}
                    inputMode="numeric"
                    maxLength={3}
                    placeholder="Age"
                    onChange={(event) => setAge(event.target.value.replace(/[^0-9]/g, ""))}
                  />
                </div>
              ) : (
                <>
                  <div className="profile-card__who">
                    <h2 className="display">{learner?.name ?? "friend"}</h2>
                    {learner?.age != null && <p>{learner.age} years old</p>}
                  </div>
                  <button type="button" className="btn btn--quiet" onClick={startEditing}>
                    Edit
                  </button>
                </>
              )}
            </div>
            {editing && (
              <div className="profile-card__foot">
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
                <button type="submit" className="btn btn--violet btn--compact" disabled={!canSave}>
                  Done
                </button>
              </div>
            )}
          </form>

          <dl className="stat-tiles">
            <div className="panel stat-tile">
              <dt className="display">{run.days}</dt>
              <dd className="mono">DAY STREAK</dd>
            </div>
            <div className="panel stat-tile">
              <dt className="display">{totals.talks}</dt>
              <dd className="mono">TALKS DONE</dd>
            </div>
            <div className="panel stat-tile">
              <dt className="display">{totals.answers}</dt>
              <dd className="mono">ANSWERS</dd>
            </div>
          </dl>
        </div>

        <div className="profile-col">
          <section className="panel">
            <h3 className="panel__title">Badges</h3>
            <ul className="badges">
              {earned.map((badge) => (
                <li key={badge.id} className={`badge badge--${badge.id} ${badge.earned ? "is-earned" : ""}`.trim()}>
                  <span className="badge__disc">{badge.earned ? <BadgeIcon badge={badge} /> : "?"}</span>
                  <span className="badge__label">{badge.label}</span>
                </li>
              ))}
              {/* A teaser for whatever comes next. */}
              <li className="badge">
                <span className="badge__disc">?</span>
                <span className="badge__label">Locked</span>
              </li>
            </ul>
          </section>

          <section className="panel panel--settings">
            <h3 className="panel__title">Settings</h3>
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
      </div>
    </div>
  );
}

function BadgeIcon({ badge }: { badge: Badge }) {
  if (badge.id === "streak") {
    return (
      <span className="flame flame--badge" aria-hidden="true">
        <i />
        <i />
      </span>
    );
  }
  if (badge.id === "first-talk") return <MicGlyph size={28} />;
  return (
    <svg viewBox="0 0 24 24" width="28" height="28" fill="currentColor" aria-hidden="true">
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M12 2.9c5.3 0 9.6 3.5 9.6 7.8 0 4.3-4.3 7.8-9.6 7.8-.86 0-1.7-.09-2.5-.26l-3.9 1.86c-.9.43-1.86-.46-1.5-1.4l.94-2.5C3.4 14.9 2.4 13 2.4 10.7c0-4.3 4.3-7.8 9.6-7.8z"
      />
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
