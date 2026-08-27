/**
 * Everything the Ella v5 design puts on screen that the Rust backend does not
 * model yet lives here, in one file, so it is obvious what is real and what is
 * still a placeholder.
 *
 * Derived from real data:  level ratio, skills done/total, weekly talks and
 *                          blooms, unit states for the first three units, the
 *                          streak length and week strip.
 * Placeholder (marked):    the "new words" count, unit names and their
 *                          coordinates on the path, per-topic category and
 *                          duration.
 *
 * When the backend grows these fields, delete the matching constant and read
 * the snapshot instead — the component API does not have to change.
 */
import type {
  AppSnapshot,
  Learner,
  SessionListItem,
  Streak,
  StreakDay,
  Topic,
  TopicPresentation,
  TopicSlot,
  WeeklyDigest,
} from "../types";

/** PLACEHOLDER — no vocabulary ledger exists yet, so this is a stand-in figure. */
const WORDS_PER_EVIDENCE = 3;

/**
 * PLACEHOLDER — the curriculum has no unit table, so the names and stage
 * coordinates still come from the design. `topicId` is the one real link: it
 * is the conversation a unit opens when it is tapped, chosen to match the
 * unit's name. The first three units take their growth state from the three
 * real skill strands, and the rest stay ahead of the learner.
 */
export const BENTO_SLOTS: TopicSlot[] = ["wide", "wave", "framed", "inset", "chat", "quote"];

/**
 * PLACEHOLDER — category, duration and the sample exchange are editorial
 * metadata about each topic; `tone` is how the card is painted. Every topic
 * carries a sample and a reply because the bento slot is decided by position,
 * so any topic can land in the card that prints them.
 */
export const TOPIC_PRESENTATION: Record<string, TopicPresentation> = {
  "street-food": {
    category: "fluency",
    minutes: 6,
    tone: "violet",
    blurb: "Describe tastes, smells and your favourite stall. About 6 minutes of talking.",
    sample: "What did you eat?",
    reply: "Hot vada pav!",
  },
  "restaurant-order": {
    category: "role-play",
    minutes: 5,
    tone: "pink",
    blurb: "Order a meal, ask what is in it, and settle the bill.",
    sample: "What would you like?",
    reply: "One thali, please.",
  },
  "booking-a-cab": {
    category: "vocabulary",
    minutes: 4,
    tone: "green",
    blurb: "Give an address, agree a fare, and ask how long it takes.",
    sample: "Where to, madam?",
    reply: "To the station.",
  },
  "job-interview": {
    category: "role-play",
    minutes: 6,
    tone: "lilac",
    blurb: "Introduce yourself and answer questions about your work.",
    sample: "Tell me about yourself.",
    reply: "I love building things.",
  },
  "doctor-clinic": {
    category: "vocabulary",
    minutes: 5,
    tone: "violet",
    blurb: "Explain how you feel and understand what to do next.",
    sample: "How are you feeling?",
    reply: "My head hurts.",
  },
  "asking-directions": {
    category: "grammar",
    minutes: 4,
    tone: "ink",
    blurb: "Find your way, then repeat the directions back.",
    sample: "Where is the bus stop?",
    reply: "Straight, then left!",
  },
  "market-bargaining": {
    category: "fluency",
    minutes: 5,
    tone: "orange",
    blurb: "Ask the price, bargain kindly, and agree a deal.",
    sample: "What's your best price?",
    reply: "Make it eighty?",
  },
};

const FALLBACK_PRESENTATION: TopicPresentation = {
  category: "fluency",
  minutes: 5,
  tone: "green",
  blurb: "A short, friendly conversation to keep your English moving.",
  sample: "Shall we talk?",
  reply: "Yes, let's!",
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

const DAY_INITIALS = ["S", "M", "T", "W", "T", "F", "S"];

function dayKey(iso: string): string {
  return new Date(iso).toISOString().slice(0, 10);
}

function addDays(date: Date, days: number): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}

/**
 * Real, as far as the snapshot reaches: `recent_sessions` only carries the last
 * five conversations, so a streak longer than that is reported as five.
 */
export function streak(sessions: SessionListItem[], today = new Date()): Streak {
  const talkedOn = new Set(sessions.map((session) => dayKey(session.started_at)));
  const todayKey = today.toISOString().slice(0, 10);

  let days = 0;
  for (let back = 0; ; back += 1) {
    const key = addDays(today, -back).toISOString().slice(0, 10);
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
    const key = date.toISOString().slice(0, 10);
    const state: StreakDay["state"] = talkedOn.has(key)
      ? "done"
      : key === todayKey
        ? "today"
        : "future";
    return { label: DAY_INITIALS[date.getDay()], state };
  });

  return { days, week };
}

/** Talks this week, from the session list. */
export function weeklyDigest(snapshot: AppSnapshot, today = new Date()): WeeklyDigest {
  const weekAgo = addDays(today, -7).getTime();
  const talks = snapshot.recent_sessions.filter(
    (session) => new Date(session.started_at).getTime() >= weekAgo,
  ).length;
  return { talks };
}

/**
 * The topic the "Ella recommends" hero offers. The backend already orders
 * topics for the learner's age, so its first entry is the opener. The
 * least-practised-strand rule left with the garden that fed it.
 */
export function recommendedTopicId(snapshot: AppSnapshot): string {
  return snapshot.topics[0]?.id ?? "street-food";
}

/** The most recent conversation the learner never finished, if there is one. */
export function unfinishedSession(snapshot: AppSnapshot): SessionListItem | undefined {
  return snapshot.recent_sessions.find((session) => session.status === "active");
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}
