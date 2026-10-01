import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EllaMascot, type EllaState } from "./EllaMascot";
import { bodyLoop, bodyTransform, POKE } from "../lib/motion";
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
  <EllaMascot variant="conversation" state={state} speech={mouth} />
);

describe("Ella's mouth on the talk stage", () => {
  it("draws her smile as the lip-sync path, in place of the stylesheet's", () => {
    const { mouth } = fakeMouth();
    const { container } = render(talkStage(mouth));
    expect(container.querySelector(".ella")).toHaveClass("ella--lipsync");
    expect(lips()).toBe(mouthGeometry(SMILE, 30, 3).path);
    // Centred on the smile's box, and a little higher to reach the U's arms.
    expect(document.querySelector(".ella__speech-mouth")?.getAttribute("transform")).toBe(
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
    expect(document.querySelector(".ella__speech-mouth")?.getAttribute("transform")).toBe(
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
    const { container } = render(<EllaMascot variant="home" speech={mouth} />);
    expect(container.querySelector(".ella")).not.toHaveClass("ella--lipsync");
    expect(container.querySelector(".ella__speech")).toBeNull();
  });
});

interface Played {
  target: Element;
  keyframes: Keyframe[];
  options: KeyframeAnimationOptions;
  cancelled: boolean;
}

/** Stands in for the Web Animations API, which jsdom lacks, and records what
 * Ella plays. Nothing finishes by itself, so whatever was not cancelled is
 * still playing. */
function recordAnimations() {
  const played: Played[] = [];
  const handle = (record: Played) => ({ cancel: () => void (record.cancelled = true) }) as unknown as Animation;
  Object.defineProperty(Element.prototype, "animate", {
    configurable: true,
    value(this: Element, keyframes: Keyframe[], options: KeyframeAnimationOptions) {
      const record = { target: this, keyframes, options, cancelled: false };
      played.push(record);
      return handle(record);
    },
  });
  Object.defineProperty(Element.prototype, "getAnimations", {
    configurable: true,
    value(this: Element) {
      return played.filter((record) => record.target === this && !record.cancelled).map(handle);
    },
  });
  return {
    playing: (target: Element) => played.filter((record) => record.target === target && !record.cancelled),
    on: (target: Element) => played.filter((record) => record.target === target),
  };
}

describe("how Ella moves, as on Ella Mobile", () => {
  afterEach(() => {
    Reflect.deleteProperty(Element.prototype, "animate");
    Reflect.deleteProperty(Element.prototype, "getAnimations");
    Reflect.deleteProperty(window, "matchMedia");
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  const inner = () => document.querySelector(".ella__inner") as HTMLElement;

  it("plays her pose's loop, and eases her into the next rather than jumping", () => {
    const animations = recordAnimations();
    const view = render(<EllaMascot variant="conversation" state="resting" decorative />);
    expect(animations.playing(inner()).map((record) => record.options)).toEqual([
      { duration: 4200, iterations: Infinity },
    ]);

    // Wherever bobbing had got her when the mic opened.
    const real = window.getComputedStyle.bind(window);
    vi.spyOn(window, "getComputedStyle").mockImplementation((element, pseudo) =>
      element === inner() ? ({ transform: "matrix(1, 0, 0, 1, 0, -5)" } as CSSStyleDeclaration) : real(element, pseudo),
    );
    view.rerender(<EllaMascot variant="conversation" state="listening" decorative />);

    const [loop, handoff] = animations.playing(inner());
    expect(animations.on(inner())).toHaveLength(3);
    expect(loop.options).toEqual({ duration: 3600, iterations: Infinity });
    expect(loop.keyframes[0].transform).toBe(bodyTransform(bodyLoop("listening", 30).at(0)));
    expect(handoff.options).toEqual({ duration: 160, easing: "ease" });
    expect(handoff.keyframes).toEqual([
      { transform: "matrix(1, 0, 0, 1, 0, -5)" },
      { transform: bodyTransform(bodyLoop("listening", 30).at(160 / 3600)) },
    ]);
    expect(document.querySelector(".ella")).toHaveClass("ella--expression-listening");
  });

  it("holds a still frame with reduced motion: the listening lean, without the loop", () => {
    const animations = recordAnimations();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({ matches: query === "(prefers-reduced-motion: reduce)" }),
    });
    render(<EllaMascot variant="conversation" state="listening" decorative />);
    expect(animations.on(inner())).toEqual([]);
    expect(inner().style.transform).toBe("translateY(-6px) rotate(2deg) scale(1, 1)");
  });

  it("squishes inside her loop when poked, so she keeps bobbing", () => {
    const animations = recordAnimations();
    render(<EllaMascot variant="mentor" pokeable decorative />);
    fireEvent.click(document.querySelector(".ella__body")!);
    fireEvent.click(document.querySelector(".ella__body")!);

    const squish = document.querySelector(".ella__squish")!;
    const pokes = animations.on(squish);
    expect(pokes).toHaveLength(2);
    expect(pokes[0].cancelled).toBe(true);
    expect(pokes[1].keyframes).toEqual(POKE.keyframes);
    expect(pokes[1].options).toEqual({ duration: 600, easing: "cubic-bezier(0.3, 1.2, 0.4, 1)" });
    expect(animations.playing(inner()).map((record) => record.options.duration)).toEqual([4200]);
  });

  it("moves Home's Ella by her mood, and keeps the learner's own still", () => {
    const animations = recordAnimations();
    const home = render(<EllaMascot variant="home" expression="homeEager" decorative />);
    expect(document.querySelector(".ella")).toHaveClass("ella--expression-homeEager");
    expect(animations.playing(inner()).map((record) => record.options.duration)).toEqual([2200]);
    home.unmount();

    render(<EllaMascot variant="profile" decorative />);
    expect(animations.on(inner())).toEqual([]);
  });

  it("leaves the recap's hero to the band that hops her", () => {
    const animations = recordAnimations();
    render(<EllaMascot variant="conversation" animate={false} pokeable decorative />);
    fireEvent.click(document.querySelector(".ella__body")!);
    expect(animations.on(inner())).toEqual([]);
    expect(animations.on(document.querySelector(".ella__squish")!)).toEqual([]);
  });

  it("keeps her smile and open eyes through the first talk, whatever she is doing", () => {
    render(<EllaMascot variant="conversation" state="thinking" faceFollowsState={false} />);
    const ella = document.querySelector(".ella");
    expect(ella).toHaveClass("ella--resting");
    expect(ella).not.toHaveClass("ella--thinking");
    expect(screen.getByRole("img", { name: "Ella is thinking" })).toBeInTheDocument();
  });

  it("blinks and grins with every Ella on screen, but not where she wears an expression", () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0);
    const view = render(
      <>
        <EllaMascot variant="corner" className="corner" decorative />
        <EllaMascot variant="mentor" className="mentor" decorative />
        <EllaMascot variant="home" className="home" expression="homeEager" decorative />
      </>,
    );
    const eye = (name: string) => (document.querySelector(`.${name} .ella__eye--left`) as HTMLElement).style.transform;

    act(() => void vi.advanceTimersByTime(2600));
    expect([eye("corner"), eye("mentor"), eye("home")]).toEqual(Array(3).fill("translate(0px, 0px) scaleY(0.12)"));
    act(() => void vi.advanceTimersByTime(5000 - 2600));
    expect(document.querySelector(".corner")).toHaveAttribute("data-grinning");
    expect(document.querySelector(".mentor")).toHaveAttribute("data-grinning");
    expect(document.querySelector(".home")).not.toHaveAttribute("data-grinning");
    act(() => void vi.advanceTimersByTime(650));
    expect(document.querySelector(".corner")).not.toHaveAttribute("data-grinning");
    view.unmount();
  });
});
