use chrono::Utc;
use std::time::Instant;
use uuid::Uuid;

use crate::{
    domain::{
        skill_seeds, topics, AppSnapshot, Learner, Message, Session, SessionSummary, SkillEvidence,
        Speaker, Topic, TurnResult, TutorRequest,
    },
    error::{EllaError, EllaResult},
    infrastructure::{audio::trim_to_speech, database::Database, engines::TutorEngine},
    telemetry::LatencyTrace,
};

pub struct AppService {
    database: Database,
    engine: Box<dyn TutorEngine>,
}

impl AppService {
    pub fn new(database: Database, engine: Box<dyn TutorEngine>) -> Self {
        Self { database, engine }
    }

    pub fn bootstrap(&self) -> EllaResult<AppSnapshot> {
        Ok(AppSnapshot {
            learner: self.database.learner()?,
            topics: topics(),
            recent_sessions: self.database.recent_sessions(5)?,
            engine_status: self.engine.status(),
            garden: self.database.garden()?,
        })
    }

    pub fn save_learner(&self, name: &str) -> EllaResult<Learner> {
        let clean = name.trim();
        if clean.chars().count() < 2 {
            return Err(EllaError::Validation(
                "Please enter at least two letters.".into(),
            ));
        }
        if clean.chars().count() > 40 {
            return Err(EllaError::Validation(
                "Please use a name shorter than 40 letters.".into(),
            ));
        }
        let created_at = self
            .database
            .learner()?
            .map(|learner| learner.created_at)
            .unwrap_or_else(now);
        let learner = Learner {
            name: clean.into(),
            level_name: "Morning Meadow".into(),
            created_at,
        };
        self.database.save_learner(&learner)?;
        Ok(learner)
    }

    pub fn start_session(&self, topic_id: &str) -> EllaResult<Session> {
        let topic = find_topic(topic_id)?;
        let learner = self.database.learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Zoe your name before starting a conversation.".into())
        })?;
        let started_at = now();
        let opening = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Zoe,
            content: self.engine.opening(&topic, &learner.name)?,
            turn: 0,
            created_at: started_at.clone(),
        };
        let session = Session {
            id: Uuid::new_v4().to_string(),
            topic_id: topic.id,
            topic_label: topic.label,
            status: "active".into(),
            started_at,
            completed_at: None,
            messages: vec![opening.clone()],
        };
        self.database.create_session(&session, &opening)?;
        Ok(session)
    }

    pub fn get_session(&self, id: &str) -> EllaResult<Session> {
        self.database.session(id)
    }

    pub fn send_text_turn(&self, session_id: &str, text: &str) -> EllaResult<TurnResult> {
        let mut trace = LatencyTrace::new("text");
        trace.stage(
            "turn:received",
            &format!("text turn ({} chars)", text.trim().chars().count()),
        );
        let outcome = self.run_turn(session_id, text, &mut trace);
        finish_turn(outcome, trace)
    }

    pub fn send_voice_turn(
        &self,
        session_id: &str,
        samples: Vec<i16>,
        sample_rate: u32,
        browser_transcript: Option<String>,
    ) -> EllaResult<TurnResult> {
        let mut trace = LatencyTrace::new("voice");
        trace.stage(
            "turn:received",
            &format!(
                "voice turn: {} samples @ {} Hz (~{:.0} ms of audio)",
                samples.len(),
                sample_rate,
                if sample_rate > 0 {
                    samples.len() as f64 / sample_rate as f64 * 1_000.0
                } else {
                    0.0
                }
            ),
        );
        let outcome = (|| {
            if sample_rate == 0 {
                return Err(EllaError::Validation(
                    "The microphone reported an invalid sample rate. Reconnect it and try again."
                        .into(),
                ));
            }
            if samples.len() > sample_rate as usize * 90 {
                return Err(EllaError::Validation(
                    "Please keep one answer under 90 seconds.".into(),
                ));
            }

            trace.stage("vad:start", "trimming captured audio to speech");
            let vad_started = Instant::now();
            let vad = trim_to_speech(&samples, sample_rate)?;
            trace.record_vad(
                vad_started.elapsed().as_secs_f64() * 1_000.0,
                vad.input_ms,
                vad.speech_ms,
            );
            trace.stage(
                "vad:done",
                &format!(
                    "took {:.1} ms | input {:.0} ms -> speech {:.0} ms (speech_detected={})",
                    vad_started.elapsed().as_secs_f64() * 1_000.0,
                    vad.input_ms,
                    vad.speech_ms,
                    vad.speech_detected
                ),
            );
            let transcript = if self.engine.uses_native_stt() {
                if !vad.speech_detected {
                    return Err(EllaError::Validation(
                        "I could not detect speech in that recording. Check the selected microphone, move closer, and try again."
                            .into(),
                    ));
                }
                trace.stage("stt:start", "sending trimmed audio to speech-to-text engine");
                let transcription = self.engine.transcribe(&vad.samples, sample_rate)?;
                trace.stage(
                    "stt:done",
                    &format!(
                        "took {:.1} ms | engine={} backend={}{} | mel={:?} encode={:?} decode={:?} | transcript=\"{}\"",
                        transcription.elapsed_ms,
                        transcription.engine,
                        transcription.backend,
                        transcription
                            .fallback_from
                            .as_deref()
                            .map(|from| format!(" (fell back from {from})"))
                            .unwrap_or_default(),
                        transcription.mel_ms,
                        transcription.encode_ms,
                        transcription.decode_ms,
                        transcription.text
                    ),
                );
                trace.record_stt(
                    transcription.elapsed_ms,
                    transcription.engine,
                    transcription.backend,
                    transcription.fallback_from,
                    transcription.mel_ms,
                    transcription.encode_ms,
                    transcription.decode_ms,
                );
                transcription.text
            } else {
                match browser_transcript {
                    Some(text) if !text.trim().is_empty() => {
                        trace.record_browser_stt();
                        trace.stage(
                            "stt:browser",
                            &format!("using browser Web Speech transcript: \"{}\"", text.trim()),
                        );
                        text
                    }
                    _ => self.engine.transcribe(&vad.samples, sample_rate)?.text,
                }
            };
            self.run_turn(session_id, &transcript, &mut trace)
        })();
        finish_turn(outcome, trace)
    }

    fn run_turn(
        &self,
        session_id: &str,
        text: &str,
        trace: &mut LatencyTrace,
    ) -> EllaResult<TurnResult> {
        let clean = text.trim();
        if clean.is_empty() {
            return Err(EllaError::Validation("Say or type something first.".into()));
        }
        if clean.chars().count() > 800 {
            return Err(EllaError::Validation(
                "Please keep one answer under 800 characters.".into(),
            ));
        }
        let db_load_started = Instant::now();
        let session = self.database.session(session_id)?;
        trace.stage(
            "db:session-loaded",
            &format!(
                "took {:.1} ms | {} prior messages",
                db_load_started.elapsed().as_secs_f64() * 1_000.0,
                session.messages.len()
            ),
        );
        if session.status != "active" {
            return Err(EllaError::Conflict(
                "This conversation has already ended.".into(),
            ));
        }
        let learner = self.database.learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Zoe your name before starting a conversation.".into())
        })?;
        let turn = session
            .messages
            .iter()
            .filter(|message| message.speaker == Speaker::Learner)
            .count() as u32
            + 1;
        let request = TutorRequest {
            learner_name: learner.name,
            topic_label: session.topic_label.clone(),
            messages: session.messages.clone(),
            learner_text: clean.into(),
            turn,
        };
        trace.stage(
            "llm:start",
            &format!("requesting tutor reply for turn {turn} ({} history messages)", request.messages.len()),
        );
        let generated = self.engine.reply(&request)?;
        trace.record_llm(generated.ttft_ms, generated.completion_ms);
        trace.stage(
            "llm:done",
            &format!(
                "ttft={:.1} ms completion={:.1} ms | {} chars reply=\"{}\"",
                generated.ttft_ms,
                generated.completion_ms,
                generated.text.trim().chars().count(),
                generated.text.trim()
            ),
        );
        let reply = generated.text.trim().to_owned();
        if reply.is_empty() {
            return Err(EllaError::Engine("Zoe returned an empty reply.".into()));
        }
        let created_at = now();
        let learner_message = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Learner,
            content: clean.into(),
            turn,
            created_at: created_at.clone(),
        };
        let zoe_message = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Zoe,
            content: reply.clone(),
            turn,
            created_at: created_at.clone(),
        };
        let (skill_id, _, _) = skill_seeds()[((turn - 1) as usize) % skill_seeds().len()];
        let db_persist_started = Instant::now();
        let skill = self.database.persist_turn(
            session_id,
            &learner_message,
            &zoe_message,
            skill_id,
            clean,
            &created_at,
        )?;
        trace.stage(
            "db:turn-persisted",
            &format!(
                "took {:.1} ms",
                db_persist_started.elapsed().as_secs_f64() * 1_000.0
            ),
        );
        // Voice is an enhancement, not a transaction dependency: if Piper is
        // missing or fails, the persisted text turn remains fully usable.
        trace.stage("tts:start", "sending reply text to speech synthesis");
        let audio = match self.engine.synthesize(&reply) {
            Ok(synthesized) => {
                trace.record_tts(synthesized.first_audio_ms, synthesized.completion_ms);
                trace.stage(
                    "tts:done",
                    &format!(
                        "first_audio={} completion={} | audio={}",
                        synthesized
                            .first_audio_ms
                            .map(|ms| format!("{ms:.1} ms"))
                            .unwrap_or_else(|| "-".into()),
                        synthesized
                            .completion_ms
                            .map(|ms| format!("{ms:.1} ms"))
                            .unwrap_or_else(|| "-".into()),
                        synthesized
                            .audio
                            .as_ref()
                            .map(|audio| format!("{} base64 chars ({})", audio.base64.len(), audio.mime_type))
                            .unwrap_or_else(|| "none".into())
                    ),
                );
                synthesized.audio
            }
            Err(error) => {
                trace.stage("tts:failed", &error.to_string());
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "tts_degraded",
                        "error": error.to_string(),
                    })
                );
                trace.record_tts(None, None);
                None
            }
        };
        Ok(TurnResult {
            learner_message,
            zoe_message,
            correction: gentle_correction(clean),
            evidence: Some(skill.into()),
            suggested_complete: turn >= 3,
            audio,
            timings: None,
        })
    }

    pub fn complete_session(&self, session_id: &str) -> EllaResult<SessionSummary> {
        let session = self.database.session(session_id)?;
        self.database.complete_session(session_id, &now())?;
        let turns = session
            .messages
            .iter()
            .filter(|message| message.speaker == Speaker::Learner)
            .count() as u32;
        let garden = self.database.garden()?;
        let best_evidence = garden
            .skills
            .iter()
            .filter(|skill| skill.last_evidence.is_some())
            .max_by_key(|skill| skill.evidence_count)
            .cloned()
            .map(SkillEvidence::from);
        Ok(SessionSummary {
            session_id: session.id,
            topic_label: session.topic_label.clone(),
            turns,
            headline: if turns >= 3 {
                "Your garden grew!".into()
            } else {
                "Every word waters the garden.".into()
            },
            encouragement: format!(
                "You kept a real conversation about {} going. That is brave practice.",
                session.topic_label.to_lowercase()
            ),
            best_evidence,
            garden,
        })
    }

    pub fn reset_demo_data(&self) -> EllaResult<AppSnapshot> {
        self.database.reset()?;
        self.bootstrap()
    }
}

fn finish_turn(outcome: EllaResult<TurnResult>, trace: LatencyTrace) -> EllaResult<TurnResult> {
    match outcome {
        Ok(mut result) => {
            result.timings = Some(trace.finish("ok", None));
            Ok(result)
        }
        Err(error) => {
            let detail = error.to_string();
            trace.finish("error", Some(&detail));
            Err(error)
        }
    }
}

fn find_topic(id: &str) -> EllaResult<Topic> {
    topics()
        .into_iter()
        .find(|topic| topic.id == id)
        .ok_or_else(|| EllaError::Validation("Choose one of the available topics.".into()))
}

fn gentle_correction(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if lower.contains("i goed") {
        Some("Try “I went” instead of “I goed.”".into())
    } else if lower.contains("i am went") {
        Some("Try “I went” when you are talking about the past.".into())
    } else {
        None
    }
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::{
        database::Database,
        engines::{DemoEngine, LocalEngine},
    };
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    fn service() -> AppService {
        AppService::new(Database::in_memory().unwrap(), Box::new(DemoEngine))
    }

    #[test]
    fn full_demo_turn_is_persisted_and_grows_a_skill() {
        let service = service();
        service.save_learner("Asha").unwrap();
        let session = service.start_session("school-life").unwrap();
        let result = service
            .send_text_turn(&session.id, "I played football with my best friend")
            .unwrap();

        assert_eq!(result.learner_message.turn, 1);
        assert_eq!(result.evidence.unwrap().new_stage, 1);
        let saved = service.get_session(&session.id).unwrap();
        assert_eq!(saved.messages.len(), 3);
        assert_eq!(saved.messages[1].speaker, Speaker::Learner);
        assert_eq!(saved.messages[2].speaker, Speaker::Zoe);
    }

    #[test]
    fn three_turns_create_a_complete_summary() {
        let service = service();
        service.save_learner("Kabir").unwrap();
        let session = service.start_session("food-i-love").unwrap();
        for text in [
            "I love dosa because it is crispy",
            "My mother cooked it last Sunday",
            "We ate together and talked for a long time",
        ] {
            service.send_text_turn(&session.id, text).unwrap();
        }
        let summary = service.complete_session(&session.id).unwrap();
        assert_eq!(summary.turns, 3);
        assert_eq!(summary.garden.total_conversations, 1);
        assert!(summary.best_evidence.is_some());
    }

    #[test]
    fn empty_turn_does_not_write_messages() {
        let service = service();
        service.save_learner("Riya").unwrap();
        let session = service.start_session("my-dreams").unwrap();
        assert!(service.send_text_turn(&session.id, "   ").is_err());
        assert_eq!(service.get_session(&session.id).unwrap().messages.len(), 1);
    }

    #[test]
    fn ended_session_rejects_more_turns() {
        let service = service();
        service.save_learner("Manu").unwrap();
        let session = service.start_session("my-dreams").unwrap();
        service.complete_session(&session.id).unwrap();
        assert!(service
            .send_text_turn(&session.id, "One more thing")
            .is_err());
    }

    #[test]
    #[ignore = "requires Canary, llama.cpp, Whisper fallback, and Piper development engines"]
    fn complete_local_voice_turn_uses_canary_and_returns_playable_audio() {
        let engine = LocalEngine::from_environment(None);
        let fixture = engine
            .synthesize("I played football with my best friend after school.")
            .unwrap()
            .audio
            .expect("Piper fixture audio");
        let wav = STANDARD.decode(fixture.base64).unwrap();
        let samples = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();

        let service = AppService::new(Database::in_memory().unwrap(), Box::new(engine));
        service.save_learner("Asha").unwrap();
        let session = service.start_session("school-life").unwrap();
        let result = service
            .send_voice_turn(
                &session.id,
                samples,
                22_050,
                Some("this browser transcript must not be used".into()),
            )
            .unwrap();

        assert!(result
            .learner_message
            .content
            .to_lowercase()
            .contains("football"));
        assert!(result.zoe_message.content.contains('?'));
        assert!(result.audio.is_some());
        let timings = result.timings.expect("structured timings");
        assert_eq!(
            timings.stt_engine.as_deref(),
            Some("canary-180m-flash-q8_0")
        );
        assert!(timings.llm_ttft_ms.is_some());
        assert!(timings.tts_first_audio_ms.is_some());
        assert!(timings.total_ms > 0);
    }
}
