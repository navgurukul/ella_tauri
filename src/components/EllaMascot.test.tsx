import { act, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EllaMascot, type EllaState } from "./EllaMascot";
import { mouthGeometry } from "../lib/mouth";
import type { SpeechMouth } from "../lib/speech";
import { SMILE, VISEMES, type MouthShape } from "../lib/visemes";

/** A mouth source the test moves by hand. */
function fakeMouth() {
  const now = { timed: null as boolean | null, started: false, shape: VISEMES.rest as MouthShape };
  const mouth: SpeechMouth = {
    get timed() {
      return now.timed;
    },
    get started() {
      return now.started;
    },
    shape: () => now.shape,
  };
  return { mouth, now };
}

/** Let `count` animation frames go by. */
async function frames(count: number) {
  for (let index = 0; index < count; index += 1) {
    await act(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
  }
}

const lips = () => document.querySelector(".ella__lips")?.getAttribute("d");
const talkStage = (mouth: SpeechMouth, state: EllaState = "resting") => (
  <EllaMascot variant="conversation" state={state} speech={mouth} entrance={false} />
);

describe("Ella's mouth on the talk stage", () => {
  it("draws her smile as the lip-sync path, in place of the stylesheet's", () => {
    const { mouth } = fakeMouth();
    const { container } = render(talkStage(mouth));
    expect(container.querySelector(".ella")).toHaveClass("ella--lipsync");
    expect(lips()).toBe(mouthGeometry(SMILE, 30, 3).path);
    // Centred on the smile's box, and a little higher to reach the U's arms.
    expect(document.querySelector(".ella__speech g")?.getAttribute("transform")).toBe(
      "translate(329 90.550) rotate(0.000)",
    );
  });

  it("grows the sound she is making out of her smile, and hands back to it after", async () => {
    const { mouth, now } = fakeMouth();
    const view = render(talkStage(mouth, "speaking"));
    // Speaking, but nothing sounding yet: she smiles through synthesis.
    await frames(2);
    expect(lips()).toBe(mouthGeometry(SMILE, 30, 3).path);

    now.timed = true;
    now.started = true;
    now.shape = VISEMES.aa;
    await frames(8);
    expect(lips()).toBe(mouthGeometry(VISEMES.aa, 30, 3).path);
    expect(document.querySelector(".ella__speech g")?.getAttribute("transform")).toBe(
      "translate(329 92.500) rotate(0.000)",
    );

    // A frame into the 160 ms handoff she is neither the vowel nor the smile.
    now.started = false;
    view.rerender(talkStage(mouth, "resting"));
    await frames(1);
    expect(lips()).not.toBe(mouthGeometry(VISEMES.aa, 30, 3).path);
    expect(lips()).not.toBe(mouthGeometry(SMILE, 30, 3).path);
    await frames(14);
    expect(lips()).toBe(mouthGeometry(SMILE, 30, 3).path);
  });

  it("leaves audio it cannot follow to the design's static o", async () => {
    const { mouth, now } = fakeMouth();
    now.timed = false;
    const view = render(talkStage(mouth, "speaking"));
    await frames(2);
    expect(view.container.querySelector(".ella")).toHaveClass("is-untimed");
    view.rerender(talkStage(mouth, "resting"));
    await frames(2);
    expect(view.container.querySelector(".ella")).not.toHaveClass("is-untimed");
  });

  it("keeps the stylesheet's mouth wherever she does not talk", () => {
    const { mouth } = fakeMouth();
    const { container } = render(<EllaMascot variant="home" speech={mouth} entrance={false} />);
    expect(container.querySelector(".ella")).not.toHaveClass("ella--lipsync");
    expect(container.querySelector(".ella__speech")).toBeNull();
  });
});
