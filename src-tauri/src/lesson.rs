//! When Ella, as mentor, has a short lesson ready, and what it is on: Ella
//! Mobile's `lesson.ts`, rule for rule.
//!
//! She has one ready when a skill is stuck (missed in two talks in a row aimed
//! at it), or after every [`LESSON_AFTER`] successful talks since the last
//! lesson the learner took, whether or not those went well. Once ready, it
//! stays ready until it is taken; what it is on is decided afresh after each
//! talk:
//! - While any skill is stuck, it teaches one of them, the one missed most:
//!   something to improve. If several were stuck and some have since been
//!   heard, it is on one still stuck.
//! - Otherwise it practises the skill a talk would aim at next, building on
//!   what grew in the latest talks; when nothing grew, it praises nothing. A
//!   lesson on a stuck skill whose skill has since been heard, with no other
//!   stuck, becomes one of these rather than going away. That skill is chosen
//!   when it is first needed and kept until the lesson is taken
//!   ([`skill_to_keep`]), so what Home and the mentor card name is what Ella
//!   teaches.
//!
//! A successful talk is one the assessment counted: not a placement, long
//! enough for notes, and scored. A lesson is taken once it is a successful
//! talk itself, which starts the count again. One that went past a waiting
//! lesson makes it overdue, and the window then asks the learner to talk to
//! Ella first.
//!
//! Everything here is worked out from what the caller read: where the
//! learner is, their progress on the skills in play there, and the cadence
//! the database keeps (`Database::lesson_cadence`).

use crate::{
    curriculum::Position,
    progress::{self, Placed, ProgressMap, SkillProgress},
};

/// Successful talks since the last lesson, after which one is ready anyway.
pub const LESSON_AFTER: u32 = 2;

/// What Ella says she noticed when a skill is stuck, and that is all she can
/// honestly say.
pub const STUCK_DETAIL: &str =
    "I've been listening for this and haven't heard it yet. Let's practise it together.";

/// What she says when the stuck skill a lesson was ready for has since been heard.
pub const CLEARED_DETAIL: &str = "Let's practise this together until it sticks.";

/// Where a learner is between lessons, as the database keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LessonCadence {
    /// Successful talks since the last lesson they took, lessons aside.
    pub talks: u32,
    /// Whether one of those began with a lesson already waiting.
    pub overdue: bool,
    /// Whether any talk since the last lesson taken, successful or not, began
    /// with a lesson waiting: one was ready, and stays ready until it is taken.
    pub held: bool,
    /// The skills that grew in the latest two of them, newest first, each
    /// once, by the labels the recap shows.
    pub strengths: Vec<String>,
    /// The skill kept for the lesson since it became ready, as the latest
    /// talk since the last lesson taken to keep one recorded it. `None` before
    /// any did.
    pub kept: Option<String>,
}

/// The lesson Ella has ready.
#[derive(Debug, Clone)]
pub struct LessonPlan {
    /// The skill it aims at: the stuck one, or the one a talk would aim at next.
    pub target: Placed,
    /// `None` for a lesson on a stuck skill. Otherwise the skills that grew in
    /// the latest talks, as the recap labels them, the one it teaches left
    /// out. Empty when nothing grew: then there is nothing to praise, and
    /// Ella praises nothing.
    pub strengths: Option<Vec<String>>,
    /// A successful talk went past it since it was ready.
    pub overdue: bool,
    /// Ready only because a lesson on a stuck skill was, whose skill has since
    /// been heard, and no other is stuck: not yet two talks since the last.
    pub cleared: bool,
}

/// The lesson Ella has ready for a learner at `position`; `None` when there
/// is none. `progress` is theirs on the skills in play there
/// (`progress::skills_in_play`), `recent` the latest talks' aims, newest first.
pub fn plan(
    position: &Position,
    progress: &ProgressMap,
    cadence: &LessonCadence,
    recent: &[String],
    today: &str,
) -> Option<LessonPlan> {
    if let Some(stuck) = progress::stuck_skill(position, progress) {
        return Some(LessonPlan {
            target: stuck,
            strengths: None,
            overdue: cadence.overdue,
            cleared: false,
        });
    }
    let by_talks = cadence.talks >= LESSON_AFTER;
    // Not yet two talks, and nothing was ready: no lesson. Something was ready
    // (a stuck skill, since heard): it stays, on something new.
    if !by_talks && !cadence.held {
        return None;
    }
    let next = kept_for(position, progress, cadence.kept.as_deref())
        .or_else(|| progress::choose_target(position, progress, recent, today))?;
    // What she teaches is something new, so it is never also what she praises.
    let strengths = cadence
        .strengths
        .iter()
        .filter(|label| **label != next.skill.label)
        .cloned()
        .collect();
    Some(LessonPlan {
        target: next,
        strengths: Some(strengths),
        overdue: cadence.overdue,
        cleared: !by_talks,
    })
}

/// The skill to keep for a lesson that builds on good talks, recorded with
/// the talk that has just ended: the one Ella has ready, unless it is already
/// kept. `None` for a lesson on a stuck skill, or none.
///
/// Without it, the lesson would follow whatever a talk would aim at next,
/// which moves each time one starts: Home could name one skill and the lesson
/// teach another.
pub fn skill_to_keep(plan: Option<&LessonPlan>, cadence: &LessonCadence) -> Option<String> {
    let plan = plan?;
    plan.strengths.as_ref()?;
    (cadence.kept.as_deref() != Some(plan.target.key.as_str())).then(|| plan.target.key.clone())
}

/// The kept skill, while it is still one a talk could aim at and still to
/// pass; `None` otherwise, when the lesson chooses again.
fn kept_for(position: &Position, progress: &ProgressMap, key: Option<&str>) -> Option<Placed> {
    let key = key?;
    let kept = progress::candidates(position, progress)
        .into_iter()
        .find(|candidate| candidate.key == key)?;
    let record = progress
        .get(key)
        .cloned()
        .unwrap_or_else(|| SkillProgress::fresh(key));
    (!progress::passed(kept.skill, &record)).then_some(kept)
}

/// What Ella says on Home and the mentor card about the lesson she has ready.
pub fn detail(plan: &LessonPlan) -> String {
    let Some(strengths) = &plan.strengths else {
        return STUCK_DETAIL.into();
    };
    if plan.cleared {
        return CLEARED_DETAIL.into();
    }
    // Nothing grew in those talks: nothing to praise, so she claims nothing.
    match strengths.first() {
        None => "You've had two talks since our last lesson. Let's practise something new together.".into(),
        Some(best) => format!(
            "You did well with {} in your last talks. Let's build on it with something new.",
            lower_first(best)
        ),
    }
}

fn lower_first(label: &str) -> String {
    let mut letters = label.chars();
    match letters.next() {
        Some(first) => first.to_lowercase().chain(letters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::by_key;

    const TODAY: &str = "2026-10-13";

    fn a1s1() -> Position {
        Position::new("A1", 1)
    }

    fn at(skill: &str) -> SkillProgress {
        SkillProgress::fresh(skill)
    }

    fn cadence(talks: u32) -> LessonCadence {
        LessonCadence {
            talks,
            ..LessonCadence::default()
        }
    }

    #[test]
    fn nothing_is_ready_before_two_talks_unless_a_skill_is_stuck() {
        assert!(plan(&a1s1(), &by_key(&[]), &cadence(1), &[], TODAY).is_none());

        let stuck = by_key(&[SkillProgress { misses: 2, ..at("A1:U1-GRA-01") }]);
        let ready = plan(&a1s1(), &stuck, &cadence(0), &[], TODAY).unwrap();
        assert_eq!(ready.target.key, "A1:U1-GRA-01");
        assert!(ready.strengths.is_none(), "a lesson on something to improve praises nothing");
        assert_eq!(detail(&ready), STUCK_DETAIL);
    }

    #[test]
    fn two_talks_make_a_lesson_on_the_skill_a_talk_would_aim_at_next() {
        let grown = LessonCadence {
            talks: 2,
            strengths: vec!["Past time words".into(), "Saying hello".into()],
            ..LessonCadence::default()
        };
        let ready = plan(&a1s1(), &by_key(&[]), &grown, &[], TODAY).unwrap();
        let aimed = progress::choose_target(&a1s1(), &by_key(&[]), &[], TODAY).unwrap();
        assert_eq!(ready.target.key, aimed.key);
        assert!(!ready.cleared && !ready.overdue);
        assert_eq!(
            detail(&ready),
            "You did well with past time words in your last talks. Let's build on it with something new."
        );
    }

    #[test]
    fn what_she_teaches_is_never_also_what_she_praises() {
        let aimed = progress::choose_target(&a1s1(), &by_key(&[]), &[], TODAY).unwrap();
        let grown = LessonCadence {
            talks: 2,
            strengths: vec![aimed.skill.label.clone()],
            ..LessonCadence::default()
        };
        let ready = plan(&a1s1(), &by_key(&[]), &grown, &[], TODAY).unwrap();
        assert_eq!(ready.strengths, Some(Vec::new()));
        assert_eq!(
            detail(&ready),
            "You've had two talks since our last lesson. Let's practise something new together."
        );
    }

    #[test]
    fn a_lesson_held_for_a_stuck_skill_since_heard_stays_on_something_new() {
        let held = LessonCadence {
            talks: 1,
            held: true,
            ..LessonCadence::default()
        };
        let ready = plan(&a1s1(), &by_key(&[]), &held, &[], TODAY).unwrap();
        assert!(ready.cleared);
        assert_eq!(detail(&ready), CLEARED_DETAIL);
    }

    #[test]
    fn the_kept_skill_holds_until_it_passes() {
        let kept = LessonCadence {
            talks: 2,
            kept: Some("A1:U1-FLU-01".into()),
            ..LessonCadence::default()
        };
        assert_eq!(plan(&a1s1(), &by_key(&[]), &kept, &[], TODAY).unwrap().target.key, "A1:U1-FLU-01");
        // Owned since: the lesson chooses again.
        let owned = SkillProgress {
            mastery: 0.8,
            sightings: 2,
            topics: vec!["cab".into(), "gbu".into()],
            ..at("A1:U1-FLU-01")
        };
        let moved = plan(&a1s1(), &by_key(&[owned]), &kept, &[], TODAY).unwrap();
        assert_ne!(moved.target.key, "A1:U1-FLU-01");
    }

    #[test]
    fn a_talk_keeps_the_skill_once_and_never_for_a_stuck_one() {
        let ready = plan(&a1s1(), &by_key(&[]), &cadence(2), &[], TODAY).unwrap();
        let key = ready.target.key.clone();
        assert_eq!(skill_to_keep(Some(&ready), &cadence(2)), Some(key.clone()));
        let kept = LessonCadence {
            talks: 2,
            kept: Some(key),
            ..LessonCadence::default()
        };
        assert_eq!(skill_to_keep(Some(&ready), &kept), None, "already kept");

        let stuck = by_key(&[SkillProgress { misses: 2, ..at("A1:U1-GRA-01") }]);
        let on_stuck = plan(&a1s1(), &stuck, &cadence(0), &[], TODAY).unwrap();
        assert_eq!(skill_to_keep(Some(&on_stuck), &cadence(0)), None);
        assert_eq!(skill_to_keep(None, &cadence(0)), None);
    }

    #[test]
    fn a_lesson_gone_past_is_overdue() {
        let past = LessonCadence {
            talks: 3,
            overdue: true,
            held: true,
            ..LessonCadence::default()
        };
        assert!(plan(&a1s1(), &by_key(&[]), &past, &[], TODAY).unwrap().overdue);
    }
}
