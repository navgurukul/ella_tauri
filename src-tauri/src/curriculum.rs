//! The curriculum every talk follows: six levels, A0 to C1, of five steps
//! each, and the skills each step works on.
//!
//! The words are Ella Docs' "Curriculum & Skills Mapping", word for word, as
//! Ella Mobile transcribed them; each skill's short label is ours. They live in
//! `shared/curriculum.json`, which the browser preview reads too, so the two
//! can never teach different things.
//!
//! A skill's id repeats in every level, so progress is kept against
//! [`skill_key`], which adds the level. The CEFR codes stay on this side of the
//! bridge and in the prompts: the window shows a level's name and its number.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Vocabulary, grammar or fluency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Strand {
    #[serde(rename = "VOC")]
    Vocabulary,
    #[serde(rename = "GRA")]
    Grammar,
    #[serde(rename = "FLU")]
    Fluency,
}

/// One thing a step teaches, written the way the learner would say it.
#[derive(Debug, Deserialize)]
pub struct Skill {
    /// `U<step>-<strand>-<nn>` as the curriculum numbers it: unique within a
    /// level only.
    pub id: String,
    pub strand: Strand,
    /// One to three words for the summary: "Past time words".
    pub label: String,
    /// "I can …", word for word.
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct Step {
    /// 1 to 5.
    pub number: u8,
    pub title: String,
    /// What the step is for, in one line.
    pub focus: String,
    pub skills: Vec<Skill>,
}

#[derive(Debug, Deserialize)]
pub struct Level {
    /// The CEFR code, `A0` to `C1`.
    pub code: String,
    /// What the window shows instead of the code.
    pub name: String,
    /// The level's "I can …" headline.
    pub goal: String,
    pub steps: Vec<Step>,
}

/// Where a learner is: a level, and a step within it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub level: String,
    pub step: u8,
}

impl Position {
    pub fn new(level: &str, step: u8) -> Self {
        Self {
            level: level.into(),
            step,
        }
    }
}

#[derive(Deserialize)]
struct Curriculum {
    start: Position,
    levels: Vec<Level>,
}

fn curriculum() -> &'static Curriculum {
    static CURRICULUM: OnceLock<Curriculum> = OnceLock::new();
    CURRICULUM.get_or_init(|| {
        serde_json::from_str(include_str!("../../shared/curriculum.json"))
            .expect("shared/curriculum.json is checked in, and a test reads it")
    })
}

/// Every level, lowest first.
pub fn levels() -> &'static [Level] {
    &curriculum().levels
}

/// Where every learner stands until a placement says otherwise: Step 1 of A2,
/// as on the phone.
pub fn start() -> Position {
    curriculum().start.clone()
}

pub fn level(code: &str) -> Option<&'static Level> {
    levels().iter().find(|level| level.code == code)
}

/// A level's place on the ladder, from 0.
pub fn level_index(code: &str) -> Option<usize> {
    levels().iter().position(|level| level.code == code)
}

pub fn step(position: &Position) -> Option<&'static Step> {
    level(&position.level)?
        .steps
        .iter()
        .find(|step| step.number == position.step)
}

/// How progress names a skill: the level, then the skill's own id,
/// `A1:U2-GRA-01`.
pub fn skill_key(level_code: &str, skill_id: &str) -> String {
    format!("{level_code}:{skill_id}")
}

/// The skill a [`skill_key`] names, with its level and step; `None` for any
/// other key.
pub fn skill_at(key: &str) -> Option<(&'static Level, &'static Step, &'static Skill)> {
    let (code, id) = key.split_once(':')?;
    let level = level(code)?;
    level.steps.iter().find_map(|step| {
        step.skills
            .iter()
            .find(|skill| skill.id == id)
            .map(|skill| (level, step, skill))
    })
}

/// The step after `position`: the next one in its level, or Step 1 of the
/// level above once the last is done. `None` past the top of the ladder, or
/// for a level the curriculum does not have.
pub fn next(position: &Position) -> Option<Position> {
    let index = level_index(&position.level)?;
    let level = &levels()[index];
    if usize::from(position.step) < level.steps.len() {
        return Some(Position::new(&level.code, position.step + 1));
    }
    levels()
        .get(index + 1)
        .map(|above| Position::new(&above.code, 1))
}

/// The name the window shows for a level; an unknown code is shown as it is.
pub fn level_name(code: &str) -> String {
    level(code).map_or_else(|| code.to_owned(), |level| level.name.clone())
}

/// Whether `code` is one of the curriculum's levels, as the placement reads
/// them back.
pub fn is_level(code: &str) -> bool {
    level(code).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn six_levels_a0_to_c1_of_five_steps_each_make_117_skills() {
        let codes: Vec<&str> = levels().iter().map(|level| level.code.as_str()).collect();
        assert_eq!(codes, ["A0", "A1", "A2", "B1", "B2", "C1"]);
        let names: Vec<&str> = levels().iter().map(|level| level.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Pre-Beginner",
                "First Words",
                "Finding My Voice",
                "Speaking Freely",
                "Almost Fluent",
                "Fluent"
            ]
        );
        for level in levels() {
            let numbers: Vec<u8> = level.steps.iter().map(|step| step.number).collect();
            assert_eq!(numbers, [1, 2, 3, 4, 5], "{}", level.code);
            assert!(level.goal.starts_with("I "), "{}: {}", level.code, level.goal);
            for step in &level.steps {
                assert!(!step.title.is_empty() && !step.focus.is_empty());
                assert!((3..=4).contains(&step.skills.len()), "{} step {}", level.code, step.number);
            }
        }
        let skills: usize = levels()
            .iter()
            .flat_map(|level| &level.steps)
            .map(|step| step.skills.len())
            .sum();
        assert_eq!(skills, 117);
    }

    #[test]
    fn every_skill_is_numbered_for_its_own_step_and_its_key_is_unique() {
        let mut keys = HashSet::new();
        for level in levels() {
            for step in &level.steps {
                for skill in &step.skills {
                    let (unit, rest) = skill.id.split_once('-').unwrap();
                    assert_eq!(unit, format!("U{}", step.number), "{} {}", level.code, skill.id);
                    let strand = match skill.strand {
                        Strand::Vocabulary => "VOC",
                        Strand::Grammar => "GRA",
                        Strand::Fluency => "FLU",
                    };
                    assert!(rest.starts_with(strand), "{} {}", level.code, skill.id);
                    let words = skill.label.split_whitespace().count();
                    assert!((1..=3).contains(&words), "{}", skill.label);
                    keys.insert(skill_key(&level.code, &skill.id));
                }
            }
        }
        assert_eq!(keys.len(), 117, "the level makes every key unique");
        assert_eq!(skill_key("A1", "U2-GRA-01"), "A1:U2-GRA-01");
    }

    #[test]
    fn the_skills_are_the_curriculums_own_words() {
        let a0 = level("A0").unwrap();
        assert_eq!(
            a0.steps[1].skills.iter().find(|k| k.id == "U2-GRA-01").unwrap().text,
            "I can say ‘I am’ and ‘I am not’ — I am happy. I am not tired."
        );
        assert_eq!(
            level("C1").unwrap().goal,
            "I can communicate on any topic with accuracy, flexibility, and no hesitation."
        );
        assert_eq!(
            step(&Position::new("A1", 2)).unwrap().title,
            "Talking about the past"
        );
        assert!(step(&Position::new("A1", 6)).is_none());
        assert!(step(&Position::new("C2", 1)).is_none());
        assert!(level("a1").is_none());
    }

    #[test]
    fn the_window_is_shown_a_levels_name_never_its_code() {
        assert_eq!(level_name("A2"), "Finding My Voice");
        assert_eq!(level_name("C1"), "Fluent");
        assert_eq!(level_name("Z9"), "Z9");
    }

    #[test]
    fn the_next_step_walks_a_level_then_starts_the_one_above_and_stops_at_the_top() {
        assert_eq!(next(&Position::new("A0", 1)), Some(Position::new("A0", 2)));
        assert_eq!(next(&Position::new("A2", 4)), Some(Position::new("A2", 5)));
        assert_eq!(next(&Position::new("A0", 5)), Some(Position::new("A1", 1)));
        assert_eq!(next(&Position::new("B2", 5)), Some(Position::new("C1", 1)));
        assert_eq!(next(&Position::new("C1", 5)), None);
        assert_eq!(next(&Position::new("Z9", 1)), None);
    }

    #[test]
    fn everyone_starts_at_step_1_of_a2_until_placement_says_otherwise() {
        assert_eq!(start(), Position::new("A2", 1));
    }

    #[test]
    fn a_progress_key_finds_its_skill_with_the_step_and_level_it_belongs_to() {
        let (level, step, skill) = skill_at("A1:U2-GRA-01").unwrap();
        assert_eq!((level.code.as_str(), step.number), ("A1", 2));
        assert_eq!(
            skill.text,
            "I can use past simple with regular verbs — I walked, I watched, I played."
        );
        assert_eq!(skill_at("C1:U5-FLU-02").unwrap().1.title, "Full mastery");
        assert!(skill_at("A1:U9-GRA-01").is_none());
        assert!(skill_at("Z9:U1-VOC-01").is_none());
        assert!(skill_at("U1-VOC-01").is_none());
        assert_eq!(skill_at("A1:U2-VOC-01").unwrap().2.label, "Past time words");
    }
}
