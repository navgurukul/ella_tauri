import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { bridge } from "./lib/bridge";
import { dayKey } from "./lib/days";
import { mouthGeometry } from "./lib/mouth";
import { SMILE } from "./lib/visemes";
import type { Assessment, EllaBridge, PhonemeSpan, SpeechSegment, TurnResult } from "./types";

/** Read the current conversation prompt without coupling tests to its markup. */
function promptText(): string {
  return document.querySelector(".talk-prompt")?.textContent ?? "";
}

/** Walk the onboarding flow: welcome, name, age, mic check, placement. */
async function onboard(name: string, age = "14") {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));

  fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: name } });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));

  fireEvent.change(screen.getByLabelText(`And how old are you, ${name}?`), { target: { value: age } });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));

  fireEvent.click(await screen.findByRole("button", { name: /skip this check/i }));
  const skipPlacement = await screen.findByRole("button", { name: "Skip" });
  expect(document.querySelector('[data-screen="onboarding-placement"] .ella--conversation')).toBeInTheDocument();
  fireEvent.click(skipPlacement);
  await screen.findByText(`Namaste, ${name}!`);
}

/** Onboard, then open Talk partners from the sidebar. */
async function openTalkPartners(name: string, age: string) {
  await onboard(name, age);
  fireEvent.click(screen.getByRole("button", { name: "Talk partners" }));
  await screen.findByText("Your mentor");
}

describe("Ella onboarding", () => {
  beforeEach(() => window.localStorage.clear());

  it("collects a name and an age before handing over to the app", async () => {
    await onboard("Aarav", "14");
    expect((await bridge.bootstrap()).learner).toMatchObject({ name: "Aarav", age: 14 });
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

  it("lets a learner in through Log in on a laptop nobody has used", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /log in/i }));

    // Nobody is saved, so there is nobody to welcome back: it asks the name.
    expect(screen.getByRole("button", { name: /take me in/i })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Welcome back! What is your name?"), { target: { value: "Ira" } });
    fireEvent.click(screen.getByRole("button", { name: /take me in/i }));

    // Straight in: no age, no mic check, no first talk.
    await screen.findByText("Namaste, Ira!");
    expect((await bridge.bootstrap()).learner).toMatchObject({ name: "Ira", age: null });
  });

  it("welcomes the saved learner back by name at Log in", async () => {
    await bridge.saveLearner("Meera", 12);
    await bridge.saveAvatarColor("#FF7A00");
    await bridge.logOut();

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /log in/i }));
    expect(screen.getByRole("heading", { name: "Welcome back, Meera!" })).toBeInTheDocument();
    expect(welcomeBackAvatarColor()).toBe("#FF7A00");
    // There is only the one learner, so there is no name to type.
    expect(screen.queryByLabelText(/what is your name/i)).not.toBeInTheDocument();

    // Go back leads to the welcome screen, and Let’s start is still there.
    fireEvent.click(screen.getByRole("button", { name: "Go back" }));
    fireEvent.click(await screen.findByRole("button", { name: /log in/i }));
    fireEvent.click(screen.getByRole("button", { name: "Take me in" }));

    await screen.findByText("Namaste, Meera!");
    expect((await bridge.bootstrap()).learner).toMatchObject({ name: "Meera", age: 12, avatar_color: "#FF7A00" });
  });

  it("keeps the welcome-back step open when logging in is refused", async () => {
    await bridge.saveLearner("Meera", 12);
    await bridge.logOut();
    const refuse = vi.spyOn(bridge, "logIn").mockRejectedValueOnce(new Error("Tell Ella your name first."));
    try {
      render(<App />);
      fireEvent.click(await screen.findByRole("button", { name: /log in/i }));
      fireEvent.click(screen.getByRole("button", { name: "Take me in" }));

      expect(await screen.findByText("Tell Ella your name first.")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "Welcome back, Meera!" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Take me in" })).toBeEnabled();
    } finally {
      refuse.mockRestore();
    }
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

  it("has the corner Ella peek up and greet the learner by name", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));

    const corner = document.querySelector(".ella--corner-ob");
    expect(corner).toBeInTheDocument();
    expect(corner).not.toHaveClass("is-peeking");
    expect(screen.queryByText("Hi, Riya!")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "Riya" } });
    expect(screen.getByText("Hi, Riya!")).toBeInTheDocument();
    expect(document.querySelector(".ella--corner-ob")).toHaveClass("is-peeking");
  });

  it("springs the age Ella up once there is an age", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));
    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "Riya" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // The age step has its own Ella rather than the corner one.
    expect(document.querySelector(".ella--corner-ob")).not.toBeInTheDocument();
    expect(document.querySelector(".ella--ob-age")).not.toHaveClass("is-up");
    fireEvent.change(screen.getByLabelText("And how old are you, Riya?"), { target: { value: "1" } });
    expect(document.querySelector(".ella--ob-age")).toHaveClass("is-up");
  });

  it("runs a real mic check and says so when the microphone will not open", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));
    fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: "Riya" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.change(screen.getByLabelText("And how old are you, Riya?"), { target: { value: "14" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await screen.findByText("Quick mic check first.");
    expect(document.querySelector(".ella--corner-ob")).toBeInTheDocument();
    expect(screen.getByText("Click, then say anything")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Start the mic check" }));

    expect(await screen.findByRole("button", { name: "Try the mic check again" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(/microphone access is not available on this device/i);
    expect(document.querySelector('[data-mic-state="error"]')).toBeInTheDocument();
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
   * The reply is heard as soon as it is written and saved: its first sentence
   * carries the whole text, which is shown the moment Ella starts saying it,
   * before the turn itself returns. It never arrives in pieces — a reply that
   * grows cannot be centred without the words already being read jumping as
   * each piece lands — and when the turn does return, nothing is said twice.
   */
  it("shows the whole reply as Ella starts saying it, never in pieces, and says it once", async () => {
    const started: number[] = [];
    class FakeAudioContext {
      state = "running";
      currentTime = 0;
      destination = {};
      async resume() {}
      async close() {}
      async decodeAudioData(buffer: ArrayBuffer) {
        return { duration: buffer.byteLength } as AudioBuffer;
      }
      createBufferSource() {
        return {
          buffer: null,
          onended: null as (() => void) | null,
          connect() {},
          stop() {},
          start(at: number) {
            started.push(at);
          },
        };
      }
    }
    Object.defineProperty(window, "AudioContext", { configurable: true, value: FakeAudioContext });
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

    // Hold the turn open so "before it returns" is a real claim.
    const audio = { mime_type: "audio/wav", base64: "AAAA" };
    let release: (() => void) | undefined;
    let turnSession = "";
    let reply = "";
    const real = bridge.sendTextTurn.bind(bridge);
    const held = vi
      .spyOn(bridge, "sendTextTurn")
      .mockImplementation(async (sessionId: string, text: string) => {
        turnSession = sessionId;
        const result = await real(sessionId, text);
        reply = result.ella_message.content;
        return new Promise<TurnResult>((resolve) => {
          // With the recording the replay button keeps, which must not play now.
          release = () => resolve({ ...result, audio, streamed_segments: 2 });
        });
      });

    try {
      const opening = promptText();
      fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
      fireEvent.change(screen.getByLabelText("Your answer"), {
        target: { value: "I ate vada pav near the station" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
      await waitFor(() => expect(reply).not.toBe(""));

      // Another line's sentence feeds nothing and shows nothing.
      emit?.({
        session_id: turnSession,
        turn: 0,
        index: 0,
        text: "Hello.",
        audio,
        ready_ms: 100,
        words: [],
        reply: "Not this reply.",
      });
      expect(promptText()).toBe(opening);

      // This reply's first sentence brings all of it, at once.
      emit?.({
        session_id: turnSession,
        turn: 1,
        index: 0,
        text: "That sounds delicious!",
        audio,
        ready_ms: 900,
        words: [{ text: "That", start_ms: 0, end_ms: 300 }],
        reply,
      });
      await waitFor(() => expect(promptText()).toBe(reply));
      expect(document.querySelectorAll(".talk-prompt .talk-word")).toHaveLength(
        reply.split(/\s+/).filter(Boolean).length,
      );
      emit?.({ session_id: turnSession, turn: 1, index: 1, text: "Who else?", audio, ready_ms: 950, words: [] });
      await waitFor(() => expect(started).toHaveLength(2));

      await act(async () => {
        release?.();
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      // The same words, still whole, and the recording kept for the replay
      // button is not played on top of what she is already saying.
      expect(promptText()).toBe(reply);
      expect(started).toHaveLength(2);
    } finally {
      held.mockRestore();
      delete (bridge as EllaBridge).onSpeechSegment;
      Reflect.deleteProperty(window, "AudioContext");
    }
  });

  /**
   * Without any sentence of it having arrived, the turn's own recording is
   * played when the turn returns, and its text shown then, whole.
   */
  it("shows and plays the whole reply when the turn returns if nothing streamed", async () => {
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
    try {
      const opening = promptText();
      fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
      fireEvent.change(screen.getByLabelText("Your answer"), {
        target: { value: "I ate vada pav near the station" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
      await waitFor(() => expect(promptText()).not.toBe(opening));
      expect(promptText()).toMatch(/who would you like to share that with/i);
      expect(emit).toBeDefined();
    } finally {
      delete (bridge as EllaBridge).onSpeechSegment;
    }
  });

  /**
   * Her mouth follows Piper's own timing of what she says: the opening streams
   * with its sounds, the talk stage opens her mouth on the vowel, and it goes
   * back to her smile once she has finished.
   */
  it("moves Ella's mouth with the sounds of what she says", async () => {
    const started: { at: number }[] = [];
    let clock: { currentTime: number; drain(): void } | undefined;
    class FakeAudioContext {
      state = "running";
      currentTime = 0;
      destination = {};
      private ended: (() => void)[] = [];
      constructor() {
        clock = this;
      }
      async resume() {}
      async close() {}
      /** A second of audio per byte of payload. */
      async decodeAudioData(buffer: ArrayBuffer) {
        return { duration: buffer.byteLength } as AudioBuffer;
      }
      createBufferSource() {
        const ended = this.ended;
        return {
          buffer: null,
          onended: null as (() => void) | null,
          connect() {},
          stop() {},
          start(at: number) {
            started.push({ at });
            ended.push(() => this.onended?.());
          },
        };
      }
      drain() {
        for (const end of this.ended.splice(0)) end();
      }
    }
    // Three seconds: a long "a" between the sentence's start and end.
    const phonemes: PhonemeSpan[] = [
      { phoneme: "^", start_ms: 0, end_ms: 100 },
      { phoneme: "a", start_ms: 100, end_ms: 900 },
      { phoneme: "$", start_ms: 900, end_ms: 3000 },
    ];
    const audio = { mime_type: "audio/wav", base64: "AAAA" };
    let emit: ((segment: SpeechSegment) => void) | undefined;
    Object.defineProperty(window, "AudioContext", { configurable: true, value: FakeAudioContext });
    (bridge as EllaBridge).onSpeechSegment = async (handler) => {
      emit = handler;
      return () => {
        emit = undefined;
      };
    };
    (bridge as EllaBridge).speakOpening = async (sessionId) => {
      // Segments arrive as events after the call, never inside it.
      await new Promise((resolve) => setTimeout(resolve, 0));
      emit?.({ session_id: sessionId, turn: 0, index: 0, text: "Ah.", audio, ready_ms: 300, words: [], phonemes });
      return { audio, speech_words: [], speech_phonemes: phonemes, streamed_segments: 1 };
    };
    const lips = () => document.querySelector(".ella--stage-talk .ella__lips")?.getAttribute("d") ?? "";
    const depth = () => {
      const ys = (lips().match(/-?\d+(\.\d+)?/g) ?? []).map(Number).filter((_, index) => index % 2 === 1);
      return Math.max(...ys) - Math.min(...ys);
    };

    try {
      await onboard("Aarav");
      fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
      await screen.findByText("End talk");
      await waitFor(() => expect(started).toHaveLength(1));
      expect(document.querySelector(".ella--stage-talk")).toHaveClass("ella--lipsync");
      expect(lips()).toBe(mouthGeometry(SMILE, 30, 3).path);

      clock!.currentTime = started[0].at + 0.5;
      await waitFor(() => expect(depth()).toBeGreaterThan(10));

      clock!.drain();
      await waitFor(() => expect(lips()).toBe(mouthGeometry(SMILE, 30, 3).path));
    } finally {
      delete (bridge as EllaBridge).onSpeechSegment;
      delete (bridge as EllaBridge).speakOpening;
      Reflect.deleteProperty(window, "AudioContext");
    }
  });

  /** The system voice reports no timings, so her mouth cannot follow it. */
  it("keeps the static open mouth for a voice it cannot time", async () => {
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    await waitFor(() => expect(document.querySelector(".ella--stage-talk")).toHaveClass("ella--speaking", "is-untimed"));
  });

  it("moves from home to a real conversation with typed fallback", async () => {
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));

    await screen.findByText("End talk");
    expect(promptText()).toMatch(/tastiest thing you ate/i);
    // Ella is scenery; the controls standing on her must stay reachable.
    const avatar = document.querySelector(".ella--conversation");
    const controls = document.querySelector(".talk-controls");
    expect(avatar?.closest('[aria-hidden="true"]')).not.toBeNull();
    expect(controls?.closest('[aria-hidden="true"]')).toBeNull();
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

  it("recaps a finished talk across the whole window: what went well, the streak and tomorrow", async () => {
    await onboard("Asha");
    const topics = (await bridge.bootstrap()).topics;
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    await typeAnswers(3);
    fireEvent.click(screen.getByRole("button", { name: "End talk" }));

    expect(await screen.findByRole("heading", { name: "Nice talking, Asha!" })).toBeInTheDocument();
    expect(document.querySelector("aside.sidebar")).not.toBeInTheDocument();
    expect(screen.getByText(topics[0].label)).toBeInTheDocument();
    // Read off the answers, with no model in the preview to look for a fix,
    // so it claims neither a fix nor that there was nothing to fix.
    const well = await screen.findByRole("region", { name: "Went well" });
    await waitFor(() => expect(well.querySelectorAll("li")).toHaveLength(2));
    expect(well).toHaveTextContent("Told what happened");
    expect(well).toHaveTextContent("Full sentences");
    expect(screen.queryByRole("region", { name: "One fix" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Streak" })).toHaveTextContent("1 day streak. New streak");

    const foot = document.querySelector(".recap__foot") as HTMLElement;
    expect(foot).toHaveTextContent(`Tomorrow${topics[1].label}Day 2`);
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await screen.findByText("Namaste, Asha!");
  });

  it("says straight away when a talk was too short for notes", async () => {
    await onboard("Ravi");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    await typeAnswers(1);
    fireEvent.click(screen.getByRole("button", { name: "End talk" }));

    expect(await screen.findByRole("heading", { name: "Short and sweet, Ravi!" })).toBeInTheDocument();
    expect(screen.getByText("Talk a little longer to get Ella’s notes.")).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "Ella is looking back over your talk…" })).not.toBeInTheDocument();
    // One answer still counts for the streak.
    expect(screen.getByRole("region", { name: "Streak" })).toHaveTextContent("1 day streak");
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

  it("leads with today's talk, shows four more topics, and the rest behind View all", async () => {
    await onboard("Riya");
    expect(screen.getByText("Today’s talk").nextElementSibling).toHaveTextContent("Street food stories");
    for (const label of [
      "Ordering at a restaurant",
      "Booking a cab",
      "A job interview",
      "At the doctor's clinic",
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(screen.queryByText("Asking for directions")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "View all" }));
    for (const label of ["Asking for directions", "Bargaining at the market", "Booking a cab"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.queryByText("Asking for directions")).not.toBeInTheDocument();
  });

  it("works the microphone with Space, as the hint under it says", async () => {
    // Let the opening finish being said, so Ella is resting and waiting.
    const speak = vi
      .spyOn(window.speechSynthesis, "speak")
      .mockImplementation((utterance) => queueMicrotask(() => utterance.onend?.({} as SpeechSynthesisEvent)));
    try {
      await onboard("Kabir");
      fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
      await screen.findByText("End talk");
      await waitFor(() =>
        expect(document.querySelector(".mic-hint")).toHaveTextContent("Click or pressSpace"),
      );

      // On a focused button Space belongs to the button, not the mic.
      fireEvent.keyDown(screen.getByRole("button", { name: "End talk" }), { code: "Space", key: " " });
      expect(screen.queryByLabelText("Your answer")).not.toBeInTheDocument();

      // jsdom has no microphone, so opening it fails over to typing — which is
      // how this shows the key reached the mic at all.
      fireEvent.keyDown(document.body, { code: "Space", key: " " });
      expect(await screen.findByLabelText("Your answer")).toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent(/could not open the microphone/i);
    } finally {
      speak.mockRestore();
    }
  });
});

describe("Ella talk partners", () => {
  beforeEach(() => window.localStorage.clear());

  it("starts a partner's chore as a conversation", async () => {
    await openTalkPartners("Aarav", "14");
    for (const name of ["Bippo", "Grumble", "Dr Wobble", "Zig"]) {
      expect(screen.getByText(name)).toBeInTheDocument();
    }
    // The debate has no chore behind it yet.
    expect(screen.getByRole("button", { name: /take a stand/i })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /talk a stall price down/i }));
    await screen.findByText("End talk");
    expect(document.querySelector(".talk-head .pill")).toHaveTextContent("Talk a stall price down");
    expect(promptText()).toMatch(/look at these shirts/i);
  });

  it("opens the doctor's goal as the matching topic", async () => {
    await openTalkPartners("Aarav", "14");
    fireEvent.click(screen.getByRole("button", { name: /explain what is wrong/i }));
    await screen.findByText("End talk");
    expect(document.querySelector(".talk-head .pill")).toHaveTextContent("At the doctor's clinic");
    expect(promptText()).toMatch(/I am the doctor here/i);
  });

  it("leaves out the goals a younger learner is not ready for", async () => {
    await openTalkPartners("Mira", "9");
    expect(screen.queryByText("Bippo")).not.toBeInTheDocument();
    expect(screen.queryByText("Grumble")).not.toBeInTheDocument();
    expect(screen.getByText("Dr Wobble")).toBeInTheDocument();
    expect(screen.getByText("Zig")).toBeInTheDocument();
  });
});

describe("Ella profile", () => {
  beforeEach(() => window.localStorage.clear());

  async function openProfile(name = "Aarav") {
    await onboard(name, "14");
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
  }

  it("edits the name, age and avatar colour", async () => {
    await openProfile();
    expect(screen.getByText("14 years old")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
    fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Aarav K" } });
    fireEvent.change(screen.getByLabelText("Your age"), { target: { value: "15" } });
    fireEvent.click(screen.getByRole("button", { name: "Orange" }));
    expect(screen.getByRole("button", { name: "Orange" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Done" }));

    expect(await screen.findByText("15 years old")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Aarav K" })).toBeInTheDocument();
    // The colour is the learner's now, not the device's.
    expect((await bridge.bootstrap()).learner).toMatchObject({
      name: "Aarav K",
      age: 15,
      avatar_color: "#FF7A00",
    });
    expect(window.localStorage.getItem("ella-avatar-color")).toBeNull();
  });

  it("only offers Done for a name and age the backend can take", async () => {
    await openProfile();
    fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
    const done = screen.getByRole("button", { name: "Done" });

    fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "A" } });
    expect(done).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Aarav" } });
    // An age once given cannot be emptied, and must be one onboarding accepts.
    fireEvent.change(screen.getByLabelText("Your age"), { target: { value: "" } });
    expect(done).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Your age"), { target: { value: "300" } });
    expect(done).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Your age"), { target: { value: "15" } });
    expect(done).toBeEnabled();
  });

  it("keeps editing open when the backend refuses the change", async () => {
    await openProfile();
    const refuse = vi
      .spyOn(bridge, "saveLearner")
      .mockRejectedValueOnce(new Error("Ella could not save that just now."));
    try {
      fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
      fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Aarav K" } });
      fireEvent.click(screen.getByRole("button", { name: "Done" }));

      expect(await screen.findByRole("alert")).toHaveTextContent("Ella could not save that just now.");
      expect(screen.getByLabelText("Your name")).toHaveValue("Aarav K");
    } finally {
      refuse.mockRestore();
    }
  });

  it("adds up the time spoken once Ella has kept how long an answer lasted", async () => {
    await bridge.saveLearner("Asha", 14);
    const read = bridge.bootstrap.bind(bridge);
    const spoken = vi.spyOn(bridge, "bootstrap").mockImplementation(async () => {
      const snapshot = await read();
      return {
        ...snapshot,
        progress: {
          ...snapshot.progress,
          days: [{ day: dayKey(new Date()), talks: 1, answers: 3, spoken_ms: 120_000 }],
          talks_finished: 1,
          answers: 3,
          spoken_ms: 7_380_000,
          spoken_answers: 3,
        },
      };
    });
    try {
      render(<App />);
      await screen.findByText("Namaste, Asha!");
      expect(document.querySelector(".card--week")).toHaveTextContent("2minutes spoken");
      fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
      await screen.findByText("My profile");
      expect(statTile("SPOKEN")).toBe("2h 3m");
    } finally {
      spoken.mockRestore();
    }
  });

  it("reruns the mic check from settings and comes back", async () => {
    await openProfile();
    fireEvent.click(screen.getByRole("button", { name: "Mic check" }));
    await screen.findByText("Quick mic check first.");
    expect(document.querySelector("aside.sidebar")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(await screen.findByText("My profile")).toBeInTheDocument();
  });

  it("logs out to the welcome screen, deletes nothing, and logs back in to the same history", async () => {
    await onboard("Aarav", "14");
    // One real talk with an answer in it, ended by hand.
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
    fireEvent.change(screen.getByLabelText("Your answer"), { target: { value: "I ate vada pav" } });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
    await waitFor(() => expect(promptText()).toMatch(/who would you like to share that with/i));
    fireEvent.click(screen.getByRole("button", { name: "End talk" }));
    fireEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(await screen.findByText("1 day")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
    fireEvent.click(screen.getByRole("button", { name: "Log out" }));
    expect(await screen.findByRole("button", { name: /let’s start/i })).toBeInTheDocument();
    expect(screen.getByText("Hi buddy!")).toBeInTheDocument();
    // Signed out, and nothing deleted.
    const signedOut = await bridge.bootstrap();
    expect(signedOut.learner).toBeNull();
    expect(signedOut.saved_learner).toEqual({ name: "Aarav", avatar_color: null });

    fireEvent.click(screen.getByRole("button", { name: /log in/i }));
    expect(screen.getByRole("heading", { name: "Welcome back, Aarav!" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Take me in" }));

    await screen.findByText("Namaste, Aarav!");
    expect(screen.getByText("1 day")).toBeInTheDocument();
    expect(screen.queryByText("Unfinished talk")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
    expect(screen.getByText("14 years old")).toBeInTheDocument();
    expect(statTile("TALKS DONE")).toBe("1");
    // A typed answer has no length to add up, so nothing is known yet.
    expect(statTile("SPOKEN")).toBe("—");
    expect(statTile("DAY STREAK")).toBe("1");
  });

  it("keeps the history when Let’s start is chosen after logging out", async () => {
    // Aarav has talked, then logged out.
    const aarav = await bridge.saveLearner("Aarav", 14);
    await bridge.saveAvatarColor("#68B506");
    const earlier = await bridge.startSession("street-food");
    await bridge.sendTextTurn(earlier.id, "I ate vada pav");
    await bridge.sendTextTurn(earlier.id, "It was spicy");
    await bridge.completeSession(earlier.id);
    await bridge.logOut();

    // Onboarding again is Aarav again, under the name and age given now.
    await onboard("Aarav K", "15");
    expect(screen.getByText("1 day")).toBeInTheDocument();
    expect(sidebarAvatarColor()).toBe("#68B506");
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
    expect(screen.getByText("15 years old")).toBeInTheDocument();
    expect(statTile("TALKS DONE")).toBe("1");
    expect(statTile("SPOKEN")).toBe("—");
    expect((await bridge.bootstrap()).learner).toEqual({
      ...aarav,
      name: "Aarav K",
      age: 15,
      avatar_color: "#68B506",
    });
  });

  it("keeps the learner's avatar colour through log out and log in", async () => {
    await openProfile("Aarav");
    fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
    fireEvent.click(screen.getByRole("button", { name: "Green" }));
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(sidebarAvatarColor()).toBe("#68B506"));
    await waitFor(async () => expect((await bridge.bootstrap()).learner?.avatar_color).toBe("#68B506"));

    // The welcome back is drawn in it, and so is everything after.
    fireEvent.click(screen.getByRole("button", { name: "Log out" }));
    fireEvent.click(await screen.findByRole("button", { name: /log in/i }));
    expect(welcomeBackAvatarColor()).toBe("#68B506");
    fireEvent.click(screen.getByRole("button", { name: "Take me in" }));
    await screen.findByText("Namaste, Aarav!");
    expect(sidebarAvatarColor()).toBe("#68B506");
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit profile" }));
    expect(screen.getByRole("button", { name: "Green" })).toHaveAttribute("aria-pressed", "true");
  });

  it("moves a colour saved for the whole device onto the learner, once", async () => {
    await bridge.saveLearner("Aarav", 14);
    window.localStorage.setItem("ella-avatar-color", "#FF3181");

    render(<App />);
    await screen.findByText("Namaste, Aarav!");
    await waitFor(() => expect(sidebarAvatarColor()).toBe("#FF3181"));
    await waitFor(() => expect(window.localStorage.getItem("ella-avatar-color")).toBeNull());
    expect((await bridge.bootstrap()).learner?.avatar_color).toBe("#FF3181");

    // It is the learner's now, not the device's, so it comes back with them.
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Log out" }));
    fireEvent.click(await screen.findByRole("button", { name: /log in/i }));
    expect(welcomeBackAvatarColor()).toBe("#FF3181");
    fireEvent.click(screen.getByRole("button", { name: "Take me in" }));
    await screen.findByText("Namaste, Aarav!");
    expect(sidebarAvatarColor()).toBe("#FF3181");
  });

  it("leaves a colour the learner picked since, and forgets the device's", async () => {
    await bridge.saveLearner("Aarav", 14);
    await bridge.saveAvatarColor("#68B506");
    window.localStorage.setItem("ella-avatar-color", "#FF3181");

    render(<App />);
    await screen.findByText("Namaste, Aarav!");
    await waitFor(() => expect(window.localStorage.getItem("ella-avatar-color")).toBeNull());
    expect(sidebarAvatarColor()).toBe("#68B506");
    expect((await bridge.bootstrap()).learner?.avatar_color).toBe("#68B506");
  });

  it("shows the toast and the saved colour when the backend refuses a colour", async () => {
    await openProfile("Aarav");
    const refuse = vi
      .spyOn(bridge, "saveAvatarColor")
      .mockRejectedValueOnce(new Error("Ella could not save that colour."));
    try {
      fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
      fireEvent.click(screen.getByRole("button", { name: "Pink" }));
      // Straight away, with nothing covering the profile.
      expect(screen.getByRole("button", { name: "Pink" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.queryByText("One moment…")).not.toBeInTheDocument();

      expect(await screen.findByRole("alert")).toHaveTextContent("Ella could not save that colour.");
      expect(screen.getByRole("button", { name: "Purple" })).toHaveAttribute("aria-pressed", "true");
    } finally {
      refuse.mockRestore();
    }
  });

  it("goes back to the saved colour when two quick presses are both refused", async () => {
    await openProfile("Aarav");
    const refuse = vi.spyOn(bridge, "saveAvatarColor").mockRejectedValue(new Error("Ella could not save that colour."));
    try {
      fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
      fireEvent.click(screen.getByRole("button", { name: "Pink" }));
      fireEvent.click(screen.getByRole("button", { name: "Green" }));
      await screen.findByRole("alert");
      // Neither was saved, so neither stays: the avatar is the stored purple,
      // not the pink that was only ever shown.
      await waitFor(() =>
        expect(screen.getByRole("button", { name: "Purple" })).toHaveAttribute("aria-pressed", "true"),
      );
      expect((await bridge.bootstrap()).learner?.avatar_color ?? null).toBeNull();
    } finally {
      refuse.mockRestore();
    }
  });

  it("keeps a colour picked while the figures after a talk are still being re-read", async () => {
    await bridge.saveLearner("Meera", 12);
    render(<App />);
    await screen.findByText("Namaste, Meera!");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");

    // Hold the re-read that follows the end of the talk until after the
    // colour has been picked and saved.
    const read = bridge.bootstrap.bind(bridge);
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const reread = vi.spyOn(bridge, "bootstrap").mockImplementationOnce(async () => {
      const stale = await read();
      await held;
      return stale;
    });
    try {
      fireEvent.click(screen.getByText("End talk"));
      await waitFor(() => expect(reread).toHaveBeenCalled());
      fireEvent.click(await screen.findByRole("button", { name: "Done" }));
      fireEvent.click(await screen.findByRole("button", { name: /view profile/i }));
      fireEvent.click(await screen.findByRole("button", { name: "Edit profile" }));
      fireEvent.click(screen.getByRole("button", { name: "Pink" }));
      await waitFor(async () => expect((await read()).learner?.avatar_color).toBe("#FF3181"));

      await act(async () => {
        release();
        await held;
      });
      expect(screen.getByRole("button", { name: "Pink" })).toHaveAttribute("aria-pressed", "true");
    } finally {
      reread.mockRestore();
    }
  });
});

/** The figure on one of the profile's stat tiles. */
function statTile(label: string): string | null {
  return screen.getByText(label).previousElementSibling?.textContent ?? null;
}

/** The colour the sidebar draws the learner's blob in. */
function sidebarAvatarColor(): string {
  return avatarColorIn(".sidebar");
}

/** The colour the welcome-back step draws the learner's blob in. */
function welcomeBackAvatarColor(): string {
  return avatarColorIn('[data-screen="onboarding-welcome-back"]');
}

function avatarColorIn(container: string): string {
  const avatar = document.querySelector<HTMLElement>(`${container} .learner-avatar`);
  return avatar?.style.getPropertyValue("--avatar") ?? "";
}

/** Name and age, skip the mic check, and arrive at the placement chat. */
async function reachPlacement(name: string, age = "14") {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /let’s start/i }));
  fireEvent.change(screen.getByLabelText("What should Ella call you?"), { target: { value: name } });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.change(screen.getByLabelText(`And how old are you, ${name}?`), { target: { value: age } });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(await screen.findByRole("button", { name: /skip this check/i }));
  await screen.findByRole("button", { name: "Skip" });
}

/** Types answers into the talk on screen, each once the last has been taken. */
async function typeAnswers(count: number) {
  fireEvent.click(await screen.findByRole("button", { name: /type instead/i }));
  for (let answer = 1; answer <= count; answer += 1) {
    const field = await screen.findByLabelText("Your answer");
    await waitFor(() => expect(field).toBeEnabled());
    fireEvent.change(field, { target: { value: `I spent the morning with my family, part ${answer}` } });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
    await waitFor(() => expect(screen.queryByLabelText("Your answer")?.getAttribute("value") ?? "").toBe(""));
  }
}

describe("Ella levels", () => {
  beforeEach(() => {
    window.localStorage.clear();
    // Ella finishes each line at once, so what follows a chat's last turn
    // comes straight after it rather than after the playback watchdog.
    vi.spyOn(window.speechSynthesis, "speak").mockImplementation((utterance: SpeechSynthesisUtterance) => {
      setTimeout(() => utterance.onend?.call(utterance, new Event("end") as SpeechSynthesisEvent), 0);
    });
  });
  afterEach(() => vi.restoreAllMocks());

  it("finds the learner's level in a placement chat and lands home with it", async () => {
    await reachPlacement("Aarav");
    expect(screen.getByText("Your level")).toBeInTheDocument();
    expect(promptText()).toBe("So Aarav, tell me about your day so far!");

    // Without a model the chat runs to the shortest length allowed: five answers.
    await typeAnswers(5);
    expect(await screen.findByText("Finding My Voice")).toBeInTheDocument();
    expect(screen.getByText("That was lovely, Aarav!")).toBeInTheDocument();
    expect(screen.getByText(/Level 3 of 6 · Step 1/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /let’s go/i }));
    await screen.findByText("Namaste, Aarav!");
    const card = screen.getByRole("button", { name: "My level: Finding My Voice. See all levels" });
    expect(card).toHaveTextContent("Step 1 of 5");
    expect(card).toHaveTextContent("0% to level 4");
    expect((await bridge.bootstrap()).standing?.placed).toBe(true);
  });

  it("goes straight home when the placement chat is skipped, placing nobody", async () => {
    await reachPlacement("Riya");
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));
    await screen.findByText("Namaste, Riya!");
    expect((await bridge.bootstrap()).standing?.placed).toBe(false);
    // The skipped chat is closed, not left waiting as an unfinished talk.
    expect(screen.queryByText("Unfinished talk")).not.toBeInTheDocument();
  });

  it("lets the learner try again when the level could not be read", async () => {
    const real = bridge.assessSession.bind(bridge);
    const failing = vi
      .spyOn(bridge, "assessSession")
      .mockRejectedValueOnce(new Error("Ella could not work out your level just now. Try again in a moment."))
      .mockImplementation(real);
    try {
      await reachPlacement("Kabir");
      await typeAnswers(5);
      expect(await screen.findByRole("alert")).toHaveTextContent("could not work out your level");
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      expect(await screen.findByText("Finding My Voice")).toBeInTheDocument();
    } finally {
      failing.mockRestore();
    }
  });

  it("shows where a talk left the learner, and opens the level map from there", async () => {
    await onboard("Meera");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    await typeAnswers(1);
    fireEvent.click(screen.getByRole("button", { name: "End talk" }));

    const level = await screen.findByRole("button", { name: /See all levels/ });
    expect(level).toHaveTextContent("Level 3");
    expect(level).toHaveTextContent("Finding My Voice");
    expect(screen.queryByText("Step complete!")).not.toBeInTheDocument();

    fireEvent.click(level);
    expect(await screen.findByRole("heading", { name: "Levels and badges" })).toBeInTheDocument();
    const path = await screen.findByRole("list", { name: "Levels" });
    expect(path.querySelectorAll(".level-stop")).toHaveLength(6);
    expect(path.querySelector(".level-stop.is-current")).toHaveTextContent("You’re here · 0% to level 4");
    expect(path.querySelector(".level-stop.is-current")).toHaveTextContent("0 of 20 skills filled");
    const page = screen.getByRole("region", { name: "Level 3: Finding My Voice" });
    expect(page).toHaveTextContent("Fill every bar to reach level 4.");
    expect(page.querySelectorAll(".level-step")).toHaveLength(5);
    expect(page.querySelector(".level-step.is-yours")).toHaveTextContent("Your step");
    // What the bar's segments stand for, in words, for a screen reader.
    expect(page.querySelector(".level-step.is-yours")).toHaveTextContent(", 0 of 4 skills");
    expect(page.querySelector(".level-step.is-yours ul")?.children).toHaveLength(4);
    expect(page.querySelector(".level-step.is-yours ul")).toHaveTextContent("(still to show)");
    expect(page.querySelector(".level-step.is-yours .level-step__bar")?.children).toHaveLength(4);
    // The badges the design files at this level, still to earn.
    expect(page).toHaveTextContent("Badges at level 3");
    expect(page).toHaveTextContent("Bargainer");
    expect(page).toHaveTextContent("Deposit back");

    // Another level's page is a press away.
    fireEvent.click(screen.getByRole("button", { name: /Pre-Beginner/ }));
    expect(screen.getByRole("region", { name: "Level 1: Pre-Beginner" })).toHaveTextContent("You’ve already passed this level.");
  });

  it("celebrates a talk that finished a step, and one that finished a level", async () => {
    const standing = {
      level_number: 3,
      level_count: 6,
      level_name: "Finding My Voice",
      step: 2,
      step_count: 5,
      step_title: "Next step",
      percent: 25,
      placed: true,
    };
    const assess = vi.spyOn(bridge, "assessSession").mockImplementation(async (sessionId) => ({
      session_id: sessionId,
      kind: "talk",
      standing,
      advanced: "step",
      skills: [{ label: "Past time words", count: 2 }],
      scored: true,
    }));
    try {
      await onboard("Ira");
      fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
      await screen.findByText("End talk");
      await typeAnswers(1);
      fireEvent.click(screen.getByRole("button", { name: "End talk" }));
      expect(await screen.findByText("Step complete!")).toBeInTheDocument();
      expect(screen.getByText("On to Step 2 of Finding My Voice.")).toBeInTheDocument();
      expect(screen.getByLabelText("Past time words: 2 talks")).toHaveTextContent("Past time words");

      assess.mockImplementation(async (sessionId) => ({
        session_id: sessionId,
        kind: "talk",
        standing: { ...standing, level_number: 4, level_name: "Speaking Freely", step: 1, percent: 0 },
        advanced: "level",
        skills: [],
        scored: true,
      }));
      fireEvent.click(screen.getByRole("button", { name: "Done" }));
      fireEvent.click(await screen.findByRole("button", { name: /start talking/i }));
      await screen.findByText("End talk");
      await typeAnswers(1);
      fireEvent.click(screen.getByRole("button", { name: "End talk" }));
      expect(await screen.findByText("Level up!")).toBeInTheDocument();
      expect(screen.getByText("You’ve reached Speaking Freely.")).toBeInTheDocument();
    } finally {
      assess.mockRestore();
    }
  });

  it("still celebrates a finished step when the learner left the summary before it was known", async () => {
    let answer: ((assessment: Assessment) => void) | undefined;
    const assess = vi.spyOn(bridge, "assessSession").mockImplementation(
      () => new Promise<Assessment>((resolve) => (answer = resolve)),
    );
    try {
      await onboard("Tara");
      fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
      await screen.findByText("End talk");
      await typeAnswers(3);
      fireEvent.click(screen.getByRole("button", { name: "End talk" }));
      expect(await screen.findAllByRole("status", { name: "Ella is looking back over your talk…" })).not.toHaveLength(0);
      fireEvent.click(screen.getByRole("button", { name: "Done" }));
      await screen.findByText("Namaste, Tara!");

      const sessionId = assess.mock.calls[0][0];
      act(() =>
        answer?.({
          session_id: sessionId,
          kind: "talk",
          standing: {
            level_number: 3,
            level_count: 6,
            level_name: "Finding My Voice",
            step: 2,
            step_count: 5,
            step_title: "Next step",
            percent: 25,
            placed: true,
          },
          advanced: "step",
          skills: [],
          scored: true,
        }),
      );
      expect(await screen.findByText("Step complete!")).toBeInTheDocument();
      expect(screen.getByText("On to Step 2 of Finding My Voice.")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "See levels" }));
      expect(await screen.findByRole("heading", { name: "Levels and badges" })).toBeInTheDocument();
      expect(screen.queryByText("Step complete!")).not.toBeInTheDocument();
    } finally {
      assess.mockRestore();
    }
  });

  it("shows a placement picked up again from Home as one, and places nobody when it is ended early", async () => {
    await bridge.saveLearner("Isha", 13);
    const open = await bridge.startPlacement();
    await bridge.sendTextTurn(open.id, "I like cricket");

    render(<App />);
    await screen.findByText("Namaste, Isha!");
    fireEvent.click(screen.getByRole("button", { name: /continue talking/i }));
    await screen.findByText("Your level");
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));

    const level = await screen.findByRole("button", { name: /See all levels/ });
    expect(level).toHaveTextContent("Level 3");
    expect(level).not.toHaveTextContent("Your level");
    expect((await bridge.bootstrap()).standing?.placed).toBe(false);
  });

  it("catches up with a level read after the learner skipped the wait for it", async () => {
    const real = bridge.assessSession.bind(bridge);
    let release: (() => void) | undefined;
    const held = vi.spyOn(bridge, "assessSession").mockImplementation(
      (sessionId) => new Promise((resolve) => (release = () => resolve(real(sessionId)))),
    );
    try {
      await reachPlacement("Om");
      await typeAnswers(5);
      expect(await screen.findByText("Ella is finding your level…")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Skip" }));
      await screen.findByText("Namaste, Om!");
      fireEvent.click(screen.getByRole("button", { name: /my level: finding my voice/i }));
      expect(await screen.findByRole("button", { name: "Not sure? Find my level" })).toBeInTheDocument();

      // The reading lands: the app reads where Om stands again, and the map
      // stops offering a placement he has now had.
      await act(async () => release?.());
      await waitFor(() =>
        expect(screen.queryByRole("button", { name: "Not sure? Find my level" })).not.toBeInTheDocument(),
      );
    } finally {
      held.mockRestore();
    }
  });

  it("does not celebrate on the next sign-in for a talk assessed after a log out", async () => {
    let answer: ((assessment: Assessment) => void) | undefined;
    const assess = vi.spyOn(bridge, "assessSession").mockImplementation(
      () => new Promise<Assessment>((resolve) => (answer = resolve)),
    );
    try {
      await onboard("Zara");
      fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
      await screen.findByText("End talk");
      await typeAnswers(3);
      fireEvent.click(screen.getByRole("button", { name: "End talk" }));
      await screen.findAllByRole("status", { name: "Ella is looking back over your talk…" });
      fireEvent.click(screen.getByRole("button", { name: "Done" }));
      fireEvent.click(await screen.findByRole("button", { name: /view profile/i }));
      fireEvent.click(await screen.findByRole("button", { name: "Log out" }));
      fireEvent.click(await screen.findByRole("button", { name: /log in/i }));
      fireEvent.click(await screen.findByRole("button", { name: "Take me in" }));
      await screen.findByText("Namaste, Zara!");

      act(() =>
        answer?.({
          session_id: assess.mock.calls[0][0],
          kind: "talk",
          standing: {
            level_number: 3,
            level_count: 6,
            level_name: "Finding My Voice",
            step: 2,
            step_count: 5,
            step_title: "Next step",
            percent: 25,
            placed: true,
          },
          advanced: "step",
          skills: [],
          scored: true,
        }),
      );
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(screen.queryByText("Step complete!")).not.toBeInTheDocument();
    } finally {
      assess.mockRestore();
    }
  });

  it("offers a learner who never had a placement the chat from the level map", async () => {
    await onboard("Dev");
    fireEvent.click(screen.getByRole("button", { name: /my level: finding my voice/i }));
    await screen.findByRole("heading", { name: "Levels and badges" });
    fireEvent.click(await screen.findByRole("button", { name: "Not sure? Find my level" }));

    // The chat takes the whole window, as a talk does.
    await screen.findByRole("button", { name: "Skip" });
    expect(document.querySelector("aside.sidebar")).not.toBeInTheDocument();
    expect(promptText()).toBe("So Dev, tell me about your day so far!");
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));
    expect(await screen.findByRole("heading", { name: "Levels and badges" })).toBeInTheDocument();

    fireEvent.click(await screen.findByRole("button", { name: "Not sure? Find my level" }));
    await typeAnswers(5);
    fireEvent.click(await screen.findByRole("button", { name: /let’s go/i }));
    await screen.findByText("Namaste, Dev!");
    fireEvent.click(screen.getByRole("button", { name: /my level: finding my voice/i }));
    await screen.findByRole("list", { name: "Levels" });
    // Placed now, so there is nothing more to find.
    expect(screen.queryByRole("button", { name: "Not sure? Find my level" })).not.toBeInTheDocument();
  });

  it("shows the learner's level on their profile", async () => {
    await onboard("Neha");
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
    fireEvent.click(screen.getByRole("button", { name: /my level: finding my voice/i }));
    expect(await screen.findByRole("heading", { name: "Levels and badges" })).toBeInTheDocument();
  });
});

describe("Ella badges", () => {
  beforeEach(() => window.localStorage.clear());

  async function openProfile(name: string, age = "14") {
    await onboard(name, age);
    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
  }

  it("lists them on the profile, and opens one's sheet over the window", async () => {
    await openProfile("Aarav");
    // The sidebar lights the learner's own row while they are on it.
    expect(screen.getByRole("button", { name: /view profile/i })).toHaveAttribute("aria-current", "page");
    const card = screen.getByRole("region", { name: "Badges" });
    expect(card).toHaveTextContent("0 of 8");
    expect(card).toHaveTextContent("Finish a talk with Ella to earn your first badge.");

    // Nothing tried yet, so what comes next is where the catalogue starts.
    const next = within(card).getAllByRole("button").map((button) => button.textContent);
    expect(next.slice(1)).toEqual([
      "Hello, EllaElla · Placement talkNot tried",
      "First talkHome · Today’s talkNot tried",
      "BargainerBippo · Talk a stall price downNot tried",
    ]);
    // Opened from the keyboard, focus comes back to the badge when it closes.
    const opener = within(card).getByRole("button", { name: /Hello, Ella/ });
    opener.focus();
    fireEvent.click(opener);
    const sheet = screen.getByRole("dialog", { name: "Hello, Ella" });
    expect(sheet).toHaveTextContent("Not tried yet");
    expect(sheet).toHaveTextContent("Say hi to Ella in your placement talk.");
    expect(sheet).toHaveTextContent("WHERE TO EARN IT");
    expect(within(sheet).getByRole("button", { name: "Close" })).toHaveFocus();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(opener).toHaveFocus();
  });

  it("keeps Tab inside an open sheet", async () => {
    await openProfile("Aarav");
    fireEvent.click(within(screen.getByRole("region", { name: "Badges" })).getByRole("button", { name: /First talk/ }));
    const sheet = screen.getByRole("dialog", { name: "First talk" });
    const close = within(sheet).getByRole("button", { name: "Close" });
    const mic = within(sheet).getByRole("button", { name: "Start this talk" });
    expect(close).toHaveFocus();
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(mic).toHaveFocus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(close).toHaveFocus();
  });

  it("brings a placement chat started from Hello, Ella's sheet back to the profile when it is skipped", async () => {
    await openProfile("Aarav");
    fireEvent.click(within(screen.getByRole("region", { name: "Badges" })).getByRole("button", { name: /Hello, Ella/ }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "Hello, Ella" })).getByRole("button", { name: "Start this talk" }));
    fireEvent.click(await screen.findByRole("button", { name: "Skip" }));
    expect(await screen.findByText("My profile")).toBeInTheDocument();
  });

  it("earns First talk with a finished talk, dated the day it ended", async () => {
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));
    await screen.findByText("End talk");
    await typeAnswers(1);
    fireEvent.click(screen.getByRole("button", { name: "End talk" }));
    fireEvent.click(await screen.findByRole("button", { name: "Done" }));
    fireEvent.click(await screen.findByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");

    const card = screen.getByRole("region", { name: "Badges" });
    // A talk a day makes a day's streak, and three days make a badge.
    expect(card).toHaveTextContent("1 of 8");
    fireEvent.click(within(card).getByRole("button", { name: "First talk" }));
    const sheet = screen.getByRole("dialog", { name: "First talk" });
    expect(sheet).toHaveTextContent("Earned today");
    expect(sheet).toHaveTextContent("WHERE YOU EARNED IT");
    // An earned badge that is not a partner's has nothing more to start.
    expect(within(sheet).queryByRole("button", { name: "Start this talk" })).not.toBeInTheDocument();
    fireEvent.click(within(sheet).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("opens what earns a badge from its sheet: a partner's scenes, or a talk", async () => {
    await openProfile("Aarav");
    fireEvent.click(screen.getByRole("button", { name: "Where to earn more" }));
    const page = await screen.findByRole("region", { name: "Level 3: Finding My Voice" });
    fireEvent.click(within(page).getByRole("button", { name: /Bargainer/ }));
    const sheet = screen.getByRole("dialog", { name: "Bargainer" });
    expect(sheet).toHaveTextContent("Talk a stall price down · 5 min");
    fireEvent.click(within(sheet).getByRole("button", { name: "Talk to Bippo" }));
    expect(await screen.findByText("Your mentor")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /view profile/i }));
    await screen.findByText("My profile");
    fireEvent.click(within(screen.getByRole("region", { name: "Badges" })).getByRole("button", { name: /First talk/ }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "First talk" })).getByRole("button", { name: "Start this talk" }));
    expect(await screen.findByText("End talk")).toBeInTheDocument();
  });

  it("files the rest under their levels and the anytime badges, and goes back to the profile", async () => {
    await openProfile("Meera", "12");
    fireEvent.click(screen.getByRole("button", { name: /my level: finding my voice/i }));
    await screen.findByRole("heading", { name: "Levels and badges" });
    expect(screen.getByRole("button", { name: /view profile/i })).toHaveAttribute("aria-current", "page");

    // Too young for Grumble's deposit, so its badge is not offered either.
    const page = screen.getByRole("region", { name: "Level 3: Finding My Voice" });
    expect(page).toHaveTextContent("0 of 1 earned");
    expect(page).not.toHaveTextContent("Deposit back");

    fireEvent.click(screen.getByRole("button", { name: /Anytime badges/ }));
    const anytime = screen.getByRole("region", { name: "Anytime badges" });
    expect(anytime).toHaveTextContent("0 of 4 earned");
    expect(within(anytime).getAllByRole("button").map((button) => button.textContent)).toEqual([
      "3-day streakAny talk, every day0 / 3",
      "7-day streakAny talk, every day0 / 7",
      "30-day streakAny talk, every day0 / 30",
      "50 talksAny talk counts0 / 50",
    ]);

    fireEvent.click(screen.getByRole("button", { name: /Pre-Beginner/ }));
    const first = screen.getByRole("region", { name: "Level 1: Pre-Beginner" });
    expect(first).toHaveTextContent("Hello, Ella");
    expect(first).toHaveTextContent("First talk");
    fireEvent.click(screen.getByRole("button", { name: /Speaking Freely/ }));
    expect(screen.getByRole("region", { name: "Level 4: Speaking Freely" })).toHaveTextContent(
      "No badges at this level yet.",
    );

    fireEvent.click(screen.getByRole("button", { name: "Back to profile" }));
    expect(await screen.findByText("My profile")).toBeInTheDocument();
  });
});
