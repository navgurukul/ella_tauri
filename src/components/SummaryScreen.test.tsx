import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SummaryScreen } from "./SummaryScreen";
import { bridge } from "../lib/bridge";
import type { Assessment, ChoreRecap, LearnerProgress, SessionSummary, Standing } from "../types";

const standing: Standing = {
  level_number: 3,
  level_count: 6,
  level_name: "Finding My Voice",
  step: 2,
  step_count: 5,
  step_title: "Talking about my day",
  percent: 30,
  placed: true,
};

const nothingYet: LearnerProgress = {
  days: [],
  talks_finished: 0,
  answers: 0,
  finished_topics: [],
  chores_met: [],
  talks: [],
  spoken_ms: 0,
  spoken_answers: 0,
};

function summaryOf(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_id: "talk-1",
    topic_label: "Welcoming campus visitors",
    turns: 4,
    headline: "You kept that conversation going",
    encouragement: "",
    short: false,
    chore: null,
    ...overrides,
  };
}

function assessed(overrides: Partial<Assessment> = {}): Assessment {
  return {
    session_id: "talk-1",
    kind: "talk",
    standing,
    advanced: null,
    skills: [
      { label: "Greeting", count: 4 },
      { label: "Directions", count: 1 },
    ],
    scored: true,
    notes: {
      went_well: ["Warm greeting", "Gave reasons"],
      fix: { said: "Myself Aarav", better: "I'm Aarav" },
      checked: true,
    },
    ...overrides,
  };
}

const stall: ChoreRecap = {
  chore_id: "market-cloth-price",
  character_id: "stall-owner",
  unit: "Rs",
  direction: "down",
  target: 400,
  figure: 380,
  agreed: true,
  met: true,
  times_met: 1,
  last_line: "Okay, okay. Rs 380, final!",
};

function renderRecap(props: Partial<Parameters<typeof SummaryScreen>[0]> = {}) {
  const handlers = {
    onRetry: vi.fn(),
    onCelebrated: vi.fn(),
    onDone: vi.fn(),
    onTryAgain: vi.fn(),
    onLevels: vi.fn(),
  };
  const view = render(
    <SummaryScreen
      summary={summaryOf()}
      assessment={null}
      assessError={null}
      before={nothingYet}
      learnerName="Aarav Sharma"
      standing={standing}
      nextTopic="Sunday vegetable run"
      {...handlers}
      {...props}
    />,
  );
  return { ...view, ...handlers };
}

describe("the recap", () => {
  afterEach(() => {
    delete (bridge as { speakFix?: unknown }).speakFix;
  });

  it("holds a place for every note while Ella looks back, then fills them in", () => {
    const { rerender, onDone, onTryAgain, onRetry, onCelebrated, onLevels } = renderRecap();
    expect(screen.getByRole("heading", { name: "Nice talking, Aarav!" })).toBeInTheDocument();
    expect(screen.getAllByRole("status", { name: "Ella is looking back over your talk…" })).toHaveLength(3);

    rerender(
      <SummaryScreen
        summary={summaryOf()}
        assessment={assessed()}
        assessError={null}
        before={nothingYet}
        learnerName="Aarav Sharma"
        standing={standing}
        nextTopic="Sunday vegetable run"
        {...{ onDone, onTryAgain, onRetry, onCelebrated, onLevels }}
      />,
    );
    expect(screen.queryByRole("status", { name: "Ella is looking back over your talk…" })).not.toBeInTheDocument();
    const well = screen.getByRole("region", { name: "Went well" });
    expect(well).toHaveTextContent("Warm greeting");
    expect(well).toHaveTextContent("Gave reasons");
    expect(well).not.toHaveTextContent("Nothing to fix");
    const fix = screen.getByRole("region", { name: "One fix" });
    expect(fix).toHaveTextContent("You said: Myself Aarav");
    expect(fix).toHaveTextContent("Better: I'm Aarav");
    expect(screen.getByLabelText("Greeting: 4 talks")).toBeInTheDocument();
    expect(screen.getByLabelText("Directions: first time")).toBeInTheDocument();
    // Three notes and the streak.
    expect(document.querySelector(".recap__tiles")?.getAttribute("style")).toContain("--columns: 4");
    expect(onCelebrated).not.toHaveBeenCalled();
  });

  it("shows what went well at once, the skills once scored, and the fix once looked for", () => {
    const summary = summaryOf({ went_well: ["Warm greeting", "Gave reasons"] });
    const { rerender, onDone, onTryAgain, onRetry, onCelebrated, onLevels } = renderRecap({ summary });
    const recap = (props: Partial<Parameters<typeof SummaryScreen>[0]>) => (
      <SummaryScreen
        summary={summary}
        assessment={null}
        assessError={null}
        before={nothingYet}
        learnerName="Aarav Sharma"
        standing={standing}
        nextTopic="Sunday vegetable run"
        {...{ onDone, onTryAgain, onRetry, onCelebrated, onLevels }}
        {...props}
      />
    );
    const waiting = () => screen.queryAllByRole("status", { name: "Ella is looking back over your talk…" });

    // Read off the learner's words as the talk closed: no model needed.
    const well = screen.getByRole("region", { name: "Went well" });
    expect(well).toHaveTextContent("Warm greeting");
    expect(well).toHaveTextContent("Gave reasons");
    expect(waiting()).toHaveLength(2);

    // Scored: the skills are in, and the fix keeps its place, still waiting.
    const scored = assessed({ notes: { went_well: ["Warm greeting", "Gave reasons"], fix: null, checked: false } });
    rerender(recap({ assessment: scored, fixPending: true }));
    expect(screen.getByLabelText("Greeting: 4 talks")).toBeInTheDocument();
    expect(waiting()).toHaveLength(1);
    expect(screen.getByRole("region", { name: "One fix" })).toContainElement(waiting()[0]);
    expect(screen.getByRole("region", { name: "Went well" })).not.toHaveTextContent("Nothing to fix");

    rerender(recap({ assessment: assessed() }));
    expect(waiting()).toHaveLength(0);
    expect(screen.getByRole("region", { name: "One fix" })).toHaveTextContent("Better: I'm Aarav");

    // Or the model looked and found nothing: the fix gives up its place.
    rerender(recap({ assessment: assessed({ notes: { went_well: ["Warm greeting", "Gave reasons"], fix: null, checked: true } }) }));
    expect(screen.queryByRole("region", { name: "One fix" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Went well" })).toHaveTextContent("Nothing to fix");
  });

  it("says there was nothing to fix only when a model looked", () => {
    renderRecap({ assessment: assessed({ notes: { went_well: ["Gave reasons", "Full sentences"], fix: null, checked: true } }) });
    expect(screen.getByRole("region", { name: "Went well" })).toHaveTextContent("Nothing to fix");
    expect(screen.queryByRole("region", { name: "One fix" })).not.toBeInTheDocument();
  });

  it("says the better line in Ella's own voice when she can", async () => {
    const speakFix = vi.fn().mockResolvedValue({ audio: null, speech_words: [], streamed_segments: 0 });
    (bridge as { speakFix?: unknown }).speakFix = speakFix;
    renderRecap({ assessment: assessed() });
    fireEvent.click(screen.getByRole("button", { name: "Hear it" }));
    await waitFor(() => expect(speakFix).toHaveBeenCalledWith("talk-1"));
  });

  it("offers to try again when the talk could not be read, and still counts the streak", () => {
    const { onRetry } = renderRecap({
      assessError: "Ella could not look back over this talk just now. Try again in a moment.",
    });
    expect(screen.getByRole("alert")).toHaveTextContent("could not look back over this talk");
    expect(screen.getByRole("alert")).toHaveTextContent("Your streak still counts.");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();
    expect(screen.getByRole("region", { name: "Streak" })).toHaveTextContent("1 day streak");
  });

  it("celebrates moving on and opens the level map from there and from the footer", () => {
    const { onLevels, onCelebrated } = renderRecap({ assessment: assessed({ advanced: "step" }) });
    expect(onCelebrated).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: /Step complete!/ }));
    fireEvent.click(
      screen.getByRole("button", { name: "Level 3: Finding My Voice, Step 2 of 5. See all levels" }),
    );
    expect(onLevels).toHaveBeenCalledTimes(2);
  });

  it("shows a goal met, and the badge it earned for the first time", () => {
    const { onDone } = renderRecap({ summary: summaryOf({ chore: stall }), assessment: assessed() });
    expect(screen.getByRole("heading", { name: "Goal met!" })).toBeInTheDocument();
    expect(screen.getByText("Bippo · STALL PRICE")).toBeInTheDocument();
    const goals = screen.getByRole("list", { name: "Your goal" });
    expect(goals).toHaveTextContent("Rs 400 or less (done)");
    expect(goals).toHaveTextContent("He agrees (done)");
    expect(screen.getByText("New badge")).toBeInTheDocument();
    expect(screen.getByText("Bargainer")).toBeInTheDocument();
    expect(screen.getByText("Okay, okay. Rs 380, final!")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onDone).toHaveBeenCalledOnce();
  });

  it("counts a goal met again", () => {
    renderRecap({ summary: summaryOf({ chore: { ...stall, times_met: 3 } }) });
    expect(screen.getByText("Earned again")).toBeInTheDocument();
    expect(screen.getByText("×3")).toBeInTheDocument();
  });

  it("says what a missed goal needed, and offers the chore again", () => {
    const { onTryAgain } = renderRecap({
      summary: summaryOf({ chore: { ...stall, figure: 450, met: false, times_met: 0, last_line: "Rs 450. Last price!" } }),
    });
    expect(screen.getByRole("heading", { name: "So close!" })).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Your goal" })).toHaveTextContent("Rs 400 or less (not this time)");
    expect(screen.getByText("Get it to Rs 400 or less to earn this badge.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onTryAgain).toHaveBeenCalledWith("market-cloth-price");
  });

  it("gives the deposit its own badge, by the name the profile lists it under", () => {
    renderRecap({
      summary: summaryOf({
        chore: { ...stall, chore_id: "deposit-refund", character_id: "landlord", direction: "up", target: 3500, figure: 3600 },
      }),
    });
    expect(screen.getByText("Grumble · DEPOSIT")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Your goal" })).toHaveTextContent("Rs 3500 or more (done)");
    expect(screen.getByText("New badge")).toBeInTheDocument();
    expect(screen.getByText("Deposit back")).toBeInTheDocument();
  });

  it("thanks nobody for a talk with nothing said in it", () => {
    renderRecap({ summary: summaryOf({ turns: 0, short: true }) });
    expect(screen.getByRole("heading", { name: "Next time, Aarav!" })).toBeInTheDocument();
    expect(screen.getByText("Say a few words next time and it counts.")).toBeInTheDocument();
    const run = screen.getByRole("region", { name: "Streak" });
    expect(run).toHaveTextContent("0 day streak");
    // Tomorrow is not the next day of a streak today has not counted for.
    expect(document.querySelector(".recap-next")).not.toHaveTextContent("Day");
  });
});
