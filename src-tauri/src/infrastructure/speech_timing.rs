//! Word timings for a synthesized sentence, so the screen can highlight the
//! word Ella is saying.
//!
//! Piper hands back audio, not timings. Two things make an estimate work
//! anyway: the sentence's duration is known exactly, and it is short — five to
//! eight words — so error cannot accumulate across a reply the way it would if
//! the whole turn were tiled at once. Each sentence is anchored on both ends
//! and only the split inside it is estimated.
//!
//! The weights below were fitted against real Piper phoneme alignments (its
//! ONNX graph can expose them, patched in memory, with the `onnx` package
//! installed) over 60 sentences in Ella's own register. Measured onset error
//! for the shipped algorithm: mean ~70 ms, median ~70 ms, p90 ~160 ms, with
//! roughly three quarters of words inside 100 ms — under a quarter of an
//! average word, so the highlight sits on the right word.
//!
//! Swapping in exact alignments later is a change to this module alone: the
//! shape it returns is what the window already draws. It would cost a 68 MB
//! `onnx` dependency and ~30 ms per sentence, and would still need a fallback,
//! because espeak merges words ("in the" becomes one phoneme group) in about
//! 8% of sentences, which leaves no safe positional mapping back to the text.

use crate::domain::WordSpan;

/// Milliseconds a syllable is worth.
const MS_PER_SYLLABLE: f64 = 36.0;
/// Milliseconds a character is worth. Syllables and characters are correlated;
/// the fit splits the credit between them and only the sum is meaningful.
const MS_PER_CHAR: f64 = 45.0;
/// The pause a full stop, question mark or exclamation mark buys.
const MS_SENTENCE_PUNCT: f64 = 57.0;
/// The pause a comma buys, which is much longer than a full stop's — the full
/// stop's own silence is trimmed off the end instead of shared out.
const MS_CLAUSE_PUNCT: f64 = 295.0;
/// Final lengthening: the last word of an utterance is drawn out.
const MS_LAST_WORD: f64 = 57.0;
/// No word gets less than this, whatever the weights say.
const MIN_WORD_MS: f64 = 40.0;

/// Amplitude below which a sample counts as silence, as a fraction of full
/// scale. Recovers Piper's leading pad to within ~12 ms on average, which is
/// what makes the first word's highlight land.
const SILENCE_FLOOR: i16 = (i16::MAX as f64 * 0.01) as i16;

/// Where each word of `text` falls inside `pcm`.
///
/// `pcm` is 16-bit little-endian mono, as the Piper daemon returns it. Returns
/// an empty vector when there is nothing to time, so a caller can treat "no
/// timings" and "no words" the same way.
pub fn word_spans(text: &str, pcm: &[u8], sample_rate: u32) -> Vec<WordSpan> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || sample_rate == 0 || pcm.len() < 2 {
        return Vec::new();
    }
    let samples = pcm.len() / 2;
    let to_ms = |count: usize| count as f64 * 1_000.0 / sample_rate as f64;
    let total_ms = to_ms(samples);

    let (lead, tail) = silence_bounds(pcm);
    // Speech has to occupy something. A sentence that is silence end to end
    // means Piper produced nothing usable, so spread the words evenly rather
    // than divide by a zero-length body.
    let body_ms = (total_ms - to_ms(lead) - to_ms(tail)).max(1.0);

    let weights: Vec<f64> = words
        .iter()
        .enumerate()
        .map(|(index, word)| weight_of(word, index + 1 == words.len()))
        .collect();
    let scale = body_ms / weights.iter().sum::<f64>();

    let mut spans = Vec::with_capacity(words.len());
    let mut at = to_ms(lead);
    for (word, weight) in words.iter().zip(&weights) {
        let end = at + weight * scale;
        spans.push(WordSpan {
            text: (*word).to_string(),
            start_ms: round_ms(at),
            end_ms: round_ms(end.min(total_ms)),
        });
        at = end;
    }
    spans
}

/// Shift every span by `offset_ms`, for stitching a whole reply's timings out
/// of its sentences.
pub fn shifted(spans: &[WordSpan], offset_ms: f64) -> Vec<WordSpan> {
    spans
        .iter()
        .map(|span| WordSpan {
            text: span.text.clone(),
            start_ms: round_ms(span.start_ms + offset_ms),
            end_ms: round_ms(span.end_ms + offset_ms),
        })
        .collect()
}

/// How many samples of silence sit at each end of the clip.
fn silence_bounds(pcm: &[u8]) -> (usize, usize) {
    let samples = pcm.len() / 2;
    let at = |index: usize| i16::from_le_bytes([pcm[index * 2], pcm[index * 2 + 1]]).saturating_abs();
    let lead = (0..samples)
        .find(|index| at(*index) >= SILENCE_FLOOR)
        .unwrap_or(samples);
    if lead == samples {
        return (0, 0);
    }
    let last = (lead..samples)
        .rev()
        .find(|index| at(*index) >= SILENCE_FLOOR)
        .unwrap_or(lead);
    (lead, samples - 1 - last)
}

fn weight_of(word: &str, last: bool) -> f64 {
    let core: String = word
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '\u{2019}')
        .collect();
    let trailing = word.trim_end_matches(|c: char| !c.is_alphanumeric());
    let punctuation = &word[trailing.len()..];
    let syllables = syllables_in(&core).max(1) as f64;
    let mut weight = syllables * MS_PER_SYLLABLE + core.chars().count() as f64 * MS_PER_CHAR;
    if punctuation.contains(['.', '!', '?']) {
        weight += MS_SENTENCE_PUNCT;
    }
    if punctuation.contains([',', ';', ':', '\u{2014}']) {
        weight += MS_CLAUSE_PUNCT;
    }
    if last {
        weight += MS_LAST_WORD;
    }
    weight.max(MIN_WORD_MS)
}

/// Vowel groups, which is the cheap English syllable estimate. "yummy" is two,
/// "through" is one.
fn syllables_in(word: &str) -> usize {
    let mut count = 0;
    let mut in_vowel = false;
    for character in word.chars() {
        let vowel = matches!(
            character.to_ascii_lowercase(),
            'a' | 'e' | 'i' | 'o' | 'u' | 'y'
        );
        if vowel && !in_vowel {
            count += 1;
        }
        in_vowel = vowel;
    }
    count
}

fn round_ms(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 22_050;

    /// PCM with `lead_ms` of silence, then tone, then `tail_ms` of silence.
    fn clip(lead_ms: f64, speech_ms: f64, tail_ms: f64) -> Vec<u8> {
        let count = |ms: f64| (ms / 1_000.0 * RATE as f64).round() as usize;
        let mut pcm = Vec::new();
        for _ in 0..count(lead_ms) {
            pcm.extend_from_slice(&0_i16.to_le_bytes());
        }
        for index in 0..count(speech_ms) {
            // Alternate sign so the clip is loud on every sample, not just some.
            let value = if index % 2 == 0 { 8_000 } else { -8_000 };
            pcm.extend_from_slice(&(value as i16).to_le_bytes());
        }
        for _ in 0..count(tail_ms) {
            pcm.extend_from_slice(&0_i16.to_le_bytes());
        }
        pcm
    }

    #[test]
    fn spans_one_word_per_word_in_order() {
        let spans = word_spans("Oh, that sounds yummy!", &clip(60.0, 1_300.0, 100.0), RATE);
        assert_eq!(
            spans.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            vec!["Oh,", "that", "sounds", "yummy!"]
        );
        for pair in spans.windows(2) {
            assert!(
                pair[0].end_ms <= pair[1].start_ms + 0.2,
                "spans overlap: {:?}",
                pair
            );
        }
    }

    /// The first word must not be highlighted during Piper's leading silence —
    /// that is the error the measured lead-in exists to remove.
    #[test]
    fn the_first_word_starts_after_the_leading_silence() {
        let spans = word_spans("Hello there my friend.", &clip(180.0, 1_200.0, 90.0), RATE);
        let start = spans[0].start_ms;
        assert!(
            (start - 180.0).abs() < 2.0,
            "first word starts at {start}ms, expected ~180ms"
        );
    }

    /// Speech has to end where speech ends, not where the clip does.
    #[test]
    fn the_last_word_ends_before_the_trailing_silence() {
        let spans = word_spans("Hello there my friend.", &clip(50.0, 1_000.0, 400.0), RATE);
        let end = spans.last().expect("spans").end_ms;
        assert!(
            end <= 1_060.0,
            "last word ends at {end}ms, past the 1050ms of speech"
        );
    }

    /// Words tile the speech with no gap: a highlight that falls between two
    /// spans is a flicker to nothing.
    #[test]
    fn spans_tile_the_speech_without_gaps() {
        let spans = word_spans(
            "It was spicy and a little sweet, with onion on top.",
            &clip(70.0, 3_000.0, 150.0),
            RATE,
        );
        for pair in spans.windows(2) {
            assert!(
                (pair[1].start_ms - pair[0].end_ms).abs() < 0.2,
                "gap between {:?} and {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// A comma is a real pause, so the word before it holds the highlight
    /// longer than the same word would elsewhere.
    #[test]
    fn a_clause_pause_lengthens_the_word_before_it() {
        let with = word_spans("hello sweet, world", &clip(0.0, 2_000.0, 0.0), RATE);
        let without = word_spans("hello sweet world", &clip(0.0, 2_000.0, 0.0), RATE);
        let span = |s: &[WordSpan]| s[1].end_ms - s[1].start_ms;
        assert!(
            span(&with) > span(&without) * 1.3,
            "comma bought only {:.0}ms over {:.0}ms",
            span(&with),
            span(&without)
        );
    }

    /// Longer words hold the highlight longer than shorter ones.
    #[test]
    fn a_longer_word_holds_the_highlight_longer() {
        let spans = word_spans("I extraordinarily go", &clip(0.0, 2_000.0, 0.0), RATE);
        let width = |index: usize| spans[index].end_ms - spans[index].start_ms;
        assert!(width(1) > width(0) * 3.0, "{:?}", spans);
        assert!(width(1) > width(2) * 3.0, "{:?}", spans);
    }

    /// A clip that is silence end to end still yields usable spans rather than
    /// dividing by a zero-length body.
    #[test]
    fn silence_does_not_divide_by_zero() {
        let spans = word_spans("Hello there.", &clip(0.0, 0.0, 500.0), RATE);
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|s| s.end_ms >= s.start_ms));
    }

    #[test]
    fn no_words_and_no_audio_yield_nothing() {
        assert!(word_spans("", &clip(0.0, 500.0, 0.0), RATE).is_empty());
        assert!(word_spans("Hello", &[], RATE).is_empty());
        assert!(word_spans("Hello", &clip(0.0, 500.0, 0.0), 0).is_empty());
    }

    /// Stitching a reply out of sentences: the second sentence's timings sit
    /// after the first sentence's audio, not on top of it.
    #[test]
    fn shifting_moves_a_sentence_into_reply_time() {
        let spans = word_spans("Hello there.", &clip(20.0, 600.0, 40.0), RATE);
        let moved = shifted(&spans, 1_000.0);
        assert_eq!(moved.len(), spans.len());
        for (before, after) in spans.iter().zip(&moved) {
            assert!((after.start_ms - before.start_ms - 1_000.0).abs() < 0.2);
            assert_eq!(after.text, before.text);
        }
    }

    /// Syllable counting is what separates "I" from "extraordinarily"; vowel
    /// groups, not vowels, so "yummy" is two and not three.
    #[test]
    fn counts_syllables_by_vowel_group() {
        assert_eq!(syllables_in("yummy"), 2);
        assert_eq!(syllables_in("through"), 1);
        assert_eq!(syllables_in("I"), 1);
        assert_eq!(syllables_in("delicious"), 3);
        // Vowel groups are an estimate, not phonology: a silent final 'e' is
        // counted and a diphthong split across letters is not. The error is
        // one syllable on a minority of words, which the fit already absorbs.
        assert_eq!(syllables_in("time"), 2);
    }
}
