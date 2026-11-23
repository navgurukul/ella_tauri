//! A learner's words: the English words they have said aloud in their talks,
//! for Profile's "My words" card and the recap's "New words" tile. Ella
//! Mobile's `vocab.ts`, rule for rule, but for what the laptop leaves out
//! (below).
//!
//! - Only spoken answers count; typed ones never do, as with levels.
//! - A word counts only when it is in the English word list
//!   (`shared/english_words.txt`, Mobile's own), which leaves out names,
//!   Hindi, abbreviations and most mishearings; it is not a little word
//!   ([`LITTLE_WORDS`]), a swear word, or the learner's own name; and Ella did
//!   not say it in her line just before (an echo). Word of the day's word is
//!   the exception in its own talk: saying it there is the point.
//! - Forms of a word count as one ("go", "going", "went"). Learners see the
//!   form they said most.
//! - A word is almost theirs once said in a talk, and theirs once said in two
//!   talks on two topics, the bar a skill needs. Word of the day talks all
//!   share one topic ([`WORD_TOPIC`]), so two of them make one topic, not two.
//! - Placement talks count too: what was said there is the learner's English.
//!
//! Nothing is kept: every view is worked out from the talks, so past talks
//! count from the day this ships, and a word's state never goes stale.
//!
//! What the laptop leaves out of Mobile's: the word cloud and the "How you
//! speak" card (`voiceOf`), which only My words' own screen shows. Swear
//! words are the ones `safety` already refuses (rustrict), rather than
//! Mobile's own list.

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};

use crate::infrastructure::safety;

/// Talks done before a learner first sees their words.
pub const FIRST_LOOK_TALKS: u32 = 3;

/// Talks a word must be said in, on as many topics, to be theirs.
pub const YOURS_TALKS: usize = 2;

/// Words a recap names, of each kind. A first use may be a mishearing, so few.
pub const RECAP_YOURS: usize = 4;
pub const RECAP_ALMOST: usize = 3;

/// Words Profile's card names: the ones said most.
pub const PROFILE_WORDS: usize = 3;

/// The shortest word that counts: shorter ones are little words or letters.
const SHORTEST: usize = 3;

/// The topic every Word of the day talk counts under, so that two of them
/// make one topic, not two.
pub const WORD_TOPIC: &str = "word-of-the-day";

/// Ella Mobile's word list: one line per word, its ESDB size, the word, then
/// its forms. Lines starting `#` are its notice.
const ENGLISH_WORDS: &str = include_str!("../../shared/english_words.txt");

/// Words too common or too empty to be worth showing, by the word they are
/// forms of: verbs every sentence needs, words that only point or join, and
/// the fillers of talk. The word list leaves out pronouns, prepositions,
/// conjunctions, determiners and numbers, but many of those are listed again
/// as a noun or an adverb ("the", "you", "one"), so they are named here too.
/// So are Hindi words spelled like English ones, which a learner mixing Hindi
/// in says often ("main", "hoon", "log"). Copied from Mobile's `LITTLE_WORDS`.
pub const LITTLE_WORDS: &[&str] = &[
  // Pointing, joining and counting words the list also has as nouns or adverbs.
  "above",
  "across",
  "after",
  "against",
  "along",
  "although",
  "among",
  "and",
  "because",
  "before",
  "behind",
  "below",
  "beside",
  "besides",
  "between",
  "but",
  "during",
  "eight",
  "eighty",
  "either",
  "eleven",
  "fifty",
  "first",
  "five",
  "forty",
  "four",
  "from",
  "her",
  "his",
  "how",
  "however",
  "hundred",
  "inside",
  "into",
  "its",
  "mine",
  "million",
  "near",
  "neither",
  "nine",
  "ninety",
  "none",
  "one",
  "onto",
  "our",
  "ours",
  "outside",
  "past",
  "plus",
  "second",
  "seven",
  "she",
  "since",
  "six",
  "sixty",
  "ten",
  "that",
  "the",
  "their",
  "theirs",
  "them",
  "these",
  "they",
  "third",
  "thirty",
  "this",
  "those",
  "though",
  "thousand",
  "three",
  "through",
  "till",
  "toward",
  "towards",
  "twelve",
  "twenty",
  "two",
  "under",
  "unless",
  "until",
  "upon",
  "what",
  "whatever",
  "when",
  "where",
  "whether",
  "which",
  "while",
  "who",
  "whom",
  "whose",
  "why",
  "with",
  "within",
  "without",
  "you",
  "your",
  "yours",
  // Hindi words spelled like English ones.
  "din",
  "hoon",
  "hum",
  "jab",
  "log",
  "main",
  "mat",
  "par",
  "tab",
  // Verbs every sentence leans on.
  "ask",
  "be",
  "become",
  "call",
  "can",
  "come",
  "could",
  "do",
  "feel",
  "find",
  "get",
  "give",
  "go",
  "have",
  "keep",
  "know",
  "let",
  "like",
  "look",
  "make",
  "may",
  "mean",
  "might",
  "must",
  "need",
  "ought",
  "put",
  "say",
  "see",
  "seem",
  "shall",
  "should",
  "take",
  "tell",
  "thank",
  "think",
  "try",
  "use",
  "want",
  "will",
  "would",
  // Words that point, count or join.
  "about",
  "again",
  "ago",
  "all",
  "almost",
  "already",
  "also",
  "always",
  "another",
  "any",
  "around",
  "away",
  "back",
  "both",
  "each",
  "else",
  "enough",
  "even",
  "ever",
  "every",
  "few",
  "here",
  "just",
  "last",
  "least",
  "less",
  "lot",
  "lots",
  "many",
  "more",
  "most",
  "much",
  "never",
  "next",
  "not",
  "now",
  "off",
  "often",
  "once",
  "only",
  "other",
  "out",
  "over",
  "own",
  "quite",
  "rather",
  "same",
  "several",
  "some",
  "sometimes",
  "soon",
  "still",
  "such",
  "than",
  "then",
  "there",
  "today",
  "tomorrow",
  "tonight",
  "too",
  "very",
  "way",
  "well",
  "yesterday",
  "yet",
  // Words with almost nothing in them.
  "anybody",
  "anyone",
  "anything",
  "bit",
  "day",
  "everybody",
  "everyone",
  "everything",
  "kind",
  "man",
  "name",
  "nobody",
  "nothing",
  "part",
  "people",
  "person",
  "somebody",
  "someone",
  "something",
  "sort",
  "stuff",
  "thing",
  "time",
  "type",
  "year",
  // Fillers, greetings and the words of talking itself.
  "actually",
  "basically",
  "bad",
  "big",
  "bye",
  "exactly",
  "fine",
  "good",
  "great",
  "hello",
  "literally",
  "madam",
  "maybe",
  "nice",
  "okay",
  "perhaps",
  "please",
  "probably",
  "really",
  "right",
  "sir",
  "small",
  "sorry",
  "sure",
  "true",
  "yeah",
  "yes",
];

/// A word of the list: the word it is a form of, which counts it, and its
/// ESDB size (larger is rarer).
#[derive(Debug, Clone, Copy)]
struct Known {
    word: &'static str,
    size: u8,
}

/// The word list as a lookup by form, built the first time a word is looked up.
fn known_words() -> &'static HashMap<&'static str, Known> {
    static KNOWN: OnceLock<HashMap<&'static str, Known>> = OnceLock::new();
    KNOWN.get_or_init(|| {
        let mut known = HashMap::new();
        for line in ENGLISH_WORDS.lines().filter(|line| !line.starts_with('#')) {
            let mut parts = line.split(' ');
            let (Some(size), Some(word)) = (parts.next(), parts.next()) else {
                continue;
            };
            let entry = Known { word, size: size.parse().unwrap_or(50) };
            known.insert(word, entry);
            for form in parts {
                known.insert(form, entry);
            }
        }
        known
    })
}

fn little_words() -> &'static HashSet<&'static str> {
    static LITTLE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    LITTLE.get_or_init(|| LITTLE_WORDS.iter().copied().collect())
}

/// One word said: the word it counts as, the form said, and how rare it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub word: &'static str,
    pub form: String,
    pub size: u8,
}

/// The lowercase words of `text`: letters with any apostrophes inside
/// ("don't"), curly apostrophes read as straight ones.
fn tokens(text: &str) -> Vec<String> {
    let text = text.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_lowercase() {
            current.push(c);
        } else if c == '\'' && !current.is_empty() && chars.peek().is_some_and(char::is_ascii_lowercase) {
            current.push(c);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The words worth counting in `text`, in order. `name` is the learner's own
/// first name, which never counts even when it is a word ("Sunny").
pub fn words_in(text: &str, name: &str) -> Vec<Said> {
    let own = name.to_lowercase();
    let mut said = Vec::new();
    for raw in tokens(text) {
        // A possessive counts as its word; other contractions are little words.
        let token = raw.strip_suffix("'s").unwrap_or(&raw);
        if token.contains('\'') || token.len() < SHORTEST || token == own {
            continue;
        }
        let Some(entry) = known_words().get(token) else {
            continue;
        };
        if little_words().contains(entry.word) || entry.word == own {
            continue;
        }
        if safety::flagged(token) || safety::flagged(entry.word) {
            continue;
        }
        said.push(Said { word: entry.word, form: token.to_owned(), size: entry.size });
    }
    said
}

/// The word a form counts as, or `None` when it would not count.
pub fn word_of(form: &str) -> Option<&'static str> {
    words_in(form, "").first().map(|said| said.word)
}

/// One spoken answer, with what counting its words needs. Answers come
/// oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpokenTurn {
    pub session: String,
    /// The talk's topic; [`WORD_TOPIC`] for every Word of the day talk.
    pub topic: String,
    /// What the learner said.
    pub text: String,
    /// Ella's line just before it, if any.
    pub before: Option<String>,
    /// The word of a Word of the day talk, which is not an echo in its own talk.
    pub target: Option<String>,
}

/// Where one of a learner's words stands.
#[derive(Debug, Clone)]
pub struct WordState {
    pub word: &'static str,
    /// How rare it is: larger is rarer.
    pub size: u8,
    /// Each form they said, and how often, in the order first said.
    pub forms: Vec<(String, u32)>,
    pub talks: HashSet<String>,
    pub topics: HashSet<String>,
    /// The answer they first said it in, by its place among them.
    pub first: usize,
    /// The answer that made it theirs; `None` while it is almost theirs.
    pub yours_at: Option<usize>,
    /// Its place among the learner's words, in the order first said: the
    /// last tie-break, as Mobile's words keep the order they were first said.
    pub order: usize,
}

impl WordState {
    /// The form they said most, the earliest on a tie.
    pub fn shown(&self) -> String {
        let mut best = (self.word, 0);
        for (form, count) in &self.forms {
            if *count > best.1 {
                best = (form.as_str(), *count);
            }
        }
        best.0.to_owned()
    }

    fn times(&self) -> u32 {
        self.forms.iter().map(|(_, count)| count).sum()
    }
}

/// Every word a learner has said, from their spoken answers, oldest first.
pub fn word_states(turns: &[SpokenTurn], name: &str) -> HashMap<&'static str, WordState> {
    let mut states: HashMap<&'static str, WordState> = HashMap::new();
    for (at, turn) in turns.iter().enumerate() {
        let echoed: HashSet<&str> = words_in(turn.before.as_deref().unwrap_or(""), name)
            .into_iter()
            .map(|said| said.word)
            .collect();
        let target = turn.target.as_deref().and_then(word_of);
        for said in words_in(&turn.text, name) {
            if echoed.contains(said.word) && Some(said.word) != target {
                continue;
            }
            let order = states.len();
            let state = states.entry(said.word).or_insert_with(|| WordState {
                word: said.word,
                size: said.size,
                forms: Vec::new(),
                talks: HashSet::new(),
                topics: HashSet::new(),
                first: at,
                yours_at: None,
                order,
            });
            match state.forms.iter_mut().find(|(form, _)| *form == said.form) {
                Some((_, count)) => *count += 1,
                None => state.forms.push((said.form, 1)),
            }
            state.talks.insert(turn.session.clone());
            state.topics.insert(turn.topic.clone());
            if state.yours_at.is_none() && state.talks.len() >= YOURS_TALKS && state.topics.len() >= YOURS_TALKS {
                state.yours_at = Some(at);
            }
        }
    }
    states
}

/// A learner's words in the order the card lists them, the most said first:
/// by the talks each was said in, then how often, then the rarest, then the
/// first said.
pub fn most_said<'a>(states: &'a HashMap<&'static str, WordState>) -> Vec<&'a WordState> {
    let mut all: Vec<&WordState> = states.values().collect();
    all.sort_by(|a, b| {
        b.talks
            .len()
            .cmp(&a.talks.len())
            .then(b.times().cmp(&a.times()))
            .then(b.size.cmp(&a.size))
            .then(a.first.cmp(&b.first))
            .then(a.order.cmp(&b.order))
    });
    all
}

/// Profile's "My words" card, as the window is sent it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnerWords {
    /// Talks done, as the badges count them.
    pub talks: u32,
    /// Whether they have done [`FIRST_LOOK_TALKS`], and are shown their words.
    pub unlocked: bool,
    /// How many words are theirs, and how many almost: nothing until unlocked.
    pub yours: u32,
    pub almost: u32,
    /// The words they say most, at most [`PROFILE_WORDS`], in the form they
    /// say most.
    pub most_said: Vec<String>,
}

/// The learner's words for Profile's card. `talks` is their talks done.
pub fn words_view(turns: &[SpokenTurn], talks: u32, name: &str) -> LearnerWords {
    if talks < FIRST_LOOK_TALKS {
        return LearnerWords { talks, ..LearnerWords::default() };
    }
    let states = word_states(turns, name);
    let yours = states.values().filter(|state| state.yours_at.is_some()).count() as u32;
    LearnerWords {
        talks,
        unlocked: true,
        yours,
        almost: states.len() as u32 - yours,
        most_said: most_said(&states).into_iter().take(PROFILE_WORDS).map(WordState::shown).collect(),
    }
}

/// What a talk's recap says about words.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TalkWords {
    /// Words this talk made theirs.
    pub yours_now: Vec<String>,
    /// Words first said in this talk, the rarest first.
    pub almost_now: Vec<String>,
}

/// What talk `session` did to a learner's words, worked out by counting with
/// and without it; `None` before they have done [`FIRST_LOOK_TALKS`] talks,
/// and when the talk made nothing new. Asked again after a later talk, it may
/// name different words: it is worked out each time, never kept.
pub fn talk_words(turns: &[SpokenTurn], session: &str, talks: u32, name: &str) -> Option<TalkWords> {
    if talks < FIRST_LOOK_TALKS {
        return None;
    }
    let without: Vec<SpokenTurn> = turns.iter().filter(|turn| turn.session != session).cloned().collect();
    let before = word_states(&without, name);
    let after = word_states(turns, name);

    let mut yours_now: Vec<&WordState> = after
        .values()
        .filter(|state| state.yours_at.is_some() && state.talks.contains(session))
        .filter(|state| before.get(state.word).map_or(true, |was| was.yours_at.is_none()))
        .collect();
    yours_now.sort_by(|a, b| b.yours_at.cmp(&a.yours_at).then(b.first.cmp(&a.first)).then(a.order.cmp(&b.order)));
    let mut almost_now: Vec<&WordState> = after
        .values()
        .filter(|state| state.yours_at.is_none() && !before.contains_key(state.word))
        .collect();
    almost_now.sort_by(|a, b| b.size.cmp(&a.size).then(b.first.cmp(&a.first)).then(a.order.cmp(&b.order)));

    let words = TalkWords {
        yours_now: yours_now.into_iter().take(RECAP_YOURS).map(WordState::shown).collect(),
        almost_now: almost_now.into_iter().take(RECAP_ALMOST).map(WordState::shown).collect(),
    };
    (!words.yours_now.is_empty() || !words.almost_now.is_empty()).then_some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(session: &str, topic: &str, text: &str) -> SpokenTurn {
        SpokenTurn { session: session.into(), topic: topic.into(), text: text.into(), ..SpokenTurn::default() }
    }

    fn words(text: &str) -> Vec<&'static str> {
        words_in(text, "").into_iter().map(|said| said.word).collect()
    }

    #[test]
    fn only_listed_words_count_and_forms_count_as_their_word() {
        assert_eq!(words("Yesterday I went to the market"), ["market"], "yesterday is a little word");
        assert_eq!(words("A busy market"), ["busy", "market"]);
        assert_eq!(words("going GOES went"), Vec::<&str>::new(), "go is a little word, in every form");
        assert_eq!(words("My brother's bicycles"), ["brother", "bicycle"], "a possessive is its word");
        assert_eq!(words("Rahul ate poha"), ["eat"], "names and Hindi are not on the list; ate is eat");
        assert_eq!(words("I don’t know"), Vec::<&str>::new(), "a contraction is a little word");
    }

    #[test]
    fn the_learners_own_name_never_counts() {
        assert!(words_in("Sunny loves sunny mornings", "Sunny").iter().all(|said| said.word != "sunny"));
    }

    #[test]
    fn swear_words_never_count() {
        assert!(words_in("that shit market", "").iter().all(|said| said.form != "shit"));
    }

    #[test]
    fn a_word_is_almost_theirs_in_one_talk_and_theirs_in_two_on_two_topics() {
        let one = word_states(&[turn("a", "food", "The market was busy")], "");
        assert!(one["market"].yours_at.is_none());

        let same_topic = word_states(
            &[turn("a", "food", "The market was busy"), turn("b", "food", "Another market")],
            "",
        );
        assert!(same_topic["market"].yours_at.is_none(), "two talks, one topic");

        let two = word_states(
            &[turn("a", "food", "The market was busy"), turn("b", "travel", "Markets at night")],
            "",
        );
        assert_eq!(two["market"].yours_at, Some(1));
        assert_eq!(two["market"].shown(), "market", "the form said first, on a tie");
    }

    #[test]
    fn a_word_ella_just_said_is_an_echo_unless_it_is_the_days_word() {
        let echo = SpokenTurn { before: Some("Do you like the market?".into()), ..turn("a", "food", "Yes, the market") };
        assert!(word_states(&[echo], "").is_empty());

        let word = SpokenTurn {
            before: Some("Do you know what brave means?".into()),
            target: Some("brave".into()),
            ..turn("w", WORD_TOPIC, "Brave means not afraid")
        };
        assert!(word_states(&[word], "").contains_key("brave"));
    }

    #[test]
    fn words_show_after_three_talks_with_the_most_said_first() {
        let turns = [
            turn("a", "food", "The market was busy and the market was loud"),
            turn("b", "travel", "We took a train to the market"),
            turn("c", "school", "My teacher likes the train"),
        ];
        let early = words_view(&turns, 2, "");
        assert_eq!(early, LearnerWords { talks: 2, ..LearnerWords::default() });

        let view = words_view(&turns, 3, "");
        assert!(view.unlocked);
        // Busy and loud tie on everything but the order they were said in.
        assert_eq!(view.most_said, ["market", "train", "busy"]);
        assert_eq!((view.yours, view.almost), (2, 3), "market and train are theirs");
    }

    #[test]
    fn a_talks_new_words_are_those_it_made_theirs_and_first_said() {
        let turns = [
            turn("a", "food", "The market was busy"),
            turn("b", "travel", "We took a train"),
            turn("c", "school", "My teacher went to the market"),
        ];
        let words = talk_words(&turns, "c", 3, "").unwrap();
        assert_eq!(words.yours_now, ["market"]);
        assert_eq!(words.almost_now, ["teacher"]);
        assert_eq!(talk_words(&turns, "c", 2, ""), None, "not before three talks");
        assert_eq!(talk_words(&turns[..1], "z", 3, ""), None, "a talk that said nothing new");
    }
}
