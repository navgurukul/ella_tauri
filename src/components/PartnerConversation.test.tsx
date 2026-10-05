import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PartnerFigure } from "./CastScreen";
import { TalkScreen } from "./TalkScreen";
import { bridge } from "../lib/bridge";
import * as speech from "../lib/speech";
import type { CastId, TurnResult } from "../types";

const CAST: CastId[] = ["stall-owner", "landlord", "doctor", "debater"];
const lips = () => document.querySelector(".ella__lips")?.getAttribute("d");

afterEach(() => {
  vi.restoreAllMocks();
});

describe("talk partner conversation states", () => {
  it.each(CAST)("holds %s's face in a still pose for each state, its eyes anchored", (id) => {
    const frames = vi.spyOn(window, "requestAnimationFrame");
    const view = render(<PartnerFigure id={id} variant="conversation" state="resting" />);
    const pupil = view.container.querySelector(".figure__pupil")!;
    const eye = view.container.querySelector(".figure__eye-aperture")!;
    const outline = eye.querySelector("path")!.getAttribute("d");
    const body = view.container.querySelector<HTMLElement>(".figure__motion")!;
    expect(pupil.parentElement).toHaveAttribute("clip-path");
    // The pointer is not followed.
    fireEvent(window, new MouseEvent("pointermove", { clientX: 400, clientY: 200 }));
    expect(pupil).toHaveAttribute("transform", "translate(0 0)");
    expect(body.style.transform).toBe("translateY(0px) rotate(0deg)");

    view.rerender(<PartnerFigure id={id} variant="conversation" state="listening" />);
    expect(body.style.transform).toBe("translateY(-3px) rotate(0.55deg)");
    expect(eye).toHaveAttribute("transform", "scale(1 1.09)");
    expect(eye.querySelector("path")!.getAttribute("d")).toBe(outline);

    view.rerender(<PartnerFigure id={id} variant="conversation" state="thinking" />);
    expect(pupil).toHaveAttribute("transform", "translate(-2.6 -3.2)");
    expect(view.container.querySelector(".figure__eye-aperture")).toBe(eye);
    expect(body.style.transform).not.toContain("scale");
    expect(frames).not.toHaveBeenCalled();
  });

  it.each(CAST)("opens %s's mouth while they talk, and closes it into their smile after", (id) => {
    const view = render(<PartnerFigure id={id} variant="conversation" state="resting" />);
    const resting = lips();
    const drawing = view.container.querySelector(".ella__lips");
    view.rerender(<PartnerFigure id={id} variant="conversation" state="speaking" />);
    expect(lips()).not.toBe(resting);
    view.rerender(<PartnerFigure id={id} variant="conversation" state="listening" />);
    expect(lips()).toBe(resting);
    expect(view.container.querySelector(".ella__lips")).toBe(drawing);
    expect(view.container.querySelector(".figure")).toHaveClass("figure--listening");
  });

  it("follows the microphone and reply through speaking, listening, thinking and resting", async () => {
    window.localStorage.clear();
    await bridge.saveLearner("Meera", 15);
    const session = await bridge.startChore("deposit-refund");
    const reply = await bridge.sendTextTurn(session.id, "Please return my deposit.");
    const capture: speech.VoiceCapture = {
      supported: true,
      start: vi.fn().mockResolvedValue(undefined),
      stop: vi.fn().mockResolvedValue({ samples: [1], streamTailSamples: [1], sampleRate: 16000, transcript: "Please return my deposit." }),
      cancel: vi.fn().mockResolvedValue(undefined),
    };
    vi.spyOn(speech, "createVoiceCapture").mockReturnValue(capture);
    let utterance: SpeechSynthesisUtterance | undefined;
    vi.spyOn(window.speechSynthesis, "speak").mockImplementation((next) => { utterance = next; });
    let answer!: (result: TurnResult) => void;
    vi.spyOn(bridge, "sendVoiceTurn").mockImplementation(() => new Promise((resolve) => { answer = resolve; }));

    const view = render(<TalkScreen session={session} partner="landlord" onSessionChange={vi.fn()} onComplete={vi.fn()} />);
    const figure = () => view.container.querySelector(".figure--conversation");
    expect(figure()).toHaveClass("figure--speaking");
    fireEvent.click(screen.getByRole("button", { name: "Interrupt Grumble and start speaking" }));
    await waitFor(() => expect(figure()).toHaveClass("figure--listening"));
    expect(capture.start).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole("button", { name: "Stop and send" }));
    await waitFor(() => expect(bridge.sendVoiceTurn).toHaveBeenCalledOnce());
    expect(figure()).toHaveClass("figure--thinking");
    await act(async () => answer(reply));
    expect(figure()).toHaveClass("figure--speaking");
    act(() => utterance?.onend?.({} as SpeechSynthesisEvent));
    expect(figure()).toHaveClass("figure--resting");
  });
});
