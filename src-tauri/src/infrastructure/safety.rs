//! Deterministic content filter checked before the learner's text ever
//! reaches the model.
//!
//! The prompt-level guardrail in `engines::ella_system_prompt` /
//! `chore_system_prompt` is not enough on its own: a live session showed the
//! model does not reliably self-correct once one turn slips into a soft,
//! validating register. Turn one accepted a compliment ("thank you, that's
//! kind"), and every turn after it stayed in that register — "that's sweet",
//! "that's bold", "that's personal" — instead of snapping back to a firm
//! decline, even though the guardrail text was present the whole time. A
//! flagged turn here never calls the model at all, so there is nothing for it
//! to compound.
//!
//! This used to also carry a hand-written phrase list alongside rustrict, to
//! cover romantic phrasing built entirely from ordinary, non-vulgar words
//! ("spend the night with you", "you are beautiful" — neither contains a bad
//! word) that rustrict's own profanity/vulgarity model does not recognize.
//! That list was removed: it can only ever enumerate phrasing already seen,
//! and every combination it doesn't happen to list slips through with no
//! warning that it was never covered. rustrict (MIT/Apache-2.0, a maintained
//! library rather than a list we guess at) is the deterministic floor now;
//! everything it does not catch — including, confirmed by testing, plain
//! compliments and "hug"/"spend the night"-style requests — falls through to
//! the LLM prompt guardrail instead, on the same terms as any other
//! phrasing not on a list. `Type::INAPPROPRIATE` (profane, offensive,
//! sexual, or severely mean) is rustrict's own recommended general-audience
//! threshold, confirmed by testing to stay clear of ordinary chore friction
//! ("that price is a rip-off!"), which only reaches its milder `MEAN` level.
use rustrict::{CensorStr, Type};

/// Whether `text` trips rustrict's general-audience profanity/vulgarity
/// threshold.
pub fn flagged(text: &str) -> bool {
    text.is(Type::INAPPROPRIATE)
}

/// A censored copy of `text` with anything rustrict recognizes as profane,
/// offensive, or sexual replaced by asterisks — a no-op on ordinary text.
/// Used so a flagged turn is never written to the transcript verbatim: the
/// database is real learner history, not a throwaway log, and this is what
/// gets read back during a transcript review like the one that found the
/// cases in this file.
pub fn censor(text: &str) -> String {
    text.censor()
}

/// A fixed decline that never repeats the flagged text back, followed by the
/// exact question the learner left unanswered — reusing `question` this way
/// covers a second failure the same live session showed: a redirect that
/// invents a new, deeper question reads as though the dodge worked instead
/// of being noticed. Falls back to a generic line on the rare turn where
/// there is no question to hand back (asked for, but never observed in
/// practice — every Ella turn ends in exactly one question by design).
pub fn redirect_reply(question: Option<&str>) -> String {
    match question {
        Some(question) => format!("That's not something I talk about. {question}"),
        None => "That's not something I talk about. Let's carry on.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_the_explicit_requests_from_the_real_session() {
        for line in [
            "I would like to go on a date with you.",
            "Do you want to kiss me?",
            "No, I want to have sex with you.",
        ] {
            assert!(flagged(line), "{line:?} should have been flagged");
        }
    }

    #[test]
    fn does_not_catch_compliments_or_hug_or_spend_the_night_requests() {
        // Confirmed by direct testing against rustrict: these contain no
        // vulgar vocabulary, so its profanity/vulgarity model does not flag
        // them. This is a known, accepted gap since the hand-written phrase
        // list that used to cover it was removed — these now fall through
        // to the LLM prompt guardrail like any other unlisted phrasing, on
        // the judgement that a permanently incomplete list is worse than no
        // list. See `engines::ella_system_prompt` for the prompt-level
        // handling of exactly this case.
        for line in [
            "You are very beautiful.",
            "A hug with you.",
            "I want to spend the night with you.",
        ] {
            assert!(!flagged(line), "{line:?} is not caught by the deterministic filter");
        }
    }

    #[test]
    fn leaves_ordinary_topic_conversation_alone() {
        for line in [
            "I ate bhel puri near my college yesterday.",
            "It was spicy and a little sweet, with onion on top.",
            "My mother used to make it at home on Sunday evenings.",
            "Can you tell me the price of this shirt?",
            "I have worked as a shop assistant for two years.",
        ] {
            assert!(!flagged(line), "{line:?} should not have been flagged");
        }
    }

    #[test]
    fn redirect_hands_back_the_exact_question_instead_of_a_new_one() {
        assert_eq!(
            redirect_reply(Some("Where did you find it?")),
            "That's not something I talk about. Where did you find it?"
        );
    }

    #[test]
    fn catches_obfuscated_profanity() {
        // Character substitution and spacing tricks a hand-written list
        // would need to enumerate one by one; rustrict is resistant to
        // these by design.
        assert!(flagged("f u c k you"));
        assert!(flagged("sh1t"));
    }

    #[test]
    fn does_not_flag_ordinary_haggling_friction() {
        // A market-cloth-price transcript has real friction in it ("that's
        // a rip-off", "you're cheating me") that must not be read as
        // INAPPROPRIATE — that is milder MEAN, which the threshold excludes.
        for line in [
            "That price is a rip-off.",
            "You are cheating me, that's too expensive.",
            "No, that is not a fair price at all.",
        ] {
            assert!(!flagged(line), "{line:?} is ordinary bargaining, not inappropriate");
        }
    }

    #[test]
    fn censor_redacts_profanity_but_leaves_ordinary_text_alone() {
        assert_ne!(censor("f u c k you"), "f u c k you");
        assert_eq!(
            censor("I ate bhel puri near my college yesterday."),
            "I ate bhel puri near my college yesterday."
        );
    }
}
