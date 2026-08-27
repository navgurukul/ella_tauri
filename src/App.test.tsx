import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { bridge } from "./lib/bridge";

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
  fireEvent.click(await screen.findByRole("button", { name: /skip for now/i }));
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
});

describe("Ella learner flow", () => {
  beforeEach(() => window.localStorage.clear());

  it("moves from home to a real conversation with typed fallback", async () => {
    await onboard("Aarav");
    fireEvent.click(screen.getByRole("button", { name: /start talking/i }));

    await screen.findByText("End talk");
    expect(promptText()).toMatch(/tastiest thing you ate/i);
    const avatar = document.querySelector(".ella--talk");
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

    await waitFor(() => {
      expect(screen.getByText(/I ate vada pav near the station/)).toBeInTheDocument();
      expect(promptText()).toMatch(/who would you like to share that with/i);
    });
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

  it("shows the garden with a unit per skill strand", async () => {
    await onboard("Kabir");
    fireEvent.click(within(screen.getByRole("navigation")).getByRole("button", { name: "Garden" }));

    expect(await screen.findByText("Your garden")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Unit 1: Hello & introductions/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Unit 5: Plans & weekend \(locked\)/ })).toBeDisabled();
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
    // Resumed through `get_session`, so the earlier messages come back with it.
    expect(screen.getByText(/I ate poha this morning/)).toBeInTheDocument();
  });

  it("starts the topic a garden unit stands for", async () => {
    await onboard("Kabir");
    fireEvent.click(within(screen.getByRole("navigation")).getByRole("button", { name: "Garden" }));

    fireEvent.click(await screen.findByRole("button", { name: /Unit 2: Food & ordering/ }));

    await screen.findByText("End talk");
    expect(promptText()).toMatch(/restaurant/i);
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
