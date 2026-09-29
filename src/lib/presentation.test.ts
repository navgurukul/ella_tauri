import { describe, expect, it, vi } from "vitest";
import { addDays, dayKey } from "./days";
import {
  badges,
  castFor,
  castName,
  goalFigure,
  nextTopicLabel,
  reachedGoal,
  streak,
  streakRecap,
  talkTotals,
  weeklyDigest,
} from "./presentation";
import type { AppSnapshot, ChoreRecap, DayActivity, LearnerProgress } from "../types";

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
    chores_met: [],
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

  it("are earned by a running streak, a finished talk and a stall price talked down", () => {
    const progress = progressWith([day(0), day(1)], {
      finished_topics: ["market-cloth-price", "street-food"],
      chores_met: ["market-cloth-price"],
    });
    expect(badges(progress, streak(progress, today))).toEqual([
      { id: "streak", label: "2-day streak", earned: true },
      { id: "first-talk", label: "First talk", earned: true },
      { id: "bargainer", label: "Bargainer", earned: true },
    ]);
  });

  it("keep the Bargainer for the stall price until its goal is met, as the recap says", () => {
    const tried = progressWith([day(0)], { finished_topics: ["market-cloth-price"] });
    expect(badges(tried, streak(tried, today)).find((badge) => badge.id === "bargainer")?.earned).toBe(false);
    // Another chore's goal is not a bargain.
    const deposit = progressWith([day(0)], { chores_met: ["deposit-refund"] });
    expect(badges(deposit, streak(deposit, today)).find((badge) => badge.id === "bargainer")?.earned).toBe(false);
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
