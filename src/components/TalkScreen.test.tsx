import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TalkScreen } from "./TalkScreen";
import { bridge } from "../lib/bridge";
import * as speech from "../lib/speech";
import type { SessionSummary } from "../types";

function fakeMic(supported = true): speech.VoiceCapture {
  const capture: speech.VoiceCapture = {
    supported,
    start: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue({
      samples: [1],
      streamTailSamples: [1],
      sampleRate: 16000,
      transcript: "Two samosas, please.",
    }),
    cancel: vi.fn().mockResolvedValue(undefined),
  };
  vi.spyOn(speech, "createVoiceCapture").mockReturnValue(capture);
  return capture;
}

/** Every line Ella starts, in order. The browser bridge has no Piper, so each
 * is said by the system voice, and the test decides when it ends. */
function linesSaid(): SpeechSynthesisUtterance[] {
  const said: SpeechSynthesisUtterance[] = [];
  vi.spyOn(window.speechSynthesis, "speak").mockImplementation((utterance) => {
    said.push(utterance);
  });
  return said;
}

async function openTalk(onComplete = vi.fn()) {
  const session = await bridge.startSession("street-food");
  render(<TalkScreen session={session} onSessionChange={vi.fn()} onComplete={onComplete} />);
  return session;
}

const finish = (line: SpeechSynthesisUtterance) => act(() => line.onend?.({} as SpeechSynthesisEvent));

/** The learner says something: the mic's level rises for a moment. */
function speakInto(mic: speech.VoiceCapture) {
  const onLevel = vi.mocked(mic.start).mock.calls.at(-1)?.[1];
  for (let block = 0; block < 10; block += 1) onLevel?.(0.4);
}

beforeEach(async () => {
  window.localStorage.clear();
  await bridge.saveLearner("Meera", 15);
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("the mic after Ella speaks", () => {
  it("opens by itself once Ella has finished her line", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    expect(said).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Interrupt Ella and start speaking" })).toBeInTheDocument();
    expect(mic.start).not.toHaveBeenCalled();

    finish(said[0]);
    expect(await screen.findByRole("button", { name: "Stop and send" })).toBeInTheDocument();
    expect(mic.start).toHaveBeenCalledOnce();
  });

  it("stays closed when her line could not be played", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();

    act(() => said[0].onerror?.({} as SpeechSynthesisErrorEvent));
    expect(await screen.findByRole("alert")).toHaveTextContent(/could not play this aloud/i);
    expect(screen.getByRole("button", { name: "Start speaking" })).toBeInTheDocument();
    expect(mic.start).not.toHaveBeenCalled();
  });

  it("stays closed where there is no microphone, rather than switching to typing", async () => {
    const mic = fakeMic(false);
    const said = linesSaid();
    await openTalk();

    finish(said[0]);
    expect(screen.getByRole("button", { name: "Start speaking" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Your answer")).not.toBeInTheDocument();
    expect(mic.start).not.toHaveBeenCalled();
  });

  it("stays closed for a learner who is typing", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    finish(said[0]);
    await screen.findByRole("button", { name: "Stop and send" });

    fireEvent.click(screen.getByRole("button", { name: "Type instead" }));
    await waitFor(() => expect(mic.cancel).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("Your answer"), { target: { value: "Two samosas, please." } });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));
    await waitFor(() => expect(said).toHaveLength(2));

    finish(said[1]);
    expect(screen.getByLabelText("Your answer")).toBeInTheDocument();
    expect(mic.start).toHaveBeenCalledOnce();
  });

  it("stays closed after the line that closes the talk, which goes on to the summary", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    const onComplete = vi.fn();
    const session = await openTalk(onComplete);
    const reply = await bridge.sendTextTurn(session.id, "Two samosas, please.");
    const summary: SessionSummary = {
      session_id: session.id,
      topic_label: session.topic_label,
      turns: 1,
      headline: "Ordered street food",
      encouragement: "Well done.",
      short: true,
    };
    vi.spyOn(bridge, "sendVoiceTurn").mockResolvedValue({ ...reply, session_summary: summary });
    finish(said[0]);
    await screen.findByRole("button", { name: "Stop and send" });
    speakInto(mic);

    fireEvent.click(screen.getByRole("button", { name: "Stop and send" }));
    await waitFor(() => expect(said).toHaveLength(2));
    expect(onComplete).not.toHaveBeenCalled();

    finish(said[1]);
    expect(onComplete).toHaveBeenCalledWith(summary);
    expect(mic.start).toHaveBeenCalledOnce();
  });

  it("puts down an open answer to hear the line again, then opens once more", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    finish(said[0]);
    await screen.findByRole("button", { name: "Stop and send" });

    fireEvent.click(screen.getByRole("button", { name: "Hear it again" }));
    expect(said).toHaveLength(2);
    expect(said[1].text).toBe(said[0].text);
    expect(screen.getByRole("button", { name: "Interrupt Ella and start speaking" })).toBeInTheDocument();
    await waitFor(() => expect(mic.cancel).toHaveBeenCalled());
    expect(mic.stop).not.toHaveBeenCalled();

    finish(said[1]);
    expect(await screen.findByRole("button", { name: "Stop and send" })).toBeInTheDocument();
    expect(mic.start).toHaveBeenCalledTimes(2);
  });
});

describe("pressing the mic once it has opened by itself", () => {
  // A learner used to pressing to answer presses as it opens: on the Windows
  // test laptop that sent a fraction of a second of silence, and Ella said she
  // could not hear them, 9 times in 58 answers.
  it("keeps listening when pressed before it has heard anything, and sends on the next press", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    finish(said[0]);

    fireEvent.click(await screen.findByRole("button", { name: "Stop and send" }));
    expect(screen.getByRole("status")).toHaveTextContent(/still listening/i);
    expect(screen.getByRole("button", { name: "Stop and send" })).toBeInTheDocument();
    expect(mic.stop).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Stop and send" }));
    await waitFor(() => expect(mic.stop).toHaveBeenCalledOnce());
    await waitFor(() => expect(said).toHaveLength(2));
  });

  it("sends at the first press once it has heard the learner", async () => {
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    finish(said[0]);
    await screen.findByRole("button", { name: "Stop and send" });

    speakInto(mic);
    fireEvent.click(screen.getByRole("button", { name: "Stop and send" }));
    await waitFor(() => expect(mic.stop).toHaveBeenCalledOnce());
  });

  it("sends at the first press when the learner opened it", async () => {
    const mic = fakeMic();
    linesSaid();
    await openTalk();

    fireEvent.click(screen.getByRole("button", { name: "Interrupt Ella and start speaking" }));
    fireEvent.click(await screen.findByRole("button", { name: "Stop and send" }));
    await waitFor(() => expect(mic.stop).toHaveBeenCalledOnce());
  });

  it("puts itself down unsent when it has heard nothing for half a minute", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    let now = 1_000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    finish(said[0]);
    await screen.findByRole("button", { name: "Stop and send" });

    now += 29_000;
    act(() => vi.advanceTimersByTime(500));
    expect(screen.getByRole("button", { name: "Stop and send" })).toBeInTheDocument();

    now += 2_000;
    act(() => vi.advanceTimersByTime(500));
    expect(await screen.findByRole("button", { name: "Start speaking" })).toBeInTheDocument();
    expect(mic.cancel).toHaveBeenCalled();
    expect(mic.stop).not.toHaveBeenCalled();
  });

  it("stays open past half a minute once it has heard the learner", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    let now = 1_000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    const mic = fakeMic();
    const said = linesSaid();
    await openTalk();
    finish(said[0]);
    await screen.findByRole("button", { name: "Stop and send" });

    speakInto(mic);
    now += 31_000;
    act(() => vi.advanceTimersByTime(500));
    expect(screen.getByRole("button", { name: "Stop and send" })).toBeInTheDocument();
    expect(mic.cancel).not.toHaveBeenCalled();
  });
});
