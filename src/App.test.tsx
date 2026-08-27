import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { bridge } from "./lib/bridge";
import type { EllaBridge, SpeechSegment, TurnResult } from "./types";

/** Read the current conversation prompt without coupling tests to its markup. */
function promptText(): string {
  return document.querySelector(".talk-prompt")?.textContent ?? "";
}

/** Walk the v6 onboarding flow: welcome, name, age, mic check, placement. */
async function onboard(name: string, age = "14") {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));

  fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: name } });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));

  fireEvent.change(screen.getByLabelText(`And how old are you, ${name}?`), { target: { value: age } });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));

  fireEvent.click(await screen.findByRole("button", { name: /skip this check/i }));
  const skipPlacement = await screen.findByRole("button", { name: /skip for now/i });
  expect(document.querySelector('[data-screen="onboarding-placement"] .ella--conversation')).toBeInTheDocument();
  fireEvent.click(skipPlacement);
  await screen.findByText(`Namaste, ${name}!`);
}

describe("Ella onboarding", () => {
  beforeEach(() => window.localStorage.clear());

  it("collects a name and an age before handing over to the app", async () => {
    await onboard("Aarav", "14");
    const saved = JSON.parse(window.localStorage.getItem("ella-desktop-state") ?? "{}");
    expect(saved.learner).toMatchObject({ name: "Aarav", age: 14 });
  });

  it("keeps Continue disabled until each answer is usable", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));

    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "A" } });
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "Asha" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    const ageField = screen.getByLabelText("And how old are you, Asha?");
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    fireEvent.change(ageField, { target: { value: "1" } });
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    fireEvent.change(ageField, { target: { value: "14" } });
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
  });

  it("lets a returning learner in through Log in", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /log in/i }));

    fireEvent.change(screen.getByLabelText(/what is your name/i), { target: { value: "Ira" } });
    fireEvent.click(screen.getByRole("button", { name: /take me in/i }));

    await screen.findByText("Namaste, Ira!");
    const saved = JSON.parse(window.localStorage.getItem("ella-desktop-state") ?? "{}");
    expect(saved.learner).toMatchObject({ name: "Ira" });
  });

  it("can step back through the flow", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));
    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "Riya" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(screen.getByLabelText("And how old are you, Riya?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Go back" }));
    expect(screen.getByLabelText("What should Ella call you?")).toHaveValue("Riya");
  });

  it("uses the expressive avatar for clear mic-check and retry states", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));
    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "Riya" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.change(screen.getByLabelText("And how old are you, Riya?"), { target: { value: "14" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    const stage = await screen.findByText("Quick mic check first.");
    expect(stage.closest('[data-screen="onboarding-miccheck"]')?.querySelector(".ella--celebration.ella--resting"))
      .toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Start the mic check" }));

    expect(await screen.findByRole("button", { name: "Try the mic check again" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(/microphone access is not available on this device/i);
    expect(document.querySelector('[data-mic-state="error"] .ella--reaction-error')).toBeInTheDocument();
  });
});

describe("Ella learner flow", () => {
  beforeEach(() => window.localStorage.clear());

  /**
   * With no clip playing there is no current word, so the reply must be drawn
   * plainly — dimming it would leave the screen looking broken between turns.
   */
  it("draws the reply plainly when nothing is being spoken", async () => {
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");

    const words = document.querySelectorAll(".talk-prompt .talk-word");
    expect(words.length).toBeGreaterThan(3);
    for (const word of words) {
      expect(word.className).toBe("talk-word");
    }
  });

  /**
   * Piper now speaks a sentence before the turn returns, which means the reply
   * would otherwise be heard several seconds before it appeared on screen. The
   * segment events carry their own text so it lands with the voice.
   */
  it("shows each sentence on screen as it is spoken, before the turn returns", async () => {
    // The Tauri bridge streams; the browser one does not, so stand one in. It
    // has to exist before the talk screen mounts, which is when it subscribes.
    let emit: ((segment: SpeechSegment) => void) | undefined;
    (bridge as EllaBridge).onSpeechSegment = async (handler) => {
      emit = handler;
      return () => {
        emit = undefined;
      };
    };
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    // Hold the turn open so "before the turn returns" is a real claim.
    let release: ((result: TurnResult) => void) | undefined;
    let turnSession = "";
    const real = bridge.sendTextTurn.bind(bridge);
    const held = vi
      .spyOn(bridge, "sendTextTurn")
      .mockImplementation(async (sessionId: string, text: string) => {
        turnSession = sessionId;
        const result = await real(sessionId, text);
        return new Promise<TurnResult>((resolve) => {
          release = () => resolve({ ...result, streamed_segments: 2 });
        });
      });

    try {
      fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
      fireEvent.change(screen.getByLabelText("Your answer"), {
        target: { value: "I ate vada pav near the station" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
      await waitFor(() => expect(emit).toBeDefined());

      const opening = promptText();
      emit?.({
        session_id: turnSession,
        turn: 1,
        index: 0,
        text: "That sounds delicious!",
        audio: { mime_type: "audio/wav", base64: "AAAA" },
        ready_ms: 900,
        words: [
          { text: "That", start_ms: 0, end_ms: 300 },
          { text: "sounds", start_ms: 300, end_ms: 700 },
          { text: "delicious!", start_ms: 700, end_ms: 1300 },
        ],
      });

      // The sentence is on screen while the turn is still in flight.
      await waitFor(() => expect(promptText()).toMatch(/that sounds delicious/i));
      expect(promptText()).not.toBe(opening);
      // Real spaces, not CSS-generated ones: the sentence must be readable and
      // copyable, not run together.
      expect(promptText()).toContain("That sounds delicious!");
      // Every word is its own element so one of them can be marked.
      expect(document.querySelectorAll(".talk-prompt .talk-word")).toHaveLength(3);

      // The second sentence arrives while the first is still being spoken. It
      // must land on its own line: sharing one centred paragraph is what made
      // "Sounds nice." jump sideways mid-read.
      const first = document.querySelector(".talk-sentence")?.textContent;
      emit?.({
        session_id: turnSession,
        turn: 1,
        index: 1,
        text: "What did you like about it?",
        audio: { mime_type: "audio/wav", base64: "AAAA" },
        ready_ms: 1600,
        words: [
          { text: "What", start_ms: 0, end_ms: 200 },
          { text: "did", start_ms: 200, end_ms: 400 },
          { text: "you", start_ms: 400, end_ms: 600 },
          { text: "like", start_ms: 600, end_ms: 800 },
          { text: "about", start_ms: 800, end_ms: 1000 },
          { text: "it?", start_ms: 1000, end_ms: 1400 },
        ],
      });
      await waitFor(() =>
        expect(document.querySelectorAll(".talk-prompt .talk-sentence")).toHaveLength(2),
      );
      const blocks = document.querySelectorAll(".talk-prompt .talk-sentence");
      expect(blocks[0].textContent).toBe(first);
      expect(blocks[1].textContent).toBe("What did you like about it?");
      // Word numbering continues across the sentence break, because the
      // highlight counts words through the whole reply.
      expect(document.querySelectorAll(".talk-prompt .talk-word")).toHaveLength(9);

      release?.({} as TurnResult);
      await waitFor(() => expect(held).toHaveResolved());

      // Sending the next turn must not restack the reply already on screen.
      // Clearing the grouping re-rendered the same words as one centred
      // paragraph, so the subtitles jumped the moment recording stopped.
      const settled = [...document.querySelectorAll(".talk-prompt .talk-sentence")].map(
        (block) => block.textContent,
      );
      expect(settled).toHaveLength(2);
      fireEvent.change(screen.getByLabelText("Your answer"), { target: { value: "It was hot" } });
      fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
      await waitFor(() => expect(held.mock.calls).toHaveLength(2));
      expect(
        [...document.querySelectorAll(".talk-prompt .talk-sentence")].map((b) => b.textContent),
      ).toEqual(settled);
    } finally {
      held.mockRestore();
      delete (bridge as EllaBridge).onSpeechSegment;
    }
  });

  it("moves from home to a real conversation with typed fallback", async () => {
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));

    await screen.findByText("End talk");
    expect(promptText()).toMatch(/tastiest thing you ate/i);
    const avatar = document.querySelector(".ella--conversation");
    const controls = document.querySelector(".talk-controls");
    expect(avatar).toHaveAttribute("aria-hidden", "true");
    expect(avatar?.parentElement).toBe(controls?.parentElement);
    expect(avatar?.contains(controls)).toBe(false);
    expect(screen.getByRole("button", { name: /start speaking|interrupt ella/i })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
    fireEvent.change(screen.getByLabelText("Your answer"), {
      target: { value: "I ate vada pav near the station" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));

    await waitFor(() => expect(promptText()).toMatch(/who would you like to share that with/i));
    // Only Ella's side is on screen: the learner's answer is not echoed back.
    expect(screen.queryByText(/near the station/)).not.toBeInTheDocument();
  });

  it("gives the conversation the whole window", async () => {
    await onboard("Kabir");
    expect(document.querySelector("aside.sidebar")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    expect(document.querySelector("aside.sidebar")).not.toBeInTheDocument();
  });

  it("returns to a usable resting state when ending a talk fails", async () => {
    await onboard("Kabir");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    const complete = vi
      .spyOn(bridge, "completeSession")
      .mockRejectedValueOnce(new Error("Could not finish the conversation."));

    try {
      fireEvent.click(screen.getByRole("button", { name: "End talk" }));
      expect(await screen.findByRole("alert")).toHaveTextContent("Could not finish the conversation.");
      expect(screen.getByRole("button", { name: "Start speaking" })).toHaveAttribute(
        "aria-pressed",
        "false",
      );
    } finally {
      complete.mockRestore();
    }
  });

  it("offers an unfinished talk on the home screen and resumes it", async () => {
    await bridge.saveLearner("Meera", 14);
    const session = await bridge.startSession("street-food");
    await bridge.sendTextTurn(session.id, "I ate poha this morning");

    render(<App />);
    await screen.findByText("Namaste, Meera!");
    expect(screen.getByText("Unfinished talk")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /continue talking/i }));

    await screen.findByText("End talk");
    // Resumed through `get_session`: Ella's reply to the earlier turn is back on
    // stage. The learner's own words stay off screen.
    expect(promptText()).toMatch(/who would you like to share that with/i);
    expect(screen.queryByText(/poha this morning/)).not.toBeInTheDocument();
  });

  it("offers every designed topic on the home bento", async () => {
    await onboard("Riya");
    expect(screen.getByText("Street food stories")).toBeInTheDocument();
    for (const label of [
      "Booking a cab",
      "A job interview",
      "At the doctor's clinic",
      "Asking for directions",
      "Bargaining at the market",
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    // The wide card splits its headline over two lines.
    expect(screen.getByText("Ordering at")).toBeInTheDocument();
    expect(screen.getByText("a restaurant")).toBeInTheDocument();
  });
});
