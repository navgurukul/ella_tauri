import type { PhonemeSpan } from "../types";
import { visemeFor, VISEMES, type VisemeCue } from "./visemes";

/**
 * The cues Ella's mouth follows for one clip, from Piper's own timing of every
 * token it spoke: the sounds, and the blanks, word spaces, stress marks and
 * sentence ends between them. Ported from Ella Mobile's `speech_alignment.dart`.
 * No timing is inferred from the text, a second synthesis or the loudness of
 * the audio.
 *
 * Throws on spans that do not tile the clip in order, which the backend never
 * sends; a clip without cues is simply spoken with the mouth closed.
 */
export function cuesFromPhonemes(spans: readonly PhonemeSpan[]): VisemeCue[] {
  // Microseconds keep every boundary an integer, so a token that ends where
  // the next one starts compares equal however the milliseconds were rounded.
  const us = (ms: number) => Math.round(ms * 1000);
  const units: Unit[] = [];
  let reached = spans.length ? us(spans[0].start_ms) : 0;
  let stress = "";
  let space = false;
  let pending: number | null = null;
  for (const span of spans) {
    const start = us(span.start_ms);
    const end = us(span.end_ms);
    if (start !== reached || end < start) throw new RangeError("Phoneme spans must tile their clip in order");
    reached = end;
    const symbol = span.phoneme;
    if (LEADING.has(symbol)) {
      pending ??= start;
      if (symbol === " ") space = true;
      if (symbol === "ˈ" || symbol === "ˌ") stress = symbol;
      continue;
    }
    const last = units[units.length - 1];
    // Length marks and diacritics lengthen the sound they follow. So does a
    // symbol with no pose, which would otherwise close the mouth mid-word.
    if (
      (TRAILING.has(symbol) || (!PAUSES.has(symbol) && visemeFor(symbol) === "rest")) &&
      last &&
      last.end === (pending ?? start)
    ) {
      last.end = end;
      last.last = start;
      pending = null;
      continue;
    }
    if (symbol === "p" && !space && last) {
      // A voiceless p is silent while the lips are shut, and Piper times that
      // silence as the last token of the vowel before it (apple, sheep), so
      // the lips meet there. A b or m is voiced as it closes.
      if (VOWELS.has(last.symbol) && last.end === (pending ?? start) && last.last - last.start >= 40_000) {
        pending = last.end = last.last;
      }
    }
    const vowel = VOWELS.has(symbol);
    if ((vowel || RELEASED.has(symbol)) && !space && last && (last.symbol === "p" || last.symbol === "b")) {
      // Piper times a p or b's closure before its own token, which is already
      // the burst and the voiced start of the sound it releases (apple, book),
      // so the lips part 12 ms into it. An m's murmur is voiced with the lips
      // shut.
      const release = last.last + 12_000;
      if (last.end === (pending ?? start) && release < last.end) pending = last.end = release;
    }
    units.push({
      symbol,
      start: pending ?? start,
      last: start,
      end,
      emphasis: vowel ? emphasisOf(stress) : 1,
      wordStart: space,
    });
    if (vowel || PAUSES.has(symbol)) stress = "";
    pending = null;
    space = false;
  }
  // Blanks at the very end stay with the sound they end.
  if (pending !== null && units.length) units[units.length - 1].end = reached;

  const cues: VisemeCue[] = [];
  let glide: number | null = null;
  for (let index = 0; index < units.length; index += 1) {
    const unit = units[index];
    let viseme = visemeFor(unit.symbol);
    const isGlide = glide !== null;
    let emphasis: number = glide ?? unit.emphasis;
    glide = null;
    // Approximants round the lips with a closer jaw than their vowel.
    if (unit.symbol === "w" || unit.symbol === "ʍ" || unit.symbol === "ɥ") emphasis = 0.7;
    // Sounds pair only inside a word: 't ␠ ʃ' is two sounds, not 'tʃ'.
    const next = units[index + 1];
    if (next && unit.end === next.start && !next.wordStart) {
      const pair = unit.symbol + next.symbol;
      if (unit.symbol === "j" && VOWELS.has(next.symbol) && VISEMES[visemeFor(next.symbol)].round >= 0.8) {
        // [j] is made by the tongue, so before a rounded vowel the lips are
        // already rounded for it, as in [ɥ]: you, few, huge.
        viseme = "oo";
        emphasis = 0.7;
      } else if (pair === "tʃ" || pair === "dʒ") {
        viseme = "sh";
        unit.end = next.end;
        index += 1;
      } else if (DIPHTHONGS.has(pair)) {
        // The second vowel keeps its own timing, making the glide, and the
        // stress of the vowel it glides from.
        viseme = visemeFor(pair);
        glide = emphasis;
      }
    }
    if (unit.end <= unit.start) continue;
    const start = unit.start / 1000;
    const end = unit.end / 1000;
    const previous = cues[cues.length - 1];
    if (viseme === "rest" && previous?.viseme === "rest" && previous.end === start) {
      // '. $' is one pause, so the sampler can tell its full length.
      previous.end = end;
      continue;
    }
    cues.push({
      start,
      end,
      viseme,
      emphasis,
      // Lax [ʊ] is less puckered than [u]; a diphthong still rounds fully.
      rounding: unit.symbol === "ʊ" && !isGlide ? 0.8 : 1,
      vowel: VOWELS.has(unit.symbol),
    });
  }
  return cues;
}

interface Unit {
  symbol: string;
  /** Where the sound starts, including any blank that leads into it (µs). */
  start: number;
  /** Where its last token starts: its own, or a length mark's after it (µs). */
  last: number;
  end: number;
  emphasis: number;
  /** A word space came before it, so it never pairs with the sound before. */
  wordStart: boolean;
}

/** Piper puts a blank after every token. Its audio already belongs to the
 * sound after it, as do word spaces and stress marks, so the mouth neither
 * closes between words nor holds a consonant over a stressed vowel. */
const LEADING = new Set(["_", " ", "ˈ", "ˌ"]);

const TRAILING = new Set([
  "ː",
  "ˑ",
  "ʰ",
  "ʲ",
  "ˤ",
  "˞",
  // Combining marks: nasal, syllabic, dental, non-syllabic, apical, laminal,
  // cedilla.
  "̃",
  "̩",
  "̪",
  "̯",
  "̺",
  "̻",
  "̧",
]);

/** The only symbols that close the mouth. A comma is kept as a short rest of
 * its own token and the blank before it. */
const PAUSES = new Set(["^", "$", ".", "!", "?", ",", ";", ":"]);

const VOWELS = new Set(["a", "e", "i", "o", "u", "æ", "ɑ", "ɒ", "ɔ", "ə", "ɛ", "ɪ", "ʊ", "ʌ", "ɚ", "ɝ", "ɜ", "ɐ", "ᵻ"]);

const DIPHTHONGS = new Set(["aɪ", "aʊ", "ɔɪ", "oʊ", "əʊ", "eɪ"]);

/** Voiced sounds besides vowels that a p or b bursts straight into (play,
 * bring). Before a stop or fricative it may be unreleased, so it keeps its
 * timing there. */
const RELEASED = new Set(["l", "ɹ", "r", "j", "w"]);

/** The pose table is the stressed articulation; unstressed vowels open less. */
function emphasisOf(stress: string): number {
  if (stress === "ˈ") return 1;
  if (stress === "ˌ") return 0.95;
  return 0.9;
}
