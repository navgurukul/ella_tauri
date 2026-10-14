import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { Glyph } from "./Glyphs";
import type { MentorLesson } from "../types";

/**
 * Ella asking for a chat before the next talk, once the learner has gone past
 * her lesson (Ella Mobile's "Let's have a chat first" sheet): talking to her
 * first is the way she points to, and carrying on with `instead`, the topic or
 * partner they picked, the quieter way past. Closing it, by Escape, the cross
 * or a click outside it, is a third: neither, for now. She asks before every
 * talk until the lesson is taken.
 */
export function LessonNudge({
  lesson,
  instead,
  busy,
  onLesson,
  onContinue,
  onClose,
}: {
  lesson: MentorLesson;
  /** What the learner picked: a topic's title, or a partner's name. */
  instead: string;
  busy: boolean;
  onLesson: () => void;
  onContinue: () => void;
  onClose: () => void;
}) {
  const title = useId();
  const card = useRef<HTMLDivElement>(null);
  const talk = useRef<HTMLButtonElement>(null);
  const closeNow = useRef(onClose);
  closeNow.current = onClose;

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // The sheet is portalled beside the app, so the whole app can go inert.
    const app = document.getElementById("root");
    const wasInert = app?.hasAttribute("inert") ?? false;
    app?.setAttribute("inert", "");
    talk.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeNow.current();
        return;
      }
      if (event.key !== "Tab" || !card.current) return;
      const stops = [...card.current.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      if (stops.length === 0) return;
      const first = stops[0];
      const last = stops[stops.length - 1];
      const at = document.activeElement;
      if (event.shiftKey && (at === first || !card.current.contains(at))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (at === last || !card.current.contains(at))) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (!wasInert) app?.removeAttribute("inert");
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  return createPortal(
    <div className="badge-sheet lesson-nudge">
      <div className="badge-sheet__scrim" onClick={onClose} aria-hidden="true" />
      <div ref={card} className="badge-sheet__card lesson-nudge__card" role="dialog" aria-modal="true" aria-labelledby={title}>
        <button className="badge-sheet__close" onClick={onClose} aria-label="Close">
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
        <span className="lesson-nudge__ella" aria-hidden="true">
          <Glyph glyph="ella" size={40} color="#9347dd" />
        </span>
        <h2 id={title} className="display lesson-nudge__title">
          Let&rsquo;s have a chat first
        </h2>
        <p className="lesson-nudge__text">
          I have a short lesson ready for you on {lowerFirst(lesson.title)}. Talk with me before you move
          on.
        </p>
        <button ref={talk} className="btn btn--violet lesson-nudge__talk" disabled={busy} onClick={onLesson}>
          Talk with Ella
        </button>
        <button className="lesson-nudge__continue" disabled={busy} onClick={onContinue}>
          Continue with {instead}
        </button>
      </div>
    </div>,
    document.body,
  );
}

/** A skill's name inside a sentence: only its first letter lowered, as the
 * backend does, so "What I did" reads "what I did". */
function lowerFirst(title: string): string {
  return title ? title[0].toLowerCase() + title.slice(1) : title;
}
