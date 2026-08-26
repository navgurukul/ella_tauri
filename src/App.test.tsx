import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import App from "./App";

describe("ZoSpeak learner flow", () => {
  beforeEach(() => window.localStorage.clear());

  it("moves from onboarding to a real conversation with typed fallback", async () => {
    render(<App />);
    expect(await screen.findByText("Speak English.")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("What should Zoe call you?"), {
      target: { value: "Asha" },
    });
    fireEvent.click(screen.getByRole("button", { name: /meet zoe/i }));

    expect(await screen.findByText("Ready to talk, Asha?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /talk about school life/i }));

    expect(await screen.findByText(/tell me about a school day/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /type instead/i }));
    fireEvent.change(screen.getByPlaceholderText(/type what you want to say/i), {
      target: { value: "I played football after school" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));

    await waitFor(() => {
      expect(screen.getByText("I played football after school")).toBeInTheDocument();
      expect(screen.getByText(/what happened next/i)).toBeInTheDocument();
    });
  });
});
