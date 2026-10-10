/**
 * Everything the Ella Desktop design puts on screen that the Rust backend does
 * not model yet lives here, in one file, so it is obvious what is real and what
 * is editorial.
 *
 * Derived from real data:  the streak length and week strip, talks, answers
 *                          and spoken time (this week and overall), which
 *                          badges are earned, when, and the goes at each — all
 *                          read off the learner's whole history, which the
 *                          backend sums up in `AppSnapshot.progress`.
 * Placeholder (marked):    the talk partners' names and blurbs, and the one
 *                          goal with nothing behind it yet; the badges'
 *                          names, colours and lines.
 *
 * When the backend grows these fields, delete the matching constant and read
 * the snapshot instead — the component API does not have to change.
 */
import { addDays, dayKey } from "./days";
import { catalogueTopic } from "./topics";
import type {
  AppSnapshot,
  BadgeGlyph,
  BadgeGoal,
  BadgeStart,
  CastId,
  CastMember,
  ChoreRecap,
  FinishedTalk,
  LearnerBadge,
  LearnerProgress,
  SessionListItem,
  Streak,
  StreakDay,
  TalkTally,
} from "../types";

/**
 * The talk partners. Three goals are real chores from `chores()` in the Rust
 * domain and start through `start_chore`, which plays the character and keeps
 * the score; the doctor's goal opens the matching free topic; the debate has no
 * chore behind it yet, so it is shown but cannot start. `minAge` mirrors each
 * chore's `min_age`, which `catalog_for` filters on.
 *
 * PLACEHOLDER — the names, blurbs and minutes are the design's. The backend's
 * personas go by the same names, so a partner never introduces themselves as
 * somebody else.
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

/** The partner who plays a chore: the one whose goal starts it. A chore's talk
 * takes the chore's id for its topic, so a talk picked up again later still
 * finds them. Null for any other talk. */
export function chorePartner(topicId: string): CastId | null {
  const member = CAST.find((candidate) =>
    candidate.goals.some((goal) => goal.start.kind === "chore" && goal.start.choreId === topicId),
  );
  return member?.id ?? null;
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
 * Talks, the answers given in them, and how long the spoken ones lasted, over
 * the last seven calendar days, today included. Each talk sits on the day of
 * its first answer, so one that ran past midnight is counted once.
 */
export function weeklyDigest(progress: LearnerProgress, today = new Date()): TalkTally {
  const week = new Set(Array.from({ length: 7 }, (_, back) => dayKey(addDays(today, -back))));
  return progress.days
    .filter((day) => week.has(day.day))
    .reduce<TalkTally>(
      (sum, day) => ({
        talks: sum.talks + day.talks,
        answers: sum.answers + day.answers,
        spokenMs: sum.spokenMs + (day.spoken_ms ?? 0),
      }),
      { talks: 0, answers: 0, spokenMs: 0 },
    );
}

/**
 * Finished talks and every answer given, for the profile, over the learner's
 * whole history. A talk counts as finished once it was ended with at least one
 * answer in it; the answers include talks that were never finished.
 */
export function talkTotals(progress: LearnerProgress): TalkTally {
  return { talks: progress.talks_finished, answers: progress.answers, spokenMs: progress.spoken_ms ?? 0 };
}

/** Whether Ella has kept the length of any spoken answer yet. Until she has,
 * spoken time is unknown rather than nothing: answers from before she kept
 * it, and typed ones, have none. */
export function spokenTimeKnown(progress: LearnerProgress): boolean {
  return (progress.spoken_answers ?? 0) > 0;
}

/** Spoken time as Home's week card counts it: seconds under a minute, and
 * whole minutes from there. */
export function spokenAmount(ms: number): { value: number; unit: "second" | "seconds" | "minute" | "minutes" } {
  const seconds = Math.round(ms / 1000);
  if (seconds > 0 && seconds < 60) return { value: seconds, unit: seconds === 1 ? "second" : "seconds" };
  const minutes = Math.round(seconds / 60);
  return { value: minutes, unit: minutes === 1 ? "minute" : "minutes" };
}

/** Spoken time as the profile prints it: `45s`, `12m`, `3h 30m`. */
export function spokenTime(ms: number): string {
  const { value, unit } = spokenAmount(ms);
  if (unit.startsWith("second") || value === 0) return `${value}s`;
  if (value < 60) return `${value}m`;
  const hours = Math.floor(value / 60);
  return value % 60 === 0 ? `${hours}h` : `${hours}h ${value % 60}m`;
}

/* ------------------------------------------------------------------ *
 * Badges
 * ------------------------------------------------------------------ */

/** The placement chat's topic id, which is how its talks are told apart. */
const PLACEMENT_TOPIC = "placement";

/** The free talk that earns the Bargainer badge by being finished. The
 * stall-price chore earns it too, once its goal is met. */
const BARGAIN_TOPIC = "market-bargaining";

/** What a badge reads off the learner's history. */
interface BadgeReading {
  /** The finished talk that first earned it, if one has. */
  earnedBy?: FinishedTalk;
  /** A streak's badge is earned by days rather than by one talk. */
  earnedOn?: string | null;
  tries?: number;
  goal?: BadgeGoal;
}

interface BadgeFacts {
  progress: LearnerProgress;
  /** The streak running now. */
  run: number;
}

interface BadgeRule {
  id: string;
  name: string;
  /** The ladder level it is listed at; null for the anytime badges. */
  level: number | null;
  color: string;
  glyph: BadgeGlyph;
  how: string;
  /** For a scene on Talk partners: the partner, and which of their goals. */
  scene?: { partner: CastId; goal: string };
  /** Otherwise, where it is earned, as its row says it and as its sheet
   * does, and what its mic starts. */
  where?: string;
  place?: string;
  line?: string;
  start?: BadgeStart;
  read: (facts: BadgeFacts) => BadgeReading;
}

/** The first day a run of talk days in a row reached `length`: the day a
 * streak badge was earned, however the run went on or broke after. */
export function streakReachedOn(progress: LearnerProgress, length: number): string | null {
  const days = progress.days.map((entry) => entry.day).sort();
  let run = 0;
  let previous: string | null = null;
  for (const day of days) {
    run = previous !== null && dayKey(addDays(fromDayKey(previous), 1)) === day ? run + 1 : 1;
    if (run >= length) return day;
    previous = day;
  }
  return null;
}

/** Local noon on a `YYYY-MM-DD`, so stepping a day never trips on a clock
 * change. */
function fromDayKey(day: string): Date {
  const [year, month, date] = day.split("-").map(Number);
  return new Date(year, month - 1, date, 12);
}

function streakBadge(length: number): BadgeRule {
  return {
    id: `streak-${length}`,
    name: `${length}-day streak`,
    level: null,
    color: "#FF7A00",
    glyph: "flame",
    how: `Talk with Ella ${length} days in a row.`,
    where: "Any talk, every day",
    place: "Any talk, every day",
    line: `${length} days in a row`,
    start: { kind: "topic", topicId: null },
    read: ({ progress, run }) => ({
      earnedOn: streakReachedOn(progress, length),
      goal: { have: run, of: length, unit: "days" },
    }),
  };
}

/**
 * Every badge, in the order the design lists them, each read off what the
 * learner has actually done: the talks they finished, in order, and the days
 * they talked. Nothing stores a badge, so one earned stays earned for as long
 * as the talk that earned it is on the laptop.
 *
 * Only badges something can count are here. The design also has "Sold!",
 * "Clear patient" and "Take a stand" for the pen, Dr Wobble and Zig: nothing
 * judges the pen's rubric yet, Dr Wobble's goal opens a free topic, and Zig
 * has no scene at all, so none of them could ever be earned. Its level-4
 * badges and the mystery ones stand for scenes nobody has made. And since a
 * scene can be played at any level, nothing is locked: a badge sits at the
 * level the design lists it under, and the learner can earn it from anywhere.
 *
 * PLACEHOLDER — the names, colours, glyphs and lines are the design's.
 */
const BADGES: BadgeRule[] = [
  {
    id: "hello",
    name: "Hello, Ella",
    level: 1,
    color: "#9347DD",
    glyph: "ella",
    how: "Say hi to Ella in your placement talk.",
    where: "Ella · Placement talk",
    place: "Ella",
    line: "Placement talk",
    start: { kind: "placement" },
    read: ({ progress }) => ({ earnedBy: progress.talks.find((talk) => talk.topic_id === PLACEMENT_TOPIC) }),
  },
  {
    id: "first-talk",
    name: "First talk",
    level: 1,
    color: "#FF3181",
    glyph: "mic",
    how: "Finish your first daily talk with Ella.",
    where: "Home · Today’s talk",
    place: "Today’s talk",
    line: "Home",
    start: { kind: "topic", topicId: null },
    read: ({ progress }) => ({ earnedBy: progress.talks.find((talk) => talk.topic_id !== PLACEMENT_TOPIC) }),
  },
  {
    id: "bargainer",
    name: "Bargainer",
    level: 3,
    color: "#FF7A00",
    glyph: "tag",
    how: "Get the cloth for Rs 400 or less, and get Bippo to agree.",
    scene: { partner: "stall-owner", goal: "market-cloth-price" },
    read: ({ progress }) => ({
      earnedBy: progress.talks.find(
        (talk) => (talk.topic_id === "market-cloth-price" && talk.goal_met) || talk.topic_id === BARGAIN_TOPIC,
      ),
      tries: progress.talks.filter((talk) => talk.topic_id === "market-cloth-price").length,
    }),
  },
  {
    id: "deposit",
    name: "Deposit back",
    level: 3,
    color: "#5B7DEF",
    glyph: "house",
    how: "Get at least Rs 3500 of your deposit back from Grumble.",
    scene: { partner: "landlord", goal: "deposit-refund" },
    read: ({ progress }) => ({
      earnedBy: progress.talks.find((talk) => talk.topic_id === "deposit-refund" && talk.goal_met),
      tries: progress.talks.filter((talk) => talk.topic_id === "deposit-refund").length,
    }),
  },
  streakBadge(3),
  streakBadge(7),
  streakBadge(30),
  {
    id: "talks-50",
    name: "50 talks",
    level: null,
    color: "#FF3181",
    glyph: "star",
    how: "Finish 50 talks of any kind.",
    where: "Any talk counts",
    place: "Any talk counts",
    line: "Daily talks and role-plays",
    start: { kind: "topic", topicId: null },
    read: ({ progress }) => ({
      earnedBy: progress.talks[49],
      goal: { have: progress.talks_finished, of: 50, unit: "talks" },
    }),
  },
];

/**
 * The badges as they stand for this learner. A scene's badge follows its goal
 * on Talk partners: left out when the learner is too young to be offered it,
 * except the Bargainer, which the free bargaining talk earns too. That talk is
 * where it points for a learner too young for Bippo, and where it says it was
 * earned when that talk is what earned it.
 */
export function learnerBadges(snapshot: AppSnapshot, today = new Date()): LearnerBadge[] {
  const facts: BadgeFacts = { progress: snapshot.progress, run: streak(snapshot.progress, today).days };
  const offered = castFor(snapshot.learner?.age);
  // Whatever level Home is offering, the free talk is there to be had.
  const bargaining = catalogueTopic(BARGAIN_TOPIC);
  const badges: LearnerBadge[] = [];
  for (const rule of BADGES) {
    const reading = rule.read(facts);
    const earnedOn = reading.earnedBy?.day ?? reading.earnedOn ?? null;
    const common = {
      id: rule.id,
      name: rule.name,
      level: rule.level,
      color: rule.color,
      glyph: rule.glyph,
      how: rule.how,
      earned: earnedOn !== null,
      earnedOn,
      tries: reading.tries ?? 0,
      goal: reading.goal ?? null,
    };
    if (rule.scene) {
      const { partner, goal: goalId } = rule.scene;
      const member = offered.find((candidate) => candidate.id === partner);
      const goal = member?.goals.find((candidate) => candidate.id === goalId);
      const byTopic = reading.earnedBy?.topic_id === BARGAIN_TOPIC;
      if (member && goal && !byTopic) {
        badges.push({
          ...common,
          where: `${member.name} · ${goal.title}`,
          place: member.name,
          line: goal.title,
          partner,
          minutes: goal.minutes,
          start: { kind: "partners" },
        });
      } else if (rule.id === "bargainer" && bargaining) {
        badges.push({
          ...common,
          where: `Home · ${bargaining.label}`,
          place: bargaining.label,
          line: "Home",
          partner: null,
          minutes: bargaining.minutes,
          start: { kind: "topic", topicId: BARGAIN_TOPIC },
        });
      }
      continue;
    }
    badges.push({
      ...common,
      where: rule.where ?? "",
      place: rule.place ?? "",
      line: rule.line ?? "",
      partner: null,
      minutes: null,
      start: rule.start ?? { kind: "topic", topicId: null },
    });
  }
  return badges;
}

/** Earned badges, the latest first; two earned the same day keep the
 * catalogue's order. */
export function earnedBadges(badges: LearnerBadge[]): LearnerBadge[] {
  return badges
    .map((badge, index) => ({ badge, index }))
    .filter(({ badge }) => badge.earned)
    .sort((left, right) => (right.badge.earnedOn ?? "").localeCompare(left.badge.earnedOn ?? "") || left.index - right.index)
    .map(({ badge }) => badge);
}

/** Badges still to earn, the ones tried most first, then in the catalogue's
 * order, as Ella Mobile lists what comes next. */
export function openBadges(badges: LearnerBadge[]): LearnerBadge[] {
  return badges
    .map((badge, index) => ({ badge, index }))
    .filter(({ badge }) => !badge.earned)
    .sort((left, right) => right.badge.tries - left.badge.tries || left.index - right.index)
    .map(({ badge }) => badge);
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "21 Aug" for a `YYYY-MM-DD`, with the year when it is not this year's. */
export function shortDate(day: string, today = new Date()): string {
  const [year, month, date] = day.split("-").map(Number);
  const label = `${date} ${MONTHS[month - 1]}`;
  return year === today.getFullYear() ? label : `${label} ${year}`;
}

/** `shortDate`, or "Today". */
function dayLabel(day: string, today: Date): string {
  return day === dayKey(today) ? "Today" : shortDate(day, today);
}

function tries(count: number): string {
  return count === 1 ? "1 try" : `${count} tries`;
}

/** The short status at the end of a badge's row: when it was earned, how far
 * along its count is, or how many goes it has had. */
export function badgeStat(badge: LearnerBadge, today = new Date()): string {
  if (badge.earnedOn) return dayLabel(badge.earnedOn, today);
  if (badge.goal) return `${Math.min(badge.goal.have, badge.goal.of)} / ${badge.goal.of}`;
  return badge.tries > 0 ? tries(badge.tries) : "Not tried";
}

/** The sheet's pill: "Earned 21 Aug", "Earned today", "12 of 30 days",
 * "Not yet · 2 tries", "Not tried yet". */
export function badgeStatus(badge: LearnerBadge, today = new Date()): string {
  if (badge.earnedOn) {
    const day = dayLabel(badge.earnedOn, today);
    return day === "Today" ? "Earned today" : `Earned ${day}`;
  }
  if (badge.goal) return `${Math.min(badge.goal.have, badge.goal.of)} of ${badge.goal.of} ${badge.goal.unit}`;
  return badge.tries > 0 ? `Not yet · ${tries(badge.tries)}` : "Not tried yet";
}

/** How far along a count is, 0-100, for its bar. */
export function badgePercent(badge: LearnerBadge): number {
  if (!badge.goal) return 0;
  return Math.min(100, Math.round((100 * badge.goal.have) / badge.goal.of));
}

/**
 * The topic the "Today's talk" card offers. The backend orders the topics for
 * the learner's level, the day and their latest talks, so its first is the one.
 */
export function recommendedTopicId(snapshot: AppSnapshot): string {
  return snapshot.topics[0]?.id ?? "street-food";
}

/** The most recent conversation the learner never finished, if there is one. */
export function unfinishedSession(snapshot: AppSnapshot): SessionListItem | undefined {
  return snapshot.recent_sessions.find((session) => session.status === "active");
}

/* ------------------------------------------------------------------ *
 * The recap
 * ------------------------------------------------------------------ */

/**
 * How the recap's role-play band tells each ledger chore: the second half of
 * its mono label, and what agreeing is called.
 *
 * PLACEHOLDER — the design's wording.
 */
export const CHORE_RECAP: Record<string, { track: string; agrees: string }> = {
  "market-cloth-price": { track: "STALL PRICE", agrees: "He agrees" },
  "deposit-refund": { track: "DEPOSIT", agrees: "He agrees" },
};

/** The badge meeting a chore's goal earns, by the name the profile lists it
 * under; none for a chore no badge follows. */
export function sceneBadge(choreId: string): string | null {
  return BADGES.find((rule) => rule.scene?.goal === choreId)?.name ?? null;
}

/** The talk partner's name as the design gives it. */
export function castName(characterId: string): string {
  return CAST.find((member) => member.id === (characterId as CastId))?.name ?? "Your partner";
}

/** The chore's goal as a figure: `Rs 400 or less`, `Rs 3500 or more`. */
export function goalFigure(chore: ChoreRecap): string {
  return `${chore.unit} ${chore.target} or ${chore.direction === "down" ? "less" : "more"}`;
}

/** Whether the figure got to the target, agreed or not. */
export function reachedGoal(chore: ChoreRecap): boolean {
  return chore.direction === "down" ? chore.figure <= chore.target : chore.figure >= chore.target;
}

export type RecapDayState = "done" | "new" | "empty";

/** What the recap's streak tile shows about the talk just finished. */
export interface StreakRecap {
  /** The streak before this talk, and after it. */
  from: number;
  to: number;
  /** Today counts, now or from an earlier talk. */
  counted: boolean;
  chip: string | null;
  week: Array<{ label: string; state: RecapDayState }>;
}

/**
 * The streak as this talk left it, from the learner's progress before it.
 * The first answer of the day adds today, which the tile flips up to; a day
 * already counted stays as it was; a talk with nothing said in it changes
 * nothing. Worked out from before the talk, rather than from the figures the
 * backend sends after it, so the number the tile flips from is known the
 * moment the recap opens.
 */
export function streakRecap(before: LearnerProgress, answered: boolean, today = new Date()): StreakRecap {
  const run = streak(before, today);
  const todayKey = dayKey(today);
  const already = before.days.some((day) => day.day === todayKey);
  const added = answered && !already;
  const to = added ? run.days + 1 : run.days;
  return {
    from: run.days,
    to,
    counted: added || already,
    chip: added ? (to === 1 ? "New streak" : null) : already ? "Done for today" : null,
    week: run.week.map((day) => ({
      label: day.label,
      state: day.state === "done" ? "done" : day.state === "today" && added ? "new" : "empty",
    })),
  };
}

/** The talk the recap suggests for tomorrow: the one Home will lead with then,
 * which the backend works out with the talk just finished moved back. Until
 * its figures with that talk in arrive, the first Ella offers that is not it. */
export function nextTopicLabel(snapshot: AppSnapshot, finishedTopicId: string | null): string | null {
  const tomorrow = snapshot.tomorrow_topic;
  if (tomorrow && tomorrow.id !== finishedTopicId) return tomorrow.label;
  return snapshot.topics.find((topic) => topic.id !== finishedTopicId)?.label ?? null;
}
