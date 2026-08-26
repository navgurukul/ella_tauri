use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Learner {
    pub name: String,
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
    Zoe,
}

impl Speaker {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Learner => "learner",
            Self::Zoe => "zoe",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "learner" => Self::Learner,
            _ => Self::Zoe,
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
pub struct SkillProgress {
    pub id: String,
    pub label: String,
    pub strand: String,
    pub evidence_count: u32,
    pub stage: u8,
    pub stage_label: String,
    pub last_evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Garden {
    pub level_name: String,
    pub total_conversations: u32,
    pub skills: Vec<SkillProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSnapshot {
    pub learner: Option<Learner>,
    pub topics: Vec<Topic>,
    pub recent_sessions: Vec<SessionListItem>,
    pub engine_status: EngineStatus,
    pub garden: Garden,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioPayload {
    pub mime_type: String,
    pub base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEvidence {
    pub skill_id: String,
    pub skill_label: String,
    pub strand: String,
    pub new_stage: u8,
    pub stage_label: String,
    pub evidence: String,
}

impl From<SkillProgress> for SkillEvidence {
    fn from(skill: SkillProgress) -> Self {
        Self {
            skill_id: skill.id,
            skill_label: skill.label,
            strand: skill.strand,
            new_stage: skill.stage,
            stage_label: skill.stage_label,
            evidence: skill.last_evidence.unwrap_or_default(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnResult {
    pub learner_message: Message,
    pub zoe_message: Message,
    pub correction: Option<String>,
    pub evidence: Option<SkillEvidence>,
    pub suggested_complete: bool,
    pub audio: Option<AudioPayload>,
    pub timings: Option<TurnTimings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub topic_label: String,
    pub turns: u32,
    pub headline: String,
    pub encouragement: String,
    pub best_evidence: Option<SkillEvidence>,
    pub garden: Garden,
}

#[derive(Debug, Clone)]
pub struct TutorRequest {
    pub learner_name: String,
    pub topic_label: String,
    pub messages: Vec<Message>,
    pub learner_text: String,
    pub turn: u32,
}

pub fn topics() -> Vec<Topic> {
    vec![
        Topic {
            id: "school-life".into(),
            label: "School life".into(),
            prompt: "Tell Zoe about a memorable day at school.".into(),
            emoji: "🎒".into(),
            color: "blue".into(),
        },
        Topic {
            id: "food-i-love".into(),
            label: "Food I love".into(),
            prompt: "Describe a meal you would happily eat again.".into(),
            emoji: "🥭".into(),
            color: "green".into(),
        },
        Topic {
            id: "my-dreams".into(),
            label: "My dreams".into(),
            prompt: "Share something you hope to do in the future.".into(),
            emoji: "✨".into(),
            color: "berry".into(),
        },
    ]
}

pub fn skill_seeds() -> [(&'static str, &'static str, &'static str); 3] {
    [
        ("descriptive-words", "Use descriptive words", "vocabulary"),
        ("past-events", "Talk about past events", "grammar"),
        ("longer-answers", "Build longer answers", "fluency"),
    ]
}

pub fn stage_label(stage: u8) -> &'static str {
    match stage {
        1 => "Seedling",
        2 => "Young plant",
        3.. => "Bloom",
        _ => "Bare plot",
    }
}
