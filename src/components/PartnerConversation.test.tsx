import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PartnerFigure } from "./CastScreen";
import { TalkScreen } from "./TalkScreen";
import { bridge } from "../lib/bridge";
import * as speech from "../lib/speech";
import { mouthGeometry } from "../lib/mouth";
import { VISEMES } from "../lib/visemes";
import type { CastId, TurnResult } from "../types";

const CAST: CastId[] = ["stall-owner", "landlord", "doctor", "debater"];
const lips = () => document.querySelector(".ella__lips")?.getAttribute("d");

async function frames(count: number) {
  for (let i = 0; i < count; i += 1) {
    await act(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
  }
}

/** Advance the rig's real animation callbacks without waiting on wall time. */
function animationClock() {
  let time = 0;
  let serial = 0;
  const pending = new Map<number, FrameRequestCallback>();
  vi.spyOn(performance, "now").mockImplementation(() => time);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    pending.set(++serial, callback);
    return serial;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => { pending.delete(id); });
  return {
    advance(ms: number) {
      const count = Math.ceil(ms / 16);
      for (let i = 0; i < count; i += 1) {
        time += ms / count;
        const callbacks = [...pending.values()];
        pending.clear();
        act(() => callbacks.forEach((callback) => callback(time)));
      }
    },
    pending: () => pending.size,
  };
}

function translation(element: Element) {
  return (element.getAttribute("transform")?.match(/-?\d+\.?\d*/g) ?? []).map(Number);
}

afterEach(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(window, "matchMedia");
});

describe("talk partner conversation states", () => {
  it.each(CAST)("keeps %s's eyes anchored while its pupils and expression respond", (id) => {
    const clock = animationClock();
    const view = render(<PartnerFigure id={id} variant="conversation" state="resting" />);
    const pupil = view.container.querySelector(".figure__pupil")!;
    const eye = view.container.querySelector(".figure__eye-aperture")!;
    const outline = eye.querySelector("path")!.getAttribute("d");
    const body = view.container.querySelector<HTMLElement>(".figure__motion")!;
    fireEvent(window, new MouseEvent("pointermove", { clientX: 400, clientY: 200 }));
    clock.advance(500);
    expect(translation(pupil)[0]).toBeGreaterThan(2);
    expect(pupil.parentElement).toHaveAttribute("clip-path");
    const before = body.style.transform;

    view.rerender(<PartnerFigure id={id} variant="conversation" state="listening" />);
    expect(body.style.transform).toBe(before); // State changes settle instead of snapping.
    clock.advance(700);
    expect(Math.abs(translation(pupil)[0])).toBeLessThan(0.01);
    expect(Math.abs(translation(pupil)[1])).toBeLessThan(0.01);
    expect(eye.querySelector("path")!.getAttribute("d")).toBe(outline);

    view.rerender(<PartnerFigure id={id} variant="conversation" state="thinking" />);
    clock.advance(700);
    expect(translation(pupil)[0]).toBeLessThan(-2);
    expect(translation(pupil)[1]).toBeLessThan(-3);
    expect(view.container.querySelector(".figure__eye-aperture")).toBe(eye);
    expect(body.style.transform).not.toContain("scale");
    view.unmount();
    expect(clock.pending()).toBe(0);
  });

  it.each(CAST)("moves %s's mouth with the audio and stops when interrupted", async (id) => {
    let shape = VISEMES.aa;
    let started = true;
    const mouth: speech.SpeechMouth = { timed: true, get started() { return started; }, shape: () => shape };
    const view = render(<PartnerFigure id={id} variant="conversation" state="speaking" speech={mouth} />);
    const restingMouth = lips();
    const drawing = view.container.querySelector(".ella__lips");
    await frames(8);
    expect(lips()).toBe(mouthGeometry(VISEMES.aa, 46, 3.5).path);
    shape = VISEMES.oo;
    await frames(2);
    expect(lips()).toBe(mouthGeometry(VISEMES.oo, 46, 3.5).path);

    started = false;
    view.rerender(<PartnerFigure id={id} variant="conversation" state="listening" speech={mouth} />);
    await frames(14);
    expect(lips()).toBe(restingMouth);
    expect(view.container.querySelector(".ella__lips")).toBe(drawing);
    expect(view.container.querySelector(".figure")).toHaveClass("figure--listening");
  });

  it("uses a fallback mouth for untimed audio and resets after speaking", async () => {
    const mouth: speech.SpeechMouth = { timed: false, started: true, shape: () => VISEMES.rest };
    const view = render(<PartnerFigure id="doctor" variant="conversation" state="speaking" speech={mouth} />);
    const resting = lips();
    await frames(8);
    expect(view.container.querySelector(".figure")).toHaveClass("is-untimed");
    expect(lips()).not.toBe(resting);
    view.rerender(<PartnerFigure id="doctor" variant="conversation" state="resting" speech={mouth} />);
    await frames(14);
    expect(view.container.querySelector(".figure")).not.toHaveClass("is-untimed");
    expect(lips()).toBe(resting);
  });

  it("keeps the listening expression with reduced motion", () => {
    Object.defineProperty(window, "matchMedia", { configurable: true, value: () => ({ matches: true }) });
    const view = render(<PartnerFigure id="doctor" variant="conversation" state="listening" />);
    expect(view.container.querySelector<HTMLElement>(".figure__motion")?.style.transform)
      .toBe("translateY(-3px) rotate(0.55deg)");
    expect(view.container.querySelector(".figure__eye-aperture")).toHaveAttribute("transform", "scale(1 1.09)");
    view.rerender(<PartnerFigure id="doctor" variant="conversation" state="thinking" />);
    expect(view.container.querySelector(".figure__pupil")).toHaveAttribute("transform", "translate(-2.6 -3.2)");
  });

  it("uses slow, small breaths while speaking, with no whole-body stretching", () => {
    const clock = animationClock();
    const view = render(<PartnerFigure id="stall-owner" variant="conversation" state="speaking" />);
    const body = view.container.querySelector<HTMLElement>(".figure__motion")!;
    const positions: number[] = [];
    for (let i = 0; i < 360; i += 1) {
      clock.advance(16);
      expect(body.style.transform).not.toContain("scale");
      positions.push(Number(body.style.transform.match(/translateY\(([-.\d]+)px\)/)![1]));
    }
    expect(Math.max(...positions) - Math.min(...positions)).toBeLessThan(2.3);
    expect(Math.max(...positions.slice(1).map((y, i) => Math.abs(y - positions[i])))).toBeLessThan(0.05);
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
