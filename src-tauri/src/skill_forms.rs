//! Whether a learner's answer shows a skill's form, for the skills whose
//! form can be read off the words: have and a past participle for the
//! present perfect, if and will for the first conditional, the skill's own
//! words for "Contrast words".
//!
//! The laptop's lesson marks an answer by this where it can, and asks its 3B
//! model only for the rest (`LocalEngine::lesson_reply`). In five lessons on
//! 2026-10-14 the model's marks agreed with a teacher's on 24 to 29 of 35
//! answers however it was asked, and it got the same ones wrong every time:
//! it praised "I live in Pune since five years" and "I did a decision", and
//! missed "I have never seen snow in my life".
//!
//! Only the form is read, never whether the rest of the sentence is right:
//! "If it rain tomorrow, I will stay home" shows the first conditional, and
//! "I have went" does not show the present perfect.

/// Whether `answer` shows the form of the skill labelled `label` (the
/// curriculum's labels are unique). `None` for a skill whose use cannot be
/// read off the words, such as telling a story or describing words.
pub fn shown(label: &str, answer: &str) -> Option<bool> {
    let words = words(answer);
    let w = Words(&words);
    let shown = match label {
        // A0
        "A and the" => w.any(&["a", "an", "the"]),
        "Saying I am" => w.phrase("i am"),
        "I have got" => w.any_phrase(&["have got", "has got", "have not got", "has not got", "'s got"]),
        "Likes and dislikes" => i_then(&words, &["like", "love", "hate"]),
        "Joining with and" => w.any(&["and"]),
        "Can and can't" => w.any(&["can", "cannot"]),
        "Positions" | "Where things are" => {
            w.any(&["on", "in", "under", "behind", "between", "above", "below", "near"])
                || w.any_phrase(&["next to", "in front of", "there is", "there are"])
        }
        "Right now" | "What I'm doing" | "Happening now" => w.followed_by(&["am", "is", "are", "'s"], 2, doing),
        // A1
        "Past time words" => {
            w.any(&["yesterday", "ago", "earlier"])
                || w.followed_by(&["last"], 1, |word| WHEN.contains(&word))
                || w.any_phrase(&["the other day"])
        }
        "Regular past" => words.iter().any(|word| regular_past(word)),
        "Irregular past" => w.any(IRREGULAR_PAST),
        "Opinion words" => i_then(&words, &["think", "feel", "prefer", "love", "hate", "like", "believe"]),
        "Using because" => w.any(&["because"]),
        "Using but" => w.any(&["but"]),
        "How often" => {
            w.any(&["always", "usually", "sometimes", "never", "often", "rarely", "seldom", "normally", "daily", "weekly"])
                || w.followed_by(&["every"], 1, |word| WHEN.contains(&word) || word == "day")
                || w.any_phrase(&["once a", "twice a"])
        }
        "Future time words" => {
            w.any(&["tomorrow", "soon", "later", "tonight"])
                || w.followed_by(&["next"], 1, |word| WHEN.contains(&word))
                || w.any_phrase(&["one day", "in the future"])
        }
        "Going to" => w.any(&["gonna"]) || going_to_do(answer, &words),
        "Sequence words" => {
            w.any(&["first", "firstly", "then", "finally", "next", "afterwards", "later"])
                || w.any_phrase(&["after that", "in the end", "at last"])
        }
        // A2
        "Present perfect" => perfect(&words),
        "Contrast words" => {
            // Not "but" or "yet", which every answer has: A1 has its own "Using
            // but", and this skill asks for the longer words.
            w.any(&["however", "although", "though", "whereas", "despite", "while"])
                || w.any_phrase(&["even though", "in spite of", "on the other hand"])
        }
        "Past continuous" => w.followed_by(&["was", "were"], 2, doing),
        "Used to" => w.followed_by(&["used"], 1, |word| word == "to"),
        "Opinion phrases" => {
            i_then(&words, &["think", "believe", "feel"])
                || w.any_phrase(&["in my opinion", "it seems", "for me", "personally"])
        }
        "First conditional" => w.any(&["if"]) && w.any(&["will"]),
        "Modal verbs" => w.any(&["might", "could", "should", "must", "may"]),
        "Linking words" => {
            w.any(&["therefore", "moreover", "furthermore", "besides", "however"])
                || w.any_phrase(&["as a result", "in addition", "on the other hand", "for example", "because of this"])
        }
        "Relative clauses" => words.iter().skip(1).any(|word| ["who", "which", "whose", "where"].contains(&word.as_str())),
        // B1
        // If and a past form, then would: not "If I have time, I would go".
        "Second conditional" => {
            w.followed_by(&["if"], 4, |word| PAST_IN_IF.contains(&word) || regular_past(word) || IRREGULAR_PAST.contains(&word))
                && w.any(&["would", "'d"])
                && !w.any(&["will"])
        }
        "Third conditional" => {
            w.any(&["if"])
                && w.followed_by(&["had", "'d"], 2, participle)
                && w.followed_by(&["would", "could", "might", "'d"], 2, |word| word == "have")
        }
        "Reported speech" => w.any(&["said", "told", "asked", "explained", "mentioned", "replied"]),
        "Hedging" => {
            w.any(&["might", "perhaps", "maybe", "probably", "possibly", "likely", "seems", "arguably"])
                || w.any_phrase(&["could be argued", "some evidence", "tend to"])
        }
        "Discourse markers" => {
            w.any(&["anyway", "actually", "basically"])
                || w.any_phrase(&["as i was saying", "by the way", "you know"])
                || words.first().is_some_and(|first| ["so", "well", "right"].contains(&first.as_str()))
        }
        "Collocations" => collocation(&words),
        "Cleft sentences" => {
            w.any_phrase(&["what i mean is", "the thing is", "the reason why", "what i like", "what i want"])
                || (w.any_phrase(&["it was", "it is", "it 's"]) && w.any(&["who"]))
        }
        _ => return None,
    };
    Some(shown)
}

/// I, then one of `verbs` within the next three words: "I like", "I really
/// like", "I do not like".
fn i_then(words: &[String], verbs: &[&str]) -> bool {
    Words(words).followed_by(&["i"], 3, |word| verbs.contains(&word))
}

/// Words that say when, after "last", "next" or "every": "last Sunday".
const WHEN: &[&str] = &[
    "week", "weekend", "month", "year", "night", "time", "morning", "evening", "summer", "winter",
    "monsoon", "holiday", "holidays", "sunday", "monday", "tuesday", "wednesday", "thursday", "friday",
    "saturday", "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december", "diwali", "festival", "class", "term", "semester",
];

/// The past forms the second conditional's if-clause takes besides an -ed
/// or irregular past: "If I were you".
const PAST_IN_IF: &[&str] = &["were", "had", "could", "did", "knew", "lived", "won"];

/// How many words `answer` has, contractions written out: "I'm reading" is
/// three.
pub fn word_count(answer: &str) -> usize {
    words(answer).len()
}

/// The answer's words, lower case, with contractions written out:
/// "I've" is "i have", "won't" is "will not", "it's" is "it 's", which
/// stands for is and has alike, and "I'd" is "i 'd", had or would.
fn words(answer: &str) -> Vec<String> {
    let text = answer.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    let mut words = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'')) {
        let word = word.trim_matches('\'');
        if word.is_empty() {
            continue;
        }
        if let Some(stem) = word.strip_suffix("n't") {
            words.push(
                match stem {
                    "wo" => "will",
                    "ca" => "can",
                    "sha" => "shall",
                    stem => stem,
                }
                .to_owned(),
            );
            words.push("not".to_owned());
        } else if let Some((stem, tail)) = word.split_once('\'') {
            words.push(stem.to_owned());
            words.push(
                match tail {
                    "ve" => "have",
                    "ll" => "will",
                    "m" => "am",
                    "re" => "are",
                    "d" => "'d",
                    "s" => "'s",
                    tail => tail,
                }
                .to_owned(),
            );
        } else {
            words.push(word.to_owned());
        }
    }
    words
}

struct Words<'a>(&'a [String]);

impl Words<'_> {
    fn any(&self, wanted: &[&str]) -> bool {
        self.0.iter().any(|word| wanted.contains(&word.as_str()))
    }

    fn phrase(&self, phrase: &str) -> bool {
        let wanted: Vec<&str> = phrase.split(' ').collect();
        self.0
            .windows(wanted.len())
            .any(|window| window.iter().zip(&wanted).all(|(word, want)| word == want))
    }

    fn any_phrase(&self, phrases: &[&str]) -> bool {
        phrases.iter().any(|phrase| self.phrase(phrase))
    }

    /// One of `first`, with a word that passes `then` among the `within`
    /// words after it: "have never really seen", "was not eating".
    fn followed_by(&self, first: &[&str], within: usize, then: impl Fn(&str) -> bool) -> bool {
        self.0.iter().enumerate().any(|(index, word)| {
            first.contains(&word.as_str()) && self.0[index + 1..].iter().take(within).any(|next| then(next))
        })
    }
}

/// A verb's -ing form: not "thing" or "morning".
fn doing(word: &str) -> bool {
    word.len() > 4 && word.ends_with("ing") && !NOT_DOING.contains(&word)
}

const NOT_DOING: &[&str] = &[
    "thing", "nothing", "something", "anything", "everything", "morning", "evening", "building", "ceiling",
    "wedding", "spring", "string", "sibling", "darling", "pudding", "during", "bring", "sing", "king", "ring",
    "wing", "sting", "swing", "cling", "fling",
    // Describing words, not something being done: "It is interesting".
    "interesting", "amazing", "boring", "exciting", "surprising", "relaxing", "tiring", "annoying",
    "confusing", "embarrassing", "charming", "shocking", "disappointing", "frightening", "satisfying",
    "refreshing", "inspiring", "loving", "caring", "missing", "outstanding", "willing", "promising",
    "challenging", "encouraging", "fascinating", "terrifying", "worrying", "amusing", "pleasing",
];

/// A verb's -ed form: not "red" or "hundred", nor the classic mistake of an
/// irregular verb made regular ("goed", "buyed"), which is no use of it.
fn regular_past(word: &str) -> bool {
    word.len() > 3 && word.ends_with("ed") && !NOT_PAST.contains(&word) && !OVERREGULAR.contains(&word)
}

const OVERREGULAR: &[&str] = &[
    "goed", "buyed", "eated", "runned", "comed", "taked", "maked", "gived", "writed", "speaked", "drinked",
    "swimmed", "singed", "catched", "teached", "thinked", "bringed", "knowed", "getted", "holded", "sitted",
    "standed", "feeled", "keeped", "leaved", "meeted", "sended", "spended", "telled", "sayed", "builded",
    "finded", "maked", "seed", "drived", "rided", "flyed", "growed", "throwed", "weared", "winned", "losed",
    "selled", "sleeped", "waked", "falled", "forgetted", "understanded", "becomed", "breaked", "choosed",
    "fighted", "hitted", "putted", "cutted", "letted", "costed", "hurted", "shutted", "beginned", "drawed",
    "blowed", "knowed", "teared", "stealed", "hanged",
];

const NOT_PAST: &[&str] = &[
    "red", "bed", "shed", "need", "seed", "speed", "feed", "weed", "hundred", "sacred", "naked", "wicked",
    "ragged", "rugged", "kindred", "beloved", "indeed", "breed", "greed", "deed", "reed", "proceed",
    "succeed", "exceed", "led", "fed", "wed", "sled",
    // Describing words, not something done: "I was bored".
    "tired", "bored", "excited", "interested", "worried", "scared", "married", "surprised", "pleased",
    "amazed", "confused", "annoyed", "embarrassed", "relaxed", "satisfied", "disappointed", "frightened",
    "shocked", "stressed", "talented", "crowded", "beloved", "advanced", "complicated", "detailed",
    "skilled", "qualified", "limited", "supposed", "named", "aged",
];

/// A past participle: an -ed form, or one of the common irregular ones.
fn participle(word: &str) -> bool {
    regular_past(word) || IRREGULAR_PARTICIPLES.contains(&word)
}

const IRREGULAR_PARTICIPLES: &[&str] = &[
    "been", "seen", "done", "gone", "eaten", "taken", "given", "written", "known", "made", "had", "met", "found",
    "bought", "brought", "thought", "told", "said", "read", "heard", "left", "lost", "won", "sung", "swum",
    "flown", "driven", "ridden", "broken", "spoken", "chosen", "forgotten", "gotten", "got", "become", "come",
    "run", "begun", "drunk", "sat", "slept", "felt", "kept", "taught", "caught", "built", "sent", "spent",
    "stood", "understood", "put", "cut", "hit", "let", "set", "paid", "held", "sold", "grown", "thrown", "worn",
    "shown", "fallen", "hidden", "bitten", "woken", "fought", "meant", "lent", "led", "fed", "shut", "hurt",
    "cost", "beaten", "forgiven", "frozen", "stolen", "torn", "born", "drawn", "blown", "shaken", "risen",
    "stuck", "struck", "hung", "rung", "dug", "bent", "dealt", "learnt", "burnt", "dreamt", "spelt",
];

const IRREGULAR_PAST: &[&str] = &[
    "went", "saw", "ate", "had", "did", "came", "took", "made", "got", "gave", "said", "told", "found", "bought",
    "brought", "thought", "knew", "met", "left", "felt", "kept", "sat", "stood", "ran", "began", "drank", "swam",
    "sang", "wrote", "spoke", "broke", "chose", "drove", "rode", "flew", "grew", "threw", "wore", "won", "lost",
    "sent", "spent", "built", "taught", "caught", "heard", "paid", "sold", "slept", "woke", "fell", "forgot",
    "understood", "became", "held", "was", "were", "fought", "meant", "led", "fed", "hung", "dug", "stole",
    "learnt", "burnt", "dreamt", "spelt", "lit", "hid", "shook", "bit", "drew", "blew", "lay", "rose",
];

/// Whether the answer says one of the common collocations: a verb and the
/// noun it goes with ("made a decision", "pays attention", "took a big
/// risk"), or an adjective and its noun ("heavy rain"). Strict, since the
/// lesson practises these: "I did a decision" is not one, and neither is an
/// answer with none.
fn collocation(words: &[String]) -> bool {
    const VERBS: &[(&[&str], &[&str])] = &[
        (
            &["make", "makes", "made", "making"],
            &["decision", "decisions", "mistake", "mistakes", "progress", "friends", "effort", "plan", "plans",
              "money", "difference", "choice", "call", "noise", "promise", "appointment", "sense"],
        ),
        (
            &["take", "takes", "took", "taken", "taking"],
            &["risk", "risks", "break", "photo", "photos", "picture", "care", "part", "notes", "time", "look",
              "test", "exam", "shower", "walk", "place", "advantage"],
        ),
        (&["pay", "pays", "paid", "paying"], &["attention", "visit", "compliment", "bill", "fine"]),
        (
            &["do", "does", "did", "done", "doing"],
            &["homework", "exercise", "favour", "favor", "best", "business", "dishes", "shopping", "research", "course"],
        ),
        (
            &["have", "has", "had", "having"],
            &["fun", "time", "look", "chance", "rest", "party", "idea", "breakfast", "lunch", "dinner"],
        ),
        (&["catch", "catches", "caught", "catching"], &["cold", "bus", "train"]),
        (&["keep", "keeps", "kept", "keeping"], &["promise", "touch", "secret", "calm"]),
        (&["break", "breaks", "broke", "broken", "breaking"], &["promise", "rules", "record"]),
        (&["save", "saves", "saved", "saving", "spend", "spends", "spent", "spending"], &["money", "time", "energy"]),
        (&["give", "gives", "gave", "given", "giving"], &["advice", "speech", "hand", "presentation"]),
        (&["tell", "tells", "told", "telling"], &["truth", "lie", "lies", "story", "joke", "jokes"]),
    ];
    const PAIRS: &[&str] = &[
        "heavy rain", "heavy traffic", "strong coffee", "strong wind", "fast food", "high price", "high prices",
        "big mistake", "best friend", "close friend", "hard work", "free time", "deep sleep",
    ];
    let w = Words(words);
    w.any_phrase(PAIRS)
        || VERBS.iter().any(|(forms, nouns)| {
            // The noun within four words of the verb: "made a big decision",
            // "spend a lot of time".
            w.followed_by(forms, 4, |word| nouns.contains(&word))
        })
}

/// Have or has and then a past participle, with nothing but an adverb
/// between: "I have never seen", "it's been". Not "I have a dog named
/// Tommy", nor "she's a good friend".
fn perfect(words: &[String]) -> bool {
    const BETWEEN: &[&str] = &[
        "never", "ever", "just", "already", "not", "always", "recently", "really", "also", "often", "still",
        "finally", "all",
    ];
    words.iter().enumerate().any(|(index, word)| {
        let after = words[index + 1..].iter().skip_while(|next| BETWEEN.contains(&next.as_str()));
        match word.as_str() {
            "have" | "has" => after.take(1).any(|next| participle(next)),
            // "'s" is mostly "is": only "it's been", "he's gone", "she's got".
            "'s" => after.take(1).any(|next| ["been", "gone", "got", "done", "had"].contains(&next.as_str())),
            _ => false,
        }
    })
}

/// "going to" and then something done, not somewhere gone: "going to
/// study", not "going to the market", "going to school" or "going to
/// Mumbai", a name being somewhere too.
fn going_to_do(answer: &str, words: &[String]) -> bool {
    let named: Vec<String> = answer
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .filter(|word| word.chars().next().is_some_and(char::is_uppercase) && *word != "I")
        .map(str::to_lowercase)
        .collect();
    const PLACES: &[&str] = &[
        "the", "a", "an", "my", "your", "his", "her", "our", "their", "this", "that", "school", "college",
        "market", "office", "home", "work", "bed", "church", "temple", "class", "town", "city",
    ];
    words.windows(3).any(|window| {
        window[0] == "going"
            && window[1] == "to"
            && !PLACES.contains(&window[2].as_str())
            && !named.contains(&window[2])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(label: &str, shown_in: &[&str], not_in: &[&str]) {
        for answer in shown_in {
            assert_eq!(shown(label, answer), Some(true), "{label}: {answer}");
        }
        for answer in not_in {
            assert_eq!(shown(label, answer), Some(false), "{label}: {answer}");
        }
    }

    #[test]
    fn the_present_perfect_is_have_and_a_past_participle() {
        check(
            "Present perfect",
            &[
                "I have lived in Pune for five years.",
                "I have visited Mumbai two times with my family.",
                "I have never seen snow in my life.",
                "I’ve eaten vada pav many times there.",
                "She has just gone home.",
                "It's been a long day.",
            ],
            &[
                "I live in Pune since five years.",
                "My father works in a bank.",
                "I have went to Mumbai.",
                "I have red shoes.",
                "I have to go now.",
                "Yes.",
            ],
        );
    }

    #[test]
    fn conditionals_are_read_by_their_if_and_will_or_would() {
        check(
            "First conditional",
            &["If it rains tomorrow, I will stay at home.", "If I get good marks, my mother will be happy.", "If I'm late, I'll call you."],
            &["If it rain tomorrow I stay home.", "I like summer more than winter.", "I will go tomorrow."],
        );
        check(
            "Second conditional",
            &["If I had more time, I would travel more.", "If I won, I'd buy a car."],
            &["If I have time, I will travel.", "I would like tea."],
        );
        check(
            "Third conditional",
            &["If I had studied harder, I would have passed.", "If she'd come, we'd have danced."],
            &["If I had time, I would go.", "I would have gone."],
        );
    }

    #[test]
    fn a_tense_with_ing_is_not_thrown_by_a_word_that_ends_like_one() {
        check(
            "Past continuous",
            &["I was eating when she called.", "They were not listening."],
            &["I was at a wedding.", "It was something new.", "I ate when she called."],
        );
        check(
            "Happening now",
            &["She is running.", "I'm reading a book.", "They are not playing now."],
            &["She runs every day.", "It is a nice morning."],
        );
    }

    #[test]
    fn a_describing_word_is_not_a_tense() {
        assert_eq!(shown("Happening now", "It is interesting."), Some(false));
        assert_eq!(shown("Past continuous", "The movie was amazing."), Some(false));
        assert_eq!(shown("Regular past", "I was tired after school."), Some(false));
        assert_eq!(shown("Present perfect", "I have interested friends."), Some(false));
        assert_eq!(word_count("I'm reading."), 3);
        assert_eq!(shown("Present perfect", "I have used my phone a lot."), Some(true));
        assert_eq!(shown("Present perfect", "I have experienced snow."), Some(true));
        assert_eq!(shown("Present perfect", "I have a dog named Tommy."), Some(false));
        assert_eq!(shown("Present perfect", "She's a good friend, she's done a lot."), Some(true));
        assert_eq!(shown("Present perfect", "He's tired and he's working."), Some(false));
    }

    #[test]
    fn what_the_reviewers_found_is_read_right() {
        check("Regular past", &["I walked home."], &["I goed home.", "Yesterday I buyed a phone."]);
        check("Present perfect", &["I have bought a phone."], &["I have eated lunch."]);
        check(
            "Second conditional",
            &["If I were rich, I would travel.", "If I had time, I'd read more."],
            &["If I have time, I would go.", "If I get money, I will buy it and I would share."],
        );
        check(
            "Likes and dislikes",
            &["I really like cricket.", "I don't like eggs.", "I also love dogs."],
            &["Cricket is fun."],
        );
        check("Opinion words", &["I really think it is good."], &["It is good."]);
        check(
            "Collocations",
            &["I spend a lot of time with my family.", "I made a lot of friends there."],
            &["I have a lot of friends there."],
        );
        check("Past time words", &["Last Sunday I went out.", "I met him last weekend."], &["I go out."]);
        check("Future time words", &["Next Sunday I will go."], &["I go on Sundays."]);
        check("How often", &["I play every Sunday.", "I run daily."], &["I play on Sunday."]);
        check("Contrast words", &["Although it rained, we played."], &["I like tea but not coffee."]);
        check("Linking words", &["As a result, we won."], &["I also like it."]);
        check("I have got", &["She's got a cat."], &["She has a cat."]);
        check("Irregular past", &["I learnt a lot."], &["I learn a lot."]);
        check("Cleft sentences", &["It's my brother who helps me."], &["It was raining that day."]);
    }

    #[test]
    fn going_to_is_a_plan_not_a_place() {
        check(
            "Going to",
            &["I am going to study tonight.", "We're gonna play cricket."],
            &["I am going to school.", "I am going to the market.", "I am going to Mumbai.", "I will study."],
        );
    }

    #[test]
    fn a_skill_of_its_own_words_is_read_by_them() {
        check("Contrast words", &["I like tea, although I drink coffee."], &["I like tea and coffee."]);
        check("Opinion phrases", &["In my opinion, phones are useful."], &["Phones are useful."]);
        check("Used to", &["I used to play cricket."], &["I play cricket."]);
        check("Modal verbs", &["You should rest."], &["You rest."]);
        check("Irregular past", &["Yesterday I went to the market."], &["I go to the market."]);
        check("Regular past", &["I watched a film."], &["I watch a film.", "I have a red bag."]);
        check("Relative clauses", &["The man who lives there is kind."], &["Who lives there?"]);
    }

    #[test]
    fn a_collocation_is_a_verb_and_the_noun_it_goes_with() {
        check(
            "Collocations",
            &[
                "I made a decision to study engineering.",
                "In class I always pay attention to the teacher.",
                "Sometimes I take a risk and try new things.",
                "I made a big decision.",
                "There was heavy rain yesterday.",
            ],
            &[
                "Last year I did a decision to study engineering.",
                "Online is good because I can learn from home.",
                "Pay.",
            ],
        );
    }

    #[test]
    fn a_skill_whose_use_is_not_in_its_words_is_left_to_the_model() {
        assert_eq!(shown("Telling a story", "First we ate, then we danced."), None);
        assert_eq!(shown("Idioms", "It was a piece of cake."), None);
        assert_eq!(shown("Describing words", "It was beautiful."), None);
    }

    #[test]
    fn every_label_it_reads_is_one_the_curriculum_has() {
        let labels: Vec<&str> = crate::curriculum::levels()
            .iter()
            .flat_map(|level| &level.steps)
            .flat_map(|step| &step.skills)
            .map(|skill| skill.label.as_str())
            .collect();
        for label in READ {
            assert!(labels.contains(label), "{label} is not in the curriculum");
            assert!(shown(label, "").is_some(), "{label}");
        }
    }

    /// Every label `shown` reads.
    const READ: &[&str] = &[
        "A and the", "Saying I am", "I have got", "Likes and dislikes", "Joining with and", "Can and can't",
        "Positions", "Where things are", "Right now", "What I'm doing", "Happening now", "Past time words",
        "Regular past", "Irregular past", "Opinion words", "Using because", "Using but", "How often",
        "Future time words", "Going to", "Sequence words", "Present perfect", "Contrast words",
        "Past continuous", "Used to", "Opinion phrases", "First conditional", "Modal verbs", "Linking words",
        "Relative clauses", "Second conditional", "Third conditional", "Reported speech", "Hedging",
        "Discourse markers", "Cleft sentences", "Collocations",
    ];
}
