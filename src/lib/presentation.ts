/**
 * Everything the Ella v5 design puts on screen that the Rust backend does not
 * model yet lives here, in one file, so it is obvious what is real and what is
 * still a placeholder.
 *
 * Derived from real data:  level ratio, skills done/total, weekly talks and
 *                          blooms, unit states for the first three units, the
 *                          streak length and week strip.
 * Placeholder (marked):    CEFR band, "new words" count, unit names and their
 *                          coordinates on the path, per-topic category and
 *                          duration.
 *
 * When the backend grows these fields, delete the matching constant and read
 * the snapshot instead — the component API does not have to change.
 */
import type {
  AppSnapshot,
  Garden,
  LevelInfo,
  SessionListItem,
  SkillStrand,
  Streak,
  StreakDay,
  Topic,
  TopicPresentation,
  TopicSlot,
  UnitNode,
  UnitState,
  WeeklyDigest,
} from "../types";

/** PLACEHOLDER — the backend stores a garden name ("Morning Meadow"), not a CEFR band. */
const CEFR_BY_LEVEL_NAME: Record<string, string> = {
  "Morning Meadow": "A2",
};
const DEFAULT_CEFR = "A2";

/** PLACEHOLDER — no vocabulary ledger exists yet, so this is a stand-in figure. */
const WORDS_PER_EVIDENCE = 3;

/**
 * PLACEHOLDER — the curriculum has no unit table, so the names and stage
 * coordinates still come from the design. `topicId` is the one real link: it
 * is the conversation a unit opens when it is tapped, chosen to match the
 * unit's name. The first three units take their growth state from the three
 * real skill strands, and the rest stay ahead of the learner.
 */
const UNIT_DEFS: Array<{
  num: number;
  name: string;
  topicId: string;
  x: number;
  y: number;
  tint: string;
}> = [
  { num: 1, name: "Hello & introductions", topicId: "job-interview", x: 140, y: 170, tint: "rgba(147, 71, 221, 0.16)" },
  { num: 2, name: "Food & ordering", topicId: "restaurant-order", x: 430, y: 170, tint: "rgba(255, 49, 129, 0.13)" },
  { num: 3, name: "My day", topicId: "street-food", x: 720, y: 170, tint: "rgba(104, 181, 6, 0.16)" },
  { num: 4, name: "Places nearby", topicId: "asking-directions", x: 760, y: 440, tint: "rgba(255, 122, 0, 0.14)" },
  { num: 5, name: "Plans & weekend", topicId: "market-bargaining", x: 470, y: 440, tint: "rgba(38, 38, 38, 0.08)" },
];

/**
 * The bento is a fixed six-cell grid in CSS — each slot is pinned to its own
 * grid column and row — so the slot belongs to the *position* on the home
 * screen, not to the topic. Topics can then be reordered by age, or promoted
 * into the hero, without two cards fighting over one cell.
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

/** Real: one "skill" per stage reached, three stages per strand. */
export function levelInfo(garden: Garden, levelName?: string): LevelInfo {
  const skillsDone = garden.skills.reduce((total, skill) => total + skill.stage, 0);
  const skillsTotal = Math.max(1, garden.skills.length * 3);
  return {
    code: CEFR_BY_LEVEL_NAME[levelName ?? garden.level_name] ?? DEFAULT_CEFR,
    skillsDone,
    skillsTotal,
    ratio: skillsDone / skillsTotal,
  };
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

/** Real talks and blooms; new words is the marked placeholder. */
export function weeklyDigest(snapshot: AppSnapshot, today = new Date()): WeeklyDigest {
  const weekAgo = addDays(today, -7).getTime();
  const talks = snapshot.recent_sessions.filter(
    (session) => new Date(session.started_at).getTime() >= weekAgo,
  ).length;
  const blooms = snapshot.garden.skills.filter((skill) => skill.stage >= 3).length;
  const evidence = snapshot.garden.skills.reduce((total, skill) => total + skill.evidence_count, 0);
  return { talks, newWords: evidence * WORDS_PER_EVIDENCE, blooms };
}

const STATE_BY_STAGE: UnitState[] = ["bare", "seedling", "young", "bloom"];

/** The first topic that practises a given skill strand, in backend order. */
function topicForStrand(topics: Topic[], strand: SkillStrand): Topic | undefined {
  return topics.find((topic) => topicPresentation(topic.id).category === strand);
}

/**
 * Units 1-3 mirror the three real skill strands, so their plants grow with the
 * garden. Every unlocked unit opens the conversation named in `UNIT_DEFS`,
 * falling back to nothing when that topic is not in the current snapshot (the
 * backend reorders topics by age, but never drops one).
 */
export function units(garden: Garden, topics: Topic[] = []): UnitNode[] {
  const tracked = garden.skills.length;
  const nodes = UNIT_DEFS.map((def, index) => {
    const skill = garden.skills[index];
    const state: UnitState = skill
      ? STATE_BY_STAGE[Math.min(3, skill.stage)]
      : index === tracked
        ? "bare"
        : "locked";
    const known = topics.length === 0 || topics.some((topic) => topic.id === def.topicId);
    return {
      ...def,
      topicId: known ? def.topicId : null,
      state,
      done: state === "bloom",
      current: false,
    };
  });
  // The first unit that is not finished is where the learner is working.
  const current = nodes.find((node) => node.state !== "bloom" && node.state !== "locked");
  if (current) current.current = true;
  return nodes;
}

/**
 * The topic the "Ella recommends" hero offers. With nothing practised yet the
 * backend's first topic already is the age-appropriate opener; once there is
 * evidence, lead with the least-practised strand so the weakest plant is the
 * one that gets watered next.
 */
export function recommendedTopicId(snapshot: AppSnapshot): string {
  const fallback = snapshot.topics[0]?.id ?? "street-food";
  const practised = snapshot.garden.skills.reduce((total, skill) => total + skill.evidence_count, 0);
  if (practised === 0) return fallback;
  const [weakest] = [...snapshot.garden.skills].sort(
    (left, right) => left.evidence_count - right.evidence_count,
  );
  if (!weakest) return fallback;
  return topicForStrand(snapshot.topics, weakest.strand)?.id ?? fallback;
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
