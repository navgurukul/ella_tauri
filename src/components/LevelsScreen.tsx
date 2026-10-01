import { useEffect, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { BadgeRow, BadgeSheet } from "./Badges";
import { EllaMascot } from "./EllaMascot";
import { Check, Glyph } from "./Glyphs";
import { MicGlyph } from "./HomeScreen";
import { bridge } from "../lib/bridge";
import { levelTone } from "../lib/curriculum";
import { learnerBadges } from "../lib/presentation";
import type { AppSnapshot, BadgeStart, LearnerBadge, LevelState, LevelView, Standing } from "../types";

/** Where a level's code would be, the learner reads its number. */
function levelLabel(number: number): string {
  return `level ${number}`;
}

/** The left column's pick: a level by its number, or the anytime badges. */
type Pick = number | "anytime";

/**
 * Levels and badges, as the Ella Desktop design lays them out: the path of
 * every level on the left, with the anytime badges beneath it, and the picked
 * one's page on the right. It opens on the learner's own level.
 *
 * A level's page is the curriculum's own: its goal, and its five steps, each a
 * bar with a segment per skill that fills once the skill passes, so filling
 * every bar is exactly what finishes the level. Beside them are the badges the
 * design lists at that level. Nothing is locked: every badge's scene can be
 * played from any level, so a badge sits at its level only as the design
 * files it.
 */
export function LevelsScreen({
  snapshot,
  avatarColor,
  busy,
  onBack,
  onPractise,
  onFindLevel,
  onBadgeStart,
}: {
  snapshot: AppSnapshot;
  avatarColor: string;
  busy: boolean;
  /** Back to the profile, which the level map belongs to. */
  onBack: () => void;
  /** Today's talk: every talk aims at a skill of the learner's step. */
  onPractise: () => void;
  /** The placement chat, for a learner who never had one. */
  onFindLevel: () => void;
  /** A badge sheet's mic: the scenes, talk or placement that earn it. */
  onBadgeStart: (start: BadgeStart) => void;
}) {
  const standing = snapshot.standing ?? null;
  const [levels, setLevels] = useState<LevelView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<Pick | null>(null);
  const [sheet, setSheet] = useState<string | null>(null);

  // Read again whenever the app re-reads where the learner stands — after
  // every assessment — so a talk that passed a skill anywhere on the ladder
  // shows on the map straight away.
  useEffect(() => {
    let live = true;
    bridge
      .levels()
      .then((ladder) => {
        if (live) setLevels(ladder);
      })
      .catch((reason: unknown) => {
        if (live) setError(message(reason));
      });
    return () => {
      live = false;
    };
  }, [standing]);

  const today = new Date();
  const badges = learnerBadges(snapshot, today);
  const anytime = badges.filter((badge) => badge.level === null);
  const current = levels?.find((level) => level.state === "current");
  const shown =
    picked === "anytime" ? null : (levels?.find((level) => level.number === picked) ?? current ?? levels?.[0]);
  const open = badges.find((badge) => badge.id === sheet) ?? null;
  const onOpen = (badge: LearnerBadge) => setSheet(badge.id);

  return (
    <div className="screen screen--scroll screen--levels" data-screen="levels">
      <header className="levels-head">
        <button className="back-button" onClick={onBack} aria-label="Back to profile">
          <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
        <h1 className="display page-title">Levels and badges</h1>
      </header>

      {!levels || (!shown && picked !== "anytime") ? (
        <div className="levels-wait" aria-live="polite">
          {error ? (
            <p className="inline-error">{error}</p>
          ) : (
            <LoaderCircle className="spin" aria-label="Loading the levels" />
          )}
        </div>
      ) : (
        <div className="levels-grid">
          <div className="levels-side">
            <ol className="level-path" aria-label="Levels">
              {levels.map((level, index) => (
                <LevelStop
                  key={level.number}
                  level={level}
                  count={levels.length}
                  last={index === levels.length - 1}
                  picked={picked !== "anytime" && level.number === shown?.number}
                  onPick={() => setPicked(level.number)}
                />
              ))}
            </ol>
            <div className="levels-side__anytime">
              <button
                className={`anytime-stop ${picked === "anytime" ? "is-picked" : ""}`.trim()}
                onClick={() => setPicked("anytime")}
                aria-pressed={picked === "anytime"}
              >
                <span className="anytime-stop__disc" aria-hidden="true">
                  <Glyph glyph="flame" size={24} color="#ffffff" />
                </span>
                <span className="anytime-stop__text">
                  <strong className="display">Anytime badges</strong>
                  <small>{earnedLine(anytime)}</small>
                </span>
              </button>
            </div>
          </div>

          {picked === "anytime" || !shown ? (
            <AnytimePage badges={anytime} today={today} onOpen={onOpen} />
          ) : (
            <LevelPage
              level={shown}
              count={levels.length}
              standing={standing}
              avatarColor={avatarColor}
              badges={badges.filter((badge) => badge.level === shown.number)}
              today={today}
              busy={busy}
              onPractise={onPractise}
              onFindLevel={onFindLevel}
              onOpen={onOpen}
            />
          )}
        </div>
      )}

      {open && (
        <BadgeSheet
          badge={open}
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

/** "2 of 4 earned". */
function earnedLine(badges: LearnerBadge[]): string {
  return `${badges.filter((badge) => badge.earned).length} of ${badges.length} earned`;
}

/** Every skill of a level, and how many of them have passed. */
function skillCounts(level: LevelView): { filled: number; total: number } {
  const skills = level.steps.flatMap((step) => step.skills);
  return { filled: skills.filter((skill) => skill.passed).length, total: skills.length };
}

function status(level: LevelView, count: number): string {
  switch (level.state) {
    case "done":
      return "Done";
    case "current":
      return level.number < count
        ? `You’re here · ${level.percent}% to ${levelLabel(level.number + 1)}`
        : `You’re here · ${level.percent}% of the way`;
    case "next":
      return `Next · opens after ${levelLabel(level.number - 1)}`;
    case "locked":
      return "Locked";
  }
}

/** One level on the path: its numbered stop, the line down to the next, its
 * name and where it stands, and for the learner's own a bar of how far. */
function LevelStop({
  level,
  count,
  last,
  picked,
  onPick,
}: {
  level: LevelView;
  count: number;
  last: boolean;
  picked: boolean;
  onPick: () => void;
}) {
  const { filled, total } = skillCounts(level);
  return (
    <li className={`level-stop is-${level.state} ladder-${levelTone(level.number)} ${last ? "is-last" : ""} ${picked ? "is-picked" : ""}`.trim()}>
      <button className="level-stop__button" onClick={onPick} aria-pressed={picked}>
        <span className="level-stop__node" aria-hidden="true">
          {level.number}
          {level.state === "done" && (
            <span className="level-stop__tick">
              <Check size={12} />
            </span>
          )}
        </span>
        <span className="level-stop__text">
          <strong className="display level-stop__name">{level.name}</strong>
          <small className="level-stop__status">{status(level, count)}</small>
          {level.state === "current" && (
            <>
              <span className="level-stop__bar">
                <span style={{ width: `${level.percent}%` }} />
              </span>
              <small className="level-stop__progress">
                {filled} of {total} skills filled
              </small>
            </>
          )}
        </span>
      </button>
    </li>
  );
}

const PILL: Record<LevelState, string> = {
  done: "Done",
  current: "You’re here",
  next: "Next level",
  locked: "Locked",
};

function LevelPage({
  level,
  count,
  standing,
  avatarColor,
  badges,
  today,
  busy,
  onPractise,
  onFindLevel,
  onOpen,
}: {
  level: LevelView;
  count: number;
  standing: Standing | null;
  avatarColor: string;
  badges: LearnerBadge[];
  today: Date;
  busy: boolean;
  onPractise: () => void;
  onFindLevel: () => void;
  onOpen: (badge: LearnerBadge) => void;
}) {
  const current = level.state === "current";
  const reached = level.state === "done" || current;
  const { filled, total } = skillCounts(level);
  const sub =
    level.state === "done"
      ? "You’ve already passed this level."
      : current
        ? level.number < count
          ? `${level.percent}% of the way to ${levelLabel(level.number + 1)}.`
          : `${level.percent}% of the way through.`
        : `Opens when you finish ${levelLabel(level.number - 1)}.`;
  const skillLine =
    level.state === "done" ? "All full" : filled === 0 && !reached ? "Not started" : `${filled} of ${total} filled`;

  return (
    <section
      className={`level-page ladder-${levelTone(level.number)} is-${level.state}`}
      aria-label={`Level ${level.number}: ${level.name}`}
    >
      <header className="level-page__head">
        <span className="display level-page__number" aria-hidden="true">
          {level.number}
        </span>
        <div className="level-page__title">
          <span className="level-page__pill">{PILL[level.state]}</span>
          <h2 className="display">{level.name}</h2>
          <p>{sub}</p>
        </div>
        {current && (
          <EllaMascot variant="profile" color={avatarColor} className="ella--level-peek" decorative />
        )}
      </header>

      <div className="level-page__body">
        <div className="level-page__col">
          <h3 className="level-page__heading">The goal</h3>
          <p className="level-page__goal">{level.goal}</p>

          <div className="level-page__heading level-page__heading--row">
            <h3>Skills</h3>
            <span>{skillLine}</span>
          </div>
          <ol className="level-steps">
            {level.steps.map((step) => {
              const yours = current && step.number === standing?.step;
              const passed = step.skills.filter((skill) => skill.passed).length;
              return (
                <li key={step.number} className={`level-step ${yours ? "is-yours" : ""}`.trim()}>
                  <span className="level-step__title">
                    {step.title}
                    {yours && <span className="level-step__here">Your step</span>}
                    <span className="sr-only">
                      , {passed} of {step.skills.length} skills
                    </span>
                  </span>
                  <span className="level-step__bar" aria-hidden="true">
                    {step.skills.map((skill) => (
                      <i key={skill.label} className={skill.passed ? "is-passed" : ""} title={skill.text} />
                    ))}
                  </span>
                  {/* The bar's segments, in words, for a screen reader. */}
                  <ul className="sr-only">
                    {step.skills.map((skill) => (
                      <li key={skill.label}>
                        {skill.text} {skill.passed ? "(done)" : "(still to show)"}
                      </li>
                    ))}
                  </ul>
                </li>
              );
            })}
          </ol>
          {current && (
            <p className="level-page__note">
              {level.number < count
                ? `Fill every bar to reach ${levelLabel(level.number + 1)}.`
                : "Fill every bar to finish this level."}
            </p>
          )}
        </div>

        <div className="level-page__col">
          <div className="level-page__heading level-page__heading--row">
            <h3>Badges at {levelLabel(level.number)}</h3>
            {badges.length > 0 && <span>{earnedLine(badges)}</span>}
          </div>
          {badges.length > 0 ? (
            <ul className="badge-list">
              {badges.map((badge) => (
                <BadgeRow key={badge.id} badge={badge} today={today} onOpen={onOpen} />
              ))}
            </ul>
          ) : (
            <p className="level-page__none">No badges at this level yet.</p>
          )}
          {current && (
            <div className="level-page__actions">
              <button className="btn btn--violet level-page__practise" onClick={onPractise} disabled={busy}>
                <MicGlyph size={16} />
                Practise with Ella
              </button>
              {standing && !standing.placed && (
                <button className="btn btn--quiet" onClick={onFindLevel} disabled={busy}>
                  Not sure? Find my level
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

/** The badges no level holds: streaks and talk counts. */
function AnytimePage({
  badges,
  today,
  onOpen,
}: {
  badges: LearnerBadge[];
  today: Date;
  onOpen: (badge: LearnerBadge) => void;
}) {
  return (
    <section className="level-page level-page--anytime" aria-label="Anytime badges">
      <header className="level-page__head">
        <span className="anytime-page__disc" aria-hidden="true">
          <Glyph glyph="flame" size={44} color="#ffffff" />
        </span>
        <div className="level-page__title">
          <h2 className="display">Anytime badges</h2>
          <p>Streaks and talk counts. These work at every level.</p>
        </div>
      </header>
      <div className="anytime-page__body">
        <div className="level-page__heading level-page__heading--row">
          <h3>Badges</h3>
          <span>{earnedLine(badges)}</span>
        </div>
        <ul className="badge-list badge-list--two">
          {badges.map((badge) => (
            <BadgeRow key={badge.id} badge={badge} today={today} onOpen={onOpen} />
          ))}
        </ul>
      </div>
    </section>
  );
}

function message(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "Something went wrong. Please try again.";
}
