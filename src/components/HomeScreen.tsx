import { EllaMascot } from "./EllaMascot";
import {
  BENTO_SLOTS,
  CATEGORY_LABEL,
  levelInfo,
  recommendedTopicId,
  streak,
  topicMeta,
  topicPresentation,
  unfinishedSession,
  units,
  weeklyDigest,
} from "../lib/presentation";
import type {
  AppSnapshot,
  SessionListItem,
  Topic,
  TopicPresentation,
  TopicSlot,
} from "../types";

export function HomeScreen({
  snapshot,
  busy,
  onStart,
  onResume,
  onGarden,
}: {
  snapshot: AppSnapshot;
  busy: boolean;
  onStart: (topic: Topic) => void;
  onResume: (sessionId: string) => void;
  onGarden: () => void;
}) {
  const name = snapshot.learner?.name ?? "friend";
  const level = levelInfo(snapshot.garden, snapshot.learner?.level_name);
  const digest = weeklyDigest(snapshot);
  const run = streak(snapshot.recent_sessions);
  const blooming = units(snapshot.garden, snapshot.topics).filter(
    (unit) => unit.state === "young" || unit.state === "seedling",
  ).length;
  // `streak` marks today "done" the moment there is a session for it, so a
  // remaining "today" cell means the learner has not talked yet.
  const talkedToday = !run.week.some((day) => day.state === "today");

  // Ella leads with the least-practised strand; everything else fills the
  // bento in backend order, taking its shape from the slot it lands in.
  const recommended = recommendedTopicId(snapshot);
  const featured =
    snapshot.topics.find((topic) => topic.id === recommended) ?? snapshot.topics[0];
  const grid = snapshot.topics.filter((topic) => topic.id !== featured?.id).slice(0, 6);

  const unfinished = unfinishedSession(snapshot);
  const finished = snapshot.recent_sessions.filter((session) => session.status === "complete");

  const topicById = (topicId: string) => snapshot.topics.find((topic) => topic.id === topicId);

  return (
    <div className="screen screen--home" data-screen="home">
      <header className="page-head">
        <h1 className="display">
          Namaste, {name}!
          <svg className="page-head__flourish" viewBox="0 0 40 20" aria-hidden="true">
            <path d="M2 16 Q 12 2 22 12 T 38 8" />
          </svg>
        </h1>
        <p className="page-head__sub">Ella is ready when you are.</p>
      </header>

      <div className="home-body">
        <div className="home-main">
          {featured && (
            <section className="hero">
              <p className="hero__eyebrow">Ella recommends</p>
              <h2 className="hero__title">{featured.label}</h2>
              <p className="hero__blurb">{topicPresentation(featured.id).blurb}</p>
              <div className="hero__actions">
                <button className="btn btn--light" disabled={busy} onClick={() => onStart(featured)}>
                  <span className="btn__mic">
                    <MicGlyph size={13} />
                  </span>
                  Start talking
                </button>
                <span className="pill pill--ghost">
                  {level.code} · {CATEGORY_LABEL[topicPresentation(featured.id).category].toLowerCase()}
                </span>
              </div>
            </section>
          )}

          <div className="section-head">
            <h3>Or pick another topic</h3>
            <span>picked for Level {level.code}</span>
          </div>

          <div className="bento">
            {grid.map((topic, index) => (
              <TopicCard
                key={topic.id}
                topic={topic}
                slot={BENTO_SLOTS[index]}
                presentation={topicPresentation(topic.id)}
                disabled={busy}
                onStart={onStart}
              />
            ))}
          </div>
        </div>

        <aside className="rail">
          {unfinished && (
            <section className="card card--resume">
              <p className="card__eyebrow card__eyebrow--dark">Unfinished talk</p>
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
                <strong className="display display--sm">
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
                ? "Nice — today is already done."
                : `Talk today to make it ${run.days + 1}!`}
            </p>
          </section>

          <section className="card card--garden">
            <div className="card__head">
              <h4>Garden {level.code}</h4>
              <span className="pill pill--green">
                {level.skillsDone} / {level.skillsTotal} skills
              </span>
            </div>
            <div className="meter">
              <span style={{ width: `${Math.round(level.ratio * 100)}%` }} />
            </div>
            <p className="card__note">
              {blooming > 0
                ? `${blooming} ${blooming === 1 ? "plant is" : "plants are"} close to blooming. One good conversation could do it.`
                : "Your plot is ready. One conversation plants the first seed."}
            </p>
            <button className="btn btn--outline btn--block" onClick={onGarden}>
              Visit garden
            </button>
          </section>

          <section className="card card--week">
            <p className="card__eyebrow">This week</p>
            <dl className="stats">
              <div>
                <dt className="display display--sm">{digest.talks}</dt>
                <dd>talks</dd>
              </div>
              <div>
                <dt className="display display--sm">{digest.newWords}</dt>
                <dd>new words</dd>
              </div>
              <div>
                <dt className="display display--sm">{digest.blooms}</dt>
                <dd>blooms</dd>
              </div>
            </dl>
          </section>

          {finished.length > 0 && (
            <section className="card card--recent">
              <p className="card__eyebrow card__eyebrow--dark">Recent talks</p>
              <ul className="recent">
                {finished.map((session) => (
                  <RecentTalk
                    key={session.id}
                    session={session}
                    topic={topicById(session.topic_id)}
                    disabled={busy}
                    onStart={onStart}
                  />
                ))}
              </ul>
            </section>
          )}
        </aside>
      </div>

      <EllaMascot className="ella--corner-home" scale={0.7} rotate={-5} />
    </div>
  );
}

/**
 * A finished conversation cannot be reopened — the backend refuses turns on it
 * — so the action here starts a fresh talk on the same topic.
 */
function RecentTalk({
  session,
  topic,
  disabled,
  onStart,
}: {
  session: SessionListItem;
  topic?: Topic;
  disabled: boolean;
  onStart: (topic: Topic) => void;
}) {
  const turns = Math.max(0, Math.floor((session.message_count - 1) / 2));
  return (
    <li className="recent__item">
      <span className="recent__text">
        <strong>{session.topic_label}</strong>
        <small className="mono">
          {turns} {turns === 1 ? "ANSWER" : "ANSWERS"}
        </small>
      </span>
      {topic && (
        <button
          className="link-button"
          disabled={disabled}
          onClick={() => onStart(topic)}
        >
          Talk again
        </button>
      )}
    </li>
  );
}

function TopicCard({
  topic,
  slot,
  presentation,
  disabled,
  onStart,
}: {
  topic: Topic;
  slot: TopicSlot;
  presentation: TopicPresentation;
  disabled: boolean;
  onStart: (topic: Topic) => void;
}) {
  const meta = topicMeta(topic.id);
  const words = topic.label.split(" ");
  const split = Math.ceil(words.length / 2);

  return (
    <button
      className={`topic topic--${slot} tone-${presentation.tone}`}
      disabled={disabled}
      onClick={() => onStart(topic)}
    >
      {slot === "wide" && (
        <>
          <span className="topic__headline">
            <span>{words.slice(0, split).join(" ")}</span>
            <span className="topic__headline--indent">{words.slice(split).join(" ")}</span>
          </span>
          <span className="topic__badge" aria-hidden="true">
            <i />
          </span>
          <span className="mono topic__meta">{meta}</span>
        </>
      )}

      {slot === "wave" && (
        <>
          <svg className="topic__wave" viewBox="0 0 60 24" aria-hidden="true">
            <path d="M2 21q9-15 18-7t18-7t18-7" />
          </svg>
          <span className="topic__title">{topic.label}</span>
          <span className="mono topic__meta">{meta}</span>
        </>
      )}

      {slot === "framed" && (
        <>
          <span className="topic__title topic__title--center">{topic.label}</span>
          <span className="mono topic__meta topic__meta--center">[ {meta} ]</span>
          <span className="topic__foot">
            <span className="bubble bubble--dark">
              {presentation.sample}
              <i aria-hidden="true" />
            </span>
            <span className="avatars" aria-hidden="true">
              <i className="tone-violet">E</i>
              <i className="tone-orange">AA</i>
              <i className="tone-green">+</i>
            </span>
          </span>
        </>
      )}

      {slot === "inset" && (
        <span className="topic__inset">
          <span className="mono topic__inset-top">
            <span>This week</span>
            <span>~{presentation.minutes} min</span>
          </span>
          <span className="topic__title">{topic.label}</span>
          <span className="mono topic__meta">{CATEGORY_LABEL[presentation.category]}</span>
        </span>
      )}

      {slot === "chat" && (
        <>
          <span className="mono topic__meta topic__meta--top">{meta}</span>
          <span className="topic__title">{topic.label}</span>
          <span className="chat">
            <span className="chat__ella">
              <i aria-hidden="true">E</i>
              <span className="mono bubble bubble--light">{presentation.sample}</span>
            </span>
            <span className="mono bubble bubble--green">{presentation.reply}</span>
          </span>
        </>
      )}

      {slot === "quote" && (
        <>
          <span className="mono bubble bubble--white">
            {presentation.sample}
            <i aria-hidden="true" />
          </span>
          <span className="topic__title topic__title--foot">{topic.label}</span>
          <span className="mono topic__meta">{meta}</span>
        </>
      )}
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

function FlameGlyph() {
  return (
    <span className="flame" aria-hidden="true">
      <i />
      <i />
    </span>
  );
}
