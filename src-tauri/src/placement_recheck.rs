//! The required level check, as on Ella Mobile (`placement_recheck.ts`): a
//! learner placed by a cut-off takes the placement chat again, once, before
//! any other talk, and it can move them up or down.
//!
//! On the phone the cut-off is a server setting, so a campaign starts without
//! a release. Ella Desktop has no server and ships its rules in the app, so
//! the cut-off is a constant here, [`BRANCH_RECHECK_CUTOFF`], with the same
//! meaning: learners placed by then owe one check once it has passed.
//!
//! Who owes one is read off the talks the laptop keeps
//! (`Database::placement_recheck_due`); nothing new is stored. The check
//! itself is a placement chat started while one is due: see
//! `AppService::assess_placement`.

use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    curriculum::{self, Position},
    progress::{self, ProgressMap},
};

/// FEATURE-BRANCH CUT-OFF — NOT A PRODUCTION DECISION.
///
/// The level check is on in this branch so its whole flow can be exercised:
/// every learner whose placement read a level before this moment owes one
/// check. It is the day the branch was started. Before this merges to
/// production, product decides whether a campaign runs at all and, if so,
/// from when; this constant is the one place that changes.
pub const BRANCH_RECHECK_CUTOFF: &str = "2026-11-25T00:00:00Z";

/// [`BRANCH_RECHECK_CUTOFF`] as a moment: learners placed by then owe a check.
pub fn cutoff() -> DateTime<Utc> {
    parse_cutoff(BRANCH_RECHECK_CUTOFF).expect("BRANCH_RECHECK_CUTOFF is RFC 3339, and a test reads it")
}

/// A cut-off in RFC 3339, `2026-12-01T00:00:00Z`. Anything else is no
/// cut-off.
pub fn parse_cutoff(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim()).ok().map(|at| at.with_timezone(&Utc))
}

/// How a cut-off is kept with the check that answered it, the way the phone
/// keeps it (`toISOString`): one spelling for one moment, so a later build
/// with the same cut-off written differently still knows the check is done.
pub fn label(cutoff: &DateTime<Utc>) -> String {
    cutoff.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Where a finished check puts a learner at `current` whom it read at
/// `level`, as Ella Mobile's `recheckedPosition`: the same level keeps their
/// step. Another level resumes the work already earned there: the first step
/// at or past the highest they ever reached on it (`highest`, 1 when never)
/// that is not complete, or its last step when every one is.
pub fn rechecked_position(current: &Position, level: &str, highest: u8, known: &ProgressMap) -> Position {
    if level == current.level {
        return current.clone();
    }
    let Some(steps) = curriculum::level(level).map(|level| &level.steps) else {
        return current.clone();
    };
    let positions: Vec<Position> = steps.iter().map(|step| Position::new(level, step.number)).collect();
    positions
        .iter()
        .find(|position| position.step >= highest && !progress::step_complete(position, known))
        .or_else(|| positions.last())
        .cloned()
        .unwrap_or_else(|| current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::SkillProgress;

    fn passed_step(position: &Position) -> Vec<SkillProgress> {
        progress::step_skills(position)
            .into_iter()
            .map(|placed| SkillProgress {
                mastery: 0.95,
                sightings: 3,
                topics: vec!["a".into(), "b".into()],
                ..SkillProgress::fresh(&placed.key)
            })
            .collect()
    }

    #[test]
    fn the_branch_cut_off_is_a_moment() {
        assert_eq!(label(&cutoff()), "2026-11-25T00:00:00.000Z");
    }

    #[test]
    fn a_cut_off_is_read_as_rfc3339_and_anything_else_is_none() {
        let at = parse_cutoff("2026-12-01T05:30:00+05:30").unwrap();
        assert_eq!(label(&at), "2026-12-01T00:00:00.000Z");
        assert_eq!(parse_cutoff(" 2026-12-01T00:00:00Z "), Some(at));
        assert_eq!(parse_cutoff(""), None);
        assert_eq!(parse_cutoff("off"), None);
        assert_eq!(parse_cutoff("2026-12-01"), None);
    }

    #[test]
    fn the_same_level_keeps_the_learners_step() {
        let current = Position::new("A2", 3);
        assert_eq!(rechecked_position(&current, "A2", 1, &ProgressMap::new()), current);
    }

    #[test]
    fn a_new_level_starts_at_its_first_step_not_complete() {
        let current = Position::new("A2", 3);
        assert_eq!(rechecked_position(&current, "B1", 1, &ProgressMap::new()), Position::new("B1", 1));
        assert_eq!(rechecked_position(&current, "A1", 1, &ProgressMap::new()), Position::new("A1", 1));
        // Step 1 of B1 already passed, as a strong learner's talks can show.
        let known = progress::by_key(&passed_step(&Position::new("B1", 1)));
        assert_eq!(rechecked_position(&current, "B1", 1, &known), Position::new("B1", 2));
    }

    #[test]
    fn a_level_they_were_on_before_resumes_where_they_had_reached() {
        let current = Position::new("B1", 2);
        assert_eq!(rechecked_position(&current, "A2", 4, &ProgressMap::new()), Position::new("A2", 4));
        // The step reached is complete: the next one.
        let known = progress::by_key(&passed_step(&Position::new("A2", 4)));
        assert_eq!(rechecked_position(&current, "A2", 4, &known), Position::new("A2", 5));
    }

    #[test]
    fn a_level_with_every_step_done_resumes_at_its_last_step() {
        let level = curriculum::level("A1").unwrap();
        let last = level.steps.last().unwrap().number;
        let known: Vec<SkillProgress> = level
            .steps
            .iter()
            .flat_map(|step| passed_step(&Position::new("A1", step.number)))
            .collect();
        let known = progress::by_key(&known);
        assert_eq!(
            rechecked_position(&Position::new("A2", 2), "A1", 1, &known),
            Position::new("A1", last)
        );
    }

    #[test]
    fn a_level_the_curriculum_does_not_have_moves_nobody() {
        let current = Position::new("A2", 3);
        assert_eq!(rechecked_position(&current, "Z9", 1, &ProgressMap::new()), current);
    }
}
