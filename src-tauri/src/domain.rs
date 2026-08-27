use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Learner {
    pub name: String,
    /// Collected during onboarding so Ella can pick age-appropriate topics.
    pub age: Option<u8>,
    pub level_name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Topic {
    pub id: String,
    pub label: String,
    pub prompt: String,
    pub emoji: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    Learner,
    Ella,
}

impl Speaker {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Learner => "learner",
            Self::Ella => "ella",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "learner" => Self::Learner,
            _ => Self::Ella,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub speaker: Speaker,
    pub content: String,
    pub turn: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub topic_id: String,
    pub topic_label: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListItem {
    pub id: String,
    pub topic_id: String,
    pub topic_label: String,
    pub status: String,
    pub started_at: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineComponent {
    pub name: String,
    pub ready: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineStatus {
    pub mode: String,
    pub label: String,
    pub ready: bool,
    pub components: Vec<EngineComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSnapshot {
    pub learner: Option<Learner>,
    pub topics: Vec<Topic>,
    pub recent_sessions: Vec<SessionListItem>,
    pub engine_status: EngineStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioPayload {
    pub mime_type: String,
    pub base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnTimings {
    pub interaction_id: String,
    pub kind: String,
    pub audio_input_ms: Option<u64>,
    pub audio_after_vad_ms: Option<u64>,
    pub vad_ms: Option<u64>,
    pub stt_ms: Option<u64>,
    pub stt_engine: Option<String>,
    pub stt_backend: Option<String>,
    pub stt_fallback_from: Option<String>,
    pub stt_mel_ms: Option<u64>,
    pub stt_encode_ms: Option<u64>,
    pub stt_decode_ms: Option<u64>,
    pub llm_ttft_ms: Option<u64>,
    pub llm_completion_ms: Option<u64>,
    pub tts_first_audio_ms: Option<u64>,
    pub tts_completion_ms: Option<u64>,
    pub total_ms: u64,
}

/// When one word of a reply is spoken, relative to the start of the clip it
/// belongs to. Lets the screen highlight the word Ella is saying.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordSpan {
    pub text: String,
    pub start_ms: f64,
    pub end_ms: f64,
}

/// One sentence of a reply, synthesized and pushed to the window while the
/// rest of the turn is still being written. Playback starts on the first of
/// these instead of waiting for the whole reply, which is most of the turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechStreamEvent {
    pub session_id: String,
    pub turn: u32,
    /// 0-based playback order. Segments must be played in this order.
    pub index: u32,
    pub text: String,
    pub audio: AudioPayload,
    /// Milliseconds from the start of the generation to this segment.
    pub ready_ms: f64,
    /// When each word of `text` is spoken, from the start of `audio`.
    pub words: Vec<WordSpan>,
}

/// The result of speaking a line the app already had — Ella's opening. Carries
/// the same three things a turn does, so the screen highlights it, and the
/// replay button can say it again, exactly as it does for a reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpokenLine {
    pub audio: Option<AudioPayload>,
    pub speech_words: Vec<WordSpan>,
    pub streamed_segments: u32,
}

/// What the live progress bar draws, and what the app decided this turn. Sent
/// only for ledger chores.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LedgerView {
    pub unit: String,
    pub current: i32,
    pub target: i32,
    pub opening: i32,
    pub progress: f32,
    pub agreed: bool,
    pub reached_target: bool,
    /// True when the character named a figure past its own limit and the turn
    /// had to be generated again. Surfaced for the bench, not for the learner.
    pub regenerated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnResult {
    pub learner_message: Message,
    pub ella_message: Message,
    pub correction: Option<String>,
    pub suggested_complete: bool,
    pub audio: Option<AudioPayload>,
    pub timings: Option<TurnTimings>,
    pub ledger: Option<LedgerView>,
    /// `Some` once the character has agreed or walked away.
    pub signal: Option<TurnSignal>,
    /// When each word of the whole reply is spoken, from the start of `audio`.
    /// Empty when there are no timings, which is also how "no audio" reads.
    pub speech_words: Vec<WordSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub topic_label: String,
    pub turns: u32,
    pub headline: String,
    pub encouragement: String,
}

#[derive(Debug, Clone)]
pub struct TutorRequest {
    pub learner_name: String,
    /// The catalog id, not just the label: the engine keys the scene a free
    /// conversation plays out in off it, so the opening line and every reply
    /// after it are the same person in the same place.
    pub topic_id: String,
    pub topic_label: String,
    pub messages: Vec<Message>,
    pub learner_text: String,
    pub turn: u32,
    /// `None` is a free conversation on a topic, the pre-chore behaviour.
    /// `Some` makes the engine somebody, with a setting and a hidden brief.
    pub chore: Option<ChoreContext>,
}

pub fn topics() -> Vec<Topic> {
    vec![
        Topic {
            id: "street-food".into(),
            label: "Street food stories".into(),
            prompt: "Describe tastes, smells and your favourite stall.".into(),
            emoji: "\u{1F35B}".into(),
            color: "violet".into(),
        },
        Topic {
            id: "restaurant-order".into(),
            label: "Ordering at a restaurant".into(),
            prompt: "Order a meal, ask about the menu, and settle the bill.".into(),
            emoji: "\u{1F37D}".into(),
            color: "pink".into(),
        },
        Topic {
            id: "booking-a-cab".into(),
            label: "Booking a cab".into(),
            prompt: "Give an address, agree a fare, and ask how long it takes.".into(),
            emoji: "\u{1F695}".into(),
            color: "green".into(),
        },
        Topic {
            id: "job-interview".into(),
            label: "A job interview".into(),
            prompt: "Introduce yourself and answer questions about your work.".into(),
            emoji: "\u{1F4BC}".into(),
            color: "lilac".into(),
        },
        Topic {
            id: "doctor-clinic".into(),
            label: "At the doctor's clinic".into(),
            prompt: "Explain how you feel and understand what to do next.".into(),
            emoji: "\u{1FA7A}".into(),
            color: "violet".into(),
        },
        Topic {
            id: "asking-directions".into(),
            label: "Asking for directions".into(),
            prompt: "Find your way and repeat the directions back.".into(),
            emoji: "\u{1F5FA}".into(),
            color: "ink".into(),
        },
        Topic {
            id: "market-bargaining".into(),
            label: "Bargaining at the market".into(),
            prompt: "Ask the price, bargain kindly, and agree a deal.".into(),
            emoji: "\u{1F6D2}".into(),
            color: "orange".into(),
        },
    ]
}

/// Onboarding promises that "Ella picks topics that fit your age", so the
/// grown-up scenarios sink below the everyday ones for younger learners. They
/// are ordered, not removed: a 12-year-old can still choose an interview, it
/// simply stops being what Ella leads with.
pub fn topics_for_age(age: Option<u8>) -> Vec<Topic> {
    let mut all = topics();
    let Some(age) = age else {
        return all;
    };
    // Stable sort, so the authored order survives inside each group.
    all.sort_by_key(|topic| u8::from(min_age(&topic.id) > age));
    all
}

fn min_age(topic_id: &str) -> u8 {
    match topic_id {
        "job-interview" => 14,
        "market-bargaining" => 10,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Cast, chores and the ledger.
//
// Everything the learner talks to is a `Character`; a `Chore` is a character
// plus a goal plus a way to tell whether the learner got it. The catalog lives
// here as constants, the way `topics()` does, so adding a chore is a pull
// request rather than a migration. Only *state* goes to SQLite.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CharacterKind {
    Ella,
    Cast,
    Mentor,
    Friend,
}

/// The blob mascot is drawn in CSS, so a character's look is a palette token
/// plus two variant names rather than an asset path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlobStyle {
    pub palette: String,
    pub eyes: String,
    pub mouth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub kind: CharacterKind,
    pub blob: BlobStyle,
    /// Piper voice id. Ella's voice ships; the rest are fetched on unlock, and
    /// a missing voice degrades to Ella's rather than blocking a chore.
    pub voice: String,
    /// System-prompt fragment: who this character is and how they speak.
    pub persona: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Transactions,
    Negotiation,
    Social,
    Work,
}

impl Track {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transactions => "transactions",
            Self::Negotiation => "negotiation",
            Self::Social => "social",
            Self::Work => "work",
        }
    }
}

/// Which way the learner is pushing the number. `Down` is a price to be talked
/// lower; `Up` is a refund, an extension, a larger portion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Down,
    Up,
}

/// When winning is a quantity, the app owns the quantity. A 3B model asked to
/// concede will concede to any number named at it, so the limit is enforced in
/// Rust and the model is never the authority on whether the chore was won.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerSpec {
    pub unit: String,
    /// Words that may appear beside the figure in the reply, lowercase.
    pub unit_aliases: Vec<String>,
    pub direction: Direction,
    pub opening: i32,
    /// A floor when pushing down, a ceiling when pushing up. Never crossed.
    pub limit: i32,
    /// The learner wins at or past this. Always sits short of `limit`, so a
    /// character that leaks its limit has not handed over the win.
    pub target: i32,
    pub max_step: i32,
    /// Used verbatim when the model breaks the limit twice in one turn. One
    /// authored line per chore is what keeps a chore unwinnable by cheese.
    pub refusal: String,
    /// Used when the model signals agreement with no words around it — it
    /// frequently replies with the bare token and nothing else, which would
    /// otherwise leave the learner with a silent turn.
    pub acceptance: String,
}

impl LedgerSpec {
    /// Is `value` a legal place for the number to stand, given where it stands
    /// now? One comparison either way; `direction` picks the sign.
    pub fn accepts(&self, current: i32, value: i32) -> bool {
        match self.direction {
            Direction::Down => {
                value >= self.limit && value <= current && (current - value) <= self.max_step
            }
            Direction::Up => {
                value <= self.limit && value >= current && (value - current) <= self.max_step
            }
        }
    }

    pub fn reached_target(&self, current: i32) -> bool {
        match self.direction {
            Direction::Down => current <= self.target,
            Direction::Up => current >= self.target,
        }
    }

    /// 0.0 at the opening, 1.0 once the target is reached. Drives the live bar.
    pub fn progress(&self, current: i32) -> f32 {
        let span = (self.target - self.opening) as f32;
        if span.abs() < f32::EPSILON {
            return 1.0;
        }
        (((current - self.opening) as f32) / span).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Criterion {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WinCondition {
    /// The app owns a number and enforces it every turn.
    Ledger(LedgerSpec),
    /// Judged after the fact, from the transcript, by one constrained pass.
    Rubric { criteria: Vec<Criterion>, pass_at: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chore {
    pub id: String,
    pub title: String,
    pub character_id: String,
    pub track: Track,
    pub level: u8,
    pub min_age: u8,
    pub interests: Vec<String>,
    /// Shown to the learner: where you are and who you are talking to.
    pub setting: String,
    /// Shown to the learner: what counts as walking away happy.
    pub learner_goal: String,
    /// Never shown: what the other side wants and how hard they hold it.
    pub character_brief: String,
    pub win: WinCondition,
    pub max_turns: u32,
}

/// Live ledger state for one turn, read from `ledger_state` and handed to the
/// engine so the system prompt can be rebuilt around it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerTurn {
    pub spec: LedgerSpec,
    pub current: i32,
    pub agreed: bool,
}

/// What the character signed off with. Extracted from a bare token rather than
/// inferred, so no model is asked to grade anything.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TurnSignal {
    Deal,
    Walk,
}

/// Everything the engine needs to be somebody, in one place.
#[derive(Debug, Clone)]
pub struct ChoreContext {
    pub chore_id: String,
    pub character: Character,
    pub setting: String,
    pub learner_goal: String,
    pub character_brief: String,
    pub max_turns: u32,
    pub ledger: Option<LedgerTurn>,
}

pub fn characters() -> Vec<Character> {
    vec![
        Character {
            id: "ella".into(),
            name: "Ella".into(),
            kind: CharacterKind::Ella,
            blob: BlobStyle { palette: "violet".into(), eyes: "open".into(), mouth: "smile".into() },
            voice: "en_IN-navgurukul-medium".into(),
            persona: "You are Ella, a warm speaking buddy for an Indian learner.".into(),
        },
        Character {
            id: "stall-owner".into(),
            name: "Ramesh".into(),
            kind: CharacterKind::Cast,
            blob: BlobStyle { palette: "orange".into(), eyes: "narrow".into(), mouth: "flat".into() },
            voice: "en_IN-navgurukul-medium".into(),
            persona: "You are Ramesh, who runs a busy cloth stall in a crowded market. \
                      You are friendly but you have sold here for twenty years and you do \
                      not give things away. You speak in short, quick sentences."
                .into(),
        },
        Character {
            id: "landlord".into(),
            name: "Mr Khanna".into(),
            kind: CharacterKind::Cast,
            blob: BlobStyle { palette: "ink".into(), eyes: "narrow".into(), mouth: "flat".into() },
            voice: "en_IN-navgurukul-medium".into(),
            persona: "You are Mr Khanna, a landlord who is polite but reluctant and \
                      always a little busy. You would rather not return money you are \
                      already holding."
                .into(),
        },
        Character {
            id: "mentor".into(),
            name: "Ella".into(),
            kind: CharacterKind::Mentor,
            blob: BlobStyle { palette: "green".into(), eyes: "open".into(), mouth: "smile".into() },
            voice: "en_IN-navgurukul-medium".into(),
            persona: "You are Ella, talking with a learner about their own English. \
                      You are encouraging and specific, and you use their own sentences \
                      back to them."
                .into(),
        },
    ]
}

/// The starter catalog. Content, not architecture: two mechanisms cover the
/// range, and adding a chore is filling in this struct.
pub fn chores() -> Vec<Chore> {
    vec![
        Chore {
            id: "market-cloth-price".into(),
            title: "Talk a stall price down".into(),
            character_id: "stall-owner".into(),
            track: Track::Negotiation,
            level: 1,
            min_age: 10,
            interests: vec!["shopping".into(), "food".into()],
            setting: "A cloth stall in a crowded market. Ramesh is folding shirts.".into(),
            learner_goal: "Get the price down to Rs 400 or less, and get him to agree.".into(),
            character_brief: "You opened at Rs 600 for the shirt. You will not go below \
                              Rs 350 under any circumstances. Come down only when the \
                              customer gives you an actual reason, and complain a little \
                              each time you do."
                .into(),
            win: WinCondition::Ledger(LedgerSpec {
                unit: "Rs".into(),
                unit_aliases: vec!["rs".into(), "rupees".into(), "rupee".into(), "₹".into()],
                direction: Direction::Down,
                opening: 600,
                limit: 350,
                target: 400,
                max_step: 75,
                refusal: "No, no. That is below my cost. I cannot go there.".into(),
                acceptance: "Alright, alright. Take it at that price.".into(),
            }),
            max_turns: 12,
        },
        Chore {
            id: "deposit-refund".into(),
            title: "Get a deposit refunded".into(),
            character_id: "landlord".into(),
            track: Track::Transactions,
            level: 1,
            min_age: 14,
            interests: vec!["work".into()],
            setting: "Mr Khanna's doorway. You moved out last week and he still has \
                      your Rs 5000 deposit. He has offered you Rs 500 back."
                .into(),
            learner_goal: "Get him to agree to return at least Rs 3500 of the deposit."
                .into(),
            character_brief: "You are holding Rs 5000 of this tenant's deposit and you \
                              would rather keep as much of it as you can. You claim there \
                              is cleaning and repainting to pay for. Always name a rupee \
                              figure you are willing to return, starting low, and always \
                              give the reason you are keeping the rest. You will go no \
                              higher than Rs 4200. Raise your figure only when the tenant \
                              makes a specific, reasonable point."
                .into(),
            win: WinCondition::Ledger(LedgerSpec {
                unit: "Rs".into(),
                unit_aliases: vec!["rs".into(), "rupees".into(), "rupee".into(), "₹".into()],
                direction: Direction::Up,
                opening: 500,
                limit: 4200,
                target: 3500,
                max_step: 1200,
                refusal: "That is too much. I have costs of my own to cover.".into(),
                acceptance: "Fine. I will return that much to you this week.".into(),
            }),
            max_turns: 12,
        },
        Chore {
            id: "sell-me-a-pen".into(),
            title: "Sell me a pen".into(),
            character_id: "landlord".into(),
            track: Track::Work,
            level: 2,
            min_age: 14,
            interests: vec!["work".into(), "school".into()],
            setting: "A practice interview. You have thirty seconds and one pen.".into(),
            learner_goal: "Sell the pen: find out what they need, say why this pen \
                           helps, answer their objection, and ask for the sale."
                .into(),
            character_brief: "You are a sceptical buyer. Raise exactly one objection \
                              about price partway through, and never volunteer what you \
                              need unless you are asked."
                .into(),
            win: WinCondition::Rubric {
                criteria: vec![
                    Criterion { id: "asked_needs".into(), label: "Asked what the buyer needs".into() },
                    Criterion { id: "named_benefit".into(), label: "Named a concrete benefit".into() },
                    Criterion { id: "handled_objection".into(), label: "Answered the objection".into() },
                    Criterion { id: "asked_close".into(), label: "Asked for the sale".into() },
                ],
                pass_at: 3,
            },
            max_turns: 10,
        },
    ]
}

pub fn find_chore(id: &str) -> Option<Chore> {
    chores().into_iter().find(|chore| chore.id == id)
}

pub fn find_character(id: &str) -> Option<Character> {
    characters().into_iter().find(|character| character.id == id)
}

/// Chores a learner should be offered, ordered. Filters on age the way
/// `topics_for_age` sorts rather than removes where it can, then ranks on how
/// many of the learner's interests a chore touches, then by ladder position.
pub fn catalog_for(age: Option<u8>, interests: &[String]) -> Vec<Chore> {
    let mut all: Vec<Chore> = chores()
        .into_iter()
        .filter(|chore| age.map(|age| chore.min_age <= age).unwrap_or(true))
        .collect();
    all.sort_by_key(|chore| {
        let overlap = chore
            .interests
            .iter()
            .filter(|tag| interests.contains(tag))
            .count();
        (std::cmp::Reverse(overlap), chore.track.as_str(), chore.level)
    });
    all
}
