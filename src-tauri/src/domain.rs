use serde::{Deserialize, Serialize};

/// The one learner on this laptop. There is only ever one, so there is no id
/// to carry: every session on the laptop is theirs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Learner {
    pub name: String,
    /// Collected during onboarding so Ella can pick age-appropriate topics.
    pub age: Option<u8>,
    pub level_name: String,
    pub created_at: String,
    /// `#RRGGBB`, or `None` until the learner picks one on the profile screen.
    /// Stored with the learner rather than in the window, so it follows them
    /// through a log out and back in.
    pub avatar_color: Option<String>,
}

/// Just enough of the saved learner for the welcome-back step to greet them
/// by name, with their own avatar, while they are logged out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnerProfile {
    pub name: String,
    pub avatar_color: Option<String>,
}

impl From<&Learner> for LearnerProfile {
    fn from(learner: &Learner) -> Self {
        Self {
            name: learner.name.clone(),
            avatar_color: learner.avatar_color.clone(),
        }
    }
}

/// One local calendar day on which the learner said something.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DayActivity {
    /// `YYYY-MM-DD` in the laptop's own timezone, which is also the calendar
    /// the window draws the week in.
    pub day: String,
    /// Talks whose first answer was given on this day. A talk that runs past
    /// midnight is counted once, on the day it began, so adding up `talks`
    /// over any run of days never counts one twice.
    pub talks: u32,
    /// Answers given on this day, whichever talk they belong to.
    pub answers: u32,
    /// How long this day's spoken answers lasted, in milliseconds. Typed
    /// answers, and spoken ones from before their length was kept, add
    /// nothing.
    #[serde(default)]
    pub spoken_ms: u64,
}

/// One finished talk: a completed session the learner said something in, as
/// the badges read them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinishedTalk {
    /// The topic, chore or placement it was: a chore session stores its chore
    /// id as its topic, and a placement chat stores `placement`.
    pub topic_id: String,
    /// `YYYY-MM-DD` it ended on, in the laptop's own timezone.
    pub day: String,
    /// A ledger chore whose character agreed to a figure at or past the
    /// target. Always false for anything else.
    pub goal_met: bool,
}

/// Lifetime figures for the learner, worked out from every session they have
/// ever had. The streak, the week strip, the badges and the totals on the
/// profile are all drawn from this; `recent_sessions` is only the short list
/// on the home screen and stops at five.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnerProgress {
    /// Every day with at least one answer, newest first. Not capped: a long
    /// streak needs all of it.
    pub days: Vec<DayActivity>,
    /// Completed talks with at least one answer. A talk that ended before the
    /// learner said anything — a placement talk that heard nothing, say — is
    /// not a talk they did.
    pub talks_finished: u32,
    /// Every answer the learner has given, in finished talks or not.
    pub answers: u32,
    /// Topic and chore ids of the finished talks, each once. The two share the
    /// field because a chore session stores its chore id as its topic.
    pub finished_topics: Vec<String>,
    /// Ledger chores whose goal a finished talk has met, each once: the
    /// character agreed to a figure at or past the target.
    #[serde(default)]
    pub chores_met: Vec<String>,
    /// Every finished talk, the first to end first: what the badges are
    /// earned from, the day each was first earned, and the goes at each.
    #[serde(default)]
    pub talks: Vec<FinishedTalk>,
    /// How long every spoken answer lasted, in milliseconds, over the answers
    /// whose length was kept: `spoken_answers` of them. Answers from before
    /// Ella kept their length count in `answers` and not here, so no spoken
    /// answers means nothing is known yet, not that nothing was said.
    #[serde(default)]
    pub spoken_ms: u64,
    #[serde(default)]
    pub spoken_answers: u32,
}

/// A topic as the window shows it, from `shared/topics.json` (`crate::topics`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Topic {
    pub id: String,
    pub label: String,
    /// `role_play`, `vocab`, `grammar`, `real_life`, `culture`, `debate` or
    /// `fluency`.
    pub kind: String,
    pub minutes: u8,
    /// The card's mono line: "ROLE-PLAY · ~5 MIN", or a cadence of its own.
    pub meta: String,
    /// The "Today's talk" card's line about it.
    pub blurb: String,
    /// What Ella says first, after greeting the learner; the tall card quotes it.
    pub opener: String,
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
    /// The learner while they are signed in; `None` after a log out, or on a
    /// laptop where nobody has told Ella their name yet.
    pub learner: Option<Learner>,
    /// Whoever is saved on this laptop, signed in or not, so "Log in" can
    /// welcome them back by name. `None` only on a fresh laptop.
    pub saved_learner: Option<LearnerProfile>,
    /// Every topic written for the learner's level, in the order Home offers
    /// them: today's talk first (`topics::offered`).
    pub topics: Vec<Topic>,
    /// The talk Home will lead with tomorrow, which the recap suggests.
    /// `None` when signed out.
    pub tomorrow_topic: Option<Topic>,
    /// The five newest sessions; empty when signed out.
    pub recent_sessions: Vec<SessionListItem>,
    /// The learner's lifetime figures; all zero when signed out.
    pub progress: LearnerProgress,
    /// Where the learner stands in the curriculum; `None` when signed out.
    pub standing: Option<Standing>,
    pub engine_status: EngineStatus,
}

/// Where the learner stands in the curriculum, as the window shows it: a
/// level's name and its number on the ladder, never its CEFR code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Standing {
    /// 1 to `level_count`.
    pub level_number: u8,
    pub level_count: u8,
    pub level_name: String,
    /// The step within the level, 1 to `step_count`.
    pub step: u8,
    pub step_count: u8,
    pub step_title: String,
    /// How far through the level they are, as a whole percent: the steps
    /// behind them and the share of this step's skills passed.
    pub percent: u8,
    /// False until a placement chat has read a level for them. A learner can
    /// move on through talks without one; the window offers it until then.
    pub placed: bool,
}

/// How far a talk moved the learner.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Advance {
    Step,
    Level,
}

/// One skill a talk counted, for the summary: its short label and how many
/// talks have shown it so far.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillGrowth {
    pub label: String,
    pub count: u32,
}

/// What a finished talk did for the learner, worked out once and kept on the
/// talk, so asking again answers the same and counts nothing twice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Assessment {
    pub session_id: String,
    /// `placement` or `talk`.
    pub kind: String,
    /// Where the learner stands after this talk.
    pub standing: Standing,
    /// A placement's last word, for its result screen; `None` for a talk.
    pub closing: Option<String>,
    /// A step or a level this talk finished; `None` when it moved nobody on.
    pub advanced: Option<Advance>,
    /// The skills this talk counted: its aim first, then the surest, at most
    /// three.
    pub skills: Vec<SkillGrowth>,
    /// Whether anything judged the talk. Not when the learner said nothing,
    /// when there is no model to judge with, or when its answer could not be
    /// read — and none of those count against the learner.
    pub scored: bool,
    /// Ella's notes for the recap. `None` for a placement, and for a talk too
    /// short to say much about (`SessionSummary::short`). Assessments kept
    /// before the recap had notes read as `None`.
    #[serde(default)]
    pub notes: Option<TalkNotes>,
}

/// What the recap says about a talk: what went well, and one thing to say
/// better.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TalkNotes {
    /// At most two short lines, read off what the learner actually said
    /// (`notes::went_well`), so they hold with or without a model.
    pub went_well: Vec<String>,
    /// One of the learner's own phrases and how to say it, when the model found
    /// a mistake worth fixing.
    pub fix: Option<Fix>,
    /// Whether a model looked for a mistake at all. Without one (demo mode, or
    /// an answer that could not be read) no fix does not mean nothing to fix.
    pub checked: bool,
}

/// A phrase the learner said, word for word, and the same phrase said right.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fix {
    pub said: String,
    pub better: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LevelState {
    Done,
    Current,
    Next,
    Locked,
}

/// One level of the ladder as it stands for the learner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LevelView {
    pub number: u8,
    pub name: String,
    /// The level's "I can …" headline.
    pub goal: String,
    pub state: LevelState,
    /// 100 for a level behind them, their progress for their own, else 0.
    pub percent: u8,
    pub steps: Vec<StepView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepView {
    pub number: u8,
    pub title: String,
    pub focus: String,
    pub skills: Vec<LevelSkillView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LevelSkillView {
    pub label: String,
    /// "I can …", word for word.
    pub text: String,
    pub passed: bool,
    /// Where it stands, as a level's page words it.
    pub standing: SkillStanding,
    /// The titles of the talks it showed in, in the order they came:
    /// "Booking a cab".
    pub topics: Vec<String>,
}

/// How far a skill has come, short of the numbers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillStanding {
    /// Passed, as its step asks, or in a step behind the learner.
    Done,
    /// Shown in a talk, not yet enough to pass.
    Shown,
    /// A talk the learner spoke in aimed at it, and it has not shown yet.
    Practising,
    /// Neither.
    NotStarted,
}

/// What a talk's instructions say about the learner: the level to pitch Ella's
/// words at, and the one skill the talk quietly aims at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pitch {
    /// The CEFR code. It goes to the model, never to the window.
    pub level: String,
    pub focus: Option<Focus>,
}

impl Pitch {
    pub fn at(level: &str) -> Self {
        Self {
            level: level.into(),
            focus: None,
        }
    }
}

/// The skill a talk aims at, and the step it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focus {
    pub step_title: String,
    pub step_focus: String,
    /// "I can …", as the curriculum words it.
    pub skill: String,
}

/// A turn of the placement chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementBrief {
    pub age: Option<u8>,
    /// This reply ends the chat: a goodbye, not another question.
    pub closing: bool,
}

/// How sure the judge would be of the learner's level, if asked now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Whether the placement chat has heard enough to read a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Readiness {
    pub ready: bool,
    pub confidence: Confidence,
}

/// A level read off a finished placement chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementReading {
    /// A CEFR code the curriculum has.
    pub level: String,
    /// One warm sentence to end on, when the model wrote a usable one.
    pub closing: Option<String>,
}

/// A skill a finished talk is judged against: its progress key and what it
/// says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scorable {
    pub key: String,
    pub text: String,
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
    /// When Ella started to speak, from the start of the turn: her first
    /// sentence handed to the window. For a reply the window plays whole, the
    /// turn's total. `None` for a turn that failed, or a log from before it.
    #[serde(default)]
    pub speech_ms: Option<u64>,
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

/// How long one token Piper spoke lasted, relative to the start of the clip it
/// belongs to: a sound, a stress mark, a word space, the blank Piper puts
/// after every token, or a sentence's start (`^`) or end (`$`). Read off the
/// same inference as the audio, so Ella's mouth follows what she really says.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhonemeSpan {
    pub phoneme: String,
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
    /// Every token of `audio`, in order. Empty when the voice cannot time
    /// them, and then her mouth only opens while she talks.
    #[serde(default)]
    pub phonemes: Vec<PhonemeSpan>,
    /// The whole reply, on a reply's first sentence only. Its text is settled
    /// before any of it is sent, so the window shows all of it the moment
    /// Ella starts saying it rather than wait for the turn to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

/// The result of speaking a line the app already had — Ella's opening. Carries
/// the same three things a turn does, so the screen highlights it, and the
/// replay button can say it again, exactly as it does for a reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpokenLine {
    pub audio: Option<AudioPayload>,
    pub speech_words: Vec<WordSpan>,
    /// Every token of `audio`, from its start, for her mouth on a replay.
    #[serde(default)]
    pub speech_phonemes: Vec<PhonemeSpan>,
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
    /// `Some` once the conversation is over and the session has been closed for
    /// the learner: the shell shows this instead of asking for another turn.
    /// Without it the closing note fired on every turn past the sixth and the
    /// conversation said goodbye again and again without ever ending.
    pub session_summary: Option<SessionSummary>,
    pub audio: Option<AudioPayload>,
    pub timings: Option<TurnTimings>,
    pub ledger: Option<LedgerView>,
    /// `Some` once the character has agreed or walked away.
    pub signal: Option<TurnSignal>,
    /// When each word of the whole reply is spoken, from the start of `audio`.
    /// Empty when there are no timings, which is also how "no audio" reads.
    pub speech_words: Vec<WordSpan>,
    /// Every token of the whole reply, from the start of `audio`. Empty when
    /// the voice cannot time them.
    #[serde(default)]
    pub speech_phonemes: Vec<PhonemeSpan>,
    /// How many `SpeechStreamEvent`s this turn sent. Above zero, the reply
    /// has been playing since before this result arrived, and `audio` is the
    /// same recording kept for the replay button: playing it again would say
    /// the turn twice.
    #[serde(default)]
    pub streamed_segments: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub topic_label: String,
    pub turns: u32,
    pub headline: String,
    pub encouragement: String,
    /// Too little said for notes (`notes::too_short`), which the recap says
    /// at once rather than after the assessment.
    pub short: bool,
    /// How a ledger chore ended; `None` for a free talk and a rubric chore.
    pub chore: Option<ChoreRecap>,
    /// What went well, read off the learner's words as the talk closes
    /// (`notes::went_well`), so the recap shows it before any model has read
    /// the talk. The assessment's notes say the same. Empty for a talk too
    /// short for notes, and for the placement chat, which has none.
    #[serde(default)]
    pub went_well: Vec<String>,
}

/// How a ledger chore ended, for the recap's role-play band.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoreRecap {
    pub chore_id: String,
    pub character_id: String,
    pub unit: String,
    pub direction: Direction,
    pub target: i32,
    /// Where the figure stood when the talk ended.
    pub figure: i32,
    pub agreed: bool,
    /// The goal met: agreed, at or past the target.
    pub met: bool,
    /// Finished talks that met this chore's goal, this one included.
    pub times_met: u32,
    /// The character's last line, for the speech bubble.
    pub last_line: Option<String>,
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
    /// The learner's level and the talk's quiet aim. Held steady for a whole
    /// session, so it can sit in llama.cpp's cached prefix.
    pub pitch: Pitch,
    /// `Some` for a turn of the placement chat, which asks its own questions
    /// instead of playing a scene.
    pub placement: Option<PlacementBrief>,
}

/// Every topic, in the catalogue's order.
pub fn topics() -> Vec<Topic> {
    crate::topics::all().iter().map(|topic| topic.topic()).collect()
}

// ---------------------------------------------------------------------------
// Cast, chores and the ledger.
//
// Everything the learner talks to is a `Character`; a `Chore` is a character
// plus a goal plus a way to tell whether the learner got it. The catalog lives
// here as constants, the way the topics live in `shared/topics.json`, so adding
// a chore is a pull request rather than a migration. Only *state* goes to
// SQLite.
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
    /// Where the scene is and who is in it. The character's instructions
    /// carry it as their setting, so it is written about both sides rather
    /// than to the learner: a "you" in it reads to the model as itself. The
    /// deposit chore's "You moved out last week and he still has your Rs 5000
    /// deposit" had the landlord tell the tenant "You are holding my deposit".
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
    /// The learner's CEFR level, which the character pitches its words at.
    pub level: String,
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
            name: "Bippo".into(),
            kind: CharacterKind::Cast,
            blob: BlobStyle { palette: "orange".into(), eyes: "narrow".into(), mouth: "flat".into() },
            voice: "en_IN-navgurukul-medium".into(),
            persona: "You are Bippo, who runs a busy cloth stall in a crowded market. \
                      You are friendly but you have sold here for twenty years and you do \
                      not give things away. You speak in short, quick sentences."
                .into(),
        },
        Character {
            id: "landlord".into(),
            name: "Grumble".into(),
            kind: CharacterKind::Cast,
            blob: BlobStyle { palette: "ink".into(), eyes: "narrow".into(), mouth: "flat".into() },
            voice: "en_IN-navgurukul-medium".into(),
            persona: "You are Grumble, a landlord who is polite but reluctant and \
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
            setting: "A cloth stall in a crowded market. Bippo is folding shirts.".into(),
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
            setting: "Grumble's doorway. His tenant moved out last week, and he \
                      still has the tenant's Rs 5000 deposit. He has offered Rs 500 of \
                      it back."
                .into(),
            learner_goal: "Get him to agree to return at least Rs 3500 of the deposit."
                .into(),
            character_brief: "The tenant paid you a deposit of Rs 5000, all of it still \
                              with you, and you would rather give back as little of it as \
                              you can. You claim there is cleaning and repainting to pay \
                              for. Always name the rupee figure you will give back, \
                              starting low, and say that the rest goes on the cleaning \
                              and repainting. You will give back no more than Rs 4200. \
                              Raise your figure only when the tenant makes a specific, \
                              reasonable point."
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
            setting: "A practice interview. The candidate has thirty seconds and one \
                      pen to sell."
                .into(),
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

/// Chores a learner should be offered, ordered. Filters on age, where
/// `topics::offered` sorts rather than removes, then ranks on how many of the
/// learner's interests a chore touches, then by ladder position.
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
