import { describe, expect, it, vi } from "vitest";
import { addDays, dayKey } from "./days";
import {
  badgeStat,
  badgeStatus,
  castFor,
  castName,
  earnedBadges,
  goalFigure,
  learnerBadges,
  nextTopicLabel,
  openBadges,
  reachedGoal,
  shortDate,
  spokenTime,
  streak,
  streakReachedOn,
  streakRecap,
  talkTotals,
  weeklyDigest,
} from "./presentation";
import type { AppSnapshot, ChoreRecap, DayActivity, FinishedTalk, LearnerBadge, LearnerProgress } from "../types";

/** Friday morning on whatever clock the tests run on. */
const today = new Date(2026, 8, 25, 10);

/** A talk day some days back, as the backend reports it. */
function day(daysAgo: number, talks = 1, answers = 2, spoken_ms = 0): DayActivity {
  return { day: dayKey(addDays(today, -daysAgo)), talks, answers, spoken_ms };
}

/** Progress made of these days, newest first; every talk in them finished
 * unless the overrides say otherwise. */
function progressWith(days: DayActivity[], overrides: Partial<LearnerProgress> = {}): LearnerProgress {
  return {
    days,
    talks_finished: days.reduce((sum, entry) => sum + entry.talks, 0),
    answers: days.reduce((sum, entry) => sum + entry.answers, 0),
    finished_topics: days.length > 0 ? ["street-food"] : [],
    chores_met: [],
    talks: [],
    spoken_ms: 0,
    spoken_answers: 0,
    ...overrides,
  };
}

/** A finished talk that ended some days back. */
function ended(topic_id: string, daysAgo: number, goal_met = false): FinishedTalk {
  return { topic_id, day: dayKey(addDays(today, -daysAgo)), goal_met };
}

/** The badges of a learner of `age` with this progress. */
function badgesOf(progress: LearnerProgress, age: number | null = 14): LearnerBadge[] {
  const snapshot = {
    learner: { name: "Asha", age, level_name: "", created_at: "" },
    topics: [
      { id: "street-food", label: "Street food stories", prompt: "", emoji: "", color: "" },
      { id: "market-bargaining", label: "Bargaining at the market", prompt: "", emoji: "", color: "" },
    ],
    recent_sessions: [],
    progress,
  } as unknown as AppSnapshot;
  return learnerBadges(snapshot, today);
}

function badge(badges: LearnerBadge[], id: string): LearnerBadge {
  const found = badges.find((candidate) => candidate.id === id);
  if (!found) throw new Error(`no ${id} badge`);
  return found;
}

describe("the streak", () => {
  it("counts days on the learner's own calendar, not UTC's", () => {
    vi.stubEnv("TZ", "Asia/Kolkata");
    try {
      // Half past midnight on Friday in Pune is still Thursday in UTC. The
      // backend put the talk on Friday, and so must the strip.
      const now = new Date("2026-09-25T00:30:00+05:30");
      const run = streak(progressWith([{ day: "2026-09-25", talks: 1, answers: 1, spoken_ms: 0 }]), now);
      expect(run.days).toBe(1);
      expect(run.week.map((entry) => entry.state)).toEqual([
        "future",
        "future",
        "future",
        "future",
        "done",
        "future",
        "future",
      ]);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("runs past a week, and marks every day of this one that was talked on", () => {
    const run = streak(progressWith([0, 1, 2, 3, 4, 5, 6, 7, 8].map((back) => day(back))), today);
    expect(run.days).toBe(9);
    // Friday: Monday to today are done, the weekend is still to come.
    expect(run.week.map((entry) => entry.state)).toEqual([
      "done",
      "done",
      "done",
      "done",
      "done",
      "future",
      "future",
    ]);
    expect(run.week.map((entry) => entry.label)).toEqual(["M", "T", "W", "T", "F", "S", "S"]);
  });

  it("counts seven days in a row as seven", () => {
    const run = streak(progressWith([0, 1, 2, 3, 4, 5, 6].map((back) => day(back))), today);
    expect(run.days).toBe(7);
  });

  it("does not drop when a lot of talking happens in one day", () => {
    const quiet = streak(progressWith([day(0, 1, 2), day(1), day(2)]), today);
    const busy = streak(progressWith([day(0, 12, 36), day(1), day(2)]), today);
    expect(quiet.days).toBe(3);
    expect(busy.days).toBe(3);
    expect(busy.week).toEqual(quiet.week);
  });

  it("keeps a streak that ran to yesterday while today is still to come", () => {
    const run = streak(progressWith([day(1), day(2)]), today);
    expect(run.days).toBe(2);
    expect(run.week[4]).toEqual({ label: "F", state: "today" });
  });

  it("counts a day with answers in a talk that began the day before", () => {
    // The talk ran past midnight, so it sits on yesterday, but today's answers
    // still make today a talk day.
    const run = streak(progressWith([day(0, 0, 2), day(1, 1, 3)]), today);
    expect(run.days).toBe(2);
  });

  it("breaks on a day with no talking", () => {
    expect(streak(progressWith([day(0), day(2), day(3)]), today).days).toBe(1);
    expect(streak(progressWith([]), today).days).toBe(0);
  });
});

describe("talk tallies", () => {
  it("counts the talks, answers and time spoken of the last seven days, today included", () => {
    const progress = progressWith([day(0, 2, 5, 90_000), day(6, 1, 2, 30_000), day(7, 3, 9, 60_000), day(30, 4, 11)]);
    expect(weeklyDigest(progress, today)).toEqual({ talks: 3, answers: 7, spokenMs: 120_000 });
  });

  it("reports the whole history's finished talks, answers and time spoken", () => {
    const progress = progressWith([day(0), day(40)], { talks_finished: 57, answers: 312, spoken_ms: 5_400_000 });
    expect(talkTotals(progress)).toEqual({ talks: 57, answers: 312, spokenMs: 5_400_000 });
  });

  it("prints time spoken as the profile does", () => {
    expect([spokenTime(0), spokenTime(44_600), spokenTime(59_400), spokenTime(90_000)]).toEqual(["0s", "45s", "59s", "2m"]);
    expect([spokenTime(12 * 60_000), spokenTime(3 * 3_600_000), spokenTime(12_600_000)]).toEqual(["12m", "3h", "3h 30m"]);
  });
});

describe("badges", () => {
  it("are all still to earn before anyone has talked", () => {
    const badges = badgesOf(progressWith([]));
    expect(badges.map((entry) => entry.id)).toEqual([
      "hello",
      "first-talk",
      "bargainer",
      "deposit",
      "streak-3",
      "streak-7",
      "streak-30",
      "talks-50",
    ]);
    expect(badges.filter((entry) => entry.earned)).toEqual([]);
    expect(badges.map((entry) => entry.level)).toEqual([1, 1, 3, 3, null, null, null, null]);
    expect(badges.map((entry) => badgeStat(entry, today))).toEqual([
      "Not tried",
      "Not tried",
      "Not tried",
      "Not tried",
      "0 / 3",
      "0 / 7",
      "0 / 30",
      "0 / 50",
    ]);
  });

  it("are dated by the talk that first earned them, and count the goes at a scene", () => {
    const progress = progressWith([], {
      talks: [
        ended("placement", 10),
        ended("street-food", 9),
        ended("market-cloth-price", 8),
        ended("market-cloth-price", 5, true),
        ended("deposit-refund", 4),
        ended("market-cloth-price", 3, true),
        ended("deposit-refund", 2),
      ],
    });
    const badges = badgesOf(progress);
    expect(badge(badges, "hello").earnedOn).toBe(dayKey(addDays(today, -10)));
    expect(badge(badges, "first-talk").earnedOn).toBe(dayKey(addDays(today, -9)));
    const bargainer = badge(badges, "bargainer");
    expect([bargainer.earnedOn, bargainer.tries]).toEqual([dayKey(addDays(today, -5)), 3]);
    expect([bargainer.where, bargainer.partner, bargainer.start]).toEqual([
      "Bippo · Talk a stall price down",
      "stall-owner",
      { kind: "partners" },
    ]);
    const deposit = badge(badges, "deposit");
    expect([deposit.earned, deposit.tries]).toEqual([false, 2]);
    expect(badgeStat(deposit, today)).toBe("2 tries");
    expect(badgeStatus(deposit, today)).toBe("Not yet · 2 tries");
  });

  it("only count a stall price as met when its goal was, as the recap says", () => {
    const tried = badgesOf(progressWith([], { talks: [ended("market-cloth-price", 1)] }));
    expect(badge(tried, "bargainer").earned).toBe(false);
    expect(badgeStat(badge(tried, "bargainer"), today)).toBe("1 try");
    // Another chore's goal is not a bargain.
    const deposit = badgesOf(progressWith([], { talks: [ended("deposit-refund", 1, true)] }));
    expect(badge(deposit, "bargainer").earned).toBe(false);
    expect(badge(deposit, "deposit").earned).toBe(true);
  });

  it("give the Bargainer for the free bargaining talk too, and say that is where it was earned", () => {
    const badges = badgesOf(progressWith([], { talks: [ended("market-bargaining", 2)] }));
    const bargainer = badge(badges, "bargainer");
    expect(bargainer.earned).toBe(true);
    expect([bargainer.where, bargainer.place, bargainer.line, bargainer.partner]).toEqual([
      "Home · Bargaining at the market",
      "Bargaining at the market",
      "Home",
      null,
    ]);
  });

  it("follow the scenes a learner is offered: too young for one, its badge is left out", () => {
    const young = badgesOf(progressWith([]), 9);
    expect(young.map((entry) => entry.id)).not.toContain("deposit");
    // Bippo is not offered at nine, but the bargaining talk is.
    expect([badge(young, "bargainer").where, badge(young, "bargainer").start]).toEqual([
      "Home · Bargaining at the market",
      { kind: "topic", topicId: "market-bargaining" },
    ]);
    expect(badgesOf(progressWith([]), 13).map((entry) => entry.id)).not.toContain("deposit");
    expect(badgesOf(progressWith([]), null).map((entry) => entry.id)).toContain("deposit");
  });

  it("earn a streak's badge the day a run first reached it, and keep it after the run breaks", () => {
    // Seven days in a row a month ago, then two days in a row up to today.
    const days = [day(0), day(1), ...[30, 31, 32, 33, 34, 35, 36].map((back) => day(back))];
    const progress = progressWith(days);
    expect(streakReachedOn(progress, 3)).toBe(dayKey(addDays(today, -34)));
    const badges = badgesOf(progress);
    expect(badge(badges, "streak-7").earnedOn).toBe(dayKey(addDays(today, -30)));
    const month = badge(badges, "streak-30");
    expect(month.earned).toBe(false);
    expect(month.goal).toEqual({ have: 2, of: 30, unit: "days" });
    expect([badgeStat(month, today), badgeStatus(month, today)]).toEqual(["2 / 30", "2 of 30 days"]);
    expect(streakReachedOn(progressWith([day(0), day(2), day(4)]), 2)).toBeNull();
  });

  it("earn 50 talks on the day the fiftieth ended", () => {
    const talks = Array.from({ length: 52 }, (_, index) => ended("street-food", 60 - index));
    const badges = badgesOf(progressWith([], { talks, talks_finished: 52 }));
    expect(badge(badges, "talks-50").earnedOn).toBe(dayKey(addDays(today, -11)));
    const almost = badge(badgesOf(progressWith([], { talks: talks.slice(0, 34), talks_finished: 34 })), "talks-50");
    expect([almost.earned, almost.goal?.have]).toEqual([false, 34]);
  });

  it("say when they were earned: today, this year, or another year", () => {
    const badges = badgesOf(progressWith([], { talks: [ended("placement", 400), ended("street-food", 0)] }));
    expect(badgeStat(badge(badges, "first-talk"), today)).toBe("Today");
    expect(badgeStatus(badge(badges, "first-talk"), today)).toBe("Earned today");
    expect(shortDate("2026-08-21", today)).toBe("21 Aug");
    expect(badgeStatus(badge(badges, "hello"), today)).toBe(`Earned ${shortDate(dayKey(addDays(today, -400)), today)}`);
    expect(shortDate(dayKey(addDays(today, -400)), today)).toMatch(/ 2025$/);
  });

  it("list the earned ones latest first, and the rest tried most first", () => {
    const badges = badgesOf(
      progressWith([day(2), day(3), day(4)], {
        talks: [ended("placement", 4), ended("street-food", 3), ended("deposit-refund", 2), ended("deposit-refund", 1)],
      }),
    );
    expect(earnedBadges(badges).map((entry) => entry.id)).toEqual(["streak-3", "first-talk", "hello"]);
    expect(openBadges(badges).map((entry) => entry.id)).toEqual([
      "deposit",
      "bargainer",
      "streak-7",
      "streak-30",
      "talks-50",
    ]);
  });
});

describe("the talk partners on offer", () => {
  const goals = (age: number | null) => castFor(age).flatMap((member) => member.goals.map((goal) => goal.id));

  it("offers everything when the age is not known", () => {
    expect(goals(null)).toEqual([
      "market-cloth-price",
      "deposit-refund",
      "sell-me-a-pen",
      "doctor-clinic",
      "take-a-stand",
    ]);
  });

  it("mirrors each chore's minimum age", () => {
    expect(goals(9)).toEqual(["doctor-clinic", "take-a-stand"]);
    expect(goals(10)).toEqual(["market-cloth-price", "doctor-clinic", "take-a-stand"]);
    expect(goals(14)).toHaveLength(5);
  });
});

describe("the recap", () => {
  it("flips the streak up a day with the first answer of the day", () => {
    const before = progressWith([day(1), day(2), day(3)]);
    const run = streakRecap(before, true, today);
    expect([run.from, run.to, run.counted, run.chip]).toEqual([3, 4, true, null]);
    // Monday to Sunday of a Friday: Tuesday to Thursday done, Friday new.
    expect(run.week.map((entry) => entry.state)).toEqual(["empty", "done", "done", "done", "new", "empty", "empty"]);
    expect(run.week.map((entry) => entry.label).join("")).toBe("MTWTFSS");
  });

  it("starts a new streak after a gap, and leaves a day already counted as it was", () => {
    const lapsed = streakRecap(progressWith([day(3)]), true, today);
    expect([lapsed.from, lapsed.to, lapsed.chip]).toEqual([0, 1, "New streak"]);

    const again = streakRecap(progressWith([day(0), day(1)]), true, today);
    expect([again.from, again.to, again.counted, again.chip]).toEqual([2, 2, true, "Done for today"]);
    expect(again.week[4].state).toBe("done");
  });

  it("changes nothing for a talk with nothing said in it", () => {
    const run = streakRecap(progressWith([day(1)]), false, today);
    expect([run.from, run.to, run.counted, run.chip]).toEqual([1, 1, false, null]);
    expect(run.week[4].state).toBe("empty");
  });

  it("names the chore's goal by which way the figure was pushed", () => {
    const stall: ChoreRecap = {
      chore_id: "market-cloth-price",
      character_id: "stall-owner",
      unit: "Rs",
      direction: "down",
      target: 400,
      figure: 420,
      agreed: true,
      met: false,
      times_met: 0,
    };
    expect(goalFigure(stall)).toBe("Rs 400 or less");
    expect(reachedGoal(stall)).toBe(false);
    expect(reachedGoal({ ...stall, figure: 400 })).toBe(true);
    const deposit: ChoreRecap = { ...stall, chore_id: "deposit-refund", character_id: "landlord", direction: "up", target: 3500, figure: 3600 };
    expect(goalFigure(deposit)).toBe("Rs 3500 or more");
    expect(reachedGoal(deposit)).toBe(true);
    expect([castName("stall-owner"), castName("landlord")]).toEqual(["Bippo", "Grumble"]);
  });

  it("suggests a different topic for tomorrow", () => {
    const snapshot = {
      topics: [
        { id: "street-food", label: "Street food", prompt: "", emoji: "", color: "" },
        { id: "booking-a-cab", label: "Booking a cab", prompt: "", emoji: "", color: "" },
      ],
    } as unknown as AppSnapshot;
    expect(nextTopicLabel(snapshot, "street-food")).toBe("Booking a cab");
    expect(nextTopicLabel(snapshot, "market-cloth-price")).toBe("Street food");
  });
});
