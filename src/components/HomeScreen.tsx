import type { CSSProperties } from "react";
import { EllaMascot } from "./EllaMascot";
import { LevelCard } from "./LevelCard";
import { FlameGlyph } from "./Sidebar";
import {
  recommendedTopicId,
  spokenAmount,
  spokenTimeKnown,
  streak,
  unfinishedSession,
  weeklyDigest,
} from "../lib/presentation";
import type { AppSnapshot, Tone, Topic } from "../types";

/** The four cards under "More topics": one tall, two small, one wide, each in
 * its own colour. Topics fill them in the order the backend offers them. */
const SLOTS: Array<{ slot: TopicSlot; tone: Tone }> = [
  { slot: "tall", tone: "green" },
  { slot: "small", tone: "pink" },
  { slot: "small", tone: "orange" },
  { slot: "wide", tone: "ink" },
];

export function HomeScreen({
  snapshot,
  busy,
  mood = "eager",
  onStart,
  onResume,
  onLevels,
  onAllTopics,
}: {
  snapshot: AppSnapshot;
  busy: boolean;
  /** How Ella greets the learner, as on Ella Mobile: eager for the next talk,
   * or happy once they have finished one. */
  mood?: "eager" | "happy";
  onStart: (topic: Topic) => void;
  onResume: (sessionId: string) => void;
  onLevels: () => void;
  /** "View all": every topic at the learner's level, on a page of its own. */
  onAllTopics: () => void;
}) {
  const name = snapshot.learner?.name ?? "friend";
  const digest = weeklyDigest(snapshot.progress);
  const run = streak(snapshot.progress);
  // `streak` marks today "done" once the learner has given an answer today, so
  // a remaining "today" cell means they have not said anything to Ella yet.
  const talkedToday = !run.week.some((day) => day.state === "today");

  const recommended = recommendedTopicId(snapshot);
  const featured =
    snapshot.topics.find((topic) => topic.id === recommended) ?? snapshot.topics[0];
  const others = snapshot.topics.filter((topic) => topic.id !== featured?.id);
  const shown = others.slice(0, SLOTS.length);

  const unfinished = unfinishedSession(snapshot);

  return (
    <div className="screen screen--home" data-screen="home">
      <header className="page-head">
        <h1 className="display">Namaste, {name}!</h1>
        <p className="page-head__sub">Ready for today&rsquo;s talk?</p>
      </header>

      <div className="home-body">
        <div className="home-main">
          {featured && (
            <section className="today">
              <EllaMascot
                variant="home"
                className="ella--home-peek"
                expression={mood === "happy" ? "homeHappy" : "homeEager"}
                decorative
              />
              <div className="today__card">
                <div className="today__text">
                  <p className="eyebrow">Today&rsquo;s talk</p>
                  <h2 className="today__title">{featured.label}</h2>
                  <p className="today__blurb">{featured.blurb}</p>
                </div>
                <button className="btn btn--violet btn--talk" disabled={busy} onClick={() => onStart(featured)}>
                  <MicGlyph size={17} />
                  Start talking
                </button>
              </div>
            </section>
          )}

          <div className="section-head">
            <h3>More topics</h3>
            {others.length > SLOTS.length && (
              <button className="section-head__link" onClick={onAllTopics}>
                View all
              </button>
            )}
          </div>

          <div className="topics">
            {shown.map((topic, index) => (
              <TopicCard
                key={topic.id}
                topic={topic}
                slot={SLOTS[index].slot}
                tone={SLOTS[index].tone}
                disabled={busy}
                onStart={onStart}
              />
            ))}
          </div>
        </div>

        <aside className="rail">
          {snapshot.standing && <LevelCard standing={snapshot.standing} onOpen={onLevels} />}

          {unfinished && (
            <section className="card card--resume">
              <p className="eyebrow">Unfinished talk</p>
              <h4>{unfinished.topic_label}</h4>
              <p className="card__note">
                You left this conversation open. Ella still remembers where you were.
              </p>
              <button
                className="btn btn--violet btn--block"
                disabled={busy}
                onClick={() => onResume(unfinished.id)}
              >
                Continue talking
              </button>
            </section>
          )}

          <section className="card card--streak">
            <span className="card--streak__orb" aria-hidden="true" />
            <div className="streak-head">
              <FlameGlyph />
              <div>
                <strong className="display">
                  {run.days} {run.days === 1 ? "day" : "days"}
                </strong>
                <small>talking streak</small>
              </div>
            </div>
            <ol className="week">
              {run.week.map((day, index) => (
                <li key={index} className={`week__day is-${day.state}`}>
                  <span className="week__mark">
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M20 6L9 17L4 12" />
                    </svg>
                  </span>
                  <small>{day.label}</small>
                </li>
              ))}
            </ol>
            <p className="streak-foot">
              {talkedToday
                ? "Nice, today is already done."
                : `Talk today to make it ${run.days + 1}!`}
            </p>
          </section>

          <section className="card card--week">
            <p className="eyebrow">This week</p>
            <dl className="stats">
              <div>
                <dt className="display">{digest.talks}</dt>
                <dd>{digest.talks === 1 ? "talk" : "talks"}</dd>
              </div>
              <WeekSpoken spokenMs={digest.spokenMs} answers={digest.answers} known={spokenTimeKnown(snapshot.progress)} />
            </dl>
          </section>
        </aside>
      </div>
    </div>
  );
}

/**
 * This week's time spoken, as the design has it. Until Ella has kept the
 * length of a spoken answer there is no time to add up — answers from before
 * she kept it, and typed ones, have none — so the answers stand in for it.
 */
function WeekSpoken({ spokenMs, answers, known }: { spokenMs: number; answers: number; known: boolean }) {
  if (!known) {
    return (
      <div>
        <dt className="display">{answers}</dt>
        <dd>{answers === 1 ? "answer spoken" : "answers spoken"}</dd>
      </div>
    );
  }
  const { value, unit } = spokenAmount(spokenMs);
  return (
    <div>
      <dt className="display">{value}</dt>
      <dd>{unit} spoken</dd>
    </div>
  );
}

/** A topic card's shape: Home's tall card quotes Ella's opener, a small one
 * stacks its title over its meta line, and a wide one sets them side by side. */
export type TopicSlot = "tall" | "small" | "wide";

export function TopicCard({
  topic,
  slot,
  tone,
  disabled,
  onStart,
  style,
}: {
  topic: Topic;
  slot: TopicSlot;
  tone: Tone;
  disabled: boolean;
  onStart: (topic: Topic) => void;
  /** Where "All topics" places it in its grid. */
  style?: CSSProperties;
}) {
  return (
    <button
      className={`topic topic--${slot} tone-${tone}`}
      disabled={disabled}
      onClick={() => onStart(topic)}
      style={style}
    >
      <span className="topic__title">{topic.label}</span>
      {slot === "tall" && (
        <span className="mono topic__bubble">
          <span className="topic__quote">{topic.opener}</span>
        </span>
      )}
      <span className="mono topic__meta">{topic.meta}</span>
    </button>
  );
}

export function MicGlyph({ size = 36 }: { size?: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill="currentColor" aria-hidden="true">
      <path d="M12 3a3 3 0 013 3v5a3 3 0 01-6 0V6a3 3 0 013-3z" />
      <path d="M6 11a6 6 0 0012 0h2a8 8 0 01-7 7.94V21h-2v-2.06A8 8 0 014 11z" />
    </svg>
  );
}
