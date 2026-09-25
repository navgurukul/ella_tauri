import { useState } from "react";
import { EllaMascot } from "./EllaMascot";
import { FlameGlyph } from "./Sidebar";
import {
  recommendedTopicId,
  streak,
  topicMeta,
  topicPresentation,
  unfinishedSession,
  weeklyDigest,
} from "../lib/presentation";
import type { AppSnapshot, Tone, Topic } from "../types";

/** The four cards under "More topics": one tall, two small, one wide, each in
 * its own colour. Topics fill them in the order the backend offers them. */
const SLOTS: Array<{ slot: "tall" | "small" | "wide"; tone: Tone }> = [
  { slot: "tall", tone: "green" },
  { slot: "small", tone: "pink" },
  { slot: "small", tone: "orange" },
  { slot: "wide", tone: "ink" },
];

/** "View all" lays every other topic out evenly, cycling these colours. */
const ALL_TONES: Tone[] = ["green", "pink", "orange", "ink", "violet"];

export function HomeScreen({
  snapshot,
  busy,
  onStart,
  onResume,
}: {
  snapshot: AppSnapshot;
  busy: boolean;
  onStart: (topic: Topic) => void;
  onResume: (sessionId: string) => void;
}) {
  const [showAll, setShowAll] = useState(false);
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
  const shown = showAll ? others : others.slice(0, SLOTS.length);

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
              <EllaMascot variant="home" className="ella--home-peek" pokeable decorative />
              <div className="today__card">
                <div className="today__text">
                  <p className="eyebrow">Today&rsquo;s talk</p>
                  <h2 className="today__title">{featured.label}</h2>
                  <p className="today__blurb">{topicPresentation(featured.id).blurb}</p>
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
              <button className="section-head__link" onClick={() => setShowAll((all) => !all)}>
                {showAll ? "Show fewer" : "View all"}
              </button>
            )}
          </div>

          <div className={`topics ${showAll ? "topics--all" : ""}`.trim()}>
            {shown.map((topic, index) => (
              <TopicCard
                key={topic.id}
                topic={topic}
                slot={showAll ? "even" : SLOTS[index].slot}
                tone={showAll ? ALL_TONES[index % ALL_TONES.length] : SLOTS[index].tone}
                disabled={busy}
                onStart={onStart}
              />
            ))}
          </div>
        </div>

        <aside className="rail">
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
              <div>
                <dt className="display">{digest.answers}</dt>
                <dd>{digest.answers === 1 ? "answer spoken" : "answers spoken"}</dd>
              </div>
            </dl>
          </section>
        </aside>
      </div>
    </div>
  );
}

function TopicCard({
  topic,
  slot,
  tone,
  disabled,
  onStart,
}: {
  topic: Topic;
  slot: "tall" | "small" | "wide" | "even";
  tone: Tone;
  disabled: boolean;
  onStart: (topic: Topic) => void;
}) {
  return (
    <button
      className={`topic topic--${slot} tone-${tone}`}
      disabled={disabled}
      onClick={() => onStart(topic)}
    >
      <span className="topic__title">{topic.label}</span>
      {slot === "tall" && (
        <span className="mono topic__bubble">{topicPresentation(topic.id).sample}</span>
      )}
      <span className="mono topic__meta">{topicMeta(topic.id)}</span>
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
