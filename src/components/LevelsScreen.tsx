import { useEffect, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { MicGlyph } from "./HomeScreen";
import { bridge } from "../lib/bridge";
import { levelTone } from "../lib/curriculum";
import type { LevelState, LevelView, Standing } from "../types";

/** Where a level's code would be, the learner reads its number. */
function levelLabel(number: number): string {
  return `level ${number}`;
}

/**
 * Every level, as Ella Mobile v7's Levels and level pages lay them out, side by
 * side on the desktop: the path on the left, the picked level on the right.
 *
 * A level's page is the curriculum's own: its goal, and its five steps, each a
 * bar with a segment per skill that fills once the skill passes, so filling
 * every bar is exactly what finishes the level. The learner's own step also
 * lists its skills in their "I can" words, which the desktop has the room for.
 */
export function LevelsScreen({
  standing,
  busy,
  onPractise,
  onFindLevel,
}: {
  standing: Standing | null;
  busy: boolean;
  /** Today's talk: every talk aims at a skill of the learner's step. */
  onPractise: () => void;
  /** The placement chat, for a learner who never had one. */
  onFindLevel: () => void;
}) {
  const [levels, setLevels] = useState<LevelView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<number | null>(null);

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

  const current = levels?.find((level) => level.state === "current");
  const shown = levels?.find((level) => level.number === picked) ?? current ?? levels?.[0];

  return (
    <div className="screen screen--scroll screen--levels" data-screen="levels">
      <header className="page-head">
        <h1 className="display page-title">Your levels</h1>
        <p className="page-head__sub">
          Six levels of five steps. Every talk quietly works on a skill of your step, and filling a
          step&rsquo;s skills moves you on.
        </p>
      </header>

      {!levels || !shown ? (
        <div className="levels-wait" aria-live="polite">
          {error ? (
            <p className="inline-error">{error}</p>
          ) : (
            <LoaderCircle className="spin" aria-label="Loading the levels" />
          )}
        </div>
      ) : (
        <div className="levels-grid">
          <ol className="level-path" aria-label="Levels">
            {levels.map((level, index) => (
              <LevelStop
                key={level.number}
                level={level}
                count={levels.length}
                step={standing?.step ?? 1}
                last={index === levels.length - 1}
                picked={level.number === shown.number}
                onPick={() => setPicked(level.number)}
              />
            ))}
          </ol>
          <LevelPage
            level={shown}
            count={levels.length}
            standing={standing}
            busy={busy}
            onPractise={onPractise}
            onFindLevel={onFindLevel}
          />
        </div>
      )}
    </div>
  );
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
  step,
  last,
  picked,
  onPick,
}: {
  level: LevelView;
  count: number;
  step: number;
  last: boolean;
  picked: boolean;
  onPick: () => void;
}) {
  const here = level.steps.find((entry) => entry.number === step);
  const passed = here?.skills.filter((skill) => skill.passed).length ?? 0;
  return (
    <li className={`level-stop is-${level.state} ladder-${levelTone(level.number)} ${last ? "is-last" : ""} ${picked ? "is-picked" : ""}`.trim()}>
      <button className="level-stop__button" onClick={onPick} aria-pressed={picked}>
        <span className="level-stop__node" aria-hidden="true">
          {level.number}
          {level.state === "done" && (
            <span className="level-stop__tick">
              <Check />
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
                  Step {step} of {level.steps.length} · {passed} of {here.skills.length} skills
                </small>
              )}
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
  busy,
  onPractise,
  onFindLevel,
}: {
  level: LevelView;
  count: number;
  standing: Standing | null;
  busy: boolean;
  onPractise: () => void;
  onFindLevel: () => void;
}) {
  const current = level.state === "current";
  const reached = level.state === "done" || current;
  const total = level.steps.reduce((sum, step) => sum + step.skills.length, 0);
  const filled = level.steps.reduce((sum, step) => sum + step.skills.filter((skill) => skill.passed).length, 0);
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
      className={`panel level-page ladder-${levelTone(level.number)} is-${level.state}`}
      aria-label={`Level ${level.number}: ${level.name}`}
    >
      <header className="level-page__head">
        <span className="display level-page__number" aria-hidden="true">
          {level.number}
        </span>
        <div className="level-page__title">
          <h2 className="display">{level.name}</h2>
          <span className="level-page__pill">{PILL[level.state]}</span>
          <p>{sub}</p>
        </div>
      </header>

      <h3 className="level-page__heading">The goal</h3>
      <p className="level-page__goal">{level.goal}</p>

      <div className="level-page__heading level-page__heading--row">
        <h3>Steps</h3>
        <span>{skillLine}</span>
      </div>
      <ol className="level-steps">
        {level.steps.map((step) => {
          const yours = current && step.number === standing?.step;
          return (
            <li key={step.number} className={`level-step ${yours ? "is-yours" : ""}`.trim()}>
              <div className="level-step__row">
                <span className="level-step__title">
                  <span className="level-step__number">{step.number}</span>
                  {step.title}
                  {yours && <span className="level-step__here">Your step</span>}
                </span>
                <span className="level-step__bar" aria-label={`${step.skills.filter((skill) => skill.passed).length} of ${step.skills.length} skills`}>
                  {step.skills.map((skill) => (
                    <i key={skill.label} className={skill.passed ? "is-passed" : ""} title={skill.text} />
                  ))}
                </span>
              </div>
              {yours && (
                <>
                  <p className="level-step__focus">{step.focus}</p>
                  <ul className="level-step__skills">
                    {step.skills.map((skill) => (
                      <li key={skill.label} className={skill.passed ? "is-passed" : ""}>
                        <span className="level-step__check" aria-hidden="true">
                          <Check />
                        </span>
                        <span>
                          {skill.text}
                          <span className="sr-only">{skill.passed ? " (done)" : " (still to show)"}</span>
                        </span>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </li>
          );
        })}
      </ol>

      {current && (
        <>
          <p className="level-page__note">
            {level.number < count
              ? `Fill every bar to reach ${levelLabel(level.number + 1)}.`
              : "Fill every bar to finish this level."}
          </p>
          <div className="level-page__actions">
            <button className="btn btn--violet level-page__practise" onClick={onPractise} disabled={busy}>
              <MicGlyph size={17} />
              Practise with Ella
            </button>
            {standing && !standing.placed && (
              <button className="btn btn--quiet" onClick={onFindLevel} disabled={busy}>
                Not sure? Find my level
              </button>
            )}
          </div>
        </>
      )}
    </section>
  );
}

function Check() {
  return (
    <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true">
      <path d="M20 6L9 17L4 12" />
    </svg>
  );
}

function message(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "Something went wrong. Please try again.";
}
