import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EllaMascot, MouthDrawing, type EllaState, type EllaVariant } from "./EllaMascot";
import { mouthGeometry } from "../lib/mouth";
import { SMILE, VISEMES } from "../lib/visemes";

const STATES: EllaState[] = ["resting", "listening", "thinking", "speaking"];
const VARIANTS: EllaVariant[] = ["welcome", "corner", "age", "conversation", "home", "mentor", "profile"];

afterEach(() => {
  Reflect.deleteProperty(Element.prototype, "animate");
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("Ella holds still", () => {
  it("asks for no animation, frame or timer in any placement or state", () => {
    vi.useFakeTimers();
    const animate = vi.fn();
    Object.defineProperty(Element.prototype, "animate", { configurable: true, value: animate });
    const frames = vi.spyOn(window, "requestAnimationFrame");
    const view = render(
      <>
        {VARIANTS.map((variant) => (
          <EllaMascot key={variant} variant={variant} decorative />
        ))}
      </>,
    );
    for (const state of STATES) {
      view.rerender(
        <>
          {VARIANTS.map((variant) => (
            <EllaMascot key={variant} variant={variant} state={state} decorative />
          ))}
        </>,
      );
    }
    fireEvent.mouseMove(window, { clientX: 400, clientY: 200 });
    fireEvent.click(document.querySelector(".ella__body")!);
    expect(animate).not.toHaveBeenCalled();
    expect(frames).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps her eyes where they are when the pointer moves", () => {
    render(<EllaMascot variant="corner" decorative />);
    fireEvent.mouseMove(window, { clientX: 10, clientY: 10 });
    const eye = document.querySelector<HTMLElement>(".ella__eye--left")!;
    expect(eye.style.transform).toBe("");
  });

  it("speaks with the stylesheet's static o on the talk stage", () => {
    const { container } = render(<EllaMascot variant="conversation" state="speaking" />);
    const ella = container.querySelector(".ella");
    expect(ella).toHaveClass("ella--speaking");
    expect(ella).not.toHaveClass("ella--lipsync");
    expect(container.querySelector(".ella__mouth--open")).toBeInTheDocument();
    expect(container.querySelector(".ella__speech")).toBeNull();
  });

  it("wears an expression's pose as a class: listening on the stage, eager on Home", () => {
    const view = render(<EllaMascot variant="conversation" state="listening" ears="listen" decorative />);
    expect(document.querySelector(".ella")).toHaveClass("ella--expression-listening", "ella--ears-listen");
    view.rerender(<EllaMascot variant="home" expression="homeEager" decorative />);
    expect(document.querySelector(".ella")).toHaveClass("ella--expression-homeEager");
  });

  it("keeps her smile and open eyes through the first talk, whatever she is doing", () => {
    render(<EllaMascot variant="conversation" state="thinking" faceFollowsState={false} />);
    const ella = document.querySelector(".ella");
    expect(ella).toHaveClass("ella--resting");
    expect(ella).not.toHaveClass("ella--thinking");
    expect(screen.getByRole("img", { name: "Ella is thinking" })).toBeInTheDocument();
  });
});

describe("a mouth drawn once", () => {
  const box = { x: 314, y: 86, width: 30, height: 13, line: 3 };
  const stage = { width: 660, height: 450 };

  it("draws a smile a little above its box's centre, to reach the U's arms", () => {
    render(<MouthDrawing shape={SMILE} open={false} box={box} stage={stage} />);
    expect(document.querySelector(".ella__lips")?.getAttribute("d")).toBe(mouthGeometry(SMILE, 30, 3).path);
    expect(document.querySelector(".ella__speech-mouth")?.getAttribute("transform")).toBe(
      "translate(329 90.550) rotate(0.000)",
    );
  });

  it("draws an open mouth on its box's centre, with its teeth and tongue", () => {
    render(<MouthDrawing shape={VISEMES.aa} open box={box} stage={stage} />);
    expect(document.querySelector(".ella__lips")?.getAttribute("d")).toBe(mouthGeometry(VISEMES.aa, 30, 3).path);
    expect(document.querySelector(".ella__speech-mouth")?.getAttribute("transform")).toBe(
      "translate(329 92.500) rotate(0.000)",
    );
    expect(document.querySelector(".ella__tongue")).toBeInTheDocument();
  });
});
