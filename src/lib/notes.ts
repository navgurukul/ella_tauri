/**
 * The browser preview's copy of the backend's recap notes (`notes.rs`), as far
 * as a preview without a model gets: whether a talk is too short for notes,
 * and what went well, read off the learner's words by the same rules. The
 * preview has no model to find a fix with, and no ledger chores, so it never
 * has a fix and never praises a deal.
 */

const MIN_ANSWERS = 3;
const MIN_WORDS = 12;

export function tooShort(answers: string[]): boolean {
  const words = answers.reduce((sum, answer) => sum + answer.split(/\s+/).filter(Boolean).length, 0);
  return answers.length < MIN_ANSWERS || words < MIN_WORDS;
}

const IRREGULAR_PAST = new Set(
  (
    "was were went had did saw ate made got came took bought told said gave found thought felt left met " +
    "ran won lost paid sat slept spoke wrote drove flew brought caught taught knew became began drank " +
    "forgot heard kept sent stood swam threw woke wore sold sang understood spent built broke chose " +
    "fell fought hid rode shook meant lent"
  ).split(" "),
);
const NOT_PAST = new Set(
  (
    "need feed seed speed weed breed bleed greed indeed proceed exceed succeed bed red shed hundred " +
    "sacred naked wicked tired excited interested bored scared worried married confused surprised " +
    "amazed pleased embarrassed"
  ).split(" "),
);
const POLITE = new Set(["please", "thank", "thanks", "sorry", "excuse", "bhaiya", "ji", "sir", "madam"]);
const QUESTION_OPENERS = new Set(
  "what where when why how who which whose can could would will shall may do does did is are".split(" "),
);

function wordsOf(text: string): string[] {
  return text
    .split(/\s+/)
    .map((word) => word.toLowerCase().replace(/[^\p{L}\p{N}]/gu, ""))
    .filter(Boolean);
}

function isPast(word: string): boolean {
  return IRREGULAR_PAST.has(word) || (word.length >= 4 && word.endsWith("ed") && !NOT_PAST.has(word));
}

function says(words: string[], phrase: string[]): boolean {
  return words.some((_, index) => phrase.every((part, offset) => words[index + offset] === part));
}

export function wentWell(answers: string[]): string[] {
  const lines = answers.map(wordsOf);
  const all = lines.flat();
  const lengths = lines.map((line) => line.length).sort((a, b) => a - b);
  const median = lengths[Math.floor(lengths.length / 2)] ?? 0;
  const first = lines[0] ?? [];
  const candidates: Array<[boolean, string]> = [
    [
      all.includes("because") || all.includes("since") || says(all, ["so", "that"]) || says(all, ["thats", "why"]),
      "Gave reasons",
    ],
    [all.filter(isPast).length >= 2, "Told what happened"],
    [
      answers.some((answer, index) => answer.includes("?") || QUESTION_OPENERS.has(lines[index][0] ?? "")),
      "Asked questions",
    ],
    [all.some((word) => POLITE.has(word)), "Stayed polite"],
    [median >= 6, "Full sentences"],
    [answers.length >= 5, "Kept it going"],
    [
      ["hi", "hello", "hey", "namaste", "namaskar"].includes(first[0] ?? "") ||
        (first[0] === "good" && ["morning", "afternoon", "evening"].includes(first[1] ?? "")),
      "Warm greeting",
    ],
    [true, "Answered each question"],
    [true, "Gave it a go"],
  ];
  return candidates
    .filter(([shown]) => shown)
    .slice(0, 2)
    .map(([, line]) => line);
}
