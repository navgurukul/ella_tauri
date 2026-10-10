import { useEffect, useId, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { BadgeRow, BadgeSheet } from "./Badges";
import { EllaMascot } from "./EllaMascot";
import { Check, Glyph, Lock } from "./Glyphs";
import { MicGlyph } from "./HomeScreen";
import { bridge } from "../lib/bridge";
import { levelTone } from "../lib/curriculum";
import { learnerBadges } from "../lib/presentation";
import type {
  AppSnapshot,
  BadgeStart,
  LearnerBadge,
  LevelSkill,
  LevelState,
  LevelStep,
  LevelView,
  SkillStanding,
  Standing,
} from "../types";

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
 * A level's page is the curriculum's own: its goal, its five steps and where
 * each of their skills stands, and beside them the badges the design lists at
 * that level. Nothing is locked: every badge's scene can be played from any
 * level, so a badge sits at its level only as the design files it.
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
                  step={learnersStep(level, standing)}
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
              step={learnersStep(shown, standing)}
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

/**
 * The step the learner is on in `level`, when it is their own. The snapshot
 * says, unless it has moved on to another level before the ladder is read
 * again; until then, their step is the first with a skill still to pass, as
 * every one behind them has.
 */
function learnersStep(level: LevelView, standing: Standing | null): number | null {
  if (level.state !== "current") return null;
  if (standing?.level_number === level.number) return standing.step;
  return level.steps.find((step) => step.skills.some((skill) => !skill.passed))?.number ?? level.steps.length;
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
 * name and where it stands, and for the learner's own a bar of how far, with
 * the step they are on and how much of it is done, which is what moves it. */
function LevelStop({
  level,
  count,
  step,
  last,
  picked,
  onPick,
}: {
  level: LevelView;
  count: number;
  /** The learner's step, on their own level. */
  step: number | null;
  last: boolean;
  picked: boolean;
  onPick: () => void;
}) {
  const here = level.steps.find((candidate) => candidate.number === step);
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
              {here && (
                <small className="level-stop__progress">
                  Step {here.number} of {level.steps.length} · {passedIn(here)} of {here.skills.length} skills
                </small>
              )}
            </>
          )}
        </span>
      </button>
    </li>
  );
}

/** How many of a step's skills have passed. */
function passedIn(step: LevelStep): number {
  return step.skills.filter((skill) => skill.passed).length;
}

const PILL: Record<LevelState, string> = {
  done: "Done",
  current: "You’re here",
  next: "Next level",
  locked: "Locked",
};

/**
 * One level's page: its number, name and where it stands on the level's
 * wash, its goal, its skills, and the badges filed under it.
 *
 * The skills are Ella Mobile's, from the final design (2a) of "Ella Level
 * Skills Options". On the learner's own level, the steps behind them are short
 * rows that open on their skills, and the steps ahead short rows that say when
 * they open. The step they are on is a card of its skills, each with two
 * circles: the first fills once the skill shows in a talk, the second once it
 * passes, and the words beside them say where it stands. Ella Mobile's card
 * for the skill Ella is teaching is left out: there are no lessons here, and
 * a talk's aim is never told to Ella's model (see `ella_system_prompt`), so
 * she teaches no one skill.
 */
function LevelPage({
  level,
  count,
  step,
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
  /** The learner's step, on their own level. */
  step: number | null;
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
  const steps = level.steps.length;
  const sub =
    level.state === "done"
      ? "You’ve already passed this level."
      : current
        ? level.number < count
          ? `Step ${step} of ${steps} on the way to ${levelLabel(level.number + 1)}.`
          : `Step ${step} of ${steps}, the last level.`
        : `Opens when you finish ${levelLabel(level.number - 1)}.`;
  const skillLine = level.state === "done" ? "All done" : current ? `Step ${step} of ${steps}` : "Not started";

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
          {current && step !== null ? <OwnLevel level={level} step={step} /> : <OtherLevel level={level} />}
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

/** The learner's own level: the steps behind them, the one they are on, the
 * steps ahead, then what the circles mean. */
function OwnLevel({ level, step }: { level: LevelView; step: number }) {
  const behind = level.steps.filter((candidate) => candidate.number < step);
  const here = level.steps.find((candidate) => candidate.number === step);
  const ahead = level.steps.filter((candidate) => candidate.number > step);
  return (
    <div className="level-skills">
      {behind.length > 0 && (
        <ul className="step-rows">
          {behind.map((done) => (
            <StepRow key={`${level.number}:${done.number}`} step={done} />
          ))}
        </ul>
      )}
      {here && <CurrentStep step={here} />}
      {ahead.length > 0 && (
        <ul className="step-rows">
          {ahead.map((later) => (
            <StepRow key={`${level.number}:${later.number}`} step={later} opensAfter={`Step ${later.number - 1}`} />
          ))}
        </ul>
      )}
      <Legend />
    </div>
  );
}

/** A level other than the learner's own, a short row per step: done ones open
 * on their skills, and the rest say when they open. */
function OtherLevel({ level }: { level: LevelView }) {
  return (
    <div className="level-skills">
      <ul className="step-rows">
        {level.steps.map((step) => (
          <StepRow
            key={`${level.number}:${step.number}`}
            step={step}
            opensAfter={
              level.state === "done"
                ? undefined
                : step.number > 1
                  ? `Step ${step.number - 1}`
                  : levelLabel(level.number - 1)
            }
          />
        ))}
      </ul>
    </div>
  );
}

/**
 * A step as one short row: what it is called and where it stands. A done one
 * opens on its skills when pressed, and closes on a second press; a locked one,
 * `opensAfter` something, says what it waits on.
 */
function StepRow({ step, opensAfter }: { step: LevelStep; opensAfter?: string }) {
  const [open, setOpen] = useState(false);
  const chips = useId();
  const locked = opensAfter !== undefined;
  const face = (
    <>
      <span className="step-row__mark" aria-hidden="true">
        {locked ? <Lock size={13} /> : <Check size={15} />}
      </span>
      <span className="step-row__text">
        <span className="step-row__title">{step.title}</span>
        <span className="step-row__note">
          Step {step.number} · {locked ? `Opens after ${opensAfter}` : "Done"}
        </span>
      </span>
    </>
  );
  if (locked) {
    return (
      <li className="step-row is-locked">
        <div className="step-row__face">{face}</div>
      </li>
    );
  }
  return (
    <li className={`step-row is-done ${open ? "is-open" : ""}`.trim()}>
      <button className="step-row__face" onClick={() => setOpen(!open)} aria-expanded={open} aria-controls={chips}>
        {face}
        {/* Turned on its span: in the app's WebKit window, a turn on the svg itself never showed. */}
        <span className="step-row__chevron" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="16" height="16">
            <path d="M6 9l6 6 6-6" />
          </svg>
        </span>
      </button>
      <ul id={chips} className="skill-chips" aria-label={`Skills of step ${step.number}`} hidden={!open}>
        {step.skills.map((skill) => (
          <li key={skill.label} className={skill.passed ? "is-passed" : ""} title={skill.text}>
            {skill.passed && <Check size={11} />}
            {skill.label}
          </li>
        ))}
      </ul>
    </li>
  );
}

/** The step the learner is on, in their level's wash: "STEP 3 · YOU'RE HERE"
 * and its title, then each of its skills and where it stands. */
function CurrentStep({ step }: { step: LevelStep }) {
  const eyebrow = useId();
  const title = useId();
  return (
    <section className="step-now" aria-labelledby={`${eyebrow} ${title}`}>
      <p id={eyebrow} className="eyebrow step-now__eyebrow">
        Step {step.number} · You’re here
      </p>
      <h4 id={title} className="display step-now__title">
        {step.title}
      </h4>
      <ul className="skill-cards">
        {step.skills.map((skill) => (
          <SkillCard key={skill.label} skill={skill} />
        ))}
      </ul>
    </section>
  );
}

const STATE: Record<SkillStanding, string> = {
  done: "Done",
  shown: "Almost there",
  practising: "Practising",
  not_started: "Not started",
};

/** What to do next with a skill under way. Passing takes two topics, so the
 * hint asks for a new one only while it has shown on one. */
function hint(skill: LevelSkill): string | null {
  switch (skill.standing) {
    case "shown":
      return skill.topics.length < 2 ? "Use it again in a talk on a new topic." : "Use it again in your talks.";
    case "practising":
      return "Keep using it in your talks.";
    default:
      return null;
  }
}

/** One of the current step's skills: its circles and the talks it showed in,
 * its name, and where it stands. */
function SkillCard({ skill }: { skill: LevelSkill }) {
  const next = hint(skill);
  const topics = skill.topics.join(", ");
  return (
    <li className={`skill-card is-${skill.standing}`}>
      <span className="skill-card__top">
        <Circles standing={skill.standing} />
        {topics && (
          <span className="skill-card__topics" title={topics}>
            <span className="sr-only">Shown in </span>
            {topics}
          </span>
        )}
      </span>
      <span className="display skill-card__name" title={skill.text}>
        {skill.label}
      </span>
      <span className="sr-only">{skill.text}</span>
      <span className="skill-card__state">{STATE[skill.standing]}</span>
      {next && <span className="skill-card__hint">{next}</span>}
    </li>
  );
}

/**
 * A skill's two circles. The first fills once the skill shows in a talk, in
 * the level's colour; both fill, green, once it passes. The next to fill is
 * dashed in the level's colour once the skill is under way, and the rest in
 * grey.
 */
function Circles({ standing }: { standing: SkillStanding }) {
  const filled = standing === "done" ? 2 : standing === "shown" ? 1 : 0;
  const underWay = standing === "shown" || standing === "practising";
  return (
    <span className={`skill-circles is-${standing}`} aria-hidden="true">
      {[0, 1].map((index) => (
        <i key={index} className={index < filled ? "is-filled" : index === filled && underWay ? "is-next" : ""}>
          {index < filled && <Check size={12} />}
        </i>
      ))}
    </span>
  );
}

/** What the circles mean, under the steps: one filled, one to fill. */
function Legend() {
  return (
    <p className="skill-legend">
      <span className="skill-circles skill-circles--legend" aria-hidden="true">
        <i className="is-filled">
          <Check size={10} />
        </i>
        <i className="is-next" />
      </span>
      The first circle fills when you use a skill in a talk. Keep using it on different topics to fill the second.
    </p>
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
