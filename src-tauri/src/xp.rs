//! XP: a number beside the streak that grows with the time the learner
//! spends saying their answers aloud, in any talk, lesson, chore, placement or
//! Word of the day, whatever their level. Like the streak, it shows effort,
//! never skill. Ella Mobile's `xp.ts`, with the differences below.
//!
//! Each answer said aloud earns its spoken time, but at most
//! [`MOST_MS_PER_WORD`] for each word heard, so a long stretch of sound with
//! few words in it earns little; every [`MS_PER_XP`] of that time is one XP.
//! Typed answers earn none: they have no spoken time (`messages.spoken_ms` is
//! `NULL`), and neither do answers from before Ella kept it.
//!
//! **What "spoken time" is here, and how it compares with the phone.** The
//! phone pays for how long its mic listened to an answer (`turns.listening_ms`),
//! pauses included. The laptop pays `messages.spoken_ms` (`Trace::spoken_ms`,
//! which is `audio_after_vad_ms`), and what that holds depends on the route
//! the answer took:
//! - With the native speech engine, answers are streamed
//!   (`AppService::begin_voice_stream`), and it is the whole recording, mic
//!   open to mic closed: the same measure as the phone's.
//! - Otherwise the answer is sent whole (`send_voice_turn`), and it is the
//!   recording with the silence before and after the speech trimmed off
//!   (`audio::trim_to_speech`, keeping 120 ms before and 180 ms after); pauses
//!   between words stay. When the VAD hears no speech at all, nothing is
//!   trimmed. Only this route earns a little less than the phone would.
//!
//! Either way the cap of [`MOST_MS_PER_WORD`] for each word heard is what keeps
//! a mic left open in silence from paying, as on the phone. If the laptop
//! should earn faster or slower, [`MS_PER_XP`] is the one number to change.
//!
//! **What it does not leave out that the phone does.** The phone pays
//! nothing for an answer its safety screen flagged, nor for one Ella asked to
//! hear again because the phone may have misheard it. The laptop keeps no
//! such mark with an answer: a flagged one is kept, censored, with its spoken
//! time, and there is no "say that again". Leaving flagged answers out would
//! need that mark kept with the answer.
//!
//! **Worked out, not kept.** The phone keeps each answer's XP and the
//! learner's total, because its server outlives changes to the rule and must
//! not rewrite what was earned. The laptop has one learner and its own
//! database, so XP is worked out from the answers each time
//! (`Database::progress`), like the streak; a change to the rule here changes
//! every answer's XP, past ones too.

use serde::{Deserialize, Serialize};

/// The spoken time that makes one XP: ten seconds.
pub const MS_PER_XP: u64 = 10_000;

/// The most spoken time one word heard earns: three seconds, room for a
/// learner who is slow and thinks between words, but not for a long stretch
/// of sound with nothing said in it.
pub const MOST_MS_PER_WORD: u64 = 3_000;

/// A learner's XP as the window shows it: in all, and earned today, with the
/// spoken time that makes one, which the XP sheet's "how you earn it" names,
/// so the window never keeps a rate of its own.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct XpView {
    pub total: u64,
    pub today: u64,
    pub ms_per_xp: u64,
}

impl Default for XpView {
    fn default() -> Self {
        Self { total: 0, today: 0, ms_per_xp: MS_PER_XP }
    }
}

/// The spoken time an answer earns, in milliseconds: its spoken time, at most
/// [`MOST_MS_PER_WORD`] for each word in `text`. Nothing for one with no
/// spoken time, which was typed or kept before its time was.
pub fn earned_ms(spoken_ms: Option<u64>, text: &str) -> u64 {
    let Some(spoken) = spoken_ms else {
        return 0;
    };
    let words = text.split_whitespace().count() as u64;
    spoken.min(words * MOST_MS_PER_WORD)
}

/// The XP that `ms` of earned time makes, whole XPs only.
pub fn xp_of(ms: u64) -> u64 {
    ms / MS_PER_XP
}

/// The learner's XP from every answer they have said aloud: `(spoken_ms,
/// text, said_today)`. Today's is what today's answers added to the total, so
/// time left over from an earlier day can make today's first XP, as on the
/// phone, and `today` never counts a part-XP twice.
pub fn tally<'a>(answers: impl IntoIterator<Item = (Option<u64>, &'a str, bool)>) -> XpView {
    let (mut all, mut today) = (0u64, 0u64);
    for (spoken_ms, text, said_today) in answers {
        let earned = earned_ms(spoken_ms, text);
        all += earned;
        if said_today {
            today += earned;
        }
    }
    XpView {
        total: xp_of(all),
        today: xp_of(all) - xp_of(all - today),
        ms_per_xp: MS_PER_XP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_earns_its_spoken_time_at_most_three_seconds_a_word() {
        assert_eq!(earned_ms(Some(4_000), "I went to the market"), 4_000);
        assert_eq!(earned_ms(Some(20_000), "yes"), 3_000, "a long sound with one word in it");
        assert_eq!(earned_ms(Some(5_000), "   "), 0, "nothing heard, nothing earned");
    }

    #[test]
    fn a_typed_answer_earns_nothing() {
        assert_eq!(earned_ms(None, "I went to the market yesterday"), 0);
    }

    #[test]
    fn ten_seconds_make_one_xp_and_part_of_one_makes_none() {
        assert_eq!(xp_of(9_999), 0);
        assert_eq!(xp_of(10_000), 1);
        assert_eq!(xp_of(25_000), 2);
    }

    #[test]
    fn todays_xp_is_what_todays_answers_added_to_the_total() {
        let words = "one two three four five six seven eight nine ten";
        // 6 s yesterday and 6 s today: 12 s, one XP, made today.
        let xp = tally([(Some(6_000), words, false), (Some(6_000), words, true)]);
        assert_eq!((xp.total, xp.today), (1, 1));

        // Nothing said today: nothing earned today.
        let xp = tally([(Some(25_000), words, false), (None, "typed today", true)]);
        assert_eq!((xp.total, xp.today), (2, 0));
    }

    #[test]
    fn nothing_said_is_no_xp() {
        assert_eq!(tally([]), XpView::default());
    }
}
