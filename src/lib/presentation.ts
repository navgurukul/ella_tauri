/**
 * Everything the Ella Desktop design puts on screen that the Rust backend does
 * not model yet lives here, in one file, so it is obvious what is real and what
 * is editorial.
 *
 * Derived from real data:  the streak length and week strip, talks and answers
 *                          (this week and overall), which badges are earned —
 *                          all read off the learner's whole history, which the
 *                          backend sums up in `AppSnapshot.progress`.
 * Placeholder (marked):    per-topic category, duration, blurb and sample
 *                          line; the talk partners' names and blurbs, and the
 *                          one goal with nothing behind it yet.
 *
 * When the backend grows these fields, delete the matching constant and read
 * the snapshot instead — the component API does not have to change.
 */
import { addDays, dayKey } from "./days";
import type {
  AppSnapshot,
  Badge,
  CastMember,
  LearnerProgress,
  SessionListItem,
  Streak,
  StreakDay,
  TalkTally,
  TopicPresentation,
} from "../types";

/**
 * PLACEHOLDER — category, duration, the blurb and the sample line are editorial
 * metadata about each topic. Every topic carries a sample because the card that
 * prints one is decided by position, so any topic can land in it.
 */
export const TOPIC_PRESENTATION: Record<string, TopicPresentation> = {
  "street-food": {
    category: "fluency",
    minutes: 6,
    blurb: "Describe tastes, smells and your favourite stall. About 6 minutes of talking.",
    sample: "What did you eat?",
  },
  "restaurant-order": {
    category: "role-play",
    minutes: 5,
    blurb: "Order a meal, ask what is in it, and settle the bill.",
    sample: "What would you like?",
  },
  "booking-a-cab": {
    category: "vocabulary",
    minutes: 4,
    blurb: "Give an address, agree a fare, and ask how long it takes.",
    sample: "Where to, madam?",
  },
  "job-interview": {
    category: "role-play",
    minutes: 6,
    blurb: "Introduce yourself and answer questions about your work.",
    sample: "Tell me about yourself.",
  },
  "doctor-clinic": {
    category: "vocabulary",
    minutes: 5,
    blurb: "Explain how you feel and understand what to do next.",
    sample: "How are you feeling?",
  },
  "asking-directions": {
    category: "grammar",
    minutes: 4,
    blurb: "Find your way, then repeat the directions back.",
    sample: "Where is the bus stop?",
  },
  "market-bargaining": {
    category: "fluency",
    minutes: 5,
    blurb: "Ask the price, bargain kindly, and agree a deal.",
    sample: "What's your best price?",
  },
};

const FALLBACK_PRESENTATION: TopicPresentation = {
  category: "fluency",
  minutes: 5,
  blurb: "A short, friendly conversation to keep your English moving.",
  sample: "Shall we talk?",
};

export function topicPresentation(topicId: string): TopicPresentation {
  return TOPIC_PRESENTATION[topicId] ?? FALLBACK_PRESENTATION;
}

export const CATEGORY_LABEL: Record<TopicPresentation["category"], string> = {
  "role-play": "ROLE-PLAY",
  vocabulary: "VOCABULARY",
  grammar: "GRAMMAR",
  fluency: "FLUENCY",
};

/** `ROLE-PLAY · ~5 MIN`, the mono micro-label on every topic card. */
export function topicMeta(topicId: string): string {
  const presentation = topicPresentation(topicId);
  return `${CATEGORY_LABEL[presentation.category]} · ~${presentation.minutes} MIN`;
}

/**
 * The talk partners. Three goals are real chores from `chores()` in the Rust
 * domain and start through `start_chore`, which plays the character and keeps
 * the score; the doctor's goal opens the matching free topic; the debate has no
 * chore behind it yet, so it is shown but cannot start. `minAge` mirrors each
 * chore's `min_age`, which `catalog_for` filters on.
 *
 * PLACEHOLDER — the names, blurbs and minutes are the design's. The backend's
 * personas still call the stall owner Ramesh and the landlord Mr Khanna.
 */
export const CAST: CastMember[] = [
  {
    id: "stall-owner",
    name: "Bippo",
    blurb: "Has sold cloth in this market for twenty years.",
    goals: [
      {
        id: "market-cloth-price",
        title: "Talk a stall price down",
        goal: "Get the price down to Rs 400 or less, and get him to agree.",
        track: "NEGOTIATION",
        minutes: 5,
        minAge: 10,
        start: { kind: "chore", choreId: "market-cloth-price" },
      },
    ],
  },
  {
    id: "landlord",
    name: "Grumble",
    blurb: "Polite, always busy, and hard to convince.",
    goals: [
      {
        id: "deposit-refund",
        title: "Get a deposit refunded",
        goal: "Get him to agree to return at least Rs 3500 of the deposit.",
        track: "TRANSACTIONS",
        minutes: 6,
        minAge: 14,
        start: { kind: "chore", choreId: "deposit-refund" },
      },
      {
        id: "sell-me-a-pen",
        title: "Sell me a pen",
        goal: "Find out what they need, say why this pen helps, and ask for the sale.",
        track: "WORK",
        minutes: 4,
        minAge: 14,
        start: { kind: "chore", choreId: "sell-me-a-pen" },
      },
    ],
  },
  {
    id: "doctor",
    name: "Dr Wobble",
    blurb: "A calm doctor who asks a lot of questions.",
    goals: [
      {
        id: "doctor-clinic",
        title: "Explain what is wrong",
        goal: "Describe how you feel and since when, then ask what to do next.",
        track: "HEALTH",
        minutes: 5,
        minAge: 0,
        start: { kind: "topic", topicId: "doctor-clinic" },
      },
    ],
  },
  {
    id: "debater",
    name: "Zig",
    blurb: "Loves an argument and never agrees first.",
    goals: [
      {
        id: "take-a-stand",
        title: "Take a stand",
        goal: "Pick a side on school uniforms and give two reasons he cannot knock down.",
        track: "DEBATE",
        minutes: 7,
        minAge: 0,
        start: { kind: "soon" },
      },
    ],
  },
];

/** The cast a learner of this age is offered: goals they are too young for are
 * left out, and so is a partner with nothing left to offer. */
export function castFor(age: number | null | undefined): CastMember[] {
  return CAST.map((member) => ({
    ...member,
    goals: member.goals.filter((goal) => age == null || goal.minAge <= age),
  })).filter((member) => member.goals.length > 0);
}

const DAY_INITIALS = ["S", "M", "T", "W", "T", "F", "S"];

/**
 * How many days in a row the learner has talked, and this week's strip. A day
 * counts once they have given an answer on it — opening a talk and saying
 * nothing does not — and the backend buckets those days on the learner's own
 * clock, so the keys below are made the same way. However many talks there
 * are in a day, it is still one day.
 */
export function streak(progress: LearnerProgress, today = new Date()): Streak {
  const talkedOn = new Set(progress.days.map((day) => day.day));
  const todayKey = dayKey(today);

  let days = 0;
  for (let back = 0; ; back += 1) {
    const key = dayKey(addDays(today, -back));
    if (!talkedOn.has(key)) {
      // Today not being done yet does not break a streak that ran to yesterday.
      if (back === 0) continue;
      break;
    }
    days += 1;
  }

  // Monday-first week containing today.
  const weekStart = addDays(today, -((today.getDay() + 6) % 7));
  const week: StreakDay[] = Array.from({ length: 7 }, (_, index) => {
    const date = addDays(weekStart, index);
    const key = dayKey(date);
    const state: StreakDay["state"] = talkedOn.has(key)
      ? "done"
      : key === todayKey
        ? "today"
        : "future";
    return { label: DAY_INITIALS[date.getDay()], state };
  });

  return { days, week };
}

/**
 * Talks and the answers spoken in them over the last seven calendar days,
 * today included. The design's "minutes spoken" has nothing to be derived
 * from — no session carries how long anyone spoke — so the answers stand in
 * for it. Each talk sits on the day of its first answer, so one that ran past
 * midnight is counted once.
 */
export function weeklyDigest(progress: LearnerProgress, today = new Date()): TalkTally {
  const week = new Set(Array.from({ length: 7 }, (_, back) => dayKey(addDays(today, -back))));
  return progress.days
    .filter((day) => week.has(day.day))
    .reduce<TalkTally>(
      (sum, day) => ({ talks: sum.talks + day.talks, answers: sum.answers + day.answers }),
      { talks: 0, answers: 0 },
    );
}

/**
 * Finished talks and every answer given, for the profile, over the learner's
 * whole history. A talk counts as finished once it was ended with at least one
 * answer in it; the answers include talks that were never finished.
 */
export function talkTotals(progress: LearnerProgress): TalkTally {
  return { talks: progress.talks_finished, answers: progress.answers };
}

/** Topics and chores that count towards the Bargainer badge. */
const BARGAINS = new Set(["market-bargaining", "market-cloth-price"]);

/**
 * The profile's badges, earned from what the learner has actually done. Nothing
 * stores badges yet, so each is read off the learner's progress: a streak
 * badge while a streak is running, and the other two once a matching talk has
 * been finished. Those two stay earned however long ago that talk was.
 */
export function badges(progress: LearnerProgress, run: Streak): Badge[] {
  return [
    {
      id: "streak",
      label: run.days > 0 ? `${run.days}-day streak` : "Day streak",
      earned: run.days > 0,
    },
    { id: "first-talk", label: "First talk", earned: progress.talks_finished > 0 },
    {
      id: "bargainer",
      label: "Bargainer",
      earned: progress.finished_topics.some((topicId) => BARGAINS.has(topicId)),
    },
  ];
}

/**
 * The topic the "Today's talk" card offers. The backend already orders topics
 * for the learner's age, so its first entry is the opener.
 */
export function recommendedTopicId(snapshot: AppSnapshot): string {
  return snapshot.topics[0]?.id ?? "street-food";
}

/** The most recent conversation the learner never finished, if there is one. */
export function unfinishedSession(snapshot: AppSnapshot): SessionListItem | undefined {
  return snapshot.recent_sessions.find((session) => session.status === "active");
}
