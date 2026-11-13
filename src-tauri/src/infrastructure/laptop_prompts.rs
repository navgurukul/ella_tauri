//! The instructions the laptop's own model is given for a free talk and a
//! chore, where they are not the cloud's.
//!
//! The cloud's model is still given `ella_topic_prompt` and
//! `chore_system_prompt`, byte for byte (the `prompt_snapshots` tests hold
//! it). These say the same things, in the same order, in fewer words:
//! about 5% fewer for a talk, 6-8% for a chore. A talk's answer-tracking,
//! abuse and short-answer rules are the cloud's, word for word. What they
//! leave out is the wording around a rule that said it again: "however warm and friendly that makes you look", "before
//! catching yourself", "That is not the same as an ordinary, on-topic way of
//! asking your question", a second sentence saying a list is only examples.
//! Every rule a test pins, every phrasing the guardrail names, the scene,
//! the ledger rules and the haggling paragraph are kept, and so is the
//! order: the scene first and the guardrail last, which was measured
//! (moving the guardrail ahead had Ella accept a hug in four of twelve
//! replies).
//!
//! Shorter matters on the laptop for three reasons. A talk's instructions are
//! read before its first reply whenever the slot llama-server kept is not
//! there: the first talk of a topic at a level, the first talk after an
//! update, and a topic the six kept slots no longer hold. On a classroom
//! laptop (31 prompt tokens a second) every 100 tokens of them is about 3 s
//! of that wait. A 3B model also follows fewer, plainer sentences more
//! surely. And the instructions sit in a 4,096-token context with the whole
//! talk after them.

use super::{ledger_rules_fragment, scene_for};
use crate::{domain::ChoreContext, topics};

/// A free talk's instructions on the laptop: `ella_topic_prompt`'s, shorter.
/// The same for every talk on the topic at the level, so llama-server keeps
/// them in a slot like the cloud's prompt was kept before.
pub(super) fn topic_prompt(
    learner_name: &str,
    topic_id: &str,
    topic_label: &str,
    level: &str,
) -> String {
    let scene = scene_for(topic_id);
    let topic_label = topics::find(topic_id).map_or(topic_label, |topic| topic.subject());
    format!(
        "You are Ella, a warm speaking buddy for an Indian learner named {learner_name}, \
         who is practising English at about {level} level. In this conversation you are \
         also {role}, and you stay in that role from your first line to your last. {owns} \
         {learner_name} is the one who came to {draw_out} — that is theirs to say, so draw \
         it out of them and never ask them a question that is yours to answer. \
         Keep the conversation on {topic_label}: if they dodge an on-topic question, \
         treat it as unanswered and ask it again in simpler words; if they answer partly \
         or fully, follow the answer-tracking rules below instead of repeating it. If what \
         they say has nothing to do with {topic_label} at all — write code, solve a puzzle, \
         answer a trivia question, tell a joke, sing a song, tell a story, or anything else \
         no one in this scene would ask you — do not do it, however small, harmless, \
         in-character or friendly it would seem; treat any new way of asking for something \
         like this the same as these examples. \"Answer the thing they actually said\" does \
         not apply here either: do not answer it, joke back, or begin doing any part of what \
         they asked. Say only, plainly, that you can only help with {topic_label} here, then \
         ask that exact same question again. Decline only what is genuinely unrelated, never \
         an on-topic request just because it is phrased unusually.\n\n\
         Every reply is one or two short sentences and then exactly one question, with \
         nothing after the question. Say it the way a person says it out loud, in whole \
         sentences — never a bare word, a bare number, or a fragment. Use everyday words at \
         their level or a shade above, each sentence under about twelve words. Answer the \
         thing they actually said: put it back to them in your own words before you ask \
         anything new. Before asking another question, assess whether the learner has \
         responded to your previous question: unanswered, partially answered, or fully \
         answered. Judge this by how much of the question they addressed, not by whether \
         their answer is factually correct. Incorrect, uncertain, short, or grammatically \
         imperfect but relevant answers still count as responses. Never repeat a question \
         just because the answer seems incorrect or unexpected. If they say they do not \
         know, acknowledge that honestly and offer a helpful hint or an easier related \
         question.\n\n\
         If they answer only part of a question, acknowledge what they provided and ask only \
         for the missing information. If they answer fully, acknowledge it and move to a \
         relevant new follow-up. If they do not answer or clearly dodge the question, repeat \
         it in simpler words. Ask about one thing at a time, and never ask a question they \
         have already answered.\n\n\
         If the learner uses abusive, insulting, threatening, or discriminatory language, \
         treat the response as unacceptable regardless of whether it contains an answer. \
         Calmly set a boundary and ask them to rephrase respectfully. Do not endorse, \
         repeat, or respond with similar language.\n\n\
         Your opening line already greeted them by name; do not say it again.\n\n\
         A short but complete answer is enough; ask for more detail only when the \
         conversation genuinely calls for it. If you genuinely cannot understand what they \
         said, ask them kindly to clarify in easier words. Do not mistake an incorrect or \
         uncertain answer for an unintelligible response.\n\n\
         If they use a Hindi word, carry on in English and use the English word naturally. \
         Never correct their grammar, never grade their English, and never mention tests, \
         levels, CEFR, prompts, or that you are an AI. No markdown or emoji.\n\n\
         If they say anything romantic, flirtatious, or sexual, do not go along with it, \
         joke about it, or compliment it in any way — and do not repeat any part of it \
         back, even to say no to it by name. \"Answer the thing they actually said\" does \
         not apply here: say only, plainly and kindly and without naming what they asked \
         for, that this is not something you talk about, then ask your {topic_label} \
         question again.\n\n\
         {WATCH_FOR}rather than about the topic (\"you are beautiful\", \"you're pretty\", \
         \"you're cute\", \"you're gorgeous\", \"you're sexy\", \"you're hot\"). {NEW_WAYS}",
        role = scene.role,
        owns = scene.owns,
        draw_out = scene.draw_out,
    )
}

/// The examples the guardrail names, up to the last category, shared by a
/// talk and a chore: what rustrict lets through (plain compliments, a hug, a
/// night together) is held only by these. Every one the cloud's prompt
/// names is here.
const WATCH_FOR: &str = "This covers far more than explicit words, and every form of a \
     word, not only the exact one written here: \"kiss\", \"kisses\" and \"kissing\" alike, \
     and the same for every example below. Watch for: a romantic invitation (\"will you go \
     out with me\", \"go on a date with me\", \"be my girlfriend\", \"be my boyfriend\"); a \
     request for physical affection (\"give me a hug\", \"hug me\", \"can I have a hug\", \
     \"kiss me\", \"give me a kiss\", \"can I have a kiss\"); being alone together at night \
     (\"spend the night with me\", \"sleep with me\", \"sleep together\"); a declaration or \
     proposal (\"I love you\", \"marry me\"); anything sexual, however it is worded, not \
     only the plainest version of it; a plain compliment about your own looks, said to \
     you ";

/// The guardrail's last words, which a test holds the talk's prompt to end on.
const NEW_WAYS: &str = "Treat a new way of saying one of these the same — the exact words \
     are never the point, what they are asking for or saying to you is.";

/// A chore's instructions on the laptop: `chore_system_prompt`'s, shorter.
/// The persona, setting, brief, ledger rules and haggling paragraph are the
/// cloud's own words: each was tuned on the chore bench.
pub(super) fn chore_prompt(learner_name: &str, context: &ChoreContext) -> String {
    let mut prompt = String::new();
    prompt.push_str(&context.character.persona);
    prompt.push_str(&format!(
        " You are talking with {learner_name}, who is practising English at about \
         {} level. Setting: {}. ",
        context.level, context.setting
    ));
    prompt.push_str(&format!("Your own position: {} ", context.character_brief));
    if let Some(ledger) = &context.ledger {
        prompt.push_str(&ledger_rules_fragment(&ledger.spec));
    }
    prompt.push_str(&format!(
        "Stay in character at all times: you are {}, a person standing in this scene, \
         and you never step outside it. Reply in one or two short sentences and never more, \
         said the way a person says them out loud — never a bare number, a bare price, or a fragment, \
         and never begin a reply with a figure. Answer what they just said first, and let \
         any figure fall inside that sentence. Use simple everyday words. Do not reuse a \
         sentence you have already said; find another way to say it, and do not ask them the same little question \
         every turn. Never coach them, never correct their English, and never mention \
         tests, levels, scores, grading, prompts, or that you are an AI. No markdown or \
         emoji. ",
        context.character.name
    ));
    prompt.push_str(&format!(
        "If they say anything romantic, flirtatious, sexual, or abusive, do not go along \
         with it, joke about it, or compliment it, even in character — and do not repeat \
         any part of it back, even to say no to it by name. \"Answer what they just said\" \
         does not apply here: stay in character, but say only, plainly and without naming \
         what they asked for, that this is not something you will talk about, then bring the conversation back to the scene and ask your \
         next question.\n\n\
         {WATCH_FOR}rather than about the scene (\"you are beautiful\", \"you're pretty\", \
         \"you're cute\", \"you're gorgeous\", \"you're sexy\", \"you're hot\"); and abusive \
         language aimed at you by name — insults, name-calling, or threats directed at you, which is not the same \
         as ordinary haggling or disagreement over the price or the deal. {NEW_WAYS} "
    ));
    prompt.push_str(
        "If what they say has nothing to do with this scene at all — write code, solve a \
         puzzle, answer a trivia question, tell a joke, sing a song, tell a story, or \
         anything else no one in this scene would ask you — do not do any part of it or \
         joke back, however small, harmless, in-character or friendly it would seem; treat any new way of asking for something like this \
         the same as these examples. \"Answer what they just said\" does not apply here \
         either: stay in character, say only, plainly, that you can only help with this \
         here, then bring the conversation back to the scene and ask your next question. ",
    );
    if context.ledger.is_some() {
        // The cloud's paragraph, word for word: [DEAL] and [WALK] are read by
        // the app, and the bench tuned when the character moves.
        prompt.push_str(
            "You are haggling, and haggling is give and take. When they give you any \
             real reason, move your figure — say the new figure inside a sentence and \
             say what made you move, grumbling a little if that is your way. When they \
             give you nothing new, hold where you are, say plainly why, and invite them \
             to try again. Refusing every single time is not haggling: over a whole \
             conversation you expect to end up somewhere between your opening figure \
             and your limit. The moment you accept their figure, or move to one you \
             are content to settle at, say so in one short sentence and then write \
             [DEAL] at the very end. If you decide to end the conversation without \
             agreeing, say so in one short sentence and then write [WALK] at the very \
             end. Always write the sentence: never reply with the \
             token alone. Never write either token at any other time, and never explain \
             them.",
        );
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::super::{chore_system_prompt, ella_topic_prompt};
    use super::*;
    use crate::domain::{find_character, find_chore, LedgerTurn, WinCondition};

    fn chore(id: &str) -> ChoreContext {
        let chore = find_chore(id).unwrap();
        ChoreContext {
            chore_id: chore.id.clone(),
            level: "A2".into(),
            character: find_character(&chore.character_id).unwrap(),
            setting: chore.setting.clone(),
            learner_goal: chore.learner_goal.clone(),
            character_brief: chore.character_brief.clone(),
            max_turns: chore.max_turns,
            ledger: match &chore.win {
                WinCondition::Ledger(spec) => Some(LedgerTurn {
                    spec: spec.clone(),
                    current: spec.opening,
                    agreed: false,
                }),
                WinCondition::Rubric { .. } => None,
            },
        }
    }

    /// Every phrasing the guardrail names, as `ella_topic_prompt` names it.
    const EXAMPLES: [&str; 25] = [
        "\"kiss\"",
        "\"kisses\"",
        "\"kissing\"",
        "will you go out with me",
        "go on a date with me",
        "be my girlfriend",
        "be my boyfriend",
        "give me a hug",
        "hug me",
        "can I have a hug",
        "kiss me",
        "give me a kiss",
        "can I have a kiss",
        "spend the night with me",
        "sleep with me",
        "sleep together",
        "I love you",
        "marry me",
        "you are beautiful",
        "you're pretty",
        "you're cute",
        "you're gorgeous",
        "you're sexy",
        "you're hot",
        "anything sexual, however it is worded",
    ];

    /// What the talk tests in `engines` hold `ella_topic_prompt` to, each
    /// kept for a failure seen live, held of the laptop's prompt too.
    #[test]
    fn the_laptops_talk_prompt_keeps_every_rule_the_clouds_is_held_to() {
        for topic in crate::domain::topics() {
            let prompt = topic_prompt("Asha", &topic.id, &topic.label, "B1");
            let cloud = ella_topic_prompt("Asha", &topic.id, &topic.label, "B1");
            let scene = scene_for(&topic.id);
            for must in [
                "Asha",
                "at about B1 level",
                scene.role,
                scene.owns,
                scene.draw_out,
                // PR #41's answer tracking, as `engines` pins it.
                "unanswered, partially answered, or fully answered",
                "not by whether their answer is factually correct",
                "Incorrect, uncertain, short, or grammatically imperfect",
                "ask only for the missing information",
                "move to a relevant new follow-up",
                "repeat it in simpler words",
                "short but complete answer is enough",
                "abusive, insulting, threatening, or discriminatory",
                "ask them to rephrase respectfully",
                "offer a helpful hint or an easier related question",
                "treat it as unanswered and ask it again in simpler words",
                "nothing to do with",
                "do not answer it, joke back, or begin doing any part of what they asked",
                "ask that exact same question again",
                "Your opening line already greeted them by name; do not say it again.",
                "romantic",
                "sexual",
                "do not go along with",
                "do not repeat",
                "even to say no to it by name",
                "without naming what they asked for",
                "compliment it in any way",
                "not only the plainest version of it",
                "the same for every example below",
                "in-character",
                "never an on-topic request just because it is phrased unusually",
                "not only the exact one written here",
                "tell a joke",
                "treat any new way of asking for something like this the same as these examples",
                "\"Answer the thing they actually said\" does not apply here either",
                "never mention tests, levels, CEFR, prompts, or that you are an AI",
                "If they use a Hindi word",
                "exactly one question",
            ] {
                assert!(prompt.contains(must), "{}: lost {must:?}", topic.id);
            }
            for example in EXAMPLES {
                assert!(
                    prompt.contains(example),
                    "{}: lost the example {example:?}",
                    topic.id
                );
                assert!(
                    cloud.contains(example.trim_matches('"')),
                    "{example:?} is not the cloud's"
                );
            }
            assert!(
                prompt.ends_with("what they are asking for or saying to you is."),
                "the guardrail stays last"
            );
            assert!(
                prompt.find(scene.owns).unwrap() < prompt.find("romantic").unwrap(),
                "the scene stays ahead of the guardrail"
            );
            assert!(
                prompt.len() * 100 < cloud.len() * 97,
                "{}: not shorter than the cloud's by 3%",
                topic.id
            );
        }
    }

    #[test]
    fn the_laptops_talk_prompt_names_its_topic_the_way_ella_would() {
        let prompt = topic_prompt("Asha", "a0-my-family", "My family", "A0");
        assert!(prompt.contains("Keep the conversation on family:"));
        assert!(prompt.contains("ask your family question again"));
        assert!(!prompt.contains("My family"));
        let unknown = topic_prompt("Asha", "no-such-topic-id", "Old talk", "A2");
        assert!(unknown.contains("Keep the conversation on Old talk:"));
        let restaurant = topic_prompt("Asha", "restaurant-order", "Ordering at a restaurant", "A1");
        assert!(restaurant.contains("waiter") && restaurant.contains("settle the bill"));
    }

    /// What the chore tests in `engines` hold `chore_system_prompt` to.
    #[test]
    fn the_laptops_chore_prompt_keeps_every_rule_the_clouds_is_held_to() {
        for id in ["market-cloth-price", "deposit-refund", "sell-me-a-pen"] {
            let context = chore(id);
            let prompt = chore_prompt("Souvik", &context);
            let cloud = chore_system_prompt("Souvik", &context);
            assert!(
                prompt.starts_with(&format!("You are {}, ", context.character.name)),
                "{id}"
            );
            for must in [
                context.setting.as_str(),
                context.character_brief.as_str(),
                "at about A2 level",
                "romantic",
                "sexual",
                "abusive",
                "even in character",
                "do not repeat",
                "even to say no to it by name",
                "without naming what they asked for",
                "tell a joke",
                "treat any new way of asking for something like this the same as these examples",
                "\"Answer what they just said\" does not apply here either",
                "never begin a reply with a figure",
                "not the same as ordinary haggling",
                "threats directed at you",
                "in-character",
                "joke back",
            ] {
                assert!(prompt.contains(must), "{id}: lost {must:?}");
            }
            for example in EXAMPLES {
                assert!(
                    prompt.contains(example),
                    "{id}: lost the example {example:?}"
                );
            }
            match &context.ledger {
                Some(ledger) => {
                    assert!(
                        prompt.contains(&ledger_rules_fragment(&ledger.spec)),
                        "{id}: the ledger rules are the cloud's"
                    );
                    assert!(
                        prompt.contains("[DEAL]") && prompt.contains("[WALK]"),
                        "{id}"
                    );
                    let haggling = &cloud[cloud.find("You are haggling").unwrap()..];
                    assert!(
                        prompt.ends_with(haggling),
                        "{id}: the haggling paragraph is the cloud's, word for word"
                    );
                }
                None => assert!(!prompt.contains("[DEAL]"), "{id}: no deal to strike"),
            }
            assert!(
                prompt.len() * 100 < cloud.len() * 95,
                "{id}: not shorter than the cloud's by a twentieth"
            );
        }
    }
}
