import { useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { PartnerFigure } from "./CastScreen";
import { EllaMascot } from "./EllaMascot";
import { FlameGlyph } from "./Sidebar";
import { bridge } from "../lib/bridge";
import { levelTone } from "../lib/curriculum";
import {
  CHORE_RECAP,
  castName,
  goalFigure,
  reachedGoal,
  sceneBadge,
  streakRecap,
  type StreakRecap,
} from "../lib/presentation";
import { speakText } from "../lib/speech";
import type {
  Assessment,
  CastId,
  ChoreRecap,
  Fix,
  LearnerProgress,
  SessionSummary,
  SkillGrowth,
  Standing,
  TalkNotes,
} from "../types";

/**
 * The recap, as the Ella Desktop Recap design draws it: a band that celebrates
 * the talk (Ella cheering, or the talk partner and how the goal went), then
 * Ella's notes in tiles — what went well, one fix, the skills that grew — the
 * streak, and what is next.
 *
 * It opens at once, while the backend works out what the talk did; the notes
 * tiles wait with three dots and fill in when it answers. The app owns that
 * question, so leaving before the answer comes loses nothing. A talk too short
 * for notes says so straight away, and a talk that could not be read offers to
 * try again — the streak counts either way.
 *
 * Nothing on it moves: the scoring that fills it in, and the getting ready of
 * the next talk, run on the same CPU as this window.
 */
export function SummaryScreen({
  summary,
  assessment,
  assessError,
  before,
  learnerName,
  standing: standingBefore,
  nextTopic,
  onRetry,
  onCelebrated,
  onDone,
  onTryAgain,
  onLevels,
}: {
  summary: SessionSummary;
  /** Null until the backend has answered. */
  assessment: Assessment | null;
  /** Why it could not answer; asking again is safe. */
  assessError: string | null;
  /** The learner's figures from before this talk, which the streak counts on from. */
  before: LearnerProgress;
  learnerName: string;
  /** Where the learner stood as the talk began, until the assessment says
   * where it left them. */
  standing: Standing | null;
  /** The topic to suggest for tomorrow. */
  nextTopic: string | null;
  onRetry: () => void;
  /** The recap has shown the step or level this talk finished, so nothing
   * else needs to. */
  onCelebrated: () => void;
  onDone: () => void;
  /** Starts the same chore again, after a goal that was missed. */
  onTryAgain: (choreId: string) => void;
  onLevels: () => void;
}) {
  useEffect(() => {
    if (assessment?.advanced) onCelebrated();
    // Once per answer; `onCelebrated` is a fresh closure every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [assessment?.session_id, assessment?.advanced]);

  const [streakRun] = useState(() => streakRecap(before, summary.turns > 0));

  const chore = summary.chore ?? null;
  const notes = assessment?.notes ?? null;
  const standing = assessment?.standing ?? standingBefore;
  const tiles = tilesFor(summary, assessment, assessError);
  const noticeSpan = Math.max(1, 4 - tiles.length);
  const columns = tiles.reduce((sum, tile) => sum + (isNotice(tile) ? noticeSpan : 1), 1);

  return (
    <div className="screen screen--recap" data-screen="summary">
      <div className="recap">
        {chore ? (
          <RoleBand chore={chore} assessment={assessment} onLevels={onLevels} />
        ) : (
          <TalkBand
            summary={summary}
            first={firstName(learnerName)}
            assessment={assessment}
            onLevels={onLevels}
          />
        )}

        <div className="recap__tiles" style={{ "--columns": columns } as CSSProperties}>
          {tiles.map((tile) => (
            <Tile key={tile} kind={tile} span={isNotice(tile) ? noticeSpan : 1}>
              {tile === "well" ? (
                <WentWell notes={notes} ready={assessment !== null} />
              ) : tile === "fix" ? (
                <OneFix fix={notes?.fix ?? null} ready={assessment !== null} sessionId={summary.session_id} />
              ) : tile === "skills" ? (
                <SkillsGrew skills={assessment?.skills ?? null} />
              ) : tile === "failed" ? (
                <Failed message={assessError ?? ""} onRetry={onRetry} />
              ) : (
                <Notice
                  line={
                    summary.turns === 0
                      ? "Say a few words next time and it counts."
                      : summary.short
                        ? "Talk a little longer to get Ella’s notes."
                        : "No notes for this talk."
                  }
                />
              )}
            </Tile>
          ))}
          <StreakTile run={streakRun} />
        </div>

        <footer className="recap__foot">
          {standing && <LevelPill standing={standing} onLevels={onLevels} />}
          <div className="recap-next">
            <span className="recap-next__label">Tomorrow</span>
            <strong className="display recap-next__topic">{nextTopic ?? "Another talk with Ella"}</strong>
            {streakRun.counted && (
              <span className="recap-next__day">
                <FlameGlyph className="flame--chip" />
                <span className="display">Day {streakRun.to + 1}</span>
              </span>
            )}
          </div>
          {chore && !chore.met && (
            <button className="recap__button recap__button--again" onClick={() => onTryAgain(chore.chore_id)}>
              Try again
            </button>
          )}
          <button className="recap__button recap__button--done" onClick={onDone}>
            Done
          </button>
        </footer>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------ the bands --- */

function TalkBand({
  summary,
  first,
  assessment,
  onLevels,
}: {
  summary: SessionSummary;
  first: string;
  assessment: Assessment | null;
  onLevels: () => void;
}) {
  const hero =
    summary.turns === 0
      ? `Next time, ${first}!`
      : summary.short
        ? `Short and sweet, ${first}!`
        : `Nice talking, ${first}!`;
  const cheer = !summary.short;
  return (
    <section className="recap-band recap-band--talk">
      <div className="recap-band__text">
        <p className="recap-band__eyebrow">{summary.topic_label}</p>
        <h1 className={`display recap-band__hero recap-band__hero--${heroSize(hero)}`}>{hero}</h1>
        <MovedOn assessment={assessment} onLevels={onLevels} />
      </div>
      <div className="recap-band__ella">
        <EllaMascot
          variant="conversation"
          mood={cheer ? "cheer" : "calm"}
          scale={0.62}
          className="ella--recap"
          decorative
        />
      </div>
    </section>
  );
}

function RoleBand({
  chore,
  assessment,
  onLevels,
}: {
  chore: ChoreRecap;
  assessment: Assessment | null;
  onLevels: () => void;
}) {
  const told = CHORE_RECAP[chore.chore_id] ?? { track: "GOAL", agrees: "They agree" };
  const badge = sceneBadge(chore.chore_id);
  const name = castName(chore.character_id);
  return (
    <section className="recap-band recap-band--role" data-character={chore.character_id}>
      <div className="recap-band__text">
        <p className="recap-band__eyebrow">
          {name} · {told.track}
        </p>
        <h1 className="display recap-band__title">{chore.met ? "Goal met!" : "So close!"}</h1>
        <ul className="recap-goals" aria-label="Your goal">
          <GoalChip done={reachedGoal(chore)}>{goalFigure(chore)}</GoalChip>
          <GoalChip done={chore.agreed}>
            {told.agrees}
          </GoalChip>
        </ul>
        <MovedOn assessment={assessment} onLevels={onLevels} />
      </div>
      {badge && <RecapBadge name={badge} chore={chore} />}
      <div className="recap-band__partner" aria-hidden="true">
        <div className="recap-band__partner-box">
          <PartnerFigure id={chore.character_id as CastId} />
        </div>
      </div>
      {chore.last_line && (
        <p className="recap-band__bubble">
          <span className="sr-only">{name} said: </span>
          {chore.last_line}
        </p>
      )}
    </section>
  );
}

function GoalChip({ done, children }: { done: boolean; children: ReactNode }) {
  return (
    <li className={`recap-goal ${done ? "is-done" : ""}`.trim()}>
      <span className="recap-goal__tick" aria-hidden="true">
        {done && (
          <span className="recap-goal__fill">
            <Check size={13} />
          </span>
        )}
      </span>
      {children}
      <span className="sr-only">{done ? " (done)" : " (not this time)"}</span>
    </li>
  );
}

/** The badge a chore's goal earns: new the first time, earned again after,
 * and locked, with what earns it, when the goal was missed. */
function RecapBadge({ name, chore }: { name: string; chore: ChoreRecap }) {
  if (!chore.met) {
    return (
      <div className="recap-badge recap-badge--locked">
        <span className="recap-badge__lock" aria-hidden="true">
          <LockGlyph />
        </span>
        <div className="recap-badge__text">
          <strong className="display">{name}</strong>
          <p>
            Get it to {goalFigure(chore)} to earn {chore.times_met > 0 ? "it again" : "this badge"}.
          </p>
        </div>
      </div>
    );
  }
  const again = chore.times_met > 1;
  return (
    <div className={`recap-badge ${again ? "recap-badge--again" : "recap-badge--new"}`}>
      <span className="recap-badge__medal" aria-hidden="true">
        {!again && <span className="recap-badge__rays" />}
        <span className="recap-badge__disc">
          <SpeechGlyph />
        </span>
        {again && <span className="display recap-badge__count">×{chore.times_met}</span>}
      </span>
      <div className="recap-badge__text">
        <span className="recap-badge__label">{again ? "Earned again" : "New badge"}</span>
        <strong className="display">{name}</strong>
      </div>
    </div>
  );
}

/** "Step complete!" or "Level up!", in the new level's colour, when the talk
 * moved the learner on. It opens the level map. */
function MovedOn({ assessment, onLevels }: { assessment: Assessment | null; onLevels: () => void }) {
  if (!assessment?.advanced) return null;
  const { standing } = assessment;
  const level = assessment.advanced === "level";
  return (
    <button className={`recap-moved ladder-${levelTone(standing.level_number)}`} onClick={onLevels}>
      <span className="recap-moved__badge" aria-hidden="true">
        {level ? standing.level_number : <Check size={18} />}
      </span>
      <span className="recap-moved__text">
        <strong className="display">{level ? "Level up!" : "Step complete!"}</strong>
        <span>
          {level
            ? `You’ve reached ${standing.level_name}.`
            : `On to Step ${standing.step} of ${standing.level_name}.`}
        </span>
      </span>
    </button>
  );
}

/* ------------------------------------------------------------ the tiles --- */

type TileKey = "well" | "fix" | "skills" | "notice" | "failed";

function isNotice(tile: TileKey | "streak") {
  return tile === "notice" || tile === "failed";
}

/** Which tiles the recap has, in reading order. Before the answer it holds a
 * place for every note; once it lands, only what the talk has fills one. */
function tilesFor(summary: SessionSummary, assessment: Assessment | null, error: string | null): TileKey[] {
  const grew: TileKey[] = assessment && assessment.skills.length > 0 ? ["skills"] : [];
  if (summary.short) return ["notice", ...grew];
  if (error) return ["failed"];
  if (!assessment) return ["well", "fix", "skills"];
  const notes = assessment.notes;
  if (!notes) return [...grew, "notice"];
  return ["well", ...(notes.fix ? (["fix"] as TileKey[]) : []), ...grew];
}

const TILE_LABEL: Record<TileKey, string> = {
  well: "Went well",
  fix: "One fix",
  skills: "Skills grew",
  notice: "Ella’s notes",
  failed: "Ella’s notes",
};

function Tile({ kind, span, children }: { kind: TileKey; span: number; children: ReactNode }) {
  return (
    <section
      className={`recap-tile recap-tile--${kind}`}
      aria-label={TILE_LABEL[kind]}
      style={span > 1 ? { gridColumn: `span ${span}` } : undefined}
    >
      {children}
    </section>
  );
}

function Waiting() {
  return (
    <span className="recap-waiting" role="status" aria-label="Ella is looking back over your talk…">
      <i />
      <i />
      <i />
    </span>
  );
}

function WentWell({ notes, ready }: { notes: TalkNotes | null; ready: boolean }) {
  return (
    <>
      {ready ? (
        <span className="recap-well__check" aria-hidden="true">
          <Check size={32} />
        </span>
      ) : (
        <span className="recap-well__ring" aria-hidden="true" />
      )}
      <h2 className="display recap-tile__title">Went well</h2>
      {ready && notes ? <WellItems notes={notes} /> : <Waiting />}
    </>
  );
}

function WellItems({ notes }: { notes: TalkNotes }) {
  // "Nothing to fix" is only said when a model looked.
  const items = notes.checked && !notes.fix ? [...notes.went_well, "Nothing to fix"] : notes.went_well;
  return (
    <ul className="recap-well__list">
      {items.map((item) => (
        <li key={item} className={`display recap-well__item ${item.length > 22 ? "is-long" : ""}`.trim()}>
          <span className="recap-well__tick" aria-hidden="true">
            <Check size={15} />
          </span>
          {item}
        </li>
      ))}
    </ul>
  );
}

function OneFix({ fix, ready, sessionId }: { fix: Fix | null; ready: boolean; sessionId: string }) {
  return (
    <>
      <h2 className="display recap-tile__title">One fix</h2>
      {ready && fix ? <FixLines fix={fix} sessionId={sessionId} /> : <Waiting />}
    </>
  );
}

function FixLines({ fix, sessionId }: { fix: Fix; sessionId: string }) {
  const [hearing, setHearing] = useState(false);
  const stop = useRef<(() => void) | null>(null);
  const live = useRef(true);
  useEffect(
    () => () => {
      live.current = false;
      stop.current?.();
    },
    [],
  );

  /** Ella says the better line, in her own voice when the backend can give
   * it, and in the system's otherwise. */
  async function hear() {
    stop.current?.();
    setHearing(true);
    const done = () => {
      if (live.current) setHearing(false);
    };
    let audio = null;
    try {
      audio = (await bridge.speakFix?.(sessionId))?.audio ?? null;
    } catch {
      // Her own voice could not be had; the system's says it instead.
    }
    if (!live.current) return;
    stop.current = speakText(fix.better, audio, { onEnd: done, onError: done });
  }

  return (
    <>
      <p className={`display recap-fix__said ${fix.said.length > 20 ? "is-long" : ""}`.trim()}>
        <span className="sr-only">You said: </span>
        {fix.said}
      </p>
      <span className="recap-fix__arrow" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="26" height="26">
          <path d="M12 5v13" />
          <path d="M6 12.5l6 6 6-6" />
        </svg>
      </span>
      <p className={`display recap-fix__better recap-fix__better--${fixSize(fix.better)}`}>
        <span className="sr-only">Better: </span>
        {fix.better}
      </p>
      <button
        className={`recap-fix__hear ${hearing ? "is-hearing" : ""}`.trim()}
        onClick={() => void hear()}
        aria-label="Hear it"
      >
        {hearing ? (
          <span className="recap-fix__wave" aria-hidden="true">
            <i />
            <i />
            <i />
            <i />
          </span>
        ) : (
          <SpeakerGlyph />
        )}
      </button>
    </>
  );
}

/** The most blocks a skill's bar stacks. */
const MAX_BLOCKS = 5;

function SkillsGrew({ skills }: { skills: SkillGrowth[] | null }) {
  return (
    <>
      <h2 className="display recap-tile__title">Skills grew</h2>
      {skills ? <SkillBars skills={skills} /> : <Waiting />}
    </>
  );
}

/** Each skill a stack of blocks, one per talk that has shown it: today's on
 * top in green, the earlier ones under it in violet. */
function SkillBars({ skills }: { skills: SkillGrowth[] }) {
  return (
    <>
      <ul className="recap-skills">
        {skills.map((skill) => {
          const blocks = Math.max(1, Math.min(MAX_BLOCKS, skill.count));
          return (
            <li
              key={skill.label}
              className="recap-skill"
              aria-label={`${skill.label}: ${skill.count === 1 ? "first time" : `${skill.count} talks`}`}
            >
              <span className="recap-skill__bar" aria-hidden="true">
                <span className={`recap-skill__today ${blocks > 1 ? "has-rest" : ""}`.trim()} />
                {blocks > 1 && (
                  <span className="recap-skill__rest" style={{ "--blocks": blocks - 1 } as CSSProperties} />
                )}
              </span>
              <span className="display recap-skill__label" aria-hidden="true">
                {skill.label}
              </span>
            </li>
          );
        })}
      </ul>
      <p className="recap-skills__key" aria-hidden="true">
        <i />
        Today
      </p>
    </>
  );
}

function Notice({ line }: { line: string }) {
  return (
    <>
      <p className="recap-tile__eyebrow">Ella’s notes</p>
      <p className="display recap-notice__line">{line}</p>
    </>
  );
}

function Failed({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <>
      <span className="recap-failed__icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="36" height="36">
          <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" />
          <path d="M19.5 4.5v4h-4" />
        </svg>
      </span>
      <h2 className="display recap-failed__title">Not just now</h2>
      <p className="recap-failed__line" role="alert">
        {message} Your streak still counts.
      </p>
      <button className="recap__button recap__button--retry" onClick={onRetry}>
        Try again
      </button>
    </>
  );
}

/** The streak as it stands now, today's day already counted. */
function StreakTile({ run }: { run: StreakRecap }) {
  return (
    <section className="recap-tile recap-tile--streak" aria-label="Streak">
      <span className="recap-streak__flame">
        <FlameGlyph className="flame--recap" />
      </span>
      <p className="sr-only">
        {run.to} day streak{run.chip ? `. ${run.chip}` : ""}
      </p>
      <div className="display recap-streak__count" aria-hidden="true">
        <span>{run.to}</span>
      </div>
      <p className="display recap-streak__label" aria-hidden="true">
        day streak
      </p>
      {run.chip && (
        <span className="display recap-streak__chip" aria-hidden="true">
          {run.chip}
        </span>
      )}
      <ol className="recap-week" aria-hidden="true">
        {run.week.map((day, index) => (
          <li key={index} className={`recap-week__day is-${day.state}`}>
            <span className="recap-week__dot">
              {day.state === "done" && <Check size={14} />}
              {day.state === "new" && (
                <span className="recap-week__fill">
                  <Check size={14} />
                </span>
              )}
            </span>
            <small>{day.label}</small>
          </li>
        ))}
      </ol>
    </section>
  );
}

/* ----------------------------------------------------------- the footer --- */

function LevelPill({ standing, onLevels }: { standing: Standing; onLevels: () => void }) {
  return (
    <button
      className={`recap-level ladder-${levelTone(standing.level_number)}`}
      onClick={onLevels}
      aria-label={`Level ${standing.level_number}: ${standing.level_name}, Step ${standing.step} of ${standing.step_count}. See all levels`}
    >
      <span className="recap-level__text">
        <span className="recap-level__label">
          Level {standing.level_number} · Step {standing.step} of {standing.step_count}
        </span>
        <strong className="display">{standing.level_name}</strong>
      </span>
      <span className="recap-level__bar" aria-hidden="true">
        <span style={{ width: `${standing.percent}%` }} />
      </span>
    </button>
  );
}

/* -------------------------------------------------------------- helpers --- */

function firstName(name: string) {
  return name.trim().split(/\s+/)[0] || "friend";
}

/** The hero line's size, stepping down as it gets longer. */
function heroSize(line: string): "lg" | "md" | "sm" {
  return line.length <= 20 ? "lg" : line.length <= 26 ? "md" : "sm";
}

function fixSize(line: string): "lg" | "md" | "sm" {
  return line.length <= 16 ? "lg" : line.length <= 30 ? "md" : "sm";
}

function Check({ size }: { size: number }) {
  return (
    <svg className="recap-check" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </svg>
  );
}

function SpeakerGlyph() {
  return (
    <svg className="recap-speaker" viewBox="0 0 24 24" width="25" height="25" aria-hidden="true">
      <path className="recap-speaker__cone" d="M4 9.5h3.2L12 5.5v13l-4.8-4H4z" />
      <path d="M15.5 9a4 4 0 010 6" />
      <path d="M18.2 6.3a8 8 0 010 11.4" />
    </svg>
  );
}

function SpeechGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="52" height="52" aria-hidden="true">
      <path
        fill="currentColor"
        fillRule="evenodd"
        clipRule="evenodd"
        d="M12 2.9c5.3 0 9.6 3.5 9.6 7.8 0 4.3-4.3 7.8-9.6 7.8-.86 0-1.7-.09-2.5-.26l-3.9 1.86c-.9.43-1.86-.46-1.5-1.4l.94-2.5C3.4 14.9 2.4 13 2.4 10.7c0-4.3 4.3-7.8 9.6-7.8z"
      />
    </svg>
  );
}

function LockGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="42" height="42" aria-hidden="true">
      <rect x="5" y="10.5" width="14" height="10" rx="2.5" />
      <path d="M8 10.5V8a4 4 0 018 0v2.5" />
    </svg>
  );
}
