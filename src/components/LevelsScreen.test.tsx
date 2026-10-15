import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LevelsScreen } from "./LevelsScreen";
import { bridge } from "../lib/bridge";
import { levelsAt } from "../lib/curriculum";
import type { AppSnapshot, Standing } from "../types";

/** Where the learner stands, as the backend reports it for level 3 of 6. */
function standing(step: number, percent: number): Standing {
  return {
    level_number: 3,
    level_count: 6,
    level_name: "Finding My Voice",
    step,
    step_count: 5,
    step_title: "Finding better words",
    percent,
    placed: true,
  };
}

function snapshot(where: Standing): AppSnapshot {
  return {
    learner: { name: "Meera", age: 12, level_name: "Finding My Voice", created_at: "2024-01-01" },
    topics: [],
    recent_sessions: [],
    progress: {
      days: [],
      talks_finished: 0,
      answers: 0,
      finished_topics: [],
      chores_met: [],
      talks: [],
      spoken_ms: 0,
      spoken_answers: 0,
    },
    standing: where,
    engine_status: { mode: "demo", label: "Demo", ready: true, components: [] },
  };
}

const handlers = {
  onBack: vi.fn(),
  onPractise: vi.fn(),
  onFindLevel: vi.fn(),
  onBadgeStart: vi.fn(),
};

function show(where: Standing) {
  return render(
    <LevelsScreen snapshot={snapshot(where)} avatarColor="#9347DD" busy={false} {...handlers} />,
  );
}

describe("Levels and badges", () => {
  afterEach(() => vi.restoreAllMocks());

  it("says how to start, what finishes a level, and what opens a badge", async () => {
    vi.spyOn(bridge, "levels").mockResolvedValue(levelsAt({ level: "A2", step: 1 }));
    show(standing(1, 0));
    await screen.findByRole("list", { name: "Levels" });

    // How to start: the practice button says what it does.
    expect(
      screen.getByText("Each talk with Ella practises the skills of your current step."),
    ).toBeInTheDocument();
    // What finishes a level and opens the next.
    expect(screen.getByText(/Finish all 5 steps to open the next level/)).toBeInTheDocument();
    // How to find what earns a badge.
    expect(screen.getByText("Open a badge to see what earns it.")).toBeInTheDocument();
    // The progress the learner can read: their step, and the level's percentage.
    expect(screen.getByText("Step 1 of 5 · 0 of 4 skills passed")).toBeInTheDocument();
    expect(screen.getByText("You’re here · 0% to level 4")).toBeInTheDocument();
    // The bar only draws that percentage, so a screen reader hears it once.
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("shows a higher level percentage as a talk moves the learner on", async () => {
    const levels = vi.spyOn(bridge, "levels").mockResolvedValue(levelsAt({ level: "A2", step: 1 }));
    const view = show(standing(1, 0));
    const path = await screen.findByRole("list", { name: "Levels" });
    const bar = () => path.querySelector(".level-stop.is-current .level-stop__bar > span");
    expect(bar()).toHaveStyle({ width: "0%" });

    // A talk passed the step's last skill: the app reads where the learner
    // stands again, and the bar and its step text move with them.
    levels.mockResolvedValue(levelsAt({ level: "A2", step: 2 }));
    view.rerender(
      <LevelsScreen snapshot={snapshot(standing(2, 20))} avatarColor="#9347DD" busy={false} {...handlers} />,
    );

    await waitFor(() => expect(bar()).toHaveStyle({ width: "20%" }));
    expect(screen.getByText("You’re here · 20% to level 4")).toBeInTheDocument();
    expect(screen.getByText("Step 2 of 5 · 0 of 4 skills passed")).toBeInTheDocument();
  });
});
