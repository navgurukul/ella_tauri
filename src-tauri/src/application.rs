use chrono::Utc;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use uuid::Uuid;

use crate::{
    domain::{
        skill_seeds, topics, topics_for_age, AppSnapshot, Learner, Message, Session, SessionSummary, SkillEvidence,
        Speaker, Topic, TurnResult, TutorRequest,
    },
    error::{EllaError, EllaResult},
    infrastructure::{
        audio::{quietest_cut_index, trim_to_speech},
        database::Database,
        engines::TutorEngine,
    },
    telemetry::LatencyTrace,
};

/// Dispatch a background chunk once this much un-transcribed audio has
/// accumulated, cutting at the quietest point of the trailing window. Kept
/// small so the tail left on the critical path after stop stays short — a
/// Canary tail failure then costs ~2s of Whisper rescue instead of 3-6s.
const STREAM_CHUNK_TARGET_SECS: usize = 6;
const STREAM_CUT_SEARCH_SECS: usize = 3;

struct StreamChunkOutcome {
    text: String,
    engine: String,
    fell_back: bool,
}

struct VoiceStream {
    session_id: String,
    sample_rate: u32,
    pending: Vec<i16>,
    total_samples: usize,
    chunks: Vec<thread::JoinHandle<StreamChunkOutcome>>,
}

pub struct AppService {
    database: Database,
    engine: Arc<dyn TutorEngine>,
    streams: Mutex<HashMap<String, VoiceStream>>,
}

impl AppService {
    pub fn new(database: Database, engine: Box<dyn TutorEngine>) -> Self {
        Self {
            database,
            engine: Arc::from(engine),
            streams: Mutex::new(HashMap::new()),
        }
    }

    pub fn bootstrap(&self) -> EllaResult<AppSnapshot> {
        let learner = self.database.learner()?;
        Ok(AppSnapshot {
            topics: topics_for_age(learner.as_ref().and_then(|learner| learner.age)),
            learner,
            recent_sessions: self.database.recent_sessions(5)?,
            engine_status: self.engine.status(),
            garden: self.database.garden()?,
        })
    }

    pub fn save_learner(&self, name: &str, age: Option<u8>) -> EllaResult<Learner> {
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
        if let Some(age) = age {
            if !(3..=120).contains(&age) {
                return Err(EllaError::Validation(
                    "Please enter an age between 3 and 120.".into(),
                ));
            }
        }
        let existing = self.database.learner()?;
        let created_at = existing
            .as_ref()
            .map(|learner| learner.created_at.clone())
            .unwrap_or_else(now);
        let learner = Learner {
            name: clean.into(),
            // Onboarding can be re-run without the age step; keep what we know.
            age: age.or_else(|| existing.as_ref().and_then(|learner| learner.age)),
            level_name: "Morning Meadow".into(),
            created_at,
        };
        self.database.save_learner(&learner)?;
        Ok(learner)
    }

    pub fn start_session(&self, topic_id: &str) -> EllaResult<Session> {
        let topic = find_topic(topic_id)?;
        let learner = self.database.learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Ella your name before starting a conversation.".into())
        })?;
        let started_at = now();
        let opening = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Ella,
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

    /// Open a streaming voice turn: audio arrives in pushes while the learner
    /// is still speaking, and chunks are transcribed in the background so only
    /// the short tail sits on the critical path after stop.
    pub fn begin_voice_stream(&self, session_id: &str) -> EllaResult<String> {
        let session = self.database.session(session_id)?;
        if session.status != "active" {
            return Err(EllaError::Conflict(
                "This conversation has already ended.".into(),
            ));
        }
        // Live chunked transcription only exists for the native STT route.
        // Without it the buffered turn is the working path -- it carries the
        // whole recording instead of the streaming tail -- so refuse here and
        // let the caller fall back to `send_voice_turn`.
        if !self.engine.uses_native_stt() {
            return Err(EllaError::Conflict(
                "Live chunked transcription needs the native speech engine.".into(),
            ));
        }
        let stream_id = Uuid::new_v4().to_string();
        let mut streams = self.streams.lock().unwrap_or_else(|p| p.into_inner());
        // A learner has one live recording at a time: drop stale streams for
        // the same session (e.g. after an interrupted turn).
        streams.retain(|_, stream| stream.session_id != session_id);
        streams.insert(
            stream_id.clone(),
            VoiceStream {
                session_id: session_id.into(),
                sample_rate: 0,
                pending: Vec::new(),
                total_samples: 0,
                chunks: Vec::new(),
            },
        );
        eprintln!("[LATENCY]     stt-stream> stream {stream_id} opened for live chunked transcription");
        Ok(stream_id)
    }

    pub fn push_voice_stream(
        &self,
        stream_id: &str,
        samples: Vec<i16>,
        sample_rate: u32,
    ) -> EllaResult<()> {
        if sample_rate == 0 {
            return Err(EllaError::Validation(
                "The microphone reported an invalid sample rate.".into(),
            ));
        }
        let mut streams = self.streams.lock().unwrap_or_else(|p| p.into_inner());
        let stream = streams.get_mut(stream_id).ok_or_else(|| {
            EllaError::Conflict("This voice stream is no longer active.".into())
        })?;
        stream.sample_rate = sample_rate;
        stream.total_samples += samples.len();
        if stream.total_samples > sample_rate as usize * 90 {
            return Err(EllaError::Validation(
                "Please keep one answer under 90 seconds.".into(),
            ));
        }
        stream.pending.extend_from_slice(&samples);

        let target = sample_rate as usize * STREAM_CHUNK_TARGET_SECS;
        while stream.pending.len() >= target {
            let search_from = stream.pending.len() - sample_rate as usize * STREAM_CUT_SEARCH_SECS;
            let cut = quietest_cut_index(
                &stream.pending,
                sample_rate,
                search_from,
                stream.pending.len(),
            )
            .max(sample_rate as usize); // never dispatch a sub-second chunk
            let head = stream.pending.drain(..cut).collect::<Vec<i16>>();
            let index = stream.chunks.len();
            let engine = Arc::clone(&self.engine);
            eprintln!(
                "[LATENCY]     stt-stream> dispatching chunk {index} (~{:.0} ms audio) while learner is still speaking",
                head.len() as f64 * 1_000.0 / sample_rate as f64
            );
            stream.chunks.push(thread::spawn(move || {
                transcribe_stream_chunk(&engine, index, head, sample_rate)
            }));
        }
        Ok(())
    }

    pub fn cancel_voice_stream(&self, stream_id: &str) {
        let mut streams = self.streams.lock().unwrap_or_else(|p| p.into_inner());
        if streams.remove(stream_id).is_some() {
            eprintln!("[LATENCY]     stt-stream> stream {stream_id} cancelled");
        }
    }

    pub fn finish_voice_stream_turn(
        &self,
        stream_id: &str,
        tail_samples: Vec<i16>,
        sample_rate: u32,
        browser_transcript: Option<String>,
    ) -> EllaResult<TurnResult> {
        let mut trace = LatencyTrace::new("voice");
        let outcome = (|| {
            let mut stream = {
                let mut streams = self.streams.lock().unwrap_or_else(|p| p.into_inner());
                streams.remove(stream_id).ok_or_else(|| {
                    EllaError::Conflict("This voice stream is no longer active.".into())
                })?
            };
            let sample_rate = if sample_rate > 0 {
                sample_rate
            } else {
                stream.sample_rate
            };
            if sample_rate == 0 {
                return Err(EllaError::Validation(
                    "The microphone reported an invalid sample rate.".into(),
                ));
            }
            stream.pending.extend_from_slice(&tail_samples);
            stream.total_samples += tail_samples.len();
            let total_ms = stream.total_samples as f64 * 1_000.0 / sample_rate as f64;
            trace.stage(
                "turn:received",
                &format!(
                    "streamed voice turn: {} background chunks + {} tail samples (~{total_ms:.0} ms total audio)",
                    stream.chunks.len(),
                    stream.pending.len(),
                ),
            );
            trace.record_vad(0.0, total_ms, total_ms);

            let transcript = if self.engine.uses_native_stt() {
                trace.stage(
                    "stt:start",
                    "transcribing tail chunk and joining background chunks",
                );
                let stt_started = Instant::now();
                let tail_outcome = if stream.pending.len() >= sample_rate as usize / 4 {
                    let index = stream.chunks.len();
                    Some(transcribe_stream_chunk(
                        &self.engine,
                        index,
                        std::mem::take(&mut stream.pending),
                        sample_rate,
                    ))
                } else {
                    None
                };
                let mut outcomes = Vec::new();
                for handle in stream.chunks {
                    match handle.join() {
                        Ok(outcome) => outcomes.push(outcome),
                        Err(_) => eprintln!(
                            "[LATENCY]     stt-stream> a background chunk thread panicked; its words are lost"
                        ),
                    }
                }
                outcomes.extend(tail_outcome);
                let stt_elapsed = stt_started.elapsed().as_secs_f64() * 1_000.0;
                let joined = outcomes
                    .iter()
                    .map(|outcome| outcome.text.trim())
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let engines_used = outcomes
                    .iter()
                    .map(|outcome| outcome.engine.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                let any_fallback = outcomes.iter().any(|outcome| outcome.fell_back);
                trace.stage(
                    "stt:done",
                    &format!(
                        "took {stt_elapsed:.1} ms on the critical path | {} chunks [{engines_used}] | transcript=\"{joined}\"",
                        outcomes.len()
                    ),
                );
                trace.record_stt(
                    stt_elapsed,
                    "streamed-chunks".into(),
                    engines_used,
                    any_fallback.then(|| "canary-chunk".into()),
                    None,
                    None,
                    None,
                );
                if joined.is_empty() {
                    return Err(EllaError::Validation(
                        "I could not hear any words in that recording. Move closer to the microphone and try again."
                            .into(),
                    ));
                }
                joined
            } else {
                match browser_transcript {
                    Some(text) if !text.trim().is_empty() => {
                        trace.record_browser_stt();
                        text
                    }
                    _ => {
                        return Err(EllaError::Engine(
                            "I captured your voice, but native speech recognition is not enabled in demo mode. Try typing or start local engine mode."
                                .into(),
                        ))
                    }
                }
            };
            self.run_turn(&stream.session_id, &transcript, &mut trace)
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
            EllaError::Conflict("Tell Ella your name before starting a conversation.".into())
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
            return Err(EllaError::Engine("Ella returned an empty reply.".into()));
        }
        let created_at = now();
        let learner_message = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Learner,
            content: clean.into(),
            turn,
            created_at: created_at.clone(),
        };
        let ella_message = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Ella,
            content: reply.clone(),
            turn,
            created_at: created_at.clone(),
        };
        let (skill_id, _, _) = skill_seeds()[((turn - 1) as usize) % skill_seeds().len()];
        let db_persist_started = Instant::now();
        let skill = self.database.persist_turn(
            session_id,
            &learner_message,
            &ella_message,
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
            ella_message,
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

/// Transcribe one streamed chunk. Runs on a background thread during
/// recording (or inline for the tail). A chunk that yields no words is an
/// empty outcome, not an error: mid-utterance silence is normal, and the
/// engine's own Canary->Whisper fallback has already been applied.
fn transcribe_stream_chunk(
    engine: &Arc<dyn TutorEngine>,
    index: usize,
    samples: Vec<i16>,
    sample_rate: u32,
) -> StreamChunkOutcome {
    let audio_ms = samples.len() as f64 * 1_000.0 / sample_rate as f64;
    let started = Instant::now();
    let (text, engine_name, fell_back) = match engine.transcribe(&samples, sample_rate) {
        Ok(transcription) => {
            let fell_back = transcription.fallback_from.is_some();
            (transcription.text, transcription.engine, fell_back)
        }
        Err(error) => {
            eprintln!("[LATENCY]     stt-stream> chunk {index} produced no words ({error})");
            (String::new(), "none".into(), true)
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    eprintln!(
        "[LATENCY]     stt-stream> chunk {index} done in {elapsed_ms:.1}ms (~{audio_ms:.0} ms audio, engine={engine_name}): \"{text}\""
    );
    StreamChunkOutcome {
        text,
        engine: engine_name,
        fell_back,
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
    fn young_learners_are_offered_everyday_topics_first() {
        let grown_up = topics_for_age(Some(30));
        assert_eq!(grown_up[0].id, "street-food");
        assert!(grown_up.iter().any(|topic| topic.id == "job-interview"));

        let child = topics_for_age(Some(8));
        // Nothing is taken away, but the grown-up scenarios sink to the bottom.
        assert_eq!(child.len(), grown_up.len());
        let interview = child.iter().position(|t| t.id == "job-interview").unwrap();
        let bargaining = child.iter().position(|t| t.id == "market-bargaining").unwrap();
        assert!(interview >= child.len() - 2);
        assert!(bargaining >= child.len() - 2);
        assert_eq!(child[0].id, "street-food");
    }

    #[test]
    fn the_home_list_carries_the_topic_each_session_belongs_to() {
        let service = service();
        service.save_learner("Meera", Some(14)).unwrap();
        let session = service.start_session("restaurant-order").unwrap();

        let listed = service.bootstrap().unwrap().recent_sessions;
        let entry = listed.iter().find(|item| item.id == session.id).unwrap();
        assert_eq!(entry.topic_id, "restaurant-order");
        assert_eq!(entry.status, "active");
    }

    #[test]
    fn an_unfinished_session_can_be_read_back_with_its_messages() {
        let service = service();
        service.save_learner("Meera", Some(14)).unwrap();
        let session = service.start_session("street-food").unwrap();
        service
            .send_text_turn(&session.id, "I ate poha this morning")
            .unwrap();

        let resumed = service.get_session(&session.id).unwrap();
        assert_eq!(resumed.status, "active");
        assert_eq!(resumed.messages.len(), 3);
        assert!(resumed.messages[1].content.contains("poha"));
    }

    #[test]
    fn full_demo_turn_is_persisted_and_grows_a_skill() {
        let service = service();
        service.save_learner("Asha", Some(14)).unwrap();
        let session = service.start_session("street-food").unwrap();
        let result = service
            .send_text_turn(&session.id, "I played football with my best friend")
            .unwrap();

        assert_eq!(result.learner_message.turn, 1);
        assert_eq!(result.evidence.unwrap().new_stage, 1);
        let saved = service.get_session(&session.id).unwrap();
        assert_eq!(saved.messages.len(), 3);
        assert_eq!(saved.messages[1].speaker, Speaker::Learner);
        assert_eq!(saved.messages[2].speaker, Speaker::Ella);
    }

    #[test]
    fn three_turns_create_a_complete_summary() {
        let service = service();
        service.save_learner("Kabir", Some(16)).unwrap();
        let session = service.start_session("restaurant-order").unwrap();
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
        service.save_learner("Riya", None).unwrap();
        let session = service.start_session("job-interview").unwrap();
        assert!(service.send_text_turn(&session.id, "   ").is_err());
        assert_eq!(service.get_session(&session.id).unwrap().messages.len(), 1);
    }

    #[test]
    fn ended_session_rejects_more_turns() {
        let service = service();
        service.save_learner("Manu", Some(21)).unwrap();
        let session = service.start_session("job-interview").unwrap();
        service.complete_session(&session.id).unwrap();
        assert!(service
            .send_text_turn(&session.id, "One more thing")
            .is_err());
    }

    #[test]
    fn demo_mode_refuses_a_live_stream_so_the_buffered_turn_is_used() {
        let service = service();
        service.save_learner("Asha", Some(14)).unwrap();
        let session = service.start_session("street-food").unwrap();
        // Without native STT the streaming tail is not enough to transcribe;
        // the caller has to fall back to the turn that carries all the audio.
        assert!(service.begin_voice_stream(&session.id).is_err());
        let error = service
            .send_voice_turn(&session.id, vec![0; 16_000], 16_000, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("local engine mode"), "{error}");
    }

    #[test]
    #[ignore = "requires Canary, llama.cpp, Whisper fallback, and Piper development engines"]
    fn streamed_voice_turn_transcribes_chunks_in_the_background() {
        let engine = LocalEngine::from_environment(None);
        let fixture = engine
            .synthesize("I played football with my best friend after school today.")
            .unwrap()
            .audio
            .expect("Piper fixture audio");
        let wav = STANDARD.decode(fixture.base64).unwrap();
        let one_pass = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        // Repeat the utterance so the stream crosses the 10s chunk target and
        // dispatches at least one background chunk before the tail.
        let samples = one_pass.repeat(4);
        let sample_rate = 22_050_u32;

        let service = AppService::new(Database::in_memory().unwrap(), Box::new(engine));
        service.save_learner("Asha", Some(14)).unwrap();
        let session = service.start_session("street-food").unwrap();
        let stream_id = service.begin_voice_stream(&session.id).unwrap();
        // Push in ~1s slices like the WebView does.
        let mut cursor = 0;
        while cursor < samples.len() - sample_rate as usize {
            let end = (cursor + sample_rate as usize).min(samples.len());
            service
                .push_voice_stream(&stream_id, samples[cursor..end].to_vec(), sample_rate)
                .unwrap();
            cursor = end;
        }
        let tail = samples[cursor..].to_vec();
        let result = service
            .finish_voice_stream_turn(&stream_id, tail, sample_rate, None)
            .unwrap();

        assert!(result
            .learner_message
            .content
            .to_lowercase()
            .contains("football"));
        assert!(result.audio.is_some());
        let timings = result.timings.expect("structured timings");
        assert_eq!(timings.stt_engine.as_deref(), Some("streamed-chunks"));
        assert!(timings.stt_ms.is_some());
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
        service.save_learner("Asha", Some(14)).unwrap();
        let session = service.start_session("street-food").unwrap();
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
        assert!(result.ella_message.content.contains('?'));
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
