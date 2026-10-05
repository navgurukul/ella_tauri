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
//! for word, and the fix is one they could hear.

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
/// the model sometimes repeats an answer before correcting it. A change that
/// only writes the same words another way is no change at all
/// (`sounds_the_same`). A line that was rewritten rather than corrected, or
/// corrected in more than a few words at once, is left alone, and so is a
/// change no grammar fix makes (`fixes_grammar`). Of what is left, a changed
/// word is preferred to one only added or dropped, and then the earliest.
pub fn fix_from(answers: &[&str], corrected: &[String]) -> Option<Fix> {
    let said: Vec<Vec<Token>> = answers.iter().map(|answer| tokens_of(answer)).collect();
    let mut found: Vec<((bool, usize, usize), Fix)> = Vec::new();
    for line in corrected {
        let fixed = tokens_of(line);
        let Some((index, segments, _)) = said
            .iter()
            .enumerate()
            .map(|(index, answer)| {
                let segments = heard_alike(answer, &fixed, diff(answer, &fixed));
                let similarity = similarity(answer, &fixed, &segments);
                (index, segments, similarity)
            })
            .filter(|(_, _, similarity)| *similarity >= 0.6)
            .max_by(|a, b| a.2.total_cmp(&b.2))
        else {
            continue;
        };
        let answer = &said[index];
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
            let next = answer.get(segment.said.end);
            if !fixes_grammar(&answer[segment.said.clone()], &fixed[segment.fixed.clone()], next) {
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

/// How much of `a` and `b` their equal `segments` cover, as a share of the
/// side it covers least.
fn similarity(a: &[Token], b: &[Token], segments: &[Segment]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (same_a, same_b) = segments
        .iter()
        .filter(|segment| segment.equal)
        .fold((0, 0), |(same_a, same_b), segment| (same_a + segment.said.len(), same_b + segment.fixed.len()));
    (same_a as f64 / a.len() as f64).min(same_b as f64 / b.len() as f64)
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

/// `segments` with every change that `sounds_the_same` counted as equal, and
/// joined to the equal runs either side of it.
fn heard_alike(said: &[Token], fixed: &[Token], segments: Vec<Segment>) -> Vec<Segment> {
    let mut joined: Vec<Segment> = Vec::new();
    for segment in segments {
        let equal = segment.equal || sounds_the_same(&said[segment.said.clone()], &fixed[segment.fixed.clone()]);
        match joined.last_mut() {
            Some(last) if last.equal && equal => {
                last.said.end = segment.said.end;
                last.fixed.end = segment.fixed.end;
            }
            _ => joined.push(Segment { equal, ..segment }),
        }
    }
    joined
}

/// Whether `said` and `fixed` are the same words written down another way.
/// The answers were heard, not written, so how a word is spelled is the
/// speech recognizer's choice and never the learner's: a "fix" that only
/// respells it shows the learner their own words crossed out and then again
/// as the fix. A recap on Windows did exactly that, its two lines a space or
/// a letter apart ("everyday" and "every day").
///
/// So these are the same: words joined, split or hyphenated; British and
/// American spellings; words said alike ("there" and "their"); a number in
/// figures or in words; a contraction and the words it stands for, and a
/// casual "gonna" and "going to", which are both right when spoken; and an
/// "um" left out.
fn sounds_the_same(said: &[Token], fixed: &[Token]) -> bool {
    // Longer changes are rewrites, never respellings, and the ways to say a
    // run of words multiply with its length.
    if said.len() > 6 || fixed.len() > 6 {
        return false;
    }
    let fixed = spoken_forms(fixed);
    spoken_forms(said).iter().any(|form| fixed.contains(form))
}

/// Every way `tokens` could be said, each written as its words' sounds with
/// the spaces left out, so "every day" and "everyday" come out the same.
fn spoken_forms(tokens: &[Token]) -> Vec<String> {
    let words = numbers_as_figures(tokens.iter().flat_map(|token| written_words(&token.text)).collect());
    let mut forms = vec![String::new()];
    for word in &words {
        let ways = ways_to_say(word);
        forms = forms
            .iter()
            .flat_map(|form| ways.iter().map(move |way| format!("{form}{way}")))
            .collect();
        forms.sort_unstable();
        forms.dedup();
    }
    forms
}

/// A token's words in lower case, with its apostrophes kept so a contraction
/// can be told from a word, and split where it is hyphenated.
fn written_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace(['’', '‘', '`'], "'")
        .replace('%', " percent")
        .replace('&', " and ")
        .split(|character: char| character.is_whitespace() || matches!(character, '-' | '–' | '—' | '/'))
        .map(|piece| {
            let kept: String = piece
                .chars()
                .filter(|character| character.is_alphanumeric() || *character == '\'')
                .collect();
            kept.trim_matches('\'').to_owned()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// `words` with each number said in words written in figures instead:
/// "twenty five" as "25", "one hundred and fifty" as "150", and "five fifty",
/// as a price or a year is said, as "550".
fn numbers_as_figures(words: Vec<String>) -> Vec<String> {
    let mut figures: Vec<String> = Vec::new();
    // The number being read: thousands and up, and the part below them.
    let mut number: Option<(u64, u64)> = None;
    let mut after_scale = false;
    for (index, word) in words.iter().enumerate() {
        let next_is_number = words.get(index + 1).is_some_and(|next| number_word(next).is_some());
        match (number_word(word), number.as_mut()) {
            (Some(NumberWord::Value(value)), None) => number = Some((0, value)),
            (Some(NumberWord::Value(value)), Some((total, part))) => {
                if *part % 100 == 0 || (value < 10 && *part % 10 == 0) {
                    // "twenty five", "hundred and fifty", "thousand five"
                    *part += value;
                } else if value >= 10 {
                    // "nineteen ninety", "five fifty"
                    *part = *part * 100 + value;
                } else {
                    // "one two" is two numbers.
                    figures.push((*total + *part).to_string());
                    number = Some((0, value));
                }
            }
            (Some(NumberWord::Scale(100)), _) => {
                let (_, part) = number.get_or_insert((0, 0));
                *part = (*part).max(1) * 100;
            }
            (Some(NumberWord::Scale(scale)), _) => {
                let (total, part) = number.get_or_insert((0, 0));
                *total += (*part).max(1) * scale;
                *part = 0;
            }
            (None, Some(_)) if word == "and" && after_scale && next_is_number => {}
            (None, _) => {
                if let Some((total, part)) = number.take() {
                    figures.push((total + part).to_string());
                }
                figures.push(word.clone());
            }
        }
        after_scale = matches!(number_word(word), Some(NumberWord::Scale(_)));
    }
    if let Some((total, part)) = number {
        figures.push((total + part).to_string());
    }
    figures
}

enum NumberWord {
    Value(u64),
    Scale(u64),
}

fn number_word(word: &str) -> Option<NumberWord> {
    const VALUES: &[&str] = &[
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: &[&str] = &["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    if let Some(value) = VALUES.iter().position(|value| *value == word) {
        return Some(NumberWord::Value(value as u64));
    }
    if let Some(tens) = TENS.iter().position(|tens| *tens == word) {
        return Some(NumberWord::Value(20 + 10 * tens as u64));
    }
    match word {
        "hundred" => Some(NumberWord::Scale(100)),
        "thousand" => Some(NumberWord::Scale(1_000)),
        "lakh" | "lakhs" => Some(NumberWord::Scale(100_000)),
        "million" => Some(NumberWord::Scale(1_000_000)),
        "crore" | "crores" => Some(NumberWord::Scale(10_000_000)),
        _ => None,
    }
}

/// The ways one word could be said: its sound (`sound_of`), and, for a
/// contraction, or a word said like one, or a casual "gonna", the words it
/// stands for. Nothing at all for an "um".
fn ways_to_say(word: &str) -> Vec<String> {
    const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "er", "erm", "ah", "hmm", "hm", "mm", "mmm"];
    if FILLERS.contains(&word) {
        return vec![String::new()];
    }
    let mut ways = vec![sound_of(word)];
    let key = key_of(word);
    let alike = SAID_ALIKE
        .iter()
        .find(|group| group.iter().any(|member| key_of(member) == key))
        .copied()
        .unwrap_or_default();
    for written in alike.iter().copied().chain([word]) {
        let stands_for = CONTRACTIONS
            .iter()
            .chain(CASUAL)
            .filter(|(short, _)| *short == written)
            .flat_map(|(_, full)| full.iter());
        for full in stands_for {
            ways.push(full.split(' ').map(sound_of).collect());
        }
    }
    ways.sort_unstable();
    ways.dedup();
    ways
}

/// A word's sound as far as these comparisons need it: spelled the American
/// way, and as the first of the words said alike with it.
fn sound_of(word: &str) -> String {
    let spelled = american(&key_of(word));
    SAID_ALIKE
        .iter()
        .find(|group| group.iter().any(|member| key_of(member) == spelled))
        .map_or(spelled, |group| key_of(group[0]))
}

fn key_of(word: &str) -> String {
    word.chars().filter(|character| character.is_alphanumeric()).collect()
}

/// A word spelled one way in Britain and India and another in America,
/// spelled the American way, so "colour" and "color" are one word.
fn american(word: &str) -> String {
    const PAIRS: &[(&str, &str)] = &[
        ("grey", "gray"), ("cheque", "check"), ("cheques", "checks"), ("tyre", "tire"), ("tyres", "tires"),
        ("programme", "program"), ("programmes", "programs"), ("practise", "practice"),
        ("practised", "practiced"), ("practising", "practicing"), ("jewellery", "jewelry"),
        ("ageing", "aging"), ("judgement", "judgment"), ("mould", "mold"), ("pyjamas", "pajamas"),
        ("plough", "plow"), ("storey", "story"), ("kerb", "curb"), ("defence", "defense"),
        ("offence", "offense"), ("licence", "license"), ("enrol", "enroll"), ("fulfil", "fulfill"),
        ("skilful", "skillful"), ("doughnut", "donut"), ("moustache", "mustache"),
        ("fuelled", "fueled"), ("fuelling", "fueling"), ("dialled", "dialed"), ("dialling", "dialing"),
    ];
    // The British ending, the American one, and how long the rest of the word
    // must be: long enough that no other word ends the same way ("our",
    // "four" and "hour" keep their spelling).
    const ENDINGS: &[(&str, &str, usize)] = &[
        ("ourites", "orites", 3), ("ourite", "orite", 3), ("ourhood", "orhood", 3), ("ourable", "orable", 3),
        ("ouring", "oring", 3), ("ourful", "orful", 3), ("oured", "ored", 3), ("ours", "ors", 3), ("our", "or", 3),
        ("isations", "izations", 4), ("isation", "ization", 4), ("ising", "izing", 4), ("ised", "ized", 4),
        ("isers", "izers", 4), ("iser", "izer", 4), ("ises", "izes", 4), ("ise", "ize", 4),
        ("ysing", "yzing", 3), ("ysed", "yzed", 3), ("yse", "yze", 3),
        ("tres", "ters", 2), ("tred", "tered", 2), ("tre", "ter", 2), ("bres", "bers", 2), ("bre", "ber", 2),
        ("ogues", "ogs", 3), ("ogue", "og", 3),
        ("elling", "eling", 3), ("ellers", "elers", 3), ("ellors", "elors", 3), ("elled", "eled", 3),
        ("eller", "eler", 3), ("ellor", "elor", 3), ("alling", "aling", 3), ("alled", "aled", 3),
    ];
    if let Some((_, us)) = PAIRS.iter().find(|(british, _)| *british == word) {
        return (*us).to_owned();
    }
    for (british, us, stem) in ENDINGS {
        if let Some(rest) = word.strip_suffix(british) {
            if rest.chars().count() >= *stem {
                return format!("{rest}{us}");
            }
        }
    }
    word.to_owned()
}

/// Words said alike, or as nearly alike as makes no difference, the first of
/// each as the one they are all compared as. Only those a learner says the
/// same: "then" and "than", or "lose" and "loose", stay apart, since a
/// learner can say the wrong one.
const SAID_ALIKE: &[&[&str]] = &[
    &["there", "their", "they're"], &["your", "you're"], &["its", "it's"], &["whose", "who's"],
    &["lets", "let's"], &["theirs", "there's"], &["to", "too", "two", "2"], &["for", "four", "fore", "4"],
    &["one", "won", "1"], &["eight", "ate", "8"], &["know", "no"], &["knew", "new"], &["here", "hear"],
    &["by", "buy", "bye"], &["right", "write", "rite"], &["weather", "whether"], &["piece", "peace"],
    &["week", "weak"], &["wait", "weight"], &["where", "wear", "ware"], &["see", "sea"], &["son", "sun"],
    &["meet", "meat"], &["would", "wood"], &["our", "hour"], &["i", "eye"], &["flower", "flour"],
    &["made", "maid"], &["mail", "male"], &["pair", "pear", "pare"], &["plane", "plain"],
    &["road", "rode", "rowed"], &["sale", "sail"], &["tale", "tail"], &["whole", "hole"],
    &["which", "witch"], &["through", "threw"], &["break", "brake"], &["sell", "cell"],
    &["sent", "cent", "scent"], &["dear", "deer"], &["fair", "fare"], &["hi", "high"],
    &["higher", "hire"], &["night", "knight"], &["not", "knot"], &["past", "passed"], &["red", "read"],
    &["seen", "scene"], &["some", "sum"], &["steal", "steel"], &["sweet", "suite"], &["tea", "tee"],
    &["waste", "waist"], &["way", "weigh"], &["aloud", "allowed"], &["bare", "bear"], &["blue", "blew"],
    &["board", "bored"], &["course", "coarse"], &["due", "dew"], &["guest", "guessed"], &["heard", "herd"],
    &["missed", "mist"], &["morning", "mourning"], &["principal", "principle"], &["rain", "reign", "rein"],
    &["real", "reel"], &["role", "roll"], &["soul", "sole"], &["stair", "stare"],
    &["stationery", "stationary"], &["toe", "tow"], &["war", "wore"], &["so", "sew", "sow"],
    &["oh", "owe"], &["be", "bee"], &["ok", "okay"], &["allright", "alright"], &["doctor", "dr"],
    &["mister", "mr"], &["rupees", "rs"],
];

/// Contractions and the words they stand for. Both are right in speech, so
/// saying one as the other is not a mistake.
const CONTRACTIONS: &[(&str, &[&str])] = &[
    ("i'm", &["i am"]), ("i've", &["i have"]), ("i'll", &["i will"]), ("i'd", &["i would", "i had"]),
    ("you're", &["you are"]), ("you've", &["you have"]), ("you'll", &["you will"]),
    ("you'd", &["you would", "you had"]), ("he's", &["he is", "he has"]), ("he'll", &["he will"]),
    ("he'd", &["he would", "he had"]), ("she's", &["she is", "she has"]), ("she'll", &["she will"]),
    ("she'd", &["she would", "she had"]), ("it's", &["it is", "it has"]), ("it'll", &["it will"]),
    ("we're", &["we are"]), ("we've", &["we have"]), ("we'll", &["we will"]),
    ("we'd", &["we would", "we had"]), ("they're", &["they are"]), ("they've", &["they have"]),
    ("they'll", &["they will"]), ("they'd", &["they would", "they had"]),
    ("that's", &["that is", "that has"]), ("there's", &["there is", "there has"]),
    ("here's", &["here is"]), ("what's", &["what is", "what has"]), ("where's", &["where is"]),
    ("who's", &["who is", "who has"]), ("how's", &["how is"]), ("let's", &["let us"]),
    ("isn't", &["is not"]), ("aren't", &["are not"]), ("wasn't", &["was not"]), ("weren't", &["were not"]),
    ("don't", &["do not"]), ("doesn't", &["does not"]), ("didn't", &["did not"]),
    ("haven't", &["have not"]), ("hasn't", &["has not"]), ("hadn't", &["had not"]),
    ("can't", &["cannot"]), ("couldn't", &["could not"]), ("won't", &["will not"]),
    ("wouldn't", &["would not"]), ("shouldn't", &["should not"]), ("mustn't", &["must not"]),
    ("needn't", &["need not"]), ("shan't", &["shall not"]),
];

/// The casual way of saying a few words, as right in speech as the careful
/// one.
const CASUAL: &[(&str, &[&str])] = &[
    ("gonna", &["going to"]), ("wanna", &["want to"]), ("gotta", &["got to"]), ("kinda", &["kind of"]),
    ("sorta", &["sort of"]), ("lemme", &["let me"]), ("gimme", &["give me"]), ("dunno", &["do not know"]),
    ("cuz", &["because"]), ("coz", &["because"]), ("cos", &["because"]),
];

/// Whether changing `said` to `fixed` is what a grammar fix does: a word put
/// in its right form ("go" to "went", "have" to "has", "brother" to
/// "brothers", "me" to "I", "a" to "an"), or small words only added ("the",
/// "is", "to", "of") or only dropped. Ella's 3B model, asked to fix only the
/// grammar, also swaps a right word for another: "give it for 350" came back
/// as "give it to 350", "their house" as "the house", "one small stall" as "a
/// small stall", and "I am from Kanpur only" lost its "only". On the recap
/// such a change reads as the learner's own words twice over, and a wrong fix
/// is worse than none, so it is never the one shown. `next` is the learner's
/// word after the change, for "since two years".
fn fixes_grammar(said: &[Token], fixed: &[Token], next: Option<&Token>) -> bool {
    let said = grammar_words(said);
    let fixed = grammar_words(fixed);
    // How long, not since when: "since two years" for "for two years".
    if said == ["since"] && fixed == ["for"] {
        return next.is_some_and(|word| says_how_long(&word.key));
    }
    let mut unpaired = said.clone();
    let mut added: Vec<&str> = Vec::new();
    for word in &fixed {
        match unpaired.iter().position(|said| same_word(said, word)) {
            Some(index) => {
                unpaired.remove(index);
            }
            None => added.push(word),
        }
    }
    let small = |word: &str| SMALL_WORDS.contains(&word);
    // A small word dropped and another put in its place is a swap.
    (unpaired.is_empty() || added.is_empty())
        && unpaired.iter().all(|word| small(word))
        && added.into_iter().all(small)
}

/// A run of tokens as words to compare forms of: contractions said in full,
/// so "I'm agree" is "I am agree", and apostrophes dropped.
fn grammar_words(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .flat_map(|token| written_words(&token.text))
        .flat_map(|word| match CONTRACTIONS.iter().find(|(short, _)| *short == word) {
            Some((_, full)) => full[0].replace("cannot", "can not").split(' ').map(str::to_owned).collect(),
            None => vec![key_of(&word)],
        })
        .collect()
}

fn says_how_long(word: &str) -> bool {
    number_word(word).is_some()
        || word.chars().all(|character| character.is_ascii_digit())
        || matches!(word, "a" | "an" | "the" | "many" | "few" | "several" | "some" | "long" | "last" | "past")
}

/// Two forms of one word: "went" and "go", "plays" and "play", "buyed" and
/// "bought", "children" and "child", "me" and "I".
fn same_word(a: &str, b: &str) -> bool {
    let a = lemmas(a);
    lemmas(b).iter().any(|lemma| a.contains(lemma))
}

/// What a word could be a form of: itself, itself without a regular ending
/// ("played", "plays", "playing", "studied", "stopped", "bigger", "quickly"),
/// and the first of any `FORMS` it or those belong to.
fn lemmas(word: &str) -> Vec<String> {
    const ENDINGS: &[(&str, &str)] = &[
        ("ies", "y"), ("ied", "y"), ("ily", "y"), ("es", ""), ("s", ""), ("ed", ""), ("ed", "e"),
        ("d", ""), ("ing", ""), ("ing", "e"), ("er", ""), ("er", "e"), ("est", ""), ("ly", ""),
    ];
    let irregular = |stem: &str| FORMS.iter().any(|forms| forms[0] == stem);
    let mut stems = vec![word.to_owned()];
    for (ending, replacement) in ENDINGS {
        let Some(rest) = word.strip_suffix(ending) else {
            continue;
        };
        // "-es" only where English adds it: "goes", "watches", "boxes".
        if *ending == "es" && !["s", "x", "z", "ch", "sh", "o"].iter().any(|end| rest.ends_with(end)) {
            continue;
        }
        let stem = format!("{rest}{replacement}");
        // "stopped", "running", "bigger"
        let mut last_two = stem.chars().rev().take(2);
        let undoubled = match (last_two.next(), last_two.next()) {
            (Some(last), Some(before)) if last == before && last.is_ascii_alphabetic() && !"aeiou".contains(last) => {
                Some(stem[..stem.len() - 1].to_owned())
            }
            _ => None,
        };
        for stem in [Some(stem), undoubled].into_iter().flatten() {
            if stem.chars().count() >= 3 || irregular(&stem) {
                stems.push(stem);
            }
        }
    }
    let mut lemmas = stems.clone();
    for stem in &stems {
        if let Some(forms) = FORMS.iter().find(|forms| forms.contains(&stem.as_str())) {
            lemmas.push(forms[0].to_owned());
        }
    }
    lemmas
}

/// Words a grammar fix adds or drops on their own: articles, the verbs that
/// carry tense and agreement, prepositions, pronouns and possessives, and
/// "there", "it", "more" and "most" ("is raining", "more better").
const SMALL_WORDS: &[&str] = &[
    "a", "an", "the", "am", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "shall", "should", "can", "could", "may", "might", "must",
    "to", "of", "in", "on", "at", "for", "with", "from", "about", "by", "into", "onto", "it", "there",
    "i", "you", "he", "she", "we", "they", "my", "your", "his", "her", "our", "their", "its", "more",
    "most",
];

/// The forms of words that do not take a regular ending, the first of each
/// as the word they are all forms of.
const FORMS: &[&[&str]] = &[
    &["be", "am", "is", "are", "was", "were", "been", "being"], &["have", "has", "had", "having"],
    &["do", "does", "did", "done", "doing"], &["will", "would"], &["can", "could"], &["shall", "should"],
    &["may", "might"], &["go", "goes", "went", "gone", "going"], &["come", "came"], &["get", "got", "gotten"],
    &["give", "gave", "given"], &["take", "took", "taken"], &["make", "made"], &["see", "saw", "seen"],
    &["say", "says", "said"], &["tell", "told"], &["think", "thought"], &["know", "knew", "known"],
    &["buy", "bought"], &["bring", "brought"], &["teach", "taught"], &["catch", "caught"],
    &["eat", "ate", "eaten"], &["drink", "drank", "drunk"], &["run", "ran"], &["swim", "swam", "swum"],
    &["begin", "began", "begun"], &["sing", "sang", "sung"], &["write", "wrote", "written"],
    &["speak", "spoke", "spoken"], &["break", "broke", "broken"], &["choose", "chose", "chosen"],
    &["drive", "drove", "driven"], &["ride", "rode", "ridden"], &["wake", "woke", "woken"],
    &["wear", "wore", "worn"], &["win", "won"], &["lose", "lost"], &["pay", "paid"], &["leave", "left"],
    &["feel", "felt"], &["keep", "kept"], &["sleep", "slept"], &["meet", "met"], &["sit", "sat"],
    &["stand", "stood"], &["understand", "understood"], &["find", "found"], &["hear", "heard"],
    &["grow", "grew", "grown"], &["throw", "threw", "thrown"], &["fly", "flew", "flown"],
    &["draw", "drew", "drawn"], &["fall", "fell", "fallen"], &["forget", "forgot", "forgotten"],
    &["hold", "held"], &["send", "sent"], &["spend", "spent"], &["build", "built"], &["lend", "lent"],
    &["mean", "meant"], &["sell", "sold"], &["fight", "fought"], &["hide", "hid", "hidden"],
    &["shake", "shook", "shaken"], &["steal", "stole", "stolen"], &["become", "became"],
    &["child", "children"], &["man", "men"], &["woman", "women"], &["person", "people"],
    &["foot", "feet"], &["tooth", "teeth"], &["mouse", "mice"], &["life", "lives"], &["wife", "wives"],
    &["knife", "knives"], &["leaf", "leaves"], &["good", "better", "best", "well"],
    &["bad", "worse", "worst"], &["many", "much", "more", "most"], &["little", "less", "least"],
    &["this", "these"], &["that", "those"], &["a", "an"], &["other", "another"],
    &["i", "me", "my", "mine", "myself"], &["he", "him", "his", "himself"],
    &["she", "her", "hers", "herself"], &["we", "us", "our", "ours", "ourselves"],
    &["they", "them", "their", "theirs", "themselves"], &["you", "your", "yours", "yourself"],
    &["it", "its", "itself"],
];

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
    fn the_same_words_written_another_way_are_no_fix() {
        // What the recap showed on Windows: the learner's own words crossed
        // out, and then again as the fix, a space or a letter apart.
        let respelled = [
            ("I play cricket with my friends everyday", "I play cricket with my friends every day."),
            ("My favourite food is biryani", "My favorite food is biryani."),
            ("We painted the walls in a bright colour", "We painted the walls in a bright color."),
            ("I realised that the shop was closed", "I realized that the shop was closed."),
            ("Last year we travelled to Goa by train", "Last year we traveled to Goa by train."),
            ("The bus stops near the city centre", "The bus stops near the city center."),
            ("After lunch we ate ice-cream together", "After lunch we ate ice cream together."),
            ("Sorry, I can not come tomorrow", "Sorry, I cannot come tomorrow."),
            ("I think their going to win today", "I think they're going to win today."),
            ("I have to brothers and one sister", "I have two brothers and one sister."),
            ("I don't no the way to the station", "I don't know the way to the station."),
            ("My brother is 25 years old now", "My brother is twenty five years old now."),
            ("I bought it for five fifty only", "I bought it for 550 only."),
            ("I'm going to the market with my mother", "I am going to the market with my mother."),
            ("We don't have school on Sunday", "We do not have school on Sunday."),
            ("I'm gonna watch a movie tonight", "I'm going to watch a movie tonight."),
            ("Then I um went back to my home", "Then I went back to my home."),
        ];
        for (said, fixed) in respelled {
            assert_eq!(fix_from(&[said], &owned(&[fixed])), None, "{said:?} -> {fixed:?}");
        }
    }

    #[test]
    fn a_fix_beside_a_respelling_is_still_found() {
        let answers = ["Yesterday I go to the theatre everyday"];
        let corrected = owned(&["Yesterday I went to the theater every day."]);
        assert_eq!(
            fix_from(&answers, &corrected),
            fix("Yesterday I go to the theatre", "Yesterday I went to the theatre"),
        );
    }

    #[test]
    fn a_word_said_differently_is_still_a_fix() {
        let answers = ["Yesterday we are very happy at the party"];
        let corrected = owned(&["Yesterday we were very happy at the party."]);
        assert_eq!(fix_from(&answers, &corrected), fix("Yesterday we are very happy", "Yesterday we were very happy"));

        let answers = ["Last week I know the answer already"];
        let corrected = owned(&["Last week I knew the answer already."]);
        assert_eq!(
            fix_from(&answers, &corrected),
            fix("Week I know the answer already", "Week I knew the answer already"),
        );

        let answers = ["I'm agree with you about that"];
        let corrected = owned(&["I agree with you about that."]);
        assert_eq!(fix_from(&answers, &corrected), fix("I'm agree with you", "I agree with you"));
    }

    #[test]
    fn a_right_word_swapped_for_another_is_no_fix() {
        // What Ella's 3B model sent back for answers that needed nothing.
        let swapped = [
            ("Can you give it for 350?", "Can you give it to 350?"),
            ("I eat it near my office, there is one small stall.", "I eat it near my office, there is a small stall."),
            ("Their house is near the railway station.", "The house is near the railway station."),
            ("I am from Kanpur only and I am studying in 10th class.", "I am from Kanpur and I am studying in 10th class."),
        ];
        for (said, fixed) in swapped {
            assert_eq!(fix_from(&[said], &owned(&[fixed])), None, "{said:?} -> {fixed:?}");
        }
    }

    #[test]
    fn a_word_put_right_or_a_small_word_added_or_dropped_is_a_fix() {
        let fixes = [
            ("My cousin and me play cricket in the colony.", "My cousin and I play cricket in the colony.",
             "Cousin and me play cricket", "Cousin and I play cricket"),
            ("I have three years experience in web development.", "I have three years of experience in web development.",
             "Three years experience in web", "Three years of experience in web"),
            ("I am working in Pune since two years.", "I am working in Pune for two years.",
             "In Pune since two years", "In Pune for two years"),
            ("She don't like spicy food at all.", "She doesn't like spicy food at all.",
             "She don't like spicy food", "She doesn't like spicy food"),
            ("Yesterday we buyed a new phone for my father.", "Yesterday we bought a new phone for my father.",
             "Yesterday we buyed a new phone", "Yesterday we bought a new phone"),
            ("He is having two cars and one bike.", "He has two cars and one bike.",
             "He is having two cars", "He has two cars"),
        ];
        for (said, fixed, said_part, better) in fixes {
            assert_eq!(fix_from(&[said], &owned(&[fixed])), fix(said_part, better), "{said:?} -> {fixed:?}");
        }
        // "since" is "for" only before how long.
        assert_eq!(fix_from(&["I am living here since my birth."], &owned(&["I am living here for my birth."])), None);
    }

    #[test]
    fn numbers_said_in_words_are_read_as_figures() {
        let figures = |words: &str| numbers_as_figures(words.split(' ').map(str::to_owned).collect());
        assert_eq!(figures("twenty five"), vec!["25"]);
        assert_eq!(figures("one hundred and fifty rupees"), vec!["150", "rupees"]);
        assert_eq!(figures("two thousand twenty six"), vec!["2026"]);
        assert_eq!(figures("nineteen ninety nine"), vec!["1999"]);
        assert_eq!(figures("two lakh fifty thousand"), vec!["250000"]);
        assert_eq!(figures("five fifty"), vec!["550"]);
        assert_eq!(figures("one two three"), vec!["1", "2", "3"]);
        assert_eq!(figures("bread and butter"), vec!["bread", "and", "butter"]);
        assert_eq!(figures("five and six"), vec!["5", "and", "6"]);
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
