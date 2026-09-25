import { describe, expect, it, vi } from "vitest";
import { addDays, dayKey } from "./days";
import { badges, castFor, streak, talkTotals, weeklyDigest } from "./presentation";
import type { DayActivity, LearnerProgress } from "../types";

/** Friday morning on whatever clock the tests run on. */
const today = new Date(2026, 8, 25, 10);

/** A talk day some days back, as the backend reports it. */
function day(daysAgo: number, talks = 1, answers = 2): DayActivity {
  return { day: dayKey(addDays(today, -daysAgo)), talks, answers };
}

/** Progress made of these days, newest first; every talk in them finished
 * unless the overrides say otherwise. */
function progressWith(days: DayActivity[], overrides: Partial<LearnerProgress> = {}): LearnerProgress {
  return {
    days,
    talks_finished: days.reduce((sum, entry) => sum + entry.talks, 0),
    answers: days.reduce((sum, entry) => sum + entry.answers, 0),
    finished_topics: days.length > 0 ? ["street-food"] : [],
    ...overrides,
  };
}

describe("the streak", () => {
  it("counts days on the learner's own calendar, not UTC's", () => {
    vi.stubEnv("TZ", "Asia/Kolkata");
    try {
      // Half past midnight on Friday in Pune is still Thursday in UTC. The
      // backend put the talk on Friday, and so must the strip.
      const now = new Date("2026-09-25T00:30:00+05:30");
      const run = streak(progressWith([{ day: "2026-09-25", talks: 1, answers: 1 }]), now);
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
  it("counts the talks and answers of the last seven days, today included", () => {
    const progress = progressWith([day(0, 2, 5), day(6, 1, 2), day(7, 3, 9), day(30, 4, 11)]);
    expect(weeklyDigest(progress, today)).toEqual({ talks: 3, answers: 7 });
  });

  it("reports the whole history's finished talks and answers", () => {
    const progress = progressWith([day(0), day(40)], { talks_finished: 57, answers: 312 });
    expect(talkTotals(progress)).toEqual({ talks: 57, answers: 312 });
  });
});

describe("badges", () => {
  it("are all locked for someone who has not talked yet", () => {
    const progress = progressWith([]);
    expect(badges(progress, streak(progress, today)).map((badge) => badge.earned)).toEqual([
      false,
      false,
      false,
    ]);
  });

  it("are earned by a running streak, a finished talk and a finished bargain", () => {
    const progress = progressWith([day(0), day(1)], { finished_topics: ["market-cloth-price", "street-food"] });
    expect(badges(progress, streak(progress, today))).toEqual([
      { id: "streak", label: "2-day streak", earned: true },
      { id: "first-talk", label: "First talk", earned: true },
      { id: "bargainer", label: "Bargainer", earned: true },
    ]);
  });

  it("do not count talking without finishing", () => {
    // Answers were given today, but nothing was finished.
    const progress = progressWith([day(0)], { talks_finished: 0, finished_topics: [] });
    const earned = badges(progress, streak(progress, today));
    expect(earned.find((badge) => badge.id === "streak")?.earned).toBe(true);
    expect(earned.find((badge) => badge.id === "first-talk")?.earned).toBe(false);
    expect(earned.find((badge) => badge.id === "bargainer")?.earned).toBe(false);
  });

  it("stay earned however long the history grows", () => {
    // Two months of talking, the one bargain long ago, and a streak that has
    // since lapsed: only the streak badge is lost.
    const days = Array.from({ length: 60 }, (_, back) => day(back + 3));
    const progress = progressWith(days, { finished_topics: ["market-bargaining", "street-food"] });
    const earned = badges(progress, streak(progress, today));
    expect(earned).toEqual([
      { id: "streak", label: "Day streak", earned: false },
      { id: "first-talk", label: "First talk", earned: true },
      { id: "bargainer", label: "Bargainer", earned: true },
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
