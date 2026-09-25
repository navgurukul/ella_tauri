import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { bridge } from "./lib/bridge";
import type { EllaBridge, SpeechSegment, TurnResult } from "./types";

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
   * The reply is released whole: its sentences are synthesized while the model
   * writes but held back until all of them are ready. Nothing of it may reach
   * the screen before then — a reply that arrives in pieces cannot be centred
   * without the words already being read jumping as each piece lands.
   */
  it("shows the whole reply at once when the turn returns, and nothing before", async () => {
    // The opening streams, so the subscription exists; a reply must not use it.
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
    let release: ((result: TurnResult) => void) | undefined;
    let turnSession = "";
    const real = bridge.sendTextTurn.bind(bridge);
    const held = vi
      .spyOn(bridge, "sendTextTurn")
      .mockImplementation(async (sessionId: string, text: string) => {
        turnSession = sessionId;
        const result = await real(sessionId, text);
        return new Promise<TurnResult>((resolve) => {
          release = () => resolve(result);
        });
      });

    try {
      const opening = promptText();
      fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
      fireEvent.change(screen.getByLabelText("Your answer"), {
        target: { value: "I ate vada pav near the station" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
      await waitFor(() => expect(held.mock.calls).toHaveLength(1));

      // A stray segment mid-turn must not put anything on screen: no queue is
      // open for a reply, so there is nothing for it to feed.
      emit?.({
        session_id: turnSession,
        turn: 1,
        index: 0,
        text: "That sounds delicious!",
        audio: { mime_type: "audio/wav", base64: "AAAA" },
        ready_ms: 900,
        words: [{ text: "That", start_ms: 0, end_ms: 300 }],
      });
      expect(promptText()).toBe(opening);

      release?.({} as TurnResult);
      await waitFor(() => expect(promptText()).toMatch(/that sounds delicious/i));

      // Whole reply, in one piece, with every word its own element so one of
      // them can be marked as spoken.
      const shown = promptText();
      expect(shown).toMatch(/who would you like to share that with/i);
      expect(document.querySelectorAll(".talk-prompt .talk-word")).toHaveLength(
        shown.split(/\s+/).filter(Boolean).length,
      );
      // Real spaces, not CSS-generated ones, so the reply stays copyable.
      expect(shown).toContain("That sounds delicious!");
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

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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
      fireEvent.click(screen.getByRole("button", { name: "Edit" }));
      fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Aarav K" } });
      fireEvent.click(screen.getByRole("button", { name: "Done" }));

      expect(await screen.findByRole("alert")).toHaveTextContent("Ella could not save that just now.");
      expect(screen.getByLabelText("Your name")).toHaveValue("Aarav K");
    } finally {
      refuse.mockRestore();
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
    fireEvent.click(await screen.findByRole("button", { name: "Back home" }));
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
    expect(statTile("ANSWERS")).toBe("1");
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
    expect(statTile("ANSWERS")).toBe("2");
    expect((await bridge.bootstrap()).learner).toEqual({
      ...aarav,
      name: "Aarav K",
      age: 15,
      avatar_color: "#68B506",
    });
  });

  it("keeps the learner's avatar colour through log out and log in", async () => {
    await openProfile("Aarav");
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
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
      fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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
      fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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
      fireEvent.click(await screen.findByRole("button", { name: /view profile/i }));
      fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
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
