import { SpeakingWave } from "./EllaMascot";
import type { WordShown } from "../types";

/**
 * Word of the day, as on Ella Mobile ("Ella Modes Options": Home's daily row,
 * 2a, saying what is inside its talk, 4c; and the word's card in its talk).
 * The meaning is never shown before the talk: Ella opens it by asking whether
 * the learner knows it.
 */

/** Home's daily card under Today's talk: "TODAY'S WORD · 2 MIN" over the
 * word, Ella Mobile's wordbook peeking from the corner. It leaves Home once
 * the word has been played today. */
export function DailyWordCard({
  word,
  disabled,
  onStart,
}: {
  word: WordShown;
  disabled: boolean;
  onStart: () => void;
}) {
  return (
    <button
      type="button"
      className="daily-word"
      data-card="word"
      disabled={disabled}
      aria-label={`Today’s word: ${word.text}, 2 min`}
      onClick={onStart}
    >
      <span className="daily-word__text" aria-hidden="true">
        <span className="word-label">Today’s word · 2 min</span>
        <span className="display daily-word__word">{word.text}</span>
      </span>
      <span className="daily-word__art" aria-hidden="true">
        <WordbookArt />
      </span>
    </button>
  );
}

/** The word's card in its talk: the word on white, and a pink disc that has
 * Ella say it. While she does, the disc shows her wave. */
export function WordCard({
  word,
  saying,
  disabled,
  onSay,
}: {
  word: WordShown;
  saying: boolean;
  disabled: boolean;
  onSay: () => void;
}) {
  return (
    <section className="word-card" aria-label={`Today’s word: ${word.text}`}>
      <span className="word-card__text" aria-hidden="true">
        <span className="word-label">Today’s word</span>
        <span className="display word-card__word">{word.text}</span>
      </span>
      <button
        type="button"
        className={`word-card__say${saying ? " is-saying" : ""}`}
        disabled={disabled}
        aria-label={`Hear “${word.text}”`}
        onClick={onSay}
      >
        {saying ? (
          <SpeakingWave />
        ) : (
          <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
            <path className="word-card__cone" d="M4 9.5h3.2L12 5.5v13l-4.8-4H4z" />
            <path className="word-card__waves" d="M15.5 9a4 4 0 010 6M18 6.5a7.5 7.5 0 010 11" />
          </svg>
        )}
      </button>
    </section>
  );
}

/** Whether `token`, a word of Ella's line as it is drawn, is today's word in
 * one of its forms: "Brave", "bravely," and "bravery!" are; "business" is
 * never busy. */
export function isWordForm(token: string, word: WordShown): boolean {
  const bare = token
    .toLowerCase()
    .replace(/['‘’]/g, "")
    .replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
  return bare !== "" && word.forms.includes(bare);
}

/** Ella Mobile's wordbook (`word_art.dart`): an open book with a pink "a" on
 * its left page, three lines on its right and an orange ribbon by the fold,
 * in a 128×96 box whose lower right the card crops. */
export function WordbookArt() {
  return (
    <svg className="wordbook" viewBox="0 0 128 96" aria-hidden="true">
      <g transform="translate(18 30)">
        <path
          fill="#c8a5eb"
          d="M7 2Q31 -3 64 9Q96 -3 121 2Q128 3 128 10L128 77Q98 72 64 86Q30 72 0 77L0 10Q0 3 7 2Z"
        />
        <path fill="#ffffff" d="M6 9Q30 5 63 18L63 77Q33 65 6 69Q5 69 5 66L5 13Q5 10 6 9Z" />
        <path fill="#ffffff" d="M65 18Q97 5 121 9Q123 10 123 13L123 66Q123 70 120 69Q94 65 65 77Z" />
        <text className="wordbook__letter" x="20.5" y="52.6">
          a
        </text>
        <rect fill="#c8a5eb" x="78" y="39" width="30" height="3.5" rx="1.75" />
        <rect fill="#c8a5eb" x="78" y="48" width="24" height="3.5" rx="1.75" />
        <rect fill="#c8a5eb" x="78" y="57" width="28" height="3.5" rx="1.75" />
        <path fill="#ff7a00" d="M70 16L78 13.5L78 30L74 26.5L70 30Z" />
      </g>
    </svg>
  );
}
