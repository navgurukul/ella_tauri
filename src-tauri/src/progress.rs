//! How Ella moves a learner through the curriculum: where the placement chat
//! stops, which skill a talk aims at, what a scored talk does to each skill,
//! and when a step is done.
//!
//! The same rules as Ella Mobile, which has them from Ella Docs' "Adaptive
//! Mastery & Target Selection":
//! - A demonstration counts only when the judge is at least 0.70 sure of it.
//!   Each one raises mastery by `0.40 × confidence × (1 − mastery)`, so strong
//!   evidence moves it faster and it never jumps straight to 1.
//! - A skill is owned at mastery 0.75, once shown in two talks on two topics:
//!   one memorised line is not the skill. From 0.25 it is emerging.
//! - A step is done when its vocabulary and fluency skills are owned and its
//!   grammar skills at least emerging. Grammar rarely shows cleanly in free
//!   talk, and should not hold a learner back.
//! - Each talk aims at the skill with the highest priority. When every skill
//!   still to go has missed twice running, the next step's skills join in, so
//!   one hard skill never blocks the way; it comes back as its priority rises.

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::{
    curriculum::{self, Position, Skill, Strand},
    domain::{Confidence, SkillGrowth, SkillStanding},
};

/// A demonstration below this is not counted at all.
pub const EVIDENCE_FLOOR: f64 = 0.7;
/// How far one demonstration moves mastery towards 1.
pub const LEARNING_RATE: f64 = 0.4;
pub const OWNED_MASTERY: f64 = 0.75;
pub const EMERGING_MASTERY: f64 = 0.25;
/// Talks in a row aimed at a skill without it showing, after which it is stuck.
pub const STUCK_AFTER: u32 = 2;
/// How many of the latest talks' aims count against aiming there again.
pub const RECENT_TARGETS: usize = 4;
/// The summary shows this many grown skills at most.
pub const MAX_GROWN_SKILLS: usize = 3;

/// The placement chat ends after this many answers at the soonest...
pub const PLACEMENT_MIN_TURNS: u32 = 5;
/// ...and after this many at the latest, about four minutes of talking.
pub const PLACEMENT_MAX_TURNS: u32 = 12;

/// How one skill is going for the learner. `skill` is its [`curriculum::skill_key`].
#[derive(Debug, Clone, PartialEq)]
pub struct SkillProgress {
    pub skill: String,
    /// How sure Ella is that the learner has it, from 0 to 1.
    pub mastery: f64,
    /// Talks in which the learner clearly showed it.
    pub sightings: u32,
    /// The topics of those talks, each once, in the order they came: a skill
    /// shown on one topic only may be a memorised line.
    pub topics: Vec<String>,
    /// Talks in a row aimed at the skill that showed none of it.
    pub misses: u32,
    /// The local day it was last shown, `YYYY-MM-DD`.
    pub last_seen: Option<String>,
}

impl SkillProgress {
    pub fn fresh(skill: &str) -> Self {
        Self {
            skill: skill.into(),
            mastery: 0.0,
            sightings: 0,
            topics: Vec::new(),
            misses: 0,
            last_seen: None,
        }
    }
}

pub type ProgressMap = HashMap<String, SkillProgress>;

pub fn by_key(progress: &[SkillProgress]) -> ProgressMap {
    progress
        .iter()
        .map(|entry| (entry.skill.clone(), entry.clone()))
        .collect()
}

fn lookup(progress: &ProgressMap, key: &str) -> SkillProgress {
    progress
        .get(key)
        .cloned()
        .unwrap_or_else(|| SkillProgress::fresh(key))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Locked,
    Emerging,
    Owned,
}

pub fn bucket(progress: &SkillProgress) -> Bucket {
    if progress.mastery >= OWNED_MASTERY && progress.sightings >= 2 && progress.topics.len() >= 2 {
        Bucket::Owned
    } else if progress.mastery >= EMERGING_MASTERY {
        Bucket::Emerging
    } else {
        Bucket::Locked
    }
}

/// Whether a skill has done what its step asks of it.
pub fn passed(skill: &Skill, progress: &SkillProgress) -> bool {
    match skill.strand {
        Strand::Grammar => bucket(progress) != Bucket::Locked,
        Strand::Vocabulary | Strand::Fluency => bucket(progress) == Bucket::Owned,
    }
}

/// Where a skill stands on a level's page. `passed` is whether it has done
/// what its step asks, or sits in a step behind the learner; `aimed`, whether
/// a talk the learner spoke in aimed at it. Shown means seen in a talk at
/// least once, but not yet as its step asks.
pub fn standing(passed: bool, progress: &SkillProgress, aimed: bool) -> SkillStanding {
    if passed {
        SkillStanding::Done
    } else if progress.sightings > 0 {
        SkillStanding::Shown
    } else if aimed {
        SkillStanding::Practising
    } else {
        SkillStanding::NotStarted
    }
}

/// A skill of the curriculum, where it sits, and the key its progress is
/// kept under.
#[derive(Debug, Clone)]
pub struct Placed {
    pub key: String,
    pub position: Position,
    pub skill: &'static Skill,
}

/// The skills of the step at `position`; none for a step the curriculum does
/// not have.
pub fn step_skills(position: &Position) -> Vec<Placed> {
    curriculum::step(position)
        .map(|step| {
            step.skills
                .iter()
                .map(|skill| Placed {
                    key: curriculum::skill_key(&position.level, &skill.id),
                    position: position.clone(),
                    skill,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every skill a talk at `position` can aim at or be scored on: its step's,
/// and the next step's, which a stuck learner moves on to and a strong one
/// may show early.
pub fn skills_in_play(position: &Position) -> Vec<Placed> {
    let mut skills = step_skills(position);
    if let Some(after) = curriculum::next(position) {
        skills.extend(step_skills(&after));
    }
    skills
}

pub fn step_complete(position: &Position, progress: &ProgressMap) -> bool {
    let skills = step_skills(position);
    !skills.is_empty()
        && skills
            .iter()
            .all(|placed| passed(placed.skill, &lookup(progress, &placed.key)))
}

/// How far through their level a learner at `position` is, as a whole
/// percent: the steps before theirs, and the share of their step's skills
/// passed. The profile, Home and the level map all ask here, so they agree.
pub fn level_percent(position: &Position, progress: &ProgressMap) -> u8 {
    let skills = step_skills(position);
    let done = skills
        .iter()
        .filter(|placed| passed(placed.skill, &lookup(progress, &placed.key)))
        .count();
    let share = if skills.is_empty() {
        0.0
    } else {
        done as f64 / skills.len() as f64
    };
    let steps = curriculum::level(&position.level).map_or(5, |level| level.steps.len());
    let behind = f64::from(position.step.saturating_sub(1));
    (100.0 * (behind + share) / steps as f64).round().clamp(0.0, 100.0) as u8
}

/// The skills a talk at `position` may aim at, in curriculum order: those of
/// the step still to pass, joined by the next step's once every one of them
/// is stuck.
pub fn candidates(position: &Position, progress: &ProgressMap) -> Vec<Placed> {
    let open = |skills: Vec<Placed>| -> Vec<Placed> {
        skills
            .into_iter()
            .filter(|placed| !passed(placed.skill, &lookup(progress, &placed.key)))
            .collect()
    };
    let here = open(step_skills(position));
    // Nothing left to pass: the step is done, and the next talk moves on.
    if here.is_empty() {
        return step_skills(position);
    }
    if !here
        .iter()
        .all(|placed| lookup(progress, &placed.key).misses >= STUCK_AFTER)
    {
        return here;
    }
    match curriculum::next(position) {
        Some(after) => here.into_iter().chain(open(step_skills(&after))).collect(),
        None => here,
    }
}

/// Whole days from `from` to `to`, both `YYYY-MM-DD`; 0 if either is unreadable.
fn days_between(from: &str, to: &str) -> i64 {
    match (
        NaiveDate::parse_from_str(from, "%Y-%m-%d"),
        NaiveDate::parse_from_str(to, "%Y-%m-%d"),
    ) {
        (Ok(from), Ok(to)) => (to - from).num_days(),
        _ => 0,
    }
}

/// How much a skill wants practice, with the docs' weights: need 0.35,
/// uncertainty 0.20, review due 0.20, curriculum order 0.15 and variety 0.10,
/// less 0.25 for each share of the recent talks that aimed at it. `index` is
/// its place among the `total` candidates.
pub fn priority(
    progress: &SkillProgress,
    index: usize,
    total: usize,
    recent: &[String],
    today: &str,
) -> f64 {
    let need = 1.0 - progress.mastery;
    let uncertainty = 1.0 / f64::from(progress.sightings + 1).sqrt();
    let review_due = match &progress.last_seen {
        None => 1.0,
        Some(seen) => 1.0 - (-(days_between(seen, today).max(0) as f64) / 7.0).exp(),
    };
    let order = 1.0 - index as f64 / total as f64;
    let variety = 1.0 / f64::from(progress.sightings + 1);
    let recency = recent
        .iter()
        .take(RECENT_TARGETS)
        .filter(|key| **key == progress.skill)
        .count() as f64
        / RECENT_TARGETS as f64;
    0.35 * need + 0.2 * uncertainty + 0.2 * review_due + 0.15 * order + 0.1 * variety
        - 0.25 * recency
}

/// The skill a talk at `position` aims at: the candidate with the highest
/// priority, the earlier one on a tie. `None` only where the curriculum has
/// no such step. `recent` is the latest talks' aims, newest first.
pub fn choose_target(
    position: &Position,
    progress: &ProgressMap,
    recent: &[String],
    today: &str,
) -> Option<Placed> {
    let pool = candidates(position, progress);
    let total = pool.len();
    let mut best: Option<(Placed, f64)> = None;
    for (index, candidate) in pool.into_iter().enumerate() {
        let score = priority(&lookup(progress, &candidate.key), index, total, recent, today);
        if best.as_ref().map_or(true, |(_, top)| score > *top) {
            best = Some((candidate, score));
        }
    }
    best.map(|(placed, _)| placed)
}

/// What one scored talk does to the skills it was scored on. `scores` holds
/// the judge's confidence for each skill the learner showed; one left out was
/// not shown. The talk's aim, when not shown, misses once more. A skill the
/// talk changed nothing about comes back equal to what went in.
pub fn apply_scores(
    progress: &[SkillProgress],
    scores: &HashMap<String, f64>,
    target: Option<&str>,
    topic_id: &str,
    today: &str,
) -> Vec<SkillProgress> {
    progress
        .iter()
        .map(|entry| {
            let confidence = scores.get(&entry.skill).copied().unwrap_or(0.0);
            if confidence >= EVIDENCE_FLOOR {
                let mut topics = entry.topics.clone();
                if !topics.iter().any(|topic| topic == topic_id) {
                    topics.push(topic_id.into());
                }
                return SkillProgress {
                    skill: entry.skill.clone(),
                    mastery: entry.mastery + LEARNING_RATE * confidence * (1.0 - entry.mastery),
                    sightings: entry.sightings + 1,
                    topics,
                    misses: 0,
                    last_seen: Some(today.into()),
                };
            }
            if target == Some(entry.skill.as_str()) {
                return SkillProgress {
                    misses: entry.misses + 1,
                    ..entry.clone()
                };
            }
            entry.clone()
        })
        .collect()
}

/// The skills a scored talk counted: those whose sightings went up, the
/// talk's aim first, then the judge's surest, at most three. `before` and
/// `after` are the same skills in the same order, as scoring left them.
pub fn grown_skills(
    before: &[SkillProgress],
    after: &[SkillProgress],
    scores: &HashMap<String, f64>,
    target: Option<&str>,
) -> Vec<SkillGrowth> {
    let mut grown: Vec<&SkillProgress> = after
        .iter()
        .zip(before)
        .filter(|(now, then)| now.sightings > then.sightings)
        .map(|(now, _)| now)
        .collect();
    let score = |entry: &SkillProgress| scores.get(&entry.skill).copied().unwrap_or(0.0);
    grown.sort_by(|left, right| {
        let aim = |entry: &SkillProgress| u8::from(target == Some(entry.skill.as_str()));
        aim(right)
            .cmp(&aim(left))
            .then(score(right).total_cmp(&score(left)))
    });
    grown
        .into_iter()
        .filter_map(|entry| {
            curriculum::skill_at(&entry.skill).map(|(_, _, skill)| SkillGrowth {
                label: skill.label.clone(),
                count: entry.sightings,
            })
        })
        .take(MAX_GROWN_SKILLS)
        .collect()
}

/// The highest level a placement may read off answers this short, whatever the
/// judge says; `None` when they are long enough for the judge to decide.
///
/// The phone tells its assessor "when the sample is very short, do not guess
/// high". A 3B model does not reliably listen — it read "college", "yes.
/// mother, father, brother" and "cricket. I like" as A1 — so here the words
/// are counted instead: answers of under three words on average stay at A0,
/// under five at A1, and under seven at A2. It only ever holds a reading down.
pub fn level_ceiling(answers: &[&str]) -> Option<&'static str> {
    if answers.is_empty() {
        return None;
    }
    let words: usize = answers.iter().map(|answer| answer.split_whitespace().count()).sum();
    let average = words as f64 / answers.len() as f64;
    if average < 3.0 {
        Some("A0")
    } else if average < 5.0 {
        Some("A1")
    } else if average < 7.0 {
        Some("A2")
    } else {
        None
    }
}

/// `level`, held down to `ceiling` when there is one.
pub fn at_most(level: &str, ceiling: Option<&str>) -> String {
    match ceiling {
        Some(ceiling) if curriculum::level_index(level) > curriculum::level_index(ceiling) => ceiling.into(),
        _ => level.into(),
    }
}

/// Whether the placement chat may end after the learner's `answers`-th answer.
///
/// Never before the fifth, however sure the judge is: nerves can hide what a
/// learner can do in their first answers. From the fifth it ends when the
/// judge is ready and highly confident, from the ninth when it is ready and
/// at least fairly confident, and at the twelfth regardless.
pub fn placement_ends(answers: u32, ready: bool, confidence: Confidence) -> bool {
    if answers >= PLACEMENT_MAX_TURNS {
        return true;
    }
    if !ready || answers < PLACEMENT_MIN_TURNS {
        return false;
    }
    if answers <= 8 {
        return confidence == Confidence::High;
    }
    matches!(confidence, Confidence::High | Confidence::Medium)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "2026-09-26";

    fn a1s1() -> Position {
        Position::new("A1", 1)
    }

    /// Progress on one skill: fresh but for what is given.
    fn at(skill: &str) -> SkillProgress {
        SkillProgress::fresh(skill)
    }

    /// Owned: strong, seen twice, on two topics.
    fn owned(skill: &str) -> SkillProgress {
        SkillProgress {
            mastery: 0.8,
            sightings: 2,
            topics: vec!["cab".into(), "gbu".into()],
            last_seen: Some(TODAY.into()),
            ..at(skill)
        }
    }

    fn keys(placed: &[Placed]) -> Vec<&str> {
        placed.iter().map(|placed| placed.key.as_str()).collect()
    }

    fn scores(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(key, score)| ((*key).into(), *score)).collect()
    }

    #[test]
    fn a_skill_is_owned_only_with_mastery_two_sightings_and_two_topics() {
        assert_eq!(bucket(&at("x")), Bucket::Locked);
        assert_eq!(bucket(&SkillProgress { mastery: 0.25, ..at("x") }), Bucket::Emerging);
        assert_eq!(bucket(&owned("x")), Bucket::Owned);
        let two_topics = vec!["a".to_string(), "b".to_string()];
        let short = |mastery, sightings, topics: Vec<String>| SkillProgress {
            mastery,
            sightings,
            topics,
            ..at("x")
        };
        assert_eq!(bucket(&short(0.74, 3, two_topics.clone())), Bucket::Emerging);
        assert_eq!(bucket(&short(0.9, 1, two_topics)), Bucket::Emerging);
        assert_eq!(bucket(&short(0.9, 3, vec!["a".into()])), Bucket::Emerging);
    }

    #[test]
    fn grammar_passes_once_emerging_while_vocabulary_and_fluency_must_be_owned() {
        let skills = step_skills(&a1s1());
        let (vocabulary, grammar) = (skills[0].skill, skills[1].skill);
        assert_eq!((vocabulary.strand, grammar.strand), (Strand::Vocabulary, Strand::Grammar));
        let emerging = SkillProgress { mastery: 0.3, ..at("x") };
        assert!(passed(grammar, &emerging));
        assert!(!passed(vocabulary, &emerging));
        assert!(passed(vocabulary, &owned("x")));
    }

    #[test]
    fn a_skill_stands_done_shown_practising_or_not_started() {
        // Shown once: short of what passing asks of vocabulary or fluency.
        let seen = SkillProgress {
            mastery: 0.38,
            sightings: 1,
            topics: vec!["cab".into()],
            ..at("x")
        };
        assert_eq!(standing(true, &at("x"), false), SkillStanding::Done, "a step behind the learner");
        assert_eq!(standing(true, &owned("x"), true), SkillStanding::Done);
        assert_eq!(standing(false, &seen, true), SkillStanding::Shown);
        assert_eq!(standing(false, &seen, false), SkillStanding::Shown, "shown in a talk aimed elsewhere");
        assert_eq!(standing(false, &at("x"), true), SkillStanding::Practising);
        // A talk that aimed at it and missed it changes nothing but its misses.
        assert_eq!(standing(false, &SkillProgress { misses: 2, ..at("x") }, true), SkillStanding::Practising);
        assert_eq!(standing(false, &at("x"), false), SkillStanding::NotStarted);
    }

    #[test]
    fn a_steps_skills_are_keyed_by_level_and_the_next_step_is_in_play_too() {
        assert_eq!(keys(&step_skills(&a1s1())), ["A1:U1-VOC-01", "A1:U1-GRA-01", "A1:U1-FLU-01"]);
        assert!(step_skills(&Position::new("A1", 9)).is_empty());
        let in_play = skills_in_play(&Position::new("A0", 5));
        assert_eq!(
            keys(&in_play),
            [
                "A0:U5-GRA-01",
                "A0:U5-GRA-02",
                "A0:U5-FLU-01",
                "A0:U5-FLU-02",
                "A1:U1-VOC-01",
                "A1:U1-GRA-01",
                "A1:U1-FLU-01"
            ]
        );
        // Nothing comes after the last step of C1.
        assert_eq!(skills_in_play(&Position::new("C1", 5)).len(), 4);
    }

    #[test]
    fn a_step_is_complete_when_every_skill_has_passed() {
        let mut progress = by_key(&[
            owned("A1:U1-VOC-01"),
            SkillProgress { mastery: 0.3, ..at("A1:U1-GRA-01") },
        ]);
        assert!(!step_complete(&a1s1(), &progress), "fluency has not shown yet");
        progress.insert("A1:U1-FLU-01".into(), owned("A1:U1-FLU-01"));
        assert!(step_complete(&a1s1(), &progress));
        assert!(!step_complete(&Position::new("Z9", 1), &progress));
    }

    #[test]
    fn a_levels_percent_is_the_steps_behind_and_the_share_of_this_one_passed() {
        assert_eq!(level_percent(&a1s1(), &by_key(&[owned("A1:U1-VOC-01")])), 7);
        assert_eq!(level_percent(&Position::new("A1", 3), &by_key(&[])), 40);
        let emerging = SkillProgress { mastery: 0.3, ..at("A1:U1-GRA-01") };
        assert_eq!(level_percent(&a1s1(), &by_key(&[owned("A1:U1-VOC-01"), emerging])), 13);
        assert_eq!(level_percent(&Position::new("Z9", 2), &by_key(&[])), 20);
    }

    #[test]
    fn a_talk_aims_at_the_steps_skills_still_to_pass() {
        let progress = by_key(&[owned("A1:U1-VOC-01")]);
        assert_eq!(keys(&candidates(&a1s1(), &progress)), ["A1:U1-GRA-01", "A1:U1-FLU-01"]);
    }

    #[test]
    fn when_every_skill_left_is_stuck_the_next_steps_skills_join_in() {
        let mut progress = by_key(&[
            owned("A1:U1-VOC-01"),
            owned("A1:U1-FLU-01"),
            SkillProgress { misses: 1, ..at("A1:U1-GRA-01") },
        ]);
        assert_eq!(keys(&candidates(&a1s1(), &progress)), ["A1:U1-GRA-01"]);
        progress.insert("A1:U1-GRA-01".into(), SkillProgress { misses: 2, ..at("A1:U1-GRA-01") });
        assert_eq!(
            keys(&candidates(&a1s1(), &progress)),
            ["A1:U1-GRA-01", "A1:U2-VOC-01", "A1:U2-GRA-01", "A1:U2-GRA-02", "A1:U2-FLU-01"]
        );
    }

    #[test]
    fn priority_matches_the_docs_worked_example() {
        let fresh: Vec<f64> = (0..4).map(|i| priority(&at(&format!("s{i}")), i, 4, &[], TODAY)).collect();
        for (actual, expected) in fresh.iter().zip([1.0, 0.9625, 0.925, 0.8875]) {
            assert!((actual - expected).abs() < 1e-9, "{actual} vs {expected}");
        }
        let practised = SkillProgress {
            mastery: 0.32,
            sightings: 1,
            topics: vec!["cab".into()],
            last_seen: Some(TODAY.into()),
            ..at("s0")
        };
        let recent = vec!["s0".to_string()];
        assert!((priority(&practised, 0, 4, &recent, TODAY) - 0.52).abs() < 0.005);
        let a_week_ago = SkillProgress { last_seen: Some("2026-09-19".into()), ..practised.clone() };
        assert!(priority(&a_week_ago, 0, 4, &[], TODAY) > priority(&practised, 0, 4, &[], TODAY));
    }

    #[test]
    fn targets_rotate_so_a_fresh_skill_beats_the_one_just_practised() {
        let first = choose_target(&a1s1(), &HashMap::new(), &[], TODAY).unwrap();
        assert_eq!(first.key, "A1:U1-VOC-01");
        assert_eq!(first.position, a1s1());

        let progress = by_key(&[SkillProgress {
            mastery: 0.32,
            sightings: 1,
            topics: vec!["cab".into()],
            last_seen: Some(TODAY.into()),
            ..at("A1:U1-VOC-01")
        }]);
        let recent = vec!["A1:U1-VOC-01".to_string()];
        assert_eq!(choose_target(&a1s1(), &progress, &recent, TODAY).unwrap().key, "A1:U1-GRA-01");
        assert!(choose_target(&Position::new("Z9", 1), &progress, &[], TODAY).is_none());
    }

    #[test]
    fn a_strong_demonstration_raises_mastery_by_the_bounded_step_and_is_recorded() {
        let after = apply_scores(&[at("k")], &scores(&[("k", 0.8)]), Some("k"), "cab", TODAY);
        assert!((after[0].mastery - 0.32).abs() < 1e-9);
        assert_eq!(
            SkillProgress { mastery: 0.0, ..after[0].clone() },
            SkillProgress {
                sightings: 1,
                topics: vec!["cab".into()],
                last_seen: Some(TODAY.into()),
                ..at("k")
            }
        );
        let again = apply_scores(&after, &scores(&[("k", 0.8)]), None, "cab", "2026-09-27");
        assert!((again[0].mastery - (0.32 + 0.4 * 0.8 * 0.68)).abs() < 1e-9);
        assert_eq!((again[0].sightings, again[0].topics.clone()), (2, vec!["cab".to_string()]));
    }

    #[test]
    fn below_the_floor_nothing_counts_and_only_the_target_misses() {
        let before = [SkillProgress { misses: 1, ..at("target") }, at("watched")];
        let after = apply_scores(
            &before,
            &scores(&[("target", 0.69), ("watched", 0.5)]),
            Some("target"),
            "gbu",
            TODAY,
        );
        assert_eq!(after[0], SkillProgress { misses: 2, ..before[0].clone() });
        assert_eq!(after[1], before[1], "an untouched skill comes back unchanged");
    }

    #[test]
    fn showing_a_stuck_skill_clears_its_misses() {
        let after = apply_scores(&[SkillProgress { misses: 3, ..at("k") }], &scores(&[("k", 0.9)]), Some("k"), "cab", TODAY);
        assert_eq!(after[0].misses, 0);
    }

    #[test]
    fn grown_skills_put_the_aim_first_then_the_surest_and_stop_at_three() {
        let keys = ["A1:U1-VOC-01", "A1:U1-GRA-01", "A1:U1-FLU-01", "A1:U2-VOC-01", "A1:U2-GRA-01"];
        let before: Vec<SkillProgress> = keys.iter().map(|key| at(key)).collect();
        let judged = scores(&[
            ("A1:U1-VOC-01", 0.75),
            ("A1:U1-GRA-01", 0.95),
            ("A1:U1-FLU-01", 0.8),
            ("A1:U2-VOC-01", 0.9),
            ("A1:U2-GRA-01", 0.5),
        ]);
        let after = apply_scores(&before, &judged, Some("A1:U1-FLU-01"), "cab", TODAY);
        let grown = grown_skills(&before, &after, &judged, Some("A1:U1-FLU-01"));
        let labels: Vec<&str> = grown.iter().map(|skill| skill.label.as_str()).collect();
        let label = |key: &str| curriculum::skill_at(key).unwrap().2.label.as_str();
        assert_eq!(labels, [label("A1:U1-FLU-01"), label("A1:U1-GRA-01"), label("A1:U2-VOC-01")]);
        assert!(grown.iter().all(|skill| skill.count == 1));
    }

    #[test]
    fn short_answers_hold_a_placement_down_and_longer_ones_leave_it_to_the_judge() {
        let weak = ["college", "yes. mother, father, brother", "cricket. I like", "no.. I go shop", "good"];
        assert_eq!(level_ceiling(&weak), Some("A0"));
        assert_eq!(level_ceiling(&["I go to college", "I have one brother", "I like cricket"]), Some("A1"));
        assert_eq!(level_ceiling(&["I go to college every day.", "I live in Nagpur with family."]), Some("A2"));
        assert_eq!(
            level_ceiling(&["Last weekend we visited my aunt in Pune and walked by the river."]),
            None
        );
        assert_eq!(level_ceiling(&[]), None);

        assert_eq!(at_most("B2", Some("A1")), "A1");
        assert_eq!(at_most("A0", Some("A1")), "A0", "never raises a reading");
        assert_eq!(at_most("B1", None), "B1");
    }

    #[test]
    fn the_placement_chat_ends_between_the_fifth_and_twelfth_answer() {
        use Confidence::*;
        assert!(!placement_ends(4, true, High), "never before the fifth");
        assert!(placement_ends(5, true, High));
        assert!(!placement_ends(5, true, Medium), "early on only a sure judge ends it");
        assert!(!placement_ends(8, true, Medium));
        assert!(placement_ends(9, true, Medium));
        assert!(!placement_ends(11, true, Low));
        assert!(!placement_ends(11, false, High), "not while the judge wants more");
        assert!(placement_ends(12, false, Low), "the twelfth ends it regardless");
    }
}
