//! The required level check, as on Ella Mobile (`placement_recheck.ts`): a
//! learner placed by a cut-off takes the placement chat again, once, before
//! any other talk, and it can move them up or down.
//!
//! On the phone the cut-off is a server setting, `ELLA_PLACEMENT_RECHECK_BEFORE`,
//! so a campaign starts without a release. Ella Desktop has no server and ships
//! its rules in the app, so the same setting is read when the app is built:
//! unset, as by default, there is no check, as on the phone. Whether a release
//! runs one, and from when, is a product decision, not this module's.
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

/// The cut-off this build was made with, if any: learners placed by then owe
/// a check. Unset, or not a time, means none, as on the phone.
pub fn cutoff() -> Option<DateTime<Utc>> {
    option_env!("ELLA_PLACEMENT_RECHECK_BEFORE").and_then(parse_cutoff)
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
