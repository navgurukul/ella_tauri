import { useLayoutEffect, useRef, type RefObject } from "react";
import type { EllaState } from "../components/EllaMascot";
import type { CastId } from "../types";

type EyeRef = RefObject<SVGGElement | null>;
interface PartnerRig {
  id: CastId;
  state: EllaState;
  root: RefObject<HTMLDivElement | null>;
  body: RefObject<HTMLDivElement | null>;
  leftEye: EyeRef;
  rightEye: EyeRef;
  leftPupil: EyeRef;
  rightPupil: EyeRef;
  leftBrow: EyeRef;
  rightBrow: EyeRef;
}

const BREATH_MS: Record<CastId, number> = {
  "stall-owner": 5600, landlord: 6800, doctor: 6200, debater: 5200,
};
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
const rounded = (value: number) => Number(value.toFixed(3));

/** A continuous clock keeps breathing in phase across state changes. The
 * face settles into new poses over a few frames, while only the pupils follow
 * the pointer. No squash/stretch or speaking bounce moves the whole drawing. */
export function usePartnerMotion(rig: PartnerRig) {
  const current = useRef(rig);
  const refresh = useRef<() => void>(() => undefined);
  current.current = rig;

  useLayoutEffect(() => {
    const media = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    let reduced = media?.matches ?? false;
    let frame = 0;
    let last = performance.now();
    const start = last;
    let nextBlink = last + 3800 + Math.random() * 2400;
    let blinkAt = 0;
    let pointerX = 0;
    let pointerY = 0;
    let y = 0;
    let turn = 0;
    let gazeX = 0;
    let gazeY = 0;
    let openness = 1;
    let browLift = 0;
    let browTilt = 0;
    let browAsymmetry = 0;

    const draw = (now: number, settle = false) => {
      frame = 0;
      const dt = Math.min(40, Math.max(0, now - last));
      last = now;
      const { id, state, body, leftEye, rightEye, leftPupil, rightPupil, leftBrow, rightBrow } = current.current;
      const listening = state === "listening";
      const thinking = state === "thinking";
      const breathe = reduced ? 0 : (1 - Math.cos((now - start) * Math.PI * 2 / BREATH_MS[id])) / 2;
      const nod = listening && !reduced ? Math.sin((now - start) * Math.PI / 4400) ** 12 : 0;
      const ease = settle || reduced ? 1 : 1 - Math.exp(-dt / 180);
      const gazeEase = settle || reduced ? 1 : 1 - Math.exp(-dt / 75);
      const toward = (from: number, to: number, amount = ease) => from + (to - from) * amount;
      y = toward(y, (listening ? -3 : thinking ? -1 : 0) - breathe * 2.2 + nod * 1.1);
      turn = toward(turn, listening ? 0.55 + nod * 0.18 : thinking ? -0.7 : 0);
      gazeX = toward(gazeX, listening ? 0 : thinking ? -2.6 : reduced ? 0 : pointerX, gazeEase);
      gazeY = toward(gazeY, listening ? 0 : thinking ? -3.2 : reduced ? 0 : pointerY, gazeEase);
      openness = toward(openness, listening ? 1.09 : thinking ? 0.88 : 1);
      browLift = toward(browLift, listening ? -2.5 : thinking ? -1 : 0);
      browTilt = toward(browTilt, thinking ? 5 : listening ? -2 : 0);
      browAsymmetry = toward(browAsymmetry, thinking ? 2 : 0);

      if (!reduced && now >= nextBlink && !blinkAt) blinkAt = now;
      const blinkProgress = blinkAt ? (now - blinkAt) / 180 : 0;
      const blink = reduced || !blinkAt ? 1 : 1 - 0.96 * Math.sin(Math.PI * Math.min(1, blinkProgress)) ** 2;
      if (blinkProgress >= 1) {
        blinkAt = 0;
        nextBlink = now + 3800 + Math.random() * 2400;
      }
      if (body.current) body.current.style.transform = `translateY(${rounded(y)}px) rotate(${rounded(turn)}deg)`;
      for (const eye of [leftEye, rightEye]) eye.current?.setAttribute("transform", `scale(1 ${rounded(openness * blink)})`);
      for (const pupil of [leftPupil, rightPupil]) pupil.current?.setAttribute("transform", `translate(${rounded(gazeX)} ${rounded(gazeY)})`);
      leftBrow.current?.setAttribute("transform", `translate(0 ${rounded(browLift)}) rotate(${rounded(browTilt)})`);
      rightBrow.current?.setAttribute("transform", `translate(0 ${rounded(browLift - browAsymmetry)}) rotate(${rounded(-browTilt)})`);
      if (!document.hidden && !reduced) frame = requestAnimationFrame(draw);
    };

    const move = (event: globalThis.PointerEvent) => {
      const box = current.current.root.current?.getBoundingClientRect();
      if (!box) return;
      pointerX = clamp((event.clientX - box.left - box.width / 2) / 110, -3, 3);
      pointerY = clamp((event.clientY - box.top - box.width * 70 / 660) / 160, -2, 2);
    };
    const leave = () => { pointerX = 0; pointerY = 0; };
    const restart = () => {
      cancelAnimationFrame(frame);
      last = performance.now();
      blinkAt = 0;
      nextBlink = last + 3800;
      draw(last, true);
    };
    const preference = () => { reduced = media?.matches ?? false; restart(); };
    window.addEventListener("pointermove", move);
    document.documentElement.addEventListener("pointerleave", leave);
    document.addEventListener("visibilitychange", restart);
    media?.addEventListener?.("change", preference);
    draw(last, true);
    // Reduced motion still needs a new held expression when state changes.
    refresh.current = () => { if (reduced) restart(); };
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointermove", move);
      document.documentElement.removeEventListener("pointerleave", leave);
      document.removeEventListener("visibilitychange", restart);
      media?.removeEventListener?.("change", preference);
      refresh.current = () => undefined;
    };
  }, [rig.id]);

  useLayoutEffect(() => refresh.current(), [rig.state]);
}
