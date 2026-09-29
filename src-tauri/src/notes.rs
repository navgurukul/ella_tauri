//! Ella's notes on a finished talk, for its recap: what went well, and one
//! thing to say better.
//!
//! What went well is read off the learner's own words by the rules below, not
//! asked of the model. Ella's 3B model, asked for praise, described what the
//! talk was about ("went to market", "have two sisters"); given a numbered
//! list of praise to pick from, it picked the same two for every talk. The
//! rules hold with or without a model, and say nothing the words do not show.
//!
//! The fix is the model's, but only as a correction of what the learner said:
//! it rewrites each answer with its mistakes fixed, and `fix_from` compares
//! the two, so the phrase shown as said is always one the learner said, word
//! for word.

use crate::domain::{ChoreRecap, Fix};

/// A talk needs this many answers before it has notes…
pub const NOTES_MIN_ANSWERS: usize = 3;
/// …and this many words across them.
pub const NOTES_MIN_WORDS: usize = 12;
/// At most this many answers go to the model for correcting, the longest.
/// Its answer is as long as they are, and on a laptop's CPU every word of it
/// is a moment the recap waits.
pub const MAX_CHECKED_ANSWERS: usize = 8;

/// Too little said for notes to say anything true.
pub fn too_short(answers: &[&str]) -> bool {
    let words: usize = answers.iter().map(|answer| answer.split_whitespace().count()).sum();
    answers.len() < NOTES_MIN_ANSWERS || words < NOTES_MIN_WORDS
}

/// The answers worth correcting: three words or more, the longest few, in
/// the order they were said.
pub fn answers_to_check<'a>(answers: &[&'a str]) -> Vec<&'a str> {
    let mut long: Vec<(usize, &str)> = answers
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, answer)| answer.split_whitespace().count() >= 3)
        .collect();
    if long.len() > MAX_CHECKED_ANSWERS {
        long.sort_by_key(|(index, answer)| (std::cmp::Reverse(answer.split_whitespace().count()), *index));
        long.truncate(MAX_CHECKED_ANSWERS);
        long.sort_by_key(|(index, _)| *index);
    }
    long.into_iter().map(|(_, answer)| answer).collect()
}

/// Two things that went well, the first two of these the learner's words
/// show, in this order. The last two always hold, so there are always two.
pub fn went_well(answers: &[&str], chore: Option<&ChoreRecap>) -> Vec<String> {
    let lines: Vec<Vec<String>> = answers.iter().map(|answer| words_of(answer)).collect();
    let all: Vec<&str> = lines.iter().flatten().map(String::as_str).collect();
    let has = |wanted: &[&str]| all.iter().any(|word| wanted.contains(word));
    let mut lengths: Vec<usize> = lines.iter().map(Vec::len).collect();
    lengths.sort_unstable();
    let median = lengths.get(lengths.len() / 2).copied().unwrap_or(0);

    let candidates = [
        (chore.is_some_and(|chore| chore.met), "Got the deal"),
        (chore.is_some() && answers.iter().any(|answer| names_a_figure(answer)), "Made an offer"),
        (has(&["because", "since"]) || says(&all, &["so", "that"]) || says(&all, &["thats", "why"]), "Gave reasons"),
        (all.iter().filter(|word| is_past(word)).count() >= 2, "Told what happened"),
        (answers.iter().any(|answer| asks(answer)), "Asked questions"),
        (has(&["please", "thank", "thanks", "sorry", "excuse", "bhaiya", "ji", "sir", "madam"]), "Stayed polite"),
        (median >= 6, "Full sentences"),
        (answers.len() >= 5, "Kept it going"),
        (lines.first().is_some_and(|first| greets(first)), "Warm greeting"),
        (true, "Answered each question"),
        (true, "Gave it a go"),
    ];
    candidates
        .iter()
        .filter(|(shown, _)| *shown)
        .take(2)
        .map(|(_, line)| (*line).to_owned())
        .collect()
}

/// The one fix, out of the model's corrected answers. Each corrected line is
/// matched to the answer it is closest to rather than by position, because
/// the model sometimes repeats an answer before correcting it. A line that
/// was rewritten rather than corrected, or corrected in more than a few words
/// at once, is left alone. Of what is left, a changed word is preferred to
/// one only added or dropped, and then the earliest.
pub fn fix_from(answers: &[&str], corrected: &[String]) -> Option<Fix> {
    let said: Vec<Vec<Token>> = answers.iter().map(|answer| tokens_of(answer)).collect();
    let mut found: Vec<((bool, usize, usize), Fix)> = Vec::new();
    for line in corrected {
        let fixed = tokens_of(line);
        let Some((index, _)) = said
            .iter()
            .enumerate()
            .map(|(index, answer)| (index, similarity(answer, &fixed)))
            .filter(|(_, similarity)| *similarity >= 0.6)
            .max_by(|a, b| a.1.total_cmp(&b.1))
        else {
            continue;
        };
        let answer = &said[index];
        let segments = diff(answer, &fixed);
        let changed: usize = segments
            .iter()
            .filter(|segment| !segment.equal)
            .map(|segment| segment.said.len() + segment.fixed.len())
            .sum();
        if changed == 0 || changed * 2 > answer.len().max(4) {
            continue;
        }
        for (position, segment) in segments.iter().enumerate() {
            if segment.equal || segment.said.len() > 3 || segment.fixed.len() > 3 {
                continue;
            }
            if let Some(fix) = phrase_around(answer, &fixed, &segments, position) {
                let added_or_dropped = segment.said.is_empty() || segment.fixed.is_empty();
                found.push(((added_or_dropped, index, segment.said.start), fix));
            }
        }
    }
    found.into_iter().min_by_key(|(rank, _)| *rank).map(|(_, fix)| fix)
}

/// A word as said, and as compared: lower case, apostrophes dropped, and
/// anything but letters and digits left out.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    key: String,
}

fn tokens_of(text: &str) -> Vec<Token> {
    text.split_whitespace()
        .filter_map(|word| {
            let key: String = word
                .to_lowercase()
                .chars()
                .filter(|character| character.is_alphanumeric())
                .collect();
            (!key.is_empty()).then(|| Token { text: word.to_owned(), key })
        })
        .collect()
}

fn words_of(text: &str) -> Vec<String> {
    tokens_of(text).into_iter().map(|token| token.key).collect()
}

/// A run of one side's words: equal on both sides, or a change.
#[derive(Debug, Clone)]
struct Segment {
    equal: bool,
    said: std::ops::Range<usize>,
    fixed: std::ops::Range<usize>,
}

/// The longest common run of words, as a table: `table[i][j]` is its length
/// for `a[i..]` and `b[j..]`.
fn common_table(a: &[Token], b: &[Token]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i].key == b[j].key {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

fn similarity(a: &[Token], b: &[Token]) -> f64 {
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    common_table(a, b)[0][0] as f64 / longest as f64
}

/// `a` and `b` as alternating runs of equal words and changes.
fn diff(a: &[Token], b: &[Token]) -> Vec<Segment> {
    let table = common_table(a, b);
    let mut segments: Vec<Segment> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut push = |equal: bool, i: usize, j: usize, di: usize, dj: usize| match segments.last_mut() {
        Some(last) if last.equal == equal => {
            last.said.end += di;
            last.fixed.end += dj;
        }
        _ => segments.push(Segment { equal, said: i..i + di, fixed: j..j + dj }),
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i].key == b[j].key {
            push(true, i, j, 1, 1);
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || table[i][j + 1] >= table[i + 1][j]) {
            push(false, i, j, 0, 1);
            j += 1;
        } else {
            push(false, i, j, 1, 0);
            i += 1;
        }
    }
    segments
}

/// The change at `position`, with a few of the learner's words either side so
/// it reads as a phrase: up to two before and three after, stopping at the end
/// of a sentence, and without a joining word left hanging at either end. The
/// words around it are the learner's on both sides, so only the fix differs.
fn phrase_around(said: &[Token], fixed: &[Token], segments: &[Segment], position: usize) -> Option<Fix> {
    const JOINING: &[&str] = &["and", "but", "so", "or", "then", "because"];
    const HANGING: &[&str] = &[
        "and", "but", "so", "or", "then", "because", "with", "to", "the", "a", "an", "of", "in",
        "on", "at", "for", "my", "your", "his", "her", "our", "their", "is", "was",
    ];
    let change = &segments[position];
    let ends_sentence = |token: &Token| token.text.ends_with(['.', '?', '!']);

    let mut before: Vec<&Token> = Vec::new();
    if let Some(previous) = position.checked_sub(1).map(|index| &segments[index]) {
        for token in said[previous.said.clone()].iter().rev().take(2) {
            if ends_sentence(token) {
                break;
            }
            before.insert(0, token);
        }
    }
    while before.first().is_some_and(|token| JOINING.contains(&token.key.as_str())) {
        before.remove(0);
    }

    let mut after: Vec<&Token> = Vec::new();
    let last_changed = said[change.said.clone()].last().or_else(|| fixed[change.fixed.clone()].last());
    if !last_changed.is_some_and(ends_sentence) {
        if let Some(next) = segments.get(position + 1) {
            for token in said[next.said.clone()].iter().take(3) {
                after.push(token);
                if ends_sentence(token) {
                    break;
                }
            }
        }
    }
    while after.last().is_some_and(|token| HANGING.contains(&token.key.as_str())) {
        after.pop();
    }

    let join = |middle: &[Token]| {
        let words: Vec<&str> = before
            .iter()
            .copied()
            .chain(middle.iter())
            .chain(after.iter().copied())
            .map(|token| token.text.as_str())
            .collect();
        tidy(&words.join(" "))
    };
    let fix = Fix {
        said: join(&said[change.said.clone()]),
        better: join(&fixed[change.fixed.clone()]),
    };
    // Two words at least on the learner's side, so it reads as something they
    // said; and a fix that only moves punctuation is not one.
    (fix.said.split_whitespace().count() >= 2 && words_of(&fix.said) != words_of(&fix.better)).then_some(fix)
}

/// No punctuation at either end, and a capital to start.
fn tidy(phrase: &str) -> String {
    let trimmed = phrase.trim_matches(|character: char| !character.is_alphanumeric());
    let mut characters = trimmed.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

fn says(words: &[&str], phrase: &[&str]) -> bool {
    words.windows(phrase.len()).any(|window| window == phrase)
}

fn names_a_figure(answer: &str) -> bool {
    let words = words_of(answer);
    words.iter().any(|word| word.len() >= 2 && word.chars().all(|character| character.is_ascii_digit()))
        || words.iter().any(|word| matches!(word.as_str(), "hundred" | "thousand"))
}

fn asks(answer: &str) -> bool {
    const OPENERS: &[&str] = &[
        "what", "where", "when", "why", "how", "who", "which", "whose", "can", "could", "would",
        "will", "shall", "may", "do", "does", "did", "is", "are",
    ];
    answer.contains('?') || words_of(answer).first().is_some_and(|first| OPENERS.contains(&first.as_str()))
}

fn greets(first_answer: &[String]) -> bool {
    match first_answer.first().map(String::as_str) {
        Some("hi" | "hello" | "hey" | "namaste" | "namaskar") => true,
        Some("good") => matches!(
            first_answer.get(1).map(String::as_str),
            Some("morning" | "afternoon" | "evening")
        ),
        _ => false,
    }
}

/// A word that puts a sentence in the past. Words ending in "-ed" count, bar
/// the ones that only look it and the feelings ("I am tired") that are not
/// telling what happened.
fn is_past(word: &str) -> bool {
    const IRREGULAR: &[&str] = &[
        "was", "were", "went", "had", "did", "saw", "ate", "made", "got", "came", "took", "bought",
        "told", "said", "gave", "found", "thought", "felt", "left", "met", "ran", "won", "lost",
        "paid", "sat", "slept", "spoke", "wrote", "drove", "flew", "brought", "caught", "taught",
        "knew", "became", "began", "drank", "forgot", "heard", "kept", "sent", "stood", "swam",
        "threw", "woke", "wore", "sold", "sang", "understood", "spent", "built", "broke", "chose",
        "fell", "fought", "hid", "rode", "shook", "meant", "lent",
    ];
    const NOT_PAST: &[&str] = &[
        "need", "feed", "seed", "speed", "weed", "breed", "bleed", "greed", "indeed", "proceed",
        "exceed", "succeed", "bed", "red", "shed", "hundred", "sacred", "naked", "wicked",
        "tired", "excited", "interested", "bored", "scared", "worried", "married", "confused",
        "surprised", "amazed", "pleased", "embarrassed",
    ];
    IRREGULAR.contains(&word) || (word.len() >= 4 && word.ends_with("ed") && !NOT_PAST.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Direction;

    fn owned(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    fn fix(said: &str, better: &str) -> Option<Fix> {
        Some(Fix { said: said.into(), better: better.into() })
    }

    #[test]
    fn a_talk_needs_three_answers_and_twelve_words_for_notes() {
        assert!(too_short(&["yes", "samosa", "good. I like"]));
        assert!(too_short(&["I went to the market with my mother", "and bought some pani puri"]));
        assert!(!too_short(&["I went to the market", "with my mother and sister", "we bought pani puri"]));
    }

    #[test]
    fn only_longer_answers_are_checked_and_at_most_eight_of_them() {
        assert_eq!(answers_to_check(&["yes", "I like it", "ok"]), vec!["I like it"]);
        let many: Vec<String> = (1..=10).map(|n| format!("answer {}", "word ".repeat(n))).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let checked = answers_to_check(&many);
        assert_eq!(checked.len(), MAX_CHECKED_ANSWERS);
        // The two shortest went, and the rest kept their order.
        assert_eq!(checked, many[2..].to_vec());
    }

    #[test]
    fn a_corrected_verb_is_the_fix_with_the_words_around_it() {
        let answers = ["Yesterday I go to market with my mother", "It was very spicy and tangy"];
        let corrected = owned(&["Yesterday I went to market with my mother.", "It was very spicy and tangy."]);
        assert_eq!(fix_from(&answers, &corrected), fix("Yesterday I go to market", "Yesterday I went to market"));
    }

    #[test]
    fn a_repeated_answer_before_its_correction_still_finds_it() {
        // The model's answer for the talk the prompt was tried on.
        let answers = [
            "Yesterday I go to market with my mother and we buyed pani puri",
            "It was very spicy and tangy. I eated six plates",
            "Yes, I will go again on Sunday because it is near to my house",
        ];
        let corrected = owned(&[
            "Yesterday I go to market with my mother and we buyed pani puri",
            "Yesterday I went to market with my mother and we bought pani puri",
            "It was very spicy and tangy. I eated six plates",
        ]);
        assert_eq!(fix_from(&answers, &corrected), fix("Yesterday I go to market", "Yesterday I went to market"));
    }

    #[test]
    fn a_changed_word_is_preferred_to_an_added_one() {
        let answers = ["My school is very big and it have a big playground", "Rahul is best player in our class"];
        let corrected = owned(&[
            "My school is very big and it has a big playground.",
            "Rahul is the best player in our class.",
        ]);
        assert_eq!(fix_from(&answers, &corrected), fix("It have a big playground", "It has a big playground"));

        let only_added = owned(&["My school is very big and it have a big playground", "Rahul is the best player in our class"]);
        assert_eq!(fix_from(&answers, &only_added), fix("Rahul is best player", "Rahul is the best player"));
    }

    #[test]
    fn a_phrase_stops_at_the_end_of_its_sentence() {
        let answers = ["Where is bus stop? I am not knowing this area"];
        let corrected = owned(&["Where is the bus stop? I am not knowing this area."]);
        assert_eq!(fix_from(&answers, &corrected), fix("Where is bus stop", "Where is the bus stop"));
    }

    #[test]
    fn nothing_changed_or_only_punctuation_is_no_fix() {
        let answers = ["I need to get to the railway station, and I'm in a bit of a hurry"];
        assert_eq!(fix_from(&answers, &owned(&["I need to get to the railway station and I'm in a bit of a hurry."])), None);
        assert_eq!(fix_from(&answers, &owned(&["I NEED TO GET TO THE RAILWAY STATION, AND I'M IN A BIT OF A HURRY"])), None);
    }

    #[test]
    fn a_rewrite_is_not_a_fix() {
        let answers = ["600 is too much. Give less price please"];
        let corrected = owned(&["Could you please lower the price a little?"]);
        assert_eq!(fix_from(&answers, &corrected), None);
        // Nor is a line that matches no answer at all.
        assert_eq!(fix_from(&answers, &owned(&["Did you eat anything today?"])), None);
    }

    #[test]
    fn a_fix_is_two_words_at_least_on_the_learners_side() {
        let answers = ["yes", "samosa is good"];
        assert_eq!(fix_from(&answers, &owned(&["Yes.", "Samosa is good."])), None);
    }

    #[test]
    fn what_went_well_is_what_the_words_show() {
        assert_eq!(
            went_well(
                &["Yesterday I went to market and bought pani puri", "It was spicy because of the chutney", "I will go again"],
                None,
            ),
            vec!["Gave reasons", "Told what happened"],
        );
        assert_eq!(
            went_well(&["Where is the bus stop?", "Thank you so much", "Okay, left at the bank"], None),
            vec!["Asked questions", "Stayed polite"],
        );
        // Nothing to single out still says something true.
        assert_eq!(went_well(&["yes", "samosa", "good I like"], None), vec!["Answered each question", "Gave it a go"]);
        // Feelings in "-ed" are not a story.
        assert!(!went_well(&["I am tired and excited", "I am bored", "I am worried"], None).contains(&"Told what happened".to_owned()));
    }

    #[test]
    fn a_chore_praises_the_deal_and_the_offer() {
        let recap = ChoreRecap {
            chore_id: "market-cloth-price".into(),
            character_id: "stall-owner".into(),
            unit: "Rs".into(),
            direction: Direction::Down,
            target: 400,
            figure: 380,
            agreed: true,
            met: true,
            times_met: 1,
            last_line: None,
        };
        let answers = ["How much for this shirt", "I will give 350 please", "Ok 380 then"];
        assert_eq!(went_well(&answers, Some(&recap)), vec!["Got the deal", "Made an offer"]);
        let missed = ChoreRecap { met: false, ..recap };
        assert_eq!(went_well(&answers, Some(&missed)), vec!["Made an offer", "Asked questions"]);
        // A number said in a free talk is not an offer.
        assert!(!went_well(&answers, None).contains(&"Made an offer".to_owned()));
    }
}
