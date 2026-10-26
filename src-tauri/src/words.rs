//! Word of the day: Ella Mobile's `words.ts`, rule for rule.
//!
//! Ella greets the learner, tells them today's word and asks whether they
//! know what it means. They say what they think it means, or that they do not
//! know; a bare yes gets "Tell me what it means." first. Her next line then
//! tells them what it means, with an example of it in use unless they knew
//! it, and asks the first of three questions that have them use it, the last
//! one for a sentence of their own.
//!
//! It is a short, warm talk rather than a quiz: after the answer to the first
//! question Ella asks one question of her own about what they told her, then
//! goes on to the next of the word's. About five answers in all.
//!
//! Which word is on the card and which of its questions Ella asks is decided
//! here, never by the model, so the two always agree. The cloud's model
//! writes her words on each answer, and judges whether they knew the word
//! (`infrastructure::word_prompt`); without it, every line is one of the
//! fixed ones here, as on the phone. The laptop's own model is never asked,
//! so a word talk leaves everything it keeps ready for the next talk alone.
//!
//! The words come in three lists, by level (`shared/words.json`). Each
//! learner goes through theirs in order, a new word on each day they play
//! one: the word stays the same all day, however often they play it, and a
//! word they missed is still waiting for them the next time. A learner who
//! has had them all starts again with the one they had longest ago.
//!
//! A learner who does not know, or whose answer Ella could not make out, gets
//! an example to follow, or the question again; the second time, Ella moves
//! on. Her own questions get no second try, and none follows an answer that
//! took one. The talk closes once the third of the word's questions is
//! answered.

use std::{collections::HashSet, sync::OnceLock};

use serde::{Deserialize, Serialize};

use crate::domain::{Fix, Topic, WordShown};

/// The talk's topic, so it reads like any talk in the history: Ella Mobile's
/// `WORD_TOPIC`. No topic in the catalogue has its id.
pub const WORD_TOPIC_ID: &str = "word-of-the-day";
pub const WORD_TOPIC_LABEL: &str = "Word of the day";

/// What Ella asks as the talk opens, after the word.
pub const KNOW_ASK: &str = "Do you know what it means?";

/// What she asks a learner who says yes without saying what it means.
pub const EXPLAIN_ASK: &str = "Tell me what it means.";

/// The most a line of Ella's takes, in characters, as on the phone: her
/// words on an answer are cut to the sentences that leave her next line room.
pub const LINE_CHARS: usize = 110;

/// What Ella says before moving on from a question the learner could not answer.
const MOVE_ON: &str = "That's okay.";

/// Her line to a word from the learner once the talk is over.
const AFTER_END: &str = "Thank you! You can finish now.";

/// A longer answer that says it does not know on its way ("I don't know,
/// maybe because she loves us") is an answer.
const STUCK_MOST_WORDS: usize = 6;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WordQuestion {
    /// What Ella asks.
    pub ask: String,
    /// Ella's own answer, for a learner who does not know what to say.
    pub example: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DayWord {
    pub word: String,
    /// What Ella says it means, a little more fully than a dictionary: "Brave
    /// means you do something hard or scary, even when you feel afraid."
    pub meaning: String,
    /// Her own example of the word in use, which she says after its meaning.
    pub example: String,
    /// The three questions, in the order Ella asks them; the last asks for a sentence.
    pub questions: Vec<WordQuestion>,
    /// Other ways the word is written or said, such as US spellings: "favorite".
    #[serde(default)]
    pub forms: Vec<String>,
}

/// One list of words, and the levels it is for.
#[derive(Debug, Deserialize)]
struct WordList {
    levels: Vec<String>,
    words: Vec<DayWord>,
}

#[derive(Debug, Deserialize)]
struct Lists {
    lists: Vec<WordList>,
}

fn lists() -> &'static [WordList] {
    static LISTS: OnceLock<Lists> = OnceLock::new();
    &LISTS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../shared/words.json"))
                .expect("shared/words.json is checked in, and a test reads it")
        })
        .lists
}

/// The topic a word talk is kept under.
pub fn topic() -> Topic {
    Topic {
        id: WORD_TOPIC_ID.into(),
        label: WORD_TOPIC_LABEL.into(),
        kind: "vocab".into(),
        minutes: 2,
        meta: "WORD OF THE DAY · ~2 MIN".into(),
        blurb: "Learn a new word and use it.".into(),
        opener: "Here is today's word.".into(),
    }
}

/// Where a word talk is, as it is kept on the talk (`sessions.word_talk`), in
/// the shape the phone keeps it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WordTalk {
    /// The word on the card, which finds its entry in the lists.
    pub word: String,
    /// Where the talk is with the word's meaning.
    pub meaning: Meaning,
    /// Which of its questions Ella asked last, from 0.
    pub question: usize,
    /// Her last line ended on a question of her own about their answer to
    /// that one, rather than on one of the word's.
    pub following: bool,
    /// The answers in a row that did not answer it.
    pub stuck: u32,
    /// Ella has asked the last question and heard its answer.
    pub done: bool,
    /// The questions whose answers, or the answer to her own question after
    /// one, used the word well, as Ella judged it, by index.
    pub used: Vec<usize>,
}

/// Where a talk is with the word's meaning: Ella has asked whether they know
/// it (`Ask`), or after a bare yes to say what it means (`Explain`), or has
/// told it and asked her first question (`Told`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Meaning {
    Ask,
    Explain,
    Told,
}

/// What Ella made of an answer to whether they know the word: they said what
/// it means (`Right`), part of it (`Close`), another meaning (`Wrong`), only
/// that they know it (`Yes`), that they do not (`No`), or she could not make
/// it out (`Unclear`); `Said` is a meaning nobody judged, without the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knew {
    Right,
    Close,
    Wrong,
    Yes,
    No,
    Unclear,
    Said,
}

impl Knew {
    /// The model's word for it; `Said` is never the model's.
    pub fn from_model(word: &str) -> Option<Self> {
        Some(match word {
            "right" => Self::Right,
            "close" => Self::Close,
            "wrong" => Self::Wrong,
            "yes" => Self::Yes,
            "no" => Self::No,
            "unclear" => Self::Unclear,
            _ => return None,
        })
    }

    /// How Ella starts the line that tells them the meaning.
    fn opening(self) -> &'static str {
        match self {
            Self::Right => "Yes, exactly!",
            Self::Close => "Close!",
            Self::Wrong => "Not quite!",
            Self::Said => "Thank you!",
            Self::Yes | Self::No | Self::Unclear => "No problem!",
        }
    }
}

/// What Ella's words on an answer lead into: a question of her own about it
/// (`Follow`), the word's next question (`Next`), or the end of the talk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordStep {
    Follow,
    Next,
    End,
}

/// A learner's answer, and what Ella made of it when the model judged it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordAnswer {
    pub said: String,
    /// The model could not tell what they meant.
    pub unclear: bool,
    /// Whether they used the word well; `None` when no model judged it.
    pub used: Option<bool>,
    /// Her question of her own about it, when the model wrote one.
    pub ask: Option<String>,
}

/// What a learner's answer does to the talk, and what Ella says because of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordMove {
    /// They did not know: an example to follow; or Ella could not make out
    /// their answer (`unclear`): the question again.
    Again { question: WordQuestion, unclear: bool },
    /// Her own question about their answer, before the word's next.
    Follow { ask: String },
    /// The next question; `missed` after a second answer that did not answer the last.
    Ask { question: WordQuestion, missed: bool },
    /// The last question is answered: with this answer (`now`), or before it.
    End { missed: bool, now: bool, used: Option<bool> },
}

/// The list of words for a learner at `level`; a level no list names gets the middle one.
pub fn words_for(level: &str) -> &'static [DayWord] {
    let lists = lists();
    &lists
        .iter()
        .find(|list| list.levels.iter().any(|known| known == level))
        .or_else(|| lists.get(1))
        .unwrap_or(&lists[0])
        .words
}

/// A word's entry, whichever list it is in.
pub fn day_word(word: &str) -> Option<&'static DayWord> {
    lists().iter().flat_map(|list| &list.words).find(|entry| entry.word == word)
}

/// Today's talk for a learner at `level`: the first word of their list they
/// have not played on an earlier day, or once they have had every one, the
/// one they had longest ago. `before` is what their earlier days' word talks
/// kept, oldest first (`Database::words_before`).
pub fn plan_word(level: &str, before: &[String]) -> WordTalk {
    let words = words_for(level);
    let played: Vec<Option<String>> = before
        .iter()
        .map(|kept| read_word_talk(kept).map(|talk| talk.word))
        .collect();
    let last_played = |word: &str| played.iter().rposition(|played| played.as_deref() == Some(word));
    let next = words
        .iter()
        .find(|entry| last_played(&entry.word).is_none())
        // Every one played: the one whose last talk is the oldest.
        .or_else(|| words.iter().min_by_key(|entry| last_played(&entry.word)))
        .expect("every list has words");
    WordTalk {
        word: next.word.clone(),
        meaning: Meaning::Ask,
        question: 0,
        following: false,
        stuck: 0,
        done: false,
        used: Vec::new(),
    }
}

impl WordTalk {
    /// The word on the card. A talk is only ever planned on a word in the lists.
    pub fn entry(&self) -> &'static DayWord {
        day_word(&self.word).unwrap_or_else(|| &words_for("")[0])
    }

    /// The word on the card, as the window shows it.
    pub fn shown(&self) -> WordShown {
        let entry = self.entry();
        WordShown {
            text: entry.word.clone(),
            meaning: entry.meaning.clone(),
            forms: forms_of(entry),
        }
    }

    /// The word's question after the one Ella asked last; `None` after the last.
    pub fn next_question(&self) -> Option<&'static WordQuestion> {
        self.entry().questions.get(self.question + 1)
    }

    /// What Ella asked last, to ask again: whether they know the word, or her question.
    pub fn current_ask(&self) -> &'static str {
        match self.meaning {
            Meaning::Ask => KNOW_ASK,
            Meaning::Explain => EXPLAIN_ASK,
            Meaning::Told => &self.entry().questions[self.question].ask,
        }
    }

    /// What comes after the learner's answer to Ella's last line. She asks one
    /// question of her own about the answer to the word's first question, so
    /// the talk is a conversation and not a quiz; but none about an answer that
    /// took an example or a second try, so a learner who finds it hard is not
    /// kept on it. With whether they know it asked first, the talk stays
    /// about five answers.
    pub fn step(&self) -> WordStep {
        if self.done || self.question + 1 >= self.entry().questions.len() {
            return WordStep::End;
        }
        if self.question > 0 || self.following || self.stuck > 0 {
            WordStep::Next
        } else {
            WordStep::Follow
        }
    }
}

/// Ella's first line: hello, today's word, and whether they know what it
/// means. `name` is the learner's first.
pub fn opening_line(talk: &WordTalk, name: &str) -> String {
    let hello = if name.is_empty() { "Hi!".to_owned() } else { format!("Hi {name}!") };
    sentences(&[&hello, &format!("Today's word is {}.", talk.entry().word), KNOW_ASK])
}

/// Moves a talk still on the word's meaning on by an answer Ella made `knew`
/// of: a first bare yes gets `EXPLAIN_ASK`; anything else her line that tells
/// them the meaning and asks her first question. A second yes with no meaning
/// hears it told, so nobody is asked twice.
pub fn after_knowing(talk: &WordTalk, knew: Knew) -> (WordTalk, String) {
    if knew == Knew::Yes && talk.meaning == Meaning::Ask {
        let next = WordTalk { meaning: Meaning::Explain, ..talk.clone() };
        return (next, sentences(&["Great!", EXPLAIN_ASK]));
    }
    let next = WordTalk { meaning: Meaning::Told, ..talk.clone() };
    (next, told_line(talk.entry(), knew))
}

/// Ella's line once she has heard whether they know the word: a few words on
/// what they said, what it means, her example of it ("For example: ...")
/// unless they knew it, and her first question.
pub fn told_line(entry: &DayWord, knew: Knew) -> String {
    let example = if knew == Knew::Right {
        String::new()
    } else {
        format!("For example: {}", entry.example)
    };
    sentences(&[knew.opening(), &entry.meaning, &example, &entry.questions[0].ask])
}

/// What Ella makes of an answer to whether they know the word without the
/// model: one that says they do not know is `No`, a bare yes is `Yes`, a bare
/// no is `No`, and anything else a meaning she cannot judge (`Said`).
pub fn scripted_knowing(said: &str) -> Knew {
    if stuck_on(said) {
        return Knew::No;
    }
    let words = comparable(said);
    if BARE_YES.contains(&words.as_str()) {
        Knew::Yes
    } else if BARE_NO.contains(&words.as_str()) {
        Knew::No
    } else {
        Knew::Said
    }
}

const BARE_YES: &[&str] = &[
    "yes", "yeah", "yep", "yup", "yes yes", "haan", "han", "ha", "ji", "ji haan", "ok", "okay", "sure",
    "of course", "i know", "i know it", "yes i know", "yes i know it", "yes i do",
];
const BARE_NO: &[&str] = &["no", "nope", "nah", "nahi", "nahin", "na", "no i dont", "no i do not"];

/// Moves the talk on by one answer to Ella's last line. An answer once the
/// talk is done changes nothing: Ella reacts, and the talk has closed.
pub fn after_answer(talk: &WordTalk, answer: &WordAnswer) -> (WordTalk, WordMove) {
    let entry = talk.entry();
    let asked = &entry.questions[talk.question];
    if talk.done {
        return (talk.clone(), WordMove::End { missed: false, now: false, used: None });
    }
    // One that holds the word was made out, whatever the model said.
    let stuck = stuck_on(&answer.said);
    let unclear = answer.unclear && !stuck && !holds_word(&answer.said, entry);
    let missed = stuck || unclear;
    // One of the word's questions gets a second try; one of her own does not.
    if missed && talk.stuck == 0 && !talk.following {
        let next = WordTalk { stuck: 1, ..talk.clone() };
        return (next, WordMove::Again { question: asked.clone(), unclear });
    }
    // Without the model's word on it, an answer holding the word used it.
    let used_it = !missed && answer.used.unwrap_or_else(|| holds_word(&answer.said, entry));
    let mut used = talk.used.clone();
    if used_it && !used.contains(&talk.question) {
        used.push(talk.question);
    }
    // Her own question about it, where one comes and the model wrote it; else
    // she goes straight on to the next of the word's.
    if let Some(ask) = answer.ask.as_ref().filter(|ask| !missed && !ask.is_empty()) {
        if talk.step() == WordStep::Follow {
            let next = WordTalk { following: true, used, ..talk.clone() };
            return (next, WordMove::Follow { ask: ask.clone() });
        }
    }
    if talk.question + 1 < entry.questions.len() {
        let next = WordTalk {
            question: talk.question + 1,
            following: false,
            stuck: 0,
            used,
            ..talk.clone()
        };
        let question = entry.questions[next.question].clone();
        return (next, WordMove::Ask { question, missed });
    }
    let next = WordTalk {
        following: false,
        stuck: 0,
        done: true,
        used,
        ..talk.clone()
    };
    (next, WordMove::End { missed, now: true, used: Some(used_it) })
}

/// Ella's line after `reaction`, her words on the learner's answer: an
/// example to follow or the question again, her own question about it, the
/// word's next question, or the end of the talk, where she praises the
/// learner by `name` and leaves them the word to use today.
pub fn move_line(step: &WordMove, reaction: &str, entry: &DayWord, name: &str) -> String {
    match step {
        WordMove::Again { question, unclear: true } => format!("Sorry, I didn't catch that. {}", question.ask),
        WordMove::Again { question, unclear: false } => format!("For example: {} Now you try!", question.example),
        WordMove::Follow { ask } => fitted(reaction, ask),
        WordMove::Ask { question, missed } => fitted(if *missed { MOVE_ON } else { reaction }, &question.ask),
        // Once the talk is over, a word more from the learner gets a short
        // line, not her goodbye again.
        WordMove::End { now: false, .. } => AFTER_END.to_owned(),
        WordMove::End { missed, used, .. } => {
            // Only for a sentence that used the word: "I saw his patient" is
            // not today's patient, and was praised as if it were.
            let praise = match (*missed, *used) {
                (false, Some(true) | None) if name.is_empty() => "Well done!".to_owned(),
                (false, Some(true) | None) => format!("Well done, {name}!"),
                _ => String::new(),
            };
            fitted(
                if *missed { MOVE_ON } else { reaction },
                &sentences(&[&praise, &format!("Try to use {} today.", entry.word)]),
            )
        }
    }
}

/// Her `words` and `then` as one line, her words cut to the sentences that
/// leave `then` room in `LINE_CHARS`; the first always stays.
fn fitted(words: &str, then: &str) -> String {
    let all = sentences_of(words);
    let mut kept = all.first().cloned().unwrap_or_default();
    for sentence in all.iter().skip(1) {
        if sentences(&[&kept, sentence, then]).chars().count() > LINE_CHARS {
            break;
        }
        kept = sentences(&[&kept, sentence]);
    }
    sentences(&[&kept, then])
}

/// Ella's few words on an answer without the model: that they used the word,
/// when they did, and otherwise only her thanks, never praise for an answer
/// she cannot judge. At the `End` step, nothing: her praise for the whole
/// talk follows, and thanks before it would only crowd it.
pub fn scripted_reaction(entry: &DayWord, said: &str, step: WordStep) -> String {
    if holds_word(said, entry) {
        format!("You used {}!", entry.word)
    } else if step == WordStep::End {
        String::new()
    } else {
        "Thank you!".to_owned()
    }
}

/// What went well in a word talk, for its recap: that they used the word,
/// in a sentence of their own when the last answer did, and then what went
/// well in any talk (`notes::went_well`); two at most.
pub fn went_well(talk: &WordTalk, answers: &[&str]) -> Vec<String> {
    let last = talk.entry().questions.len() - 1;
    let mut lines = Vec::new();
    if talk.used.contains(&last) {
        lines.push(format!("Used {} in your own sentence", talk.word));
    } else if !talk.used.is_empty() {
        lines.push(format!("Used {} in your answers", talk.word));
    }
    lines.extend(crate::notes::went_well(answers, None));
    lines.truncate(2);
    lines
}

/// Whether `fix` only puts the word where the app heard another, as when it
/// wrote brief for brave: that is never the learner's mistake to fix.
pub fn misheard_word(fix: &Fix, entry: &DayWord) -> bool {
    holds_word(&fix.better, entry) && !holds_word(&fix.said, entry)
}

/// Whether `said` says the learner does not know, and no more.
pub fn stuck_on(said: &str) -> bool {
    is_stuck(said) && said.split_whitespace().count() <= STUCK_MOST_WORDS
}

/// Whether `said` says only that the learner does not know, or nothing at
/// all: Ella Mobile's `pictures.isStuck`.
fn is_stuck(said: &str) -> bool {
    let words = comparable(said);
    if words.is_empty() || STUCK_ALONE.contains(&words.as_str()) {
        return true;
    }
    let words: Vec<&str> = words.split(' ').collect();
    let any = |options: &[&str], word: &str| options.contains(&word);
    let nahi = |word: &str| any(&["nah", "nahi", "nahn", "nahin"], word);
    let knowing = |word: &str| any(&["pata", "maloom", "malum", "maalum"], word);
    (0..words.len()).any(|at| {
        let rest = &words[at..];
        let starts = |phrase: &[&str]| rest.starts_with(phrase);
        starts(&["dont", "know"])
            || starts(&["do", "not", "know"])
            || starts(&["didnt", "know"])
            || starts(&["did", "not", "know"])
            || starts(&["dunno"])
            || starts(&["no", "idea"])
            || starts(&["not", "sure"])
            // "pata nahi", "pata hi nahin", and "nahi pata".
            || (knowing(rest[0]) && rest.get(1).is_some_and(|next| nahi(next)))
            || (knowing(rest[0]) && rest.get(1) == Some(&"hi") && rest.get(2).is_some_and(|next| nahi(next)))
            || (nahi(rest[0]) && rest.get(1).is_some_and(|next| knowing(next)))
    })
}

/// Answers that say nothing on their own.
const STUCK_ALONE: &[&str] = &[
    "hmm", "hm", "umm", "um", "uh", "er", "ah", "what", "sorry", "pardon", "help", "i cant", "i cannot", "pass",
    "skip",
];

/// Whether `said` holds the word in one of its everyday forms: brave, braver,
/// bravely; happy, happier, happily; decide, decided, deciding; and any the
/// entry lists of its own.
pub fn holds_word(said: &str, entry: &DayWord) -> bool {
    let heard = comparable(said);
    let heard: HashSet<&str> = heard.split(' ').collect();
    forms_of(entry).iter().any(|form| heard.contains(form.as_str()))
}

/// What the endings below make of a word that is another word, never it in
/// use: "business" is not busy, nor "lately" late, "patients" patient (the
/// learner's word is being calm, not the sick), or "cooker" cook.
const NOT_FORMS: &[&str] = &["business", "lately", "patients", "cooker", "cookers"];

/// The word's forms, as `comparable` has them.
pub fn forms_of(entry: &DayWord) -> Vec<String> {
    let word = comparable(&entry.word);
    let mut forms = vec![word.clone()];
    forms.extend(entry.forms.iter().map(|form| comparable(form)));
    for ending in ["s", "es", "ed", "d", "er", "r", "est", "st", "ing", "ly", "ness"] {
        forms.push(format!("{word}{ending}"));
    }
    // happy: happier, happiest, happily, happiness, happied.
    if let Some(stem) = word.strip_suffix('y') {
        for ending in ["ier", "iest", "ily", "iness", "ied", "ies"] {
            forms.push(format!("{stem}{ending}"));
        }
    }
    // decide: deciding; brave: bravery.
    if let Some(stem) = word.strip_suffix('e') {
        for ending in ["ing", "ery", "ation"] {
            forms.push(format!("{stem}{ending}"));
        }
    }
    let mut seen = HashSet::new();
    forms
        .into_iter()
        .filter(|form| !NOT_FORMS.contains(&form.as_str()) && seen.insert(form.clone()))
        .collect()
}

/// A kept talk's state, or `None` if what is kept is not one.
pub fn read_word_talk(kept: &str) -> Option<WordTalk> {
    let value: serde_json::Value = serde_json::from_str(kept).ok()?;
    let word = value.get("word")?.as_str()?.to_owned();
    let question = usize::try_from(value.get("question")?.as_u64()?).ok()?;
    let meaning = match value.get("meaning").and_then(serde_json::Value::as_str) {
        Some("ask") => Meaning::Ask,
        Some("explain") => Meaning::Explain,
        _ => Meaning::Told,
    };
    Some(WordTalk {
        word,
        meaning,
        question,
        following: value.get("following").and_then(serde_json::Value::as_bool) == Some(true),
        stuck: value
            .get("stuck")
            .and_then(serde_json::Value::as_u64)
            .and_then(|stuck| u32::try_from(stuck).ok())
            .unwrap_or(0),
        done: value.get("done").and_then(serde_json::Value::as_bool) == Some(true),
        used: value
            .get("used")
            .and_then(serde_json::Value::as_array)
            .map(|used| {
                used.iter()
                    .filter_map(serde_json::Value::as_u64)
                    .filter_map(|index| usize::try_from(index).ok())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// A talk's state as it is kept.
pub fn keep(talk: &WordTalk) -> String {
    serde_json::to_string(talk).expect("a word talk is plain data")
}

/// Words as an answer and the word can be compared: case, apostrophes and
/// other punctuation aside, and the repeated letters of a filler ("hmmm").
pub fn comparable(text: &str) -> String {
    text.chars()
        .filter(|character| !matches!(character, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}'))
        .flat_map(char::to_lowercase)
        .filter(|character| !matches!(character, '\'' | '‘' | '’'))
        .collect::<String>()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| match word {
            _ if is_filler(word, 'h') => "hmm",
            _ if is_filler(word, 'u') => "umm",
            _ => word,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "hmm" or "ummmm": `first` and two or more m's.
fn is_filler(word: &str, first: char) -> bool {
    word.strip_prefix(first)
        .is_some_and(|rest| rest.len() >= 2 && rest.chars().all(|character| character == 'm'))
}

/// The sentences of `text`, each with its own stops, as Ella Mobile's
/// `sentencesOf` cuts them.
pub fn sentences_of(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut stopped = false;
    for character in text.chars() {
        let stop = matches!(character, '.' | '!' | '?');
        if stopped && !stop {
            found.push(std::mem::take(&mut current));
        }
        stopped = stop;
        current.push(character);
    }
    found.push(current);
    found
        .into_iter()
        .map(|sentence| sentence.trim().to_owned())
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

/// `parts` that say anything, one after the other.
fn sentences(parts: &[&str]) -> String {
    parts.iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn talk(word: &str) -> WordTalk {
        WordTalk {
            word: word.into(),
            meaning: Meaning::Ask,
            question: 0,
            following: false,
            stuck: 0,
            done: false,
            used: Vec::new(),
        }
    }

    fn told(word: &str) -> WordTalk {
        WordTalk { meaning: Meaning::Told, ..talk(word) }
    }

    fn answer(said: &str) -> WordAnswer {
        WordAnswer { said: said.into(), ..WordAnswer::default() }
    }

    #[test]
    fn three_lists_of_fifteen_each_word_with_three_questions_the_last_for_a_sentence() {
        let lists = lists();
        assert_eq!(lists.len(), 3);
        let levels: Vec<&str> = lists.iter().flat_map(|list| list.levels.iter().map(String::as_str)).collect();
        assert_eq!(levels, ["A0", "A1", "A2", "B1", "B2", "C1"]);
        let mut seen = HashSet::new();
        for list in lists {
            assert_eq!(list.words.len(), 15);
            for entry in &list.words {
                assert!(seen.insert(entry.word.clone()), "{} is in two lists", entry.word);
                assert_eq!(entry.questions.len(), 3, "{}", entry.word);
                let last = &entry.questions[2].ask;
                assert!(
                    last.starts_with("Say a sentence with") || last.starts_with("Use ") || last.starts_with("Say how"),
                    "{}: {last}",
                    entry.word
                );
                assert!(entry.meaning.contains(&capitalised(&entry.word)) || entry.meaning.contains(&entry.word));
            }
        }
    }

    fn capitalised(word: &str) -> String {
        let mut characters = word.chars();
        characters.next().map(|first| first.to_uppercase().chain(characters).collect()).unwrap_or_default()
    }

    #[test]
    fn each_level_gets_its_own_list_and_an_unknown_one_the_middle() {
        assert_eq!(words_for("A0")[0].word, "happy");
        assert_eq!(words_for("A1")[0].word, "happy");
        assert_eq!(words_for("A2")[0].word, "brave");
        assert_eq!(words_for("B1")[0].word, "brave");
        assert_eq!(words_for("C1")[0].word, "resilient");
        assert_eq!(words_for("Z9")[0].word, "brave");
    }

    #[test]
    fn a_learner_goes_through_their_list_in_order_and_then_from_the_one_had_longest_ago() {
        let kept = |word: &str| keep(&told(word));
        assert_eq!(plan_word("A2", &[]).word, "brave");
        assert_eq!(plan_word("A2", &[kept("brave")]).word, "patient");
        // A word from another list, after a level up, takes nobody's place.
        assert_eq!(plan_word("A2", &[kept("happy"), kept("brave")]).word, "patient");
        let all: Vec<String> = words_for("A2").iter().map(|entry| kept(&entry.word)).collect();
        assert_eq!(plan_word("A2", &all).word, "brave");
        let mut again = all.clone();
        again.push(kept("brave"));
        assert_eq!(plan_word("A2", &again).word, "patient");
        // Unreadable rows are no word at all.
        assert_eq!(plan_word("A2", &["not json".into()]).word, "brave");
        let planned = plan_word("B2", &[]);
        assert_eq!(planned.meaning, Meaning::Ask);
        assert_eq!((planned.question, planned.stuck, planned.done), (0, 0, false));
    }

    #[test]
    fn the_talk_opens_on_hello_the_word_and_whether_they_know_it() {
        assert_eq!(
            opening_line(&talk("brave"), "Aarav"),
            "Hi Aarav! Today's word is brave. Do you know what it means?"
        );
        assert_eq!(opening_line(&talk("brave"), ""), "Hi! Today's word is brave. Do you know what it means?");
    }

    #[test]
    fn a_bare_yes_is_asked_for_the_meaning_once_and_anything_else_hears_it_told() {
        let (asked, line) = after_knowing(&talk("brave"), Knew::Yes);
        assert_eq!(asked.meaning, Meaning::Explain);
        assert_eq!(line, "Great! Tell me what it means.");
        let (told_now, line) = after_knowing(&asked, Knew::Yes);
        assert_eq!(told_now.meaning, Meaning::Told);
        assert!(line.starts_with("No problem! Brave means"), "{line}");

        let (_, right) = after_knowing(&talk("brave"), Knew::Right);
        assert_eq!(
            right,
            "Yes, exactly! Brave means you do something hard or scary, even when you feel afraid. When were you brave?"
        );
        let (_, close) = after_knowing(&talk("brave"), Knew::Close);
        assert_eq!(
            close,
            "Close! Brave means you do something hard or scary, even when you feel afraid. For example: My little \
             brother was brave when he got his injection. When were you brave?"
        );
    }

    #[test]
    fn without_the_model_a_bare_yes_or_no_is_read_and_anything_else_is_thanked() {
        assert_eq!(scripted_knowing("Yes"), Knew::Yes);
        assert_eq!(scripted_knowing("Haan, I know it."), Knew::Said);
        assert_eq!(scripted_knowing("haan"), Knew::Yes);
        assert_eq!(scripted_knowing("No."), Knew::No);
        assert_eq!(scripted_knowing("I don't know"), Knew::No);
        assert_eq!(scripted_knowing("pata nahi"), Knew::No);
        assert_eq!(scripted_knowing("It means not afraid"), Knew::Said);
    }

    #[test]
    fn only_a_short_answer_says_they_do_not_know() {
        for said in ["I don't know", "dunno", "no idea", "Hmmm", "not sure", "mujhe pata hi nahin", "nahi pata", ""] {
            assert!(stuck_on(said), "{said:?}");
        }
        for said in [
            "I don't know, maybe because she loves us very much",
            "I was brave at the dentist",
            "I know my sister is brave",
        ] {
            assert!(!stuck_on(said), "{said:?}");
        }
    }

    #[test]
    fn the_word_is_heard_in_its_everyday_forms_but_never_in_another_word() {
        let busy = day_word("busy").unwrap();
        assert!(holds_word("I am busy today", busy));
        assert!(holds_word("I was busier last week", busy));
        assert!(!holds_word("My business is small", busy));
        let brave = day_word("brave").unwrap();
        assert!(holds_word("That was bravery!", brave));
        assert!(holds_word("He bravely went", brave));
        let favourite = day_word("favourite").unwrap();
        assert!(holds_word("My favorite food", favourite));
        let patient = day_word("patient").unwrap();
        assert!(!holds_word("The patients waited", patient));
        let happy = day_word("happy").unwrap();
        assert!(holds_word("I was happier then", happy));
        assert!(holds_word("Happiness is family", happy));
    }

    #[test]
    fn a_first_answer_follows_with_her_question_and_the_rest_go_on_to_the_words() {
        let start = told("brave");
        assert_eq!(start.step(), WordStep::Follow);
        let (followed, step) = after_answer(
            &start,
            &WordAnswer { ask: Some("Were you scared at first?".into()), used: Some(true), ..answer("At the dentist") },
        );
        assert_eq!(step, WordMove::Follow { ask: "Were you scared at first?".into() });
        assert!(followed.following);
        assert_eq!(followed.used, vec![0]);
        assert_eq!(followed.step(), WordStep::Next);

        let (second, step) = after_answer(&followed, &answer("Yes a little"));
        assert_eq!(second.question, 1);
        assert!(!second.following);
        assert!(matches!(step, WordMove::Ask { missed: false, .. }));
        // Her own question never comes again.
        let (third, step) = after_answer(
            &second,
            &WordAnswer { ask: Some("Why?".into()), ..answer("My grandmother is brave") },
        );
        assert_eq!(third.question, 2);
        assert!(matches!(step, WordMove::Ask { .. }));
        assert_eq!(third.used, vec![0, 1]);
        assert_eq!(third.step(), WordStep::End);

        let (done, step) = after_answer(&third, &answer("My friend was brave on stage"));
        assert!(done.done);
        assert_eq!(step, WordMove::End { missed: false, now: true, used: Some(true) });
        let (after, step) = after_answer(&done, &answer("Bye"));
        assert_eq!(after, done);
        assert_eq!(step, WordMove::End { missed: false, now: false, used: None });
    }

    #[test]
    fn a_learner_who_does_not_know_gets_an_example_once_then_ella_moves_on() {
        let start = told("brave");
        let (again, step) = after_answer(&start, &answer("I don't know"));
        assert_eq!(again.stuck, 1);
        let line = move_line(&step, "", day_word("brave").unwrap(), "Aarav");
        assert_eq!(line, "For example: I was brave at the dentist. Now you try!");
        let (moved, step) = after_answer(&again, &answer("I don't know"));
        assert_eq!(moved.question, 1);
        assert_eq!(moved.stuck, 0);
        assert_eq!(
            move_line(&step, "Nice.", day_word("brave").unwrap(), "Aarav"),
            "That's okay. Who is the bravest person you know?"
        );
    }

    #[test]
    fn an_answer_she_could_not_make_out_is_asked_again_unless_it_holds_the_word() {
        let start = told("brave");
        let unclear = WordAnswer { unclear: true, ..answer("fish table go") };
        let (_, step) = after_answer(&start, &unclear);
        assert_eq!(
            move_line(&step, "", day_word("brave").unwrap(), ""),
            "Sorry, I didn't catch that. When were you brave?"
        );
        let heard = WordAnswer { unclear: true, ..answer("brave dentist") };
        let (next, step) = after_answer(&start, &heard);
        assert_eq!(next.question, 1);
        assert!(matches!(step, WordMove::Ask { missed: false, .. }));
    }

    #[test]
    fn the_last_answer_ends_on_praise_only_for_a_sentence_that_used_the_word() {
        let entry = day_word("patient").unwrap();
        let last = WordTalk { question: 2, ..told("patient") };
        let (_, step) = after_answer(&last, &WordAnswer { used: Some(true), ..answer("My teacher is patient") });
        assert_eq!(
            move_line(&step, "What a kind teacher.", entry, "Aarav"),
            "What a kind teacher. Well done, Aarav! Try to use patient today."
        );
        let (_, step) = after_answer(&last, &WordAnswer { used: Some(false), ..answer("I saw his patient") });
        assert_eq!(move_line(&step, "Patient here means calm.", entry, "Aarav"), "Patient here means calm. Try to use patient today.");
        let (_, step) = after_answer(&last, &answer("My teacher is patient"));
        assert_eq!(
            move_line(&step, &scripted_reaction(entry, "My teacher is patient", WordStep::End), entry, ""),
            "You used patient! Well done! Try to use patient today."
        );
    }

    #[test]
    fn her_words_are_cut_to_the_sentences_that_leave_her_question_room() {
        let line = fitted(
            "The dentist! You were really brave that day. I think that is wonderful and very grown up of you, truly.",
            "Who is the bravest person you know?",
        );
        assert_eq!(line, "The dentist! You were really brave that day. Who is the bravest person you know?");
        assert!(line.chars().count() <= LINE_CHARS);
        // The first sentence always stays.
        let long = "A".repeat(120);
        assert!(fitted(&format!("{long}. More."), "Q?").starts_with(&long));
    }

    #[test]
    fn without_the_model_she_says_they_used_it_or_thanks_them() {
        let brave = day_word("brave").unwrap();
        assert_eq!(scripted_reaction(brave, "I was brave", WordStep::Next), "You used brave!");
        assert_eq!(scripted_reaction(brave, "At school", WordStep::Next), "Thank you!");
        assert_eq!(scripted_reaction(brave, "At school", WordStep::End), "");
    }

    #[test]
    fn a_kept_talk_reads_back_and_a_talk_kept_before_the_meaning_was_asked_is_told() {
        let mut state = told("brave");
        state.used = vec![0, 2];
        state.following = true;
        assert_eq!(read_word_talk(&keep(&state)), Some(state));
        let old = read_word_talk(r#"{"word":"brave","question":1}"#).unwrap();
        assert_eq!(old.meaning, Meaning::Told);
        assert_eq!(old.question, 1);
        assert_eq!(read_word_talk(r#"{"question":1}"#), None);
        assert_eq!(read_word_talk("oops"), None);
    }

    #[test]
    fn the_recap_says_they_used_the_word_and_never_fixes_the_word_misheard() {
        let mut state = told("brave");
        let answers = ["At the dentist", "Yes a little", "My grandmother is brave", "My friend was brave on stage"];
        state.used = vec![1, 2];
        assert_eq!(went_well(&state, &answers)[0], "Used brave in your own sentence");
        assert_eq!(went_well(&state, &answers).len(), 2);
        state.used = vec![0];
        assert_eq!(went_well(&state, &answers)[0], "Used brave in your answers");
        state.used = Vec::new();
        assert!(!went_well(&state, &answers)[0].contains("brave"));

        let brave = day_word("brave").unwrap();
        let fix = |said: &str, better: &str| Fix { said: said.into(), better: better.into() };
        assert!(misheard_word(&fix("I was brief", "I was brave"), brave));
        assert!(!misheard_word(&fix("I is brave", "I am brave"), brave));
        assert!(!misheard_word(&fix("I go to school", "I went to school"), brave));
    }

    #[test]
    fn sentences_are_cut_after_their_stops() {
        assert_eq!(sentences_of("Hi Aarav! Today's word is brave. Ok?"), ["Hi Aarav!", "Today's word is brave.", "Ok?"]);
        assert_eq!(sentences_of("Wow!! Nice"), ["Wow!!", "Nice"]);
        assert!(sentences_of("  ").is_empty());
    }

    #[test]
    fn answers_compare_without_case_punctuation_or_a_long_hum() {
        assert_eq!(comparable("Hmmmm... I DON'T know!"), "hmm i dont know");
        assert_eq!(comparable("Ummmmm"), "umm");
        assert_eq!(comparable("It’s brave"), "its brave");
    }
}
