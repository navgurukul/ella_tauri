//! What Ella talks about: Ella Mobile's topics, the design's eleven and then
//! twelve for each level, and the order Home offers them in.
//!
//! They live in `shared/topics.json`, which the browser preview reads too, so
//! the two can never offer different talks. Mobile's words are kept as they
//! are; the desktop's seven topics from before keep their ids, since the talks
//! on the laptop, the badges and Dr Wobble's card name them. Each topic also
//! carries what the language model's instructions need, which mobile's much
//! larger model does without: how they name the topic, and the scene Ella
//! plays in it (`engines::ella_topic_prompt`).

use std::sync::OnceLock;

use chrono::{Local, NaiveDate};
use serde::Deserialize;

use crate::domain::Topic;

/// How many recent topics Home holds back before offering them again, as on
/// the phone.
pub const RECENT_TOPICS: u32 = 20;

/// Who Ella is in a talk, and what the learner came to do. A topic without a
/// role of its own is Ella as herself, which the catalogue's defaults say.
#[derive(Debug, Deserialize)]
struct SceneText {
    role: Option<String>,
    owns: Option<String>,
    draw_out: String,
}

#[derive(Debug, Deserialize)]
pub struct TopicEntry {
    pub id: String,
    pub label: String,
    /// How the instructions name the topic, where the label does not read as
    /// one: "family" for "My family".
    subject: Option<String>,
    /// `role_play`, `vocab`, `grammar`, `real_life`, `culture`, `debate` or
    /// `fluency`.
    pub kind: String,
    pub minutes: u8,
    /// The card's mono line: "ROLE-PLAY · ~5 MIN", or a cadence of its own.
    pub meta: String,
    pub blurb: String,
    /// What comes before the opener, with `{name}` for the learner's name.
    greeting: Option<String>,
    /// What Ella says first, after the greeting; the tall card quotes it.
    pub opener: String,
    /// The CEFR levels whose learners are offered it.
    pub levels: Vec<String>,
    /// Younger learners are offered it after everything else.
    #[serde(default)]
    pub min_age: u8,
    scene: SceneText,
}

#[derive(Debug, Deserialize)]
struct Defaults {
    greeting: String,
    role: String,
    owns: String,
}

#[derive(Debug, Deserialize)]
struct Catalogue {
    defaults: Defaults,
    topics: Vec<TopicEntry>,
}

fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        serde_json::from_str(include_str!("../../shared/topics.json"))
            .expect("shared/topics.json is checked in, and a test reads it")
    })
}

/// Every topic, in the catalogue's order.
pub fn all() -> &'static [TopicEntry] {
    &catalogue().topics
}

pub fn find(id: &str) -> Option<&'static TopicEntry> {
    all().iter().find(|topic| topic.id == id)
}

/// Ella as herself, for a talk with no scene of its own.
pub fn default_role() -> &'static str {
    &catalogue().defaults.role
}

pub fn default_owns() -> &'static str {
    &catalogue().defaults.owns
}

impl TopicEntry {
    pub fn subject(&'static self) -> &'static str {
        self.subject.as_deref().unwrap_or(&self.label)
    }

    /// Who Ella is in this talk, as a second-person clause.
    pub fn role(&'static self) -> &'static str {
        self.scene.role.as_deref().unwrap_or_else(|| default_role())
    }

    /// What is hers to answer in it.
    pub fn owns(&'static self) -> &'static str {
        self.scene.owns.as_deref().unwrap_or_else(|| default_owns())
    }

    /// What the learner came to do, as what Ella draws out of them.
    pub fn draw_out(&'static self) -> &'static str {
        &self.scene.draw_out
    }

    /// Whether Ella plays somebody here, rather than herself.
    pub fn plays_a_role(&self) -> bool {
        self.scene.role.is_some()
    }

    /// Ella's first line: the greeting, with the learner's name, then the
    /// opener.
    pub fn opening(&self, learner_name: &str) -> String {
        let greeting = self.greeting.as_deref().unwrap_or(&catalogue().defaults.greeting);
        format!("{} {}", greeting.replace("{name}", learner_name), self.opener)
    }

    /// The topic as the window shows it.
    pub fn topic(&self) -> Topic {
        Topic {
            id: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind.clone(),
            minutes: self.minutes,
            meta: self.meta.clone(),
            blurb: self.blurb.clone(),
            opener: self.opener.clone(),
        }
    }
}

/// Every topic for a learner at `level`, in Home's order: Home lays out the
/// first five, today's talk first, and "View all" the rest after them.
///
/// Topics not talked about lately come first, in catalogue order turned by one
/// place a day, so Home holds still through a day and moves on the next; a
/// topic just talked about drops back. After them come the ones talked about
/// longest ago. `recent` is the latest talks' topics, newest first, and `day`
/// the day's number (`day_number`). A level the catalogue has nothing for is
/// offered everything. All of that is Ella Mobile's `orderedTopics`.
///
/// Then the desktop's own promise, made at onboarding, that Ella picks topics
/// that fit the learner's age: a topic they are too young for is offered after
/// every other, not taken away.
pub fn offered(level: &str, recent: &[String], day: i64, age: Option<u8>) -> Vec<&'static TopicEntry> {
    let at_level: Vec<&'static TopicEntry> = all()
        .iter()
        .filter(|topic| topic.levels.iter().any(|code| code == level))
        .collect();
    let pool = if at_level.is_empty() {
        all().iter().collect()
    } else {
        at_level
    };

    let fresh: Vec<&'static TopicEntry> = pool
        .iter()
        .copied()
        .filter(|topic| !recent.contains(&topic.id))
        .collect();
    let turn = if fresh.is_empty() {
        0
    } else {
        day.rem_euclid(fresh.len() as i64) as usize
    };
    let stale = recent
        .iter()
        .rev()
        .filter_map(|id| pool.iter().copied().find(|topic| &topic.id == id));
    let mut ordered: Vec<&'static TopicEntry> = fresh[turn..]
        .iter()
        .chain(&fresh[..turn])
        .copied()
        .chain(stale)
        .collect();

    if let Some(age) = age {
        // Stable, so each group keeps its order.
        ordered.sort_by_key(|topic| topic.min_age > age);
    }
    ordered
}

/// Days since 1 January 1970, for a calendar day.
pub fn day_number(day: NaiveDate) -> i64 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("a real date");
    (day - epoch).num_days()
}

/// Today's number on the laptop's own calendar, which the streak and the week
/// strip are drawn in too, so Home moves on at the learner's midnight.
pub fn today() -> i64 {
    day_number(Local::now().date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curriculum;
    use std::collections::HashSet;

    fn ids(topics: &[&TopicEntry]) -> Vec<String> {
        topics.iter().map(|topic| topic.id.clone()).collect()
    }

    #[test]
    fn mobiles_eighty_three_topics_reach_every_level() {
        assert_eq!(all().len(), 83);
        let unique: HashSet<&str> = all().iter().map(|topic| topic.id.as_str()).collect();
        assert_eq!(unique.len(), 83, "every id is a topic's own");
        for level in curriculum::levels() {
            let count = all()
                .iter()
                .filter(|topic| topic.levels.contains(&level.code))
                .count();
            assert!(count >= 12, "{} has {count} topics", level.code);
        }
        for topic in all() {
            assert!(!topic.levels.is_empty(), "{} is offered at no level", topic.id);
            for code in &topic.levels {
                assert!(curriculum::is_level(code), "{}: {code} is not a level", topic.id);
            }
            assert!((3..=8).contains(&topic.minutes), "{}", topic.id);
            assert!(!topic.blurb.is_empty() && !topic.opener.is_empty(), "{}", topic.id);
            assert!(!topic.draw_out().is_empty(), "{} says nothing of what it is for", topic.id);
        }
    }

    #[test]
    fn the_desktops_own_topics_keep_their_ids() {
        // Talks on the laptop, the Bargainer badge and Dr Wobble's card name
        // them, so none of them may go missing.
        for id in [
            "street-food",
            "restaurant-order",
            "booking-a-cab",
            "job-interview",
            "doctor-clinic",
            "asking-directions",
            "market-bargaining",
        ] {
            assert!(find(id).is_some(), "{id}");
        }
        assert_eq!(find("doctor-clinic").unwrap().label, "Talking to the doctor");
        assert!(find("placement").is_none(), "the placement chat is not a topic");
    }

    #[test]
    fn an_opening_is_the_greeting_then_the_opener() {
        assert_eq!(
            find("street-food").unwrap().opening("Asha"),
            "Hi Asha! Tell me about the tastiest thing you ate this week. Where did you find it?"
        );
        assert_eq!(
            find("job-interview").unwrap().opening("Asha"),
            "Hello Asha! Thank you for coming in. To start, could you tell me a little about yourself?"
        );
        assert_eq!(
            find("b2-pitch").unwrap().opening("Asha"),
            "Good morning, Asha. You have two minutes to pitch your idea to our panel. What have you brought us?"
        );
        for topic in all() {
            let opening = topic.opening("Asha");
            assert!(opening.contains("Asha"), "{}", topic.id);
            assert!(opening.ends_with(&topic.opener), "{}", topic.id);
            // Greeted by name, the learner is never asked for it, nor greeted
            // a second time.
            let lower = topic.opener.to_lowercase();
            assert!(!lower.contains("your name"), "{}: {}", topic.id, topic.opener);
            assert!(
                !["hi", "hello", "good morning"].iter().any(|word| lower.starts_with(word)),
                "{}: {}",
                topic.id,
                topic.opener
            );
        }
    }

    #[test]
    fn a_topic_named_from_the_learners_side_is_named_otherwise_to_ella() {
        // "Keep the conversation on My family" reads to the model as its own
        // family, so a topic named the way the learner says it carries a
        // subject. A subject also follows "ask your ... question again", so it
        // is a bare noun phrase.
        for topic in all() {
            let label = topic.label.to_lowercase();
            if label.split_whitespace().any(|word| ["my", "i", "me"].contains(&word)) {
                assert!(topic.subject.is_some(), "{} is named from the learner's side", topic.id);
            }
            if let Some(subject) = &topic.subject {
                let lower = subject.to_lowercase();
                assert!(
                    !["my ", "your ", "their ", "the ", "a ", "an "].iter().any(|start| lower.starts_with(start)),
                    "{}: {subject}",
                    topic.id
                );
            }
        }
    }

    #[test]
    fn a_role_play_has_a_role_and_ground_of_its_own() {
        for topic in all().iter().filter(|topic| topic.kind == "role_play") {
            assert!(topic.plays_a_role(), "{} is a role-play with Ella as herself", topic.id);
        }
        for topic in all().iter().filter(|topic| topic.plays_a_role()) {
            assert_ne!(topic.role(), default_role(), "{}", topic.id);
            assert_ne!(topic.owns(), default_owns(), "{} plays a role with nothing of its own", topic.id);
        }
        assert!(!find("street-food").unwrap().plays_a_role(), "Ella as herself, over chai");
    }

    #[test]
    fn home_offers_the_topics_written_for_the_learners_level() {
        let a0 = offered("A0", &[], 0, None);
        assert_eq!(a0.len(), 12);
        assert!(a0.iter().all(|topic| topic.levels == ["A0"]));
        let a1: HashSet<String> = ids(&offered("A1", &[], 0, None)).into_iter().collect();
        for id in ["campus-visitors", "sunday-vegetable-turn", "restaurant-order", "booking-a-cab", "asking-directions", "a1-hobbies"] {
            assert!(a1.contains(id), "{id}");
        }
        assert!(!a1.contains("street-food"), "street food is written for A2");
        let b2 = ids(&offered("B2", &[], 0, None));
        assert!(b2.contains(&"job-interview".to_string()), "the interview grows with the learner");
        assert_eq!(offered("Z9", &[], 0, None).len(), all().len(), "an unknown level is offered everything");
    }

    #[test]
    fn home_holds_still_through_a_day_and_turns_one_place_the_next() {
        let day = 20_371;
        let first = ids(&offered("A2", &[], day, None));
        // As `offeredTopics` has it in the browser preview on the same day.
        assert_eq!(first[0], "a2-when-i-was-little");
        assert_eq!(ids(&offered("A2", &[], day, None)), first, "the same all day");
        let next = ids(&offered("A2", &[], day + 1, None));
        assert_ne!(next, first);
        let mut turned = first.clone();
        turned.rotate_left(1);
        assert_eq!(next, turned, "tomorrow's first topic is today's second");
        // The turn is on the catalogue's order, whatever the day.
        let catalogue: Vec<String> = all()
            .iter()
            .filter(|topic| topic.levels.contains(&"A2".to_string()))
            .map(|topic| topic.id.clone())
            .collect();
        let at = (day as usize) % catalogue.len();
        assert_eq!(first[0], catalogue[at]);
        assert_eq!(ids(&offered("A2", &[], -1, None))[0], *catalogue.last().unwrap(), "days before 1970 turn too");
    }

    #[test]
    fn a_topic_just_talked_about_drops_back_behind_the_fresh_ones() {
        let day = 20_371;
        let fresh = ids(&offered("A2", &[], day, None));
        let recent = vec![fresh[0].clone(), fresh[3].clone(), "market-cloth-price".to_string()];
        let after = ids(&offered("A2", &recent, day, None));
        assert_eq!(after.len(), fresh.len(), "nothing is taken away, and a chore is not a topic");
        // Talked about longest ago first, the latest last.
        assert_eq!(&after[after.len() - 2..], &[fresh[3].clone(), fresh[0].clone()]);
        assert!(!after[..after.len() - 2].contains(&fresh[0]));
        // The fresh ones still turn, among themselves.
        let rest: Vec<String> = after[..after.len() - 2].to_vec();
        assert_eq!(rest.len(), fresh.len() - 2);
        let everything_recent: Vec<String> = fresh.iter().rev().cloned().collect();
        assert_eq!(
            ids(&offered("A2", &everything_recent, day, None)),
            fresh,
            "with every topic recent, the one talked about longest ago comes first"
        );
    }

    #[test]
    fn a_younger_learner_meets_the_grown_up_topics_last() {
        let day = 3;
        let anyone = ids(&offered("B1", &[], day, None));
        let twelve = ids(&offered("B1", &[], day, Some(12)));
        assert_eq!(twelve.last().map(String::as_str), Some("job-interview"));
        assert_eq!(twelve.len(), anyone.len(), "sunk, not taken away");
        let mut without: Vec<String> = anyone.iter().filter(|id| *id != "job-interview").cloned().collect();
        without.push("job-interview".into());
        assert_eq!(twelve, without, "everything else keeps its order");
        assert_eq!(ids(&offered("B1", &[], day, Some(14))), anyone, "old enough, nothing moves");
        let nine = ids(&offered("A2", &[], day, Some(9)));
        assert_eq!(nine.last().map(String::as_str), Some("market-bargaining"));
    }

    #[test]
    fn a_days_number_counts_from_1970() {
        assert_eq!(day_number(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()), 0);
        assert_eq!(day_number(NaiveDate::from_ymd_opt(2026, 10, 10).unwrap()), 20_736);
        assert!(today() > 20_000);
    }
}
