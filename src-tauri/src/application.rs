use chrono::{Local, Utc};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use uuid::Uuid;

use crate::{
    curriculum::{self, Position},
    domain::{
        find_character, find_chore, topics, topics_for_age, Advance, AppSnapshot, Assessment,
        ChoreContext, ChoreRecap, Focus, LedgerTurn, AudioPayload, LedgerView, Learner, LearnerProfile,
        LearnerProgress, LevelSkillView, LevelState, LevelView, Message, Pitch, PlacementBrief,
        PlacementReading, Readiness, Scorable, Session, SessionSummary, SpeechStreamEvent,
        SpokenLine, Standing, StepView, TalkNotes, TurnSignal, WinCondition, Speaker, Topic,
        TurnResult, TutorRequest, WordSpan,
    },
    error::{EllaError, EllaResult},
    infrastructure::{
        audio::{quietest_cut_index, trim_to_speech, VadOutput},
        database::{AssessmentWrite, Database},
        engines::{trailing_question, GeneratedReply, SpeechSegment, SpeechSink, TutorEngine, FREE_TOPIC_TURNS},
        safety,
    },
    notes,
    progress::{self, ProgressMap, SkillProgress},
    telemetry::LatencyTrace,
};

/// The placement chat is a session like any other, marked with this kind. Its
/// topic id and label are what Home's list shows if it is ever left open.
const PLACEMENT_KIND: &str = "placement";
pub const PLACEMENT_TOPIC_ID: &str = "placement";
const PLACEMENT_LABEL: &str = "First talk";

/// How the shell delivers a finished sentence to the window mid-turn.
///
/// Registered once at startup. Without one — tests, the chore bench — nothing
/// streams and every turn returns its audio whole, exactly as before.
pub trait SpeechBroadcast: Send + Sync {
    fn speak(&self, event: SpeechStreamEvent);
}

/// Ties the engine's sentence stream to one session's turn.
struct TurnSpeech {
    broadcast: Arc<dyn SpeechBroadcast>,
    session_id: String,
    turn: u32,
}

impl SpeechSink for TurnSpeech {
    fn segment(&self, segment: SpeechSegment) {
        self.broadcast.speak(SpeechStreamEvent {
            session_id: self.session_id.clone(),
            turn: self.turn,
            index: segment.index,
            text: segment.text,
            audio: segment.audio,
            ready_ms: segment.ready_ms,
            words: segment.words,
        });
    }
}

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
    /// Why this chunk produced nothing. Kept so a turn where every chunk failed
    /// can report the real cause instead of guessing at the microphone.
    error: Option<String>,
}

struct VoiceStream {
    session_id: String,
    sample_rate: u32,
    pending: Vec<i16>,
    total_samples: usize,
    chunks: Vec<thread::JoinHandle<StreamChunkOutcome>>,
}

/// A placement check started after the chat's latest exchange. It runs while
/// Ella speaks and the learner thinks of an answer, and the next turn reads
/// it: see `AppService::placement_closes`.
struct PendingCheck {
    /// How many answers it judged.
    answers: u32,
    verdict: Verdict,
}

enum Verdict {
    Running(thread::JoinHandle<Option<Readiness>>),
    /// Read by a turn that has not been saved yet. Kept, so that if the turn
    /// fails, the answer tried again is judged the same.
    Read(Option<Readiness>),
}

pub struct AppService {
    database: Database,
    engine: Arc<dyn TutorEngine>,
    streams: Mutex<HashMap<String, VoiceStream>>,
    speech: Mutex<Option<Arc<dyn SpeechBroadcast>>>,
    /// At most one per placement chat in progress, by session id.
    placement_checks: Mutex<HashMap<String, PendingCheck>>,
}

impl AppService {
    pub fn new(database: Database, engine: Box<dyn TutorEngine>) -> Self {
        Self {
            database,
            engine: Arc::from(engine),
            streams: Mutex::new(HashMap::new()),
            speech: Mutex::new(None),
            placement_checks: Mutex::new(HashMap::new()),
        }
    }

    /// Called from the app's exit event: Tauri never drops managed state, so
    /// this is the only point at which the engine's processes and GPU buffers
    /// can be released, and the only point at which the whole of the
    /// database's write-ahead log is folded into `ella.sqlite3`.
    ///
    /// The checkpoint goes first because it is quick and the engine can take
    /// seconds to let go. Both are safe to run twice, which they are on every
    /// ordinary quit: the exit event calls this, and then Tauri's resource
    /// cleanup drops `EngineShutdownGuard`, which calls it again. The Windows
    /// updater skips the exit event, so there only the guard calls it.
    pub fn shutdown(&self) {
        if let Err(error) = self.database.checkpoint() {
            // Nothing is lost: every commit is already durable in the log.
            // The next launch reads it back from there, and a later
            // checkpoint (at the latest, the next quit) folds it in.
            eprintln!("[database] shutdown: could not fold the write-ahead log into ella.sqlite3: {error}");
        }
        self.engine.shutdown();
    }

    /// Hand the service somewhere to push sentences as they are synthesized.
    /// Called once, after the window exists.
    pub fn set_speech_broadcast(&self, broadcast: Arc<dyn SpeechBroadcast>) {
        *self
            .speech
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(broadcast);
    }

    fn speech_broadcast(&self) -> Option<Arc<dyn SpeechBroadcast>> {
        self.speech
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Everything the window needs to draw. Signed out, the only learner
    /// data in it is `saved_learner`, the name and colour the welcome-back
    /// step greets them with; their talks and figures wait for "Log in".
    pub fn bootstrap(&self) -> EllaResult<AppSnapshot> {
        let saved = self.database.learner()?;
        let saved_learner = saved.as_ref().map(|(learner, _)| LearnerProfile::from(learner));
        let learner = saved.and_then(|(learner, signed_in)| signed_in.then_some(learner));
        let (recent_sessions, progress, standing) = if learner.is_some() {
            (
                self.database.recent_sessions(5)?,
                self.database.progress()?,
                Some(self.standing()?),
            )
        } else {
            (Vec::new(), LearnerProgress::default(), None)
        };
        Ok(AppSnapshot {
            topics: topics_for_age(learner.as_ref().and_then(|learner| learner.age)),
            learner,
            saved_learner,
            recent_sessions,
            progress,
            standing,
            engine_status: self.engine.status(),
        })
    }

    /// Where the learner is, and whether a placement chat has read a level for
    /// them. Someone nothing has put anywhere yet — or whose stored level the
    /// curriculum no longer has — stands at the curriculum's start, as every
    /// new learner does on the phone. A learner can move on through talks
    /// without a placement, so the two are kept apart.
    fn placement(&self) -> EllaResult<(Position, bool)> {
        let position = self
            .database
            .position()?
            .and_then(known_position)
            .unwrap_or_else(curriculum::start);
        Ok((position, self.database.placed()?))
    }

    /// The learner's place in the curriculum, as the window shows it.
    fn standing(&self) -> EllaResult<Standing> {
        let (position, placed) = self.placement()?;
        let keys = keys_of(&progress::step_skills(&position));
        let known = progress::by_key(&self.database.skill_progress(&keys)?);
        Ok(standing_at(&position, placed, &known))
    }

    /// The onboarding name step, which also signs the learner in. There is
    /// one learner per laptop, so this always saves that learner: signed in,
    /// it corrects them; signed out, "Let's start" is the same person
    /// onboarding again, and their talks and progress stay theirs.
    pub fn save_learner(&self, name: &str, age: Option<u8>) -> EllaResult<Learner> {
        let clean = clean_name(name)?;
        if let Some(age) = age {
            if !(3..=120).contains(&age) {
                return Err(EllaError::Validation(
                    "Please enter an age between 3 and 120.".into(),
                ));
            }
        }
        // Onboarding can be re-run without the age step; the database keeps
        // the age it already knows when none is given. `level_name` is
        // v0.1.6's column, kept to the level the learner is actually at.
        let (position, _) = self.placement()?;
        self.database
            .save_learner(clean, age, &curriculum::level_name(&position.level), &now())
    }

    /// "Log in" on the welcome screen: the learner saved on this laptop
    /// comes back to everything they had. There is nobody else it could be,
    /// so there is no name to type.
    pub fn log_in(&self) -> EllaResult<Learner> {
        self.database
            .sign_in()?
            .ok_or_else(|| EllaError::Conflict("Tell Ella your name first.".into()))
    }

    /// Sign out and nothing more: the learner, their talks and their progress
    /// stay on this laptop for the next time they log in.
    pub fn log_out(&self) -> EllaResult<AppSnapshot> {
        self.database.sign_out()?;
        self.bootstrap()
    }

    pub fn save_avatar_color(&self, color: &str) -> EllaResult<Learner> {
        let signed_out = || EllaError::Conflict("Tell Ella your name first.".into());
        if self.database.signed_in_learner()?.is_none() {
            return Err(signed_out());
        }
        if !is_hex_color(color) {
            return Err(EllaError::Validation(
                "Please choose one of the colours on the profile screen.".into(),
            ));
        }
        // `None` here means a log out landed in between; the colour was not
        // stored, and saying so is the same as if it had come first.
        self.database.save_avatar_color(color)?.ok_or_else(signed_out)
    }

    /// Say Ella's opening line aloud.
    ///
    /// Kept out of `start_session` so the conversation is on screen before
    /// Piper is asked for anything: the screen appears immediately and Ella
    /// starts talking a sentence later, rather than the whole thing waiting on
    /// synthesis. Sentences stream, so the opening is highlighted word by word
    /// like every reply after it.
    pub fn speak_opening(&self, session_id: &str) -> EllaResult<SpokenLine> {
        let session = self.database.session(session_id)?;
        let opening = session
            .messages
            .iter()
            .find(|message| message.speaker == Speaker::Ella)
            .map(|message| message.content.clone())
            .ok_or_else(|| {
                EllaError::Conflict("This conversation has no opening line to say.".into())
            })?;
        let speech: Option<Arc<dyn SpeechSink>> = self.speech_broadcast().map(|broadcast| {
            Arc::new(TurnSpeech {
                broadcast,
                session_id: session_id.to_owned(),
                // Turn 0 is the opening: it answers nothing.
                turn: 0,
            }) as Arc<dyn SpeechSink>
        });
        let synthesized = self.engine.speak(&opening, speech)?;
        Ok(SpokenLine {
            // Zero means the window heard nothing yet and has to play the
            // recording — which is what demo mode and a Piper-less install do.
            streamed_segments: synthesized.segments,
            audio: synthesized.audio,
            speech_words: synthesized.words,
        })
    }

    /// Said aloud when a voice turn comes back with no words at all — the
    /// learner should hear that Ella missed them, not just read it. Mirrors
    /// `speak_opening`: nothing here is persisted, because there is no
    /// learner content to pair it with. `turn: 0` is a lie in the same
    /// harmless way it is there — this line answers nothing, so it does not
    /// need a real turn number.
    pub fn speak_retry_prompt(&self, session_id: &str) -> EllaResult<SpokenLine> {
        let session = self.database.session(session_id)?;
        let text = "I couldn't quite hear that, could you try again?";
        let speech: Option<Arc<dyn SpeechSink>> = self.speech_broadcast().map(|broadcast| {
            Arc::new(TurnSpeech {
                broadcast,
                session_id: session.id.clone(),
                turn: 0,
            }) as Arc<dyn SpeechSink>
        });
        let synthesized = self.engine.speak(text, speech)?;
        Ok(SpokenLine {
            streamed_segments: synthesized.segments,
            audio: synthesized.audio,
            speech_words: synthesized.words,
        })
    }

    /// The recap's one fix, said the right way, so the learner can hear it.
    /// Only ever the fix kept on the talk: the window names the talk, not the
    /// words. Synthesized whole, since nothing is waiting to highlight it.
    pub fn speak_fix(&self, session_id: &str) -> EllaResult<SpokenLine> {
        let meta = self.database.session_curriculum(session_id)?;
        let fix = meta
            .assessment
            .as_deref()
            .map(read_assessment)
            .transpose()?
            .and_then(|assessment| assessment.notes)
            .and_then(|notes| notes.fix)
            .ok_or_else(|| EllaError::NotFound("There is nothing to hear for this talk.".into()))?;
        let synthesized = self.engine.speak(&fix.better, None)?;
        Ok(SpokenLine {
            streamed_segments: synthesized.segments,
            audio: synthesized.audio,
            speech_words: synthesized.words,
        })
    }

    /// A talk on a topic. Every one quietly aims at a skill of the learner's
    /// step, picked here and kept on the session, so each turn's instructions
    /// say the same and the talk is scored against it when it ends.
    pub fn start_session(&self, topic_id: &str) -> EllaResult<Session> {
        let topic = find_topic(topic_id)?;
        let learner = self.database.signed_in_learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Ella your name before starting a conversation.".into())
        })?;
        let (position, _) = self.placement()?;
        let target = self.choose_target(&position)?;
        let pitch = Pitch {
            level: position.level.clone(),
            focus: target.as_deref().and_then(focus_for),
        };
        let started_at = now();
        let opening = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Ella,
            content: self.engine.opening(&topic, &learner.name, &pitch)?,
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
        self.database.create_session(
            &session,
            &opening,
            None,
            target.as_deref(),
            Some(&position.level),
        )?;
        Ok(session)
    }

    /// The placement chat: a friendly first talk that climbs from easy
    /// questions to harder ones until it has heard enough to read a level,
    /// between five answers and twelve. Onboarding opens it, and so does the
    /// level map for a learner who never had one.
    pub fn start_placement(&self) -> EllaResult<Session> {
        let learner = self.database.signed_in_learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Ella your name before starting a conversation.".into())
        })?;
        let started_at = now();
        let opening = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Ella,
            content: self.engine.placement_opening(&learner.name, learner.age)?,
            turn: 0,
            created_at: started_at.clone(),
        };
        let session = Session {
            id: Uuid::new_v4().to_string(),
            topic_id: PLACEMENT_TOPIC_ID.into(),
            topic_label: PLACEMENT_LABEL.into(),
            status: "active".into(),
            started_at,
            completed_at: None,
            messages: vec![opening.clone()],
        };
        self.database
            .create_session(&session, &opening, Some(PLACEMENT_KIND), None, None)?;
        Ok(session)
    }

    /// The skill a new talk at `position` aims at: the one that most wants
    /// practice, with the latest talks' aims held back.
    fn choose_target(&self, position: &Position) -> EllaResult<Option<String>> {
        let in_play = keys_of(&progress::skills_in_play(position));
        if in_play.is_empty() {
            return Ok(None);
        }
        let known = progress::by_key(&self.database.skill_progress(&in_play)?);
        let recent = self.database.recent_targets()?;
        Ok(progress::choose_target(position, &known, &recent, &today()).map(|placed| placed.key))
    }

    /// Start a chore. The session row is the same shape as a free conversation;
    /// what differs is that it names a chore and a character, and — for ledger
    /// chores — opens a ledger at the authored opening figure.
    pub fn start_chore(&self, chore_id: &str) -> EllaResult<Session> {
        let chore = find_chore(chore_id)
            .ok_or_else(|| EllaError::NotFound(format!("No chore called {chore_id}.")))?;
        let character = find_character(&chore.character_id).ok_or_else(|| {
            EllaError::Engine(format!(
                "Chore {chore_id} names character {} which is not in the cast.",
                chore.character_id
            ))
        })?;
        let learner = self.database.signed_in_learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Ella your name before starting a conversation.".into())
        })?;

        let ledger_opening = match &chore.win {
            WinCondition::Ledger(spec) => Some(spec.opening),
            WinCondition::Rubric { .. } => None,
        };
        let (position, _) = self.placement()?;
        let context = ChoreContext {
            chore_id: chore.id.clone(),
            level: position.level.clone(),
            character: character.clone(),
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
        };

        let started_at = now();
        let opening = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Ella,
            content: self.engine.opening_in_chore(&context, &learner.name)?,
            turn: 0,
            created_at: started_at.clone(),
        };
        let session = Session {
            id: Uuid::new_v4().to_string(),
            topic_id: chore.id.clone(),
            topic_label: chore.title.clone(),
            status: "active".into(),
            started_at,
            completed_at: None,
            messages: vec![opening.clone()],
        };
        self.database.create_chore_session(
            &session,
            &opening,
            &chore.id,
            &character.id,
            ledger_opening,
            &position.level,
            &now(),
        )?;
        Ok(session)
    }

    /// Rebuild the chore context for a turn, with the ledger figure as it
    /// currently stands and the character pitched at `level`. `None` for a
    /// free-topic session.
    fn chore_context_for(&self, session_id: &str, level: &str) -> EllaResult<Option<ChoreContext>> {
        let Some((chore_id, character_id)) = self.database.session_chore(session_id)? else {
            return Ok(None);
        };
        let Some(chore) = find_chore(&chore_id) else {
            // The catalog moved under a live session. Degrade to a free
            // conversation rather than failing the turn.
            return Ok(None);
        };
        let character = find_character(&character_id).unwrap_or_else(|| {
            find_character("ella").expect("the cast always contains Ella")
        });
        let ledger = match (&chore.win, self.database.ledger_state(session_id)?) {
            (WinCondition::Ledger(spec), Some((current, agreed))) => Some(LedgerTurn {
                spec: spec.clone(),
                current,
                agreed,
            }),
            _ => None,
        };
        Ok(Some(ChoreContext {
            chore_id: chore.id,
            level: level.into(),
            character,
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger,
        }))
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
                    // Every chunk came back empty. The real cause (an engine
                    // failure vs. a quiet learner) is worth knowing when we're
                    // chasing a bug, but it is not worth reading out loud to a
                    // learner — so it goes to the log and the trace, not the
                    // message a caller sees. `NO_SPEECH_DETECTED:` is a stable
                    // marker the frontend matches on to decide whether to have
                    // Ella speak a retry prompt; the sentence after it is what
                    // shows up if that fails and the generic error path is
                    // used instead.
                    let reasons = outcomes
                        .iter()
                        .filter_map(|outcome| outcome.error.as_deref())
                        .collect::<Vec<_>>();
                    eprintln!(
                        "[LATENCY]     stt-stream> all {} parts came back empty; first cause: {}",
                        outcomes.len(),
                        reasons.first().copied().unwrap_or("no chunk reported an error")
                    );
                    return Err(EllaError::Validation(
                        "NO_SPEECH_DETECTED: I couldn't hear any words in that recording. \
                         Move closer to the microphone and try again."
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
        let learner = self.database.signed_in_learner()?.ok_or_else(|| {
            EllaError::Conflict("Tell Ella your name before starting a conversation.".into())
        })?;
        let turn = session
            .messages
            .iter()
            .filter(|message| message.speaker == Speaker::Learner)
            .count() as u32
            + 1;
        let curriculum_meta = self.database.session_curriculum(session_id)?;
        // The level the talk began at, so its instructions read the same from
        // the first turn to the last — and stay in llama.cpp's cached prefix —
        // even when another talk's assessment moves the learner on meanwhile.
        let level = match curriculum_meta.level_code.clone() {
            Some(level) => level,
            None => self.placement()?.0.level,
        };
        // A chore session carries a character, a setting and a hidden brief; a
        // free-topic session carries none of that and behaves exactly as before.
        let chore_context = self.chore_context_for(session_id, &level)?;
        let flagged = safety::flagged(clean);
        // A placement answer the filter flagged never ends the chat: the reply
        // to it asks the dodged question again, which only makes sense if the
        // chat carries on. The pending check is left for the next answer.
        let brief = (curriculum_meta.kind.as_deref() == Some(PLACEMENT_KIND)).then(|| {
            PlacementBrief {
                age: learner.age,
                closing: !flagged && self.placement_closes(session_id, turn),
            }
        });
        let pitch = Pitch {
            level,
            focus: curriculum_meta.target_skill.as_deref().and_then(focus_for),
        };
        let (learner_name, learner_age) = (learner.name.clone(), learner.age);
        let request = TutorRequest {
            learner_name: learner.name,
            topic_id: session.topic_id.clone(),
            topic_label: session.topic_label.clone(),
            messages: session.messages.clone(),
            learner_text: clean.into(),
            turn,
            chore: chore_context.clone(),
            pitch,
            placement: brief.clone(),
        };
        let mut generated = if flagged {
            // Skip the model entirely: a live session showed it does not
            // reliably self-correct once one turn slips into a soft,
            // validating register, so a flagged turn must never reach it.
            // Handing back the learner's own unanswered question keeps the
            // conversation from reading as though the dodge worked.
            let question = request
                .messages
                .iter()
                .rev()
                .find(|message| message.speaker == Speaker::Ella)
                .and_then(|message| trailing_question(&message.content));
            trace.stage("llm:skipped", "learner turn flagged by the content filter");
            GeneratedReply::plain(safety::redirect_reply(question), 0.0, 0.0)
        } else {
            trace.stage(
                "llm:start",
                &format!("requesting tutor reply for turn {turn} ({} history messages)", request.messages.len()),
            );
            self.engine.reply(&request)?
        };
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
        // A no-op on ordinary text; redacts anything rustrict recognizes as
        // profane, offensive, or sexual before it is written to the
        // transcript. `request.learner_text` (already sent to the model, or
        // not, above) is left as `clean` — this only affects what gets
        // persisted and read back later.
        let learner_message = Message {
            id: Uuid::new_v4().to_string(),
            speaker: Speaker::Learner,
            content: safety::censor(clean),
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

        let db_persist_started = Instant::now();
        self.database.persist_turn(session_id, &learner_message, &ella_message)?;
        trace.stage(
            "db:turn-persisted",
            &format!(
                "took {:.1} ms",
                db_persist_started.elapsed().as_secs_f64() * 1_000.0
            ),
        );
        // Voice is an enhancement, not a transaction dependency: if Piper is
        // missing or fails, the persisted text turn remains fully usable.
        //
        // A reply whose sentences were synthesized during generation is already
        // recorded — and, when it streamed, already playing. Only turns that
        // produced no usable pipeline audio pay for Piper on the clock here.
        let (audio, speech_words) = match generated.speech.take() {
            Some(synthesized) => {
                trace.record_tts(synthesized.first_audio_ms, synthesized.completion_ms);
                trace.stage(
                    "tts:overlapped",
                    &format!(
                        "reply synthesized during generation | first_audio={} completion={}",
                        synthesized
                            .first_audio_ms
                            .map(|ms| format!("{ms:.1} ms"))
                            .unwrap_or_else(|| "-".into()),
                        synthesized
                            .completion_ms
                            .map(|ms| format!("{ms:.1} ms"))
                            .unwrap_or_else(|| "-".into()),
                    ),
                );
                (synthesized.audio, synthesized.words)
            }
            None => self.synthesize_on_clock(&reply, trace),
        };
        // The app owns the number. `named_figure` is only ever accepted when it
        // is legal for this spec, so a character that concedes too far in prose
        // still does not move the ledger.
        let ledger_view = match chore_context.as_ref().and_then(|c| c.ledger.clone()) {
            Some(ledger) => {
                let next = generated
                    .named_figure
                    .filter(|value| ledger.spec.accepts(ledger.current, *value))
                    .unwrap_or(ledger.current);
                let agreed = ledger.agreed || generated.signal == Some(TurnSignal::Deal);
                self.database
                    .save_ledger_state(session_id, next, agreed, &created_at)?;
                Some(LedgerView {
                    unit: ledger.spec.unit.clone(),
                    current: next,
                    target: ledger.spec.target,
                    opening: ledger.spec.opening,
                    progress: ledger.spec.progress(next),
                    agreed,
                    reached_target: ledger.spec.reached_target(next),
                    regenerated: generated.regenerated,
                })
            }
            None => None,
        };

        // A chore is over when the character signs off, when the ledger has
        // gone as far as it goes, or when the turn budget runs out — not on a
        // fixed count. A free conversation has no decision to reach, so three
        // turns of practice remains the point at which stopping is fine.
        //
        // The placement chat is its own: it ends when it has heard enough.
        let suggested_complete = match (&brief, chore_context.as_ref()) {
            (Some(brief), _) => brief.closing,
            (None, Some(context)) => {
                generated.signal.is_some()
                    || ledger_view.as_ref().is_some_and(|ledger| ledger.agreed)
                    || turn >= context.max_turns
            }
            (None, None) => turn >= 3,
        };

        // Over for good, not merely a fine place to stop. Ella has just spoken
        // the closing line `free_closing_note` asked her for, so the session is
        // closed here instead of waiting for a button nothing was pressing:
        // that note fires on every turn from `FREE_TOPIC_TURNS` onward, and
        // with nothing acting on it the cab conversation wished the learner
        // well on turn 6 and then again on turn 7.
        //
        // Only the hard endings close the session. `[DEAL]` is deliberately not
        // one of them: the deposit bench ran one more turn after signing off and
        // spent it on "Thank you for understanding", which is a conversation
        // ending the way conversations do. `suggested_complete` already offers
        // the learner the way out at that point.
        //
        // The placement chat closes on its goodbye, and its level is read
        // afterwards, by `assess_session`, while Ella is still saying it.
        let conversation_over = match (&brief, chore_context.as_ref()) {
            (Some(brief), _) => brief.closing,
            (None, Some(context)) => turn >= context.max_turns,
            (None, None) => turn >= FREE_TOPIC_TURNS,
        };
        // `persist_turn` above already wrote this turn, so the summary counts it.
        let session_summary = if conversation_over {
            Some(self.complete_session(session_id)?)
        } else {
            if brief.is_some() {
                self.schedule_placement_check(session_id, &learner_name, learner_age, turn);
            }
            None
        };

        Ok(TurnResult {
            learner_message,
            ella_message,
            correction: gentle_correction(clean),
            suggested_complete,
            session_summary,
            audio,
            timings: None,
            ledger: ledger_view,
            signal: generated.signal,
            speech_words,
        })
    }

    /// Synthesize the whole reply with the learner waiting.
    ///
    /// The path for turns that produced no usable pipeline audio: demo mode, a
    /// missing Piper, or a ledger reply that was regenerated after its
    /// sentences had already been read.
    fn synthesize_on_clock(
        &self,
        reply: &str,
        trace: &mut LatencyTrace,
    ) -> (Option<AudioPayload>, Vec<WordSpan>) {
        trace.stage("tts:start", "sending reply text to speech synthesis");
        match self.engine.synthesize(reply) {
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
                (synthesized.audio, synthesized.words)
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
                (None, Vec::new())
            }
        }
    }

    pub fn complete_session(&self, session_id: &str) -> EllaResult<SessionSummary> {
        let session = self.database.session(session_id)?;
        self.database.complete_session(session_id, &now())?;
        // A placement chat ended early — Skip — has a check nobody will read.
        self.placement_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
        let answers = answers_in(&session);
        let turns = answers.len() as u32;
        let headline = if turns >= 3 {
            "You kept that conversation going".into()
        } else {
            "Every answer counts".into()
        };
        // A talk the learner never answered in is not one they finished, so
        // it does not add to the count, and quoting the unchanged number back
        // at them would read as though it had.
        let encouragement = if turns == 0 {
            "Say a few words next time and it counts. Come back and talk to Ella again.".into()
        } else {
            // The same figure the profile shows: the learner's finished talks
            // over their whole history, this one included.
            let conversations = self.database.progress()?.talks_finished;
            format!(
                "That is {conversations} conversation{} finished. Come back and talk to Ella again.",
                if conversations == 1 { "" } else { "s" }
            )
        };
        let short = notes::too_short(&answers);
        let chore = self.chore_recap(&session)?;
        Ok(SessionSummary {
            session_id: session.id,
            topic_label: session.topic_label,
            turns,
            headline,
            encouragement,
            short,
            chore,
        })
    }

    /// How a ledger chore's talk ended, read off its ledger. Asked for once the
    /// talk is closed, so `times_met` counts it too.
    fn chore_recap(&self, session: &Session) -> EllaResult<Option<ChoreRecap>> {
        let Some((chore_id, character_id)) = self.database.session_chore(&session.id)? else {
            return Ok(None);
        };
        let Some(WinCondition::Ledger(spec)) = find_chore(&chore_id).map(|chore| chore.win) else {
            return Ok(None);
        };
        let Some((figure, agreed)) = self.database.ledger_state(&session.id)? else {
            return Ok(None);
        };
        let times_met = self
            .database
            .finished_ledgers(&chore_id)?
            .into_iter()
            .filter(|(current, agreed)| *agreed && spec.reached_target(*current))
            .count() as u32;
        let last_line = session
            .messages
            .iter()
            .rev()
            .find(|message| message.speaker == Speaker::Ella)
            .map(|message| message.content.clone());
        Ok(Some(ChoreRecap {
            chore_id,
            character_id,
            unit: spec.unit.clone(),
            direction: spec.direction,
            target: spec.target,
            figure,
            agreed,
            met: agreed && spec.reached_target(figure),
            times_met,
            last_line,
        }))
    }

    /// Ella's notes on a finished talk: what went well, read off the learner's
    /// words, and one fix, if the model finds a mistake in them. `None` for a
    /// talk too short to say anything about. A fix that could not be read
    /// leaves the notes without one rather than failing the assessment, so the
    /// talk still counts; `checked` says so, and nothing claims there was
    /// nothing to fix.
    fn notes_for(&self, session: &Session) -> EllaResult<Option<TalkNotes>> {
        let answers = answers_in(session);
        if notes::too_short(&answers) {
            return Ok(None);
        }
        let chore = self.chore_recap(session)?;
        let went_well = notes::went_well(&answers, chore.as_ref());
        let checked = notes::answers_to_check(&answers);
        let (fix, looked) = match self.engine.correct(&checked) {
            Ok(Some(corrected)) => (notes::fix_from(&checked, &corrected), true),
            Ok(None) => (None, false),
            Err(error) => {
                eprintln!("[recap] correcting {}: {error}", session.id);
                (None, false)
            }
        };
        Ok(Some(TalkNotes {
            went_well,
            fix,
            checked: looked,
        }))
    }

    /// Whether the learner's `answers`-th answer is the placement chat's last.
    ///
    /// Twelve answers end it regardless. Without a model it ends at the
    /// shortest length allowed, as on the phone. Otherwise the check started
    /// after the previous exchange decides, by `progress::placement_ends`.
    ///
    /// That check judged the answers before this one. The phone's model judges
    /// the latest answer too, in the same reply; here that would put another
    /// model call in front of every turn from the fifth, seconds on a laptop's
    /// CPU. Asked a turn early it has had all of Ella's reply and the learner's
    /// answer to finish in, so the turn rarely waits on it. The chat still ends
    /// no sooner than the fifth answer and no later than the twelfth, and the
    /// level is read off every answer. A check that failed, or never started,
    /// keeps the chat going.
    fn placement_closes(&self, session_id: &str, answers: u32) -> bool {
        let pending = self
            .placement_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
        if answers >= progress::PLACEMENT_MAX_TURNS {
            return true;
        }
        if !self.engine.judges() {
            return answers >= progress::PLACEMENT_MIN_TURNS;
        }
        let Some(pending) = pending else {
            return false;
        };
        let readiness = match pending.verdict {
            Verdict::Running(handle) => handle.join().ok().flatten(),
            Verdict::Read(readiness) => readiness,
        };
        // Put back until this turn is saved: the next exchange's check
        // replaces it, and closing the chat clears it.
        self.placement_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                session_id.to_owned(),
                PendingCheck {
                    answers: pending.answers,
                    verdict: Verdict::Read(readiness),
                },
            );
        match readiness {
            Some(readiness) => {
                eprintln!(
                    "[placement] the check after {} answers: ready={} confidence={:?}",
                    pending.answers, readiness.ready, readiness.confidence
                );
                progress::placement_ends(answers, readiness.ready, readiness.confidence)
            }
            None => false,
        }
    }

    /// Starts the check the next answer reads: has the chat heard enough? From
    /// the fourth answer, so that the fifth can be the last, and only with a
    /// model to ask. It runs on its own thread while the reply is played.
    fn schedule_placement_check(
        &self,
        session_id: &str,
        learner_name: &str,
        age: Option<u8>,
        answers: u32,
    ) {
        if !self.engine.judges() || answers + 1 < progress::PLACEMENT_MIN_TURNS {
            return;
        }
        let Ok(session) = self.database.session(session_id) else {
            return;
        };
        let engine = Arc::clone(&self.engine);
        let name = learner_name.to_owned();
        let handle = thread::spawn(move || {
            match engine.placement_readiness(&name, age, &session.messages) {
                Ok(readiness) => readiness,
                Err(error) => {
                    eprintln!("[placement] the check after {answers} answers failed: {error}");
                    None
                }
            }
        });
        self.placement_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                session_id.to_owned(),
                PendingCheck {
                    answers,
                    verdict: Verdict::Running(handle),
                },
            );
    }

    /// What a finished talk did for the learner. Worked out the first time it
    /// is asked for and kept on the talk, so every later ask answers the same
    /// and nothing is counted twice.
    ///
    /// A placement chat reads a level off the talk and puts the learner at
    /// Step 1 of it. Once a placement has put them somewhere, another one only
    /// moves them up: a short chat on a nervous day must not take back what
    /// their talks have earned.
    ///
    /// Any other talk the learner spoke in is scored on the skills of their
    /// step and the next. Each demonstration the judge is sure of raises that
    /// skill, and a finished step moves them on: to the next step, or after
    /// the last to Step 1 of the next level. Without a model nothing is
    /// scored. A model whose answer cannot be read is an error, so the window
    /// can ask again rather than have the talk count for nothing.
    pub fn assess_session(&self, session_id: &str) -> EllaResult<Assessment> {
        let learner = self
            .database
            .signed_in_learner()?
            .ok_or_else(|| EllaError::Conflict("Tell Ella your name first.".into()))?;
        let session = self.database.session(session_id)?;
        let meta = self.database.session_curriculum(session_id)?;
        if let Some(kept) = &meta.assessment {
            return read_assessment(kept);
        }
        if session.status != "complete" {
            return Err(EllaError::Conflict("This talk is still going.".into()));
        }
        let spoke = session
            .messages
            .iter()
            .any(|message| message.speaker == Speaker::Learner);
        let kept = if meta.kind.as_deref() == Some(PLACEMENT_KIND) {
            self.assess_placement(&learner, &session)?
        } else {
            self.assess_talk(&session, meta.target_skill, spoke)?
        };
        read_assessment(&kept)
    }

    fn assess_placement(&self, learner: &Learner, session: &Session) -> EllaResult<String> {
        let answers: Vec<&str> = session
            .messages
            .iter()
            .filter(|message| message.speaker == Speaker::Learner)
            .map(|message| message.content.as_str())
            .collect();
        // A chat ended before it could close itself — Skip, or End talk on one
        // left open and picked up again from Home — places nobody unless it
        // heard as many answers as the shortest placement does. The level is
        // read before the transaction opens: the model takes a while, and the
        // database is not held while it does.
        let reading = if answers.len() as u32 >= progress::PLACEMENT_MIN_TURNS {
            let read = self.engine.place(&learner.name, &session.messages).map_err(|error| {
                eprintln!("[curriculum] placement {}: {error}", session.id);
                // Shown as it is, so it says what to do.
                EllaError::Conflict(
                    "Ella could not work out your level just now. Try again in a moment.".into(),
                )
            })?;
            // Without a model the chat still ends somewhere: where everyone
            // starts, as on the phone.
            let read = read.unwrap_or_else(|| PlacementReading {
                level: curriculum::start().level,
                closing: None,
            });
            // Very short answers are held down whatever was read off them.
            Some(PlacementReading {
                level: progress::at_most(&read.level, progress::level_ceiling(&answers)),
                ..read
            })
        } else {
            None
        };
        let closing = reading
            .as_ref()
            .and_then(|reading| reading.closing.clone())
            .unwrap_or_else(|| format!("That was lovely, {}!", first_name(&learner.name)));
        let session_id = session.id.clone();
        self.database.commit_assessment(&session.id, move |store| {
            let stored = store.position()?.and_then(known_position);
            let moved = match (&reading, &stored) {
                (None, _) => None,
                (Some(reading), None) => Some(Position::new(&reading.level, 1)),
                (Some(reading), Some(current)) => (curriculum::level_index(&reading.level)
                    > curriculum::level_index(&current.level))
                .then(|| Position::new(&reading.level, 1)),
            };
            let placed = store.placed()? || reading.is_some();
            let position = moved.clone().or(stored).unwrap_or_else(curriculum::start);
            let known = progress::by_key(&store.skill_progress(&keys_of(&progress::step_skills(&position)))?);
            let assessment = Assessment {
                session_id,
                kind: PLACEMENT_KIND.into(),
                standing: standing_at(&position, placed, &known),
                closing: Some(closing),
                advanced: None,
                skills: Vec::new(),
                scored: reading.is_some(),
                notes: None,
            };
            Ok(AssessmentWrite {
                skills: Vec::new(),
                position: moved,
                placed: reading.is_some(),
                assessment: to_json(&assessment)?,
            })
        })
    }

    fn assess_talk(&self, session: &Session, target: Option<String>, spoke: bool) -> EllaResult<String> {
        let (position, _) = self.placement()?;
        let in_play = progress::skills_in_play(&position);
        let scores = if spoke && !in_play.is_empty() {
            let skills: Vec<Scorable> = in_play
                .iter()
                .map(|placed| Scorable {
                    key: placed.key.clone(),
                    text: placed.skill.text.clone(),
                })
                .collect();
            self.engine.score(&skills, &session.messages).map_err(|error| {
                eprintln!("[curriculum] scoring {}: {error}", session.id);
                EllaError::Conflict(
                    "Ella could not look back over this talk just now. Try again in a moment.".into(),
                )
            })?
        } else {
            None
        };
        // After the scores: when the model cannot be reached at all, their
        // error is the one the window offers to retry, before any time is
        // spent on notes.
        let notes = self.notes_for(session)?;
        let today = today();
        let topic_id = session.topic_id.clone();
        let session_id = session.id.clone();
        self.database.commit_assessment(&session.id, move |store| {
            // Where the learner stands as the transaction sees it: another
            // talk may have moved them since this one's skills were picked.
            let stored = store.position()?.and_then(known_position);
            let placed = store.placed()?;
            let position = stored.unwrap_or_else(curriculum::start);
            let mut write = AssessmentWrite {
                skills: Vec::new(),
                position: None,
                placed: false,
                assessment: String::new(),
            };
            let mut advanced = None;
            let mut grown = Vec::new();
            let mut now_at = position.clone();
            if let Some(scores) = &scores {
                let mut keys = keys_of(&progress::skills_in_play(&position));
                keys.extend(scores.keys().cloned());
                keys.extend(target.clone());
                let before = store.skill_progress(&distinct(keys))?;
                let after = progress::apply_scores(&before, scores, target.as_deref(), &topic_id, &today);
                grown = progress::grown_skills(&before, &after, scores, target.as_deref());
                write.skills = after
                    .iter()
                    .zip(&before)
                    .filter(|(now, then)| now != then)
                    .map(|(now, _)| now.clone())
                    .collect();
                if progress::step_complete(&position, &progress::by_key(&after)) {
                    if let Some(next) = curriculum::next(&position) {
                        advanced = Some(if next.level == position.level {
                            Advance::Step
                        } else {
                            Advance::Level
                        });
                        now_at = next.clone();
                        write.position = Some(next);
                    }
                }
            }
            // Where they stand after it, with this talk's scores counted.
            let mut known = progress::by_key(&store.skill_progress(&keys_of(&progress::step_skills(&now_at)))?);
            for skill in &write.skills {
                known.insert(skill.skill.clone(), skill.clone());
            }
            let assessment = Assessment {
                session_id,
                kind: "talk".into(),
                standing: standing_at(&now_at, placed, &known),
                closing: None,
                advanced,
                skills: grown,
                scored: scores.is_some(),
                notes,
            };
            write.assessment = to_json(&assessment)?;
            Ok(write)
        })
    }

    /// The level map: every level, lowest first, as it stands for the learner.
    /// Those below theirs are done, theirs is current, the one above is next,
    /// and the rest are locked. A step behind them is one they finished, so its
    /// skills have passed; any other skill shows what it has been scored, since
    /// a strong learner can pass one before they reach its step.
    pub fn levels(&self) -> EllaResult<Vec<LevelView>> {
        if self.database.signed_in_learner()?.is_none() {
            return Err(EllaError::Conflict("Tell Ella your name first.".into()));
        }
        let (position, _) = self.placement()?;
        let current = curriculum::level_index(&position.level).unwrap_or(0);
        let behind =
            |index: usize, step: u8| index < current || (index == current && step < position.step);

        // One read for every skill still ahead: this step's, the steps after
        // it, and every level above.
        let mut ahead = Vec::new();
        for (index, level) in curriculum::levels().iter().enumerate() {
            for step in &level.steps {
                if !behind(index, step.number) {
                    ahead.extend(keys_of(&progress::step_skills(&Position::new(&level.code, step.number))));
                }
            }
        }
        let known = progress::by_key(&self.database.skill_progress(&ahead)?);
        let percent = progress::level_percent(&position, &known);

        let mut levels = Vec::new();
        for (index, level) in curriculum::levels().iter().enumerate() {
            let state = if index < current {
                LevelState::Done
            } else if index == current {
                LevelState::Current
            } else if index == current + 1 {
                LevelState::Next
            } else {
                LevelState::Locked
            };
            let mut steps = Vec::new();
            for step in &level.steps {
                let mut skills = Vec::new();
                for skill in &step.skills {
                    let key = curriculum::skill_key(&level.code, &skill.id);
                    let record = known
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| SkillProgress::fresh(&key));
                    skills.push(LevelSkillView {
                        label: skill.label.clone(),
                        text: skill.text.clone(),
                        passed: behind(index, step.number) || progress::passed(skill, &record),
                    });
                }
                steps.push(StepView {
                    number: step.number,
                    title: step.title.clone(),
                    focus: step.focus.clone(),
                    skills,
                });
            }
            levels.push(LevelView {
                number: (index + 1) as u8,
                name: level.name.clone(),
                goal: level.goal.clone(),
                state,
                percent: match state {
                    LevelState::Done => 100,
                    LevelState::Current => percent,
                    LevelState::Next | LevelState::Locked => 0,
                },
                steps,
            });
        }
        Ok(levels)
    }
}

/// A stored place the curriculum still has, its step kept inside its level;
/// `None` for a level it does not have.
fn known_position(position: Position) -> Option<Position> {
    let level = curriculum::level(&position.level)?;
    let last = u8::try_from(level.steps.len()).unwrap_or(u8::MAX).max(1);
    Some(Position::new(&level.code, position.step.clamp(1, last)))
}

/// Where a learner at `position` stands, as the window shows it.
fn standing_at(position: &Position, placed: bool, known: &ProgressMap) -> Standing {
    let levels = curriculum::levels();
    let index = curriculum::level_index(&position.level).unwrap_or(0);
    let level = &levels[index];
    Standing {
        level_number: (index + 1) as u8,
        level_count: levels.len() as u8,
        level_name: level.name.clone(),
        step: position.step,
        step_count: level.steps.len() as u8,
        step_title: curriculum::step(position)
            .map(|step| step.title.clone())
            .unwrap_or_default(),
        percent: progress::level_percent(position, known),
        placed,
    }
}

/// What a talk's instructions say about the skill it aims at.
fn focus_for(target: &str) -> Option<Focus> {
    curriculum::skill_at(target).map(|(_, step, skill)| Focus {
        step_title: step.title.clone(),
        step_focus: step.focus.clone(),
        skill: skill.text.clone(),
    })
}

fn keys_of(placed: &[progress::Placed]) -> Vec<String> {
    placed.iter().map(|placed| placed.key.clone()).collect()
}

/// `keys` with each one once, in the order they first came.
fn distinct(keys: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    keys.into_iter().filter(|key| seen.insert(key.clone())).collect()
}

/// Today on the laptop's own calendar, the one the learner lives by.
fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

fn to_json(assessment: &Assessment) -> EllaResult<String> {
    Ok(serde_json::to_string(assessment)?)
}

/// What the learner said in a talk, answer by answer.
fn answers_in(session: &Session) -> Vec<&str> {
    session
        .messages
        .iter()
        .filter(|message| message.speaker == Speaker::Learner)
        .map(|message| message.content.as_str())
        .collect()
}

fn read_assessment(kept: &str) -> EllaResult<Assessment> {
    Ok(serde_json::from_str(kept)?)
}

/// What the name step accepts: two to forty characters once the spaces
/// around them are trimmed, as it always has.
fn clean_name(name: &str) -> EllaResult<&str> {
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
    Ok(clean)
}

/// `#RRGGBB` and nothing else: the window draws the avatar with the stored
/// value as a CSS colour, so anything looser would be handed straight to it.
fn is_hex_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|character| character.is_ascii_hexdigit())
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
    // The streaming path otherwise never gets the batch turn's silence trim: a
    // learner who waits before speaking has that whole wait sent to STT in the
    // same untrimmed buffer as the words that came after it, which is exactly
    // the shape of input CanaryStt::transcribe's own doc comment already
    // warns is unreliable. Trimming here gives every chunk - background or
    // tail - what the batch path already gave a whole recording, and a chunk
    // the VAD finds nothing in never has to wait on an engine to say so.
    let vad = trim_to_speech(&samples, sample_rate).unwrap_or(VadOutput {
        samples,
        input_ms: audio_ms,
        speech_ms: audio_ms,
        speech_detected: true,
    });
    if !vad.speech_detected {
        eprintln!(
            "[LATENCY]     stt-stream> chunk {index} is silence (~{audio_ms:.0} ms) - skipping STT"
        );
        return StreamChunkOutcome {
            text: String::new(),
            engine: "silence".into(),
            fell_back: false,
            error: Some("no speech detected by the energy VAD".into()),
        };
    }
    let started = Instant::now();
    let (text, engine_name, fell_back, error) = match engine.transcribe(&vad.samples, sample_rate) {
        Ok(transcription) => {
            let fell_back = transcription.fallback_from.is_some();
            (transcription.text, transcription.engine, fell_back, None)
        }
        Err(error) => {
            eprintln!("[LATENCY]     stt-stream> chunk {index} produced no words ({error})");
            (String::new(), "none".into(), true, Some(error.to_string()))
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    eprintln!(
        "[LATENCY]     stt-stream> chunk {index} done in {elapsed_ms:.1}ms (~{audio_ms:.0} ms audio, {:.0} ms after trim, engine={engine_name}): \"{text}\"",
        vad.speech_ms
    );
    StreamChunkOutcome {
        text,
        engine: engine_name,
        fell_back,
        error,
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
        engines::{DemoEngine, EnginePaths, LocalEngine},
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
    fn full_demo_turn_is_persisted() {
        let service = service();
        service.save_learner("Asha", Some(14)).unwrap();
        let session = service.start_session("street-food").unwrap();
        let result = service
            .send_text_turn(&session.id, "I played football with my best friend")
            .unwrap();

        assert_eq!(result.learner_message.turn, 1);
        let saved = service.get_session(&session.id).unwrap();
        assert_eq!(saved.messages.len(), 3);
        assert_eq!(saved.messages[1].speaker, Speaker::Learner);
        assert_eq!(saved.messages[2].speaker, Speaker::Ella);
    }

    #[test]
    fn a_free_conversation_closes_itself_on_its_last_turn() {
        // The cab transcript wished the learner well on turn 6 and again on
        // turn 7, because `free_closing_note` said "this is your last reply"
        // from turn 6 onward and nothing ever ended the session.
        let service = service();
        service.save_learner("Souvik", Some(16)).unwrap();
        let session = service.start_session("booking-a-cab").unwrap();
        for turn in 1..FREE_TOPIC_TURNS {
            let result = service.send_text_turn(&session.id, "I need to go to the station").unwrap();
            assert!(
                result.session_summary.is_none(),
                "turn {turn} is not the end of the conversation"
            );
        }
        let last = service
            .send_text_turn(&session.id, "Thank you, see you tomorrow")
            .unwrap();
        let summary = last
            .session_summary
            .expect("the last turn has to hand back a summary");
        assert_eq!(summary.turns, FREE_TOPIC_TURNS);
        assert_eq!(service.get_session(&session.id).unwrap().status, "complete");
        // And no seventh goodbye: the session is closed, so there is no turn
        // left to say one on.
        assert!(
            service.send_text_turn(&session.id, "Hello again").is_err(),
            "a closed conversation cannot be spoken to"
        );
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
        assert!(!summary.headline.is_empty());
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

    /// One answered talk, finished. Returns the session id.
    fn answered_talk(service: &AppService, topic_id: &str) -> String {
        let session = service.start_session(topic_id).unwrap();
        service
            .send_text_turn(&session.id, "I ate poha this morning")
            .unwrap();
        service.complete_session(&session.id).unwrap();
        session.id
    }

    fn profile(name: &str, avatar_color: Option<&str>) -> Option<LearnerProfile> {
        Some(LearnerProfile {
            name: name.into(),
            avatar_color: avatar_color.map(Into::into),
        })
    }

    #[test]
    fn log_out_keeps_everything_and_log_in_brings_it_back() {
        let service = service();
        service.save_learner("Asha", Some(14)).unwrap();
        let asha = service.save_avatar_color("#FF8800").unwrap();
        let talk = answered_talk(&service, "street-food");
        let chore = service.start_chore("market-cloth-price").unwrap();
        let signed_in = service.bootstrap().unwrap();
        assert_eq!(signed_in.learner.as_ref(), Some(&asha));
        assert_eq!(signed_in.saved_learner, profile("Asha", Some("#FF8800")));
        assert_eq!(signed_in.recent_sessions.len(), 2);
        assert_eq!(signed_in.progress.talks_finished, 1);

        let signed_out = service.log_out().unwrap();
        assert_eq!(signed_out.learner, None);
        assert!(signed_out.recent_sessions.is_empty());
        assert_eq!(signed_out.progress, LearnerProgress::default());
        assert_eq!(
            signed_out.saved_learner,
            profile("Asha", Some("#FF8800")),
            "the welcome-back step still knows who to greet"
        );
        assert_eq!(signed_out.topics, topics_for_age(None));
        assert_eq!(service.bootstrap().unwrap(), signed_out, "and so does the next launch");
        // Logging out deleted nothing.
        assert_eq!(service.get_session(&talk).unwrap().messages.len(), 3);
        assert_eq!(service.get_session(&chore.id).unwrap().messages.len(), 1);

        // Signed out, nothing can be said or changed on the learner's behalf.
        let refused = service.start_session("street-food").unwrap_err();
        assert!(matches!(refused, EllaError::Conflict(_)));
        assert_eq!(
            refused.to_string(),
            "Tell Ella your name before starting a conversation."
        );
        assert!(matches!(
            service.start_chore("market-cloth-price"),
            Err(EllaError::Conflict(_))
        ));
        assert!(matches!(
            service.send_text_turn(&chore.id, "Four hundred?"),
            Err(EllaError::Conflict(_))
        ));
        assert_eq!(
            service.save_avatar_color("#000000").unwrap_err().to_string(),
            "Tell Ella your name first."
        );
        assert_eq!(service.get_session(&chore.id).unwrap().messages.len(), 1);

        assert_eq!(service.log_in().unwrap(), asha, "the same learner, unchanged");
        assert_eq!(service.bootstrap().unwrap(), signed_in, "with everything they had");
        // Logging in twice is harmless.
        assert_eq!(service.log_in().unwrap(), asha);
        assert!(service.send_text_turn(&chore.id, "Four hundred?").is_ok());
    }

    #[test]
    fn lets_start_after_a_log_out_is_the_same_learner_again() {
        let service = service();
        let asha = service.save_learner("Asha", Some(14)).unwrap();
        service.save_avatar_color("#FF8800").unwrap();
        answered_talk(&service, "street-food");
        answered_talk(&service, "booking-a-cab");
        let before = service.bootstrap().unwrap();
        service.log_out().unwrap();

        // The five onboarding steps again, with a longer name this time.
        let again = service.save_learner("  Asha Rao ", None).unwrap();
        assert_eq!(again.name, "Asha Rao");
        assert_eq!(again.age, Some(14), "no age given keeps the one we know");
        assert_eq!(again.created_at, asha.created_at);
        assert_eq!(again.avatar_color.as_deref(), Some("#FF8800"));
        let snapshot = service.bootstrap().unwrap();
        assert_eq!(snapshot.learner, Some(again.clone()), "signed straight in");
        assert_eq!(snapshot.saved_learner, profile("Asha Rao", Some("#FF8800")));
        assert_eq!(snapshot.recent_sessions, before.recent_sessions);
        assert_eq!(snapshot.progress, before.progress);
        assert_eq!(snapshot.progress.talks_finished, 2);

        // A new age is taken, and the topics follow it.
        let younger = service.save_learner("Asha Rao", Some(8)).unwrap();
        assert_eq!(younger.age, Some(8));
        assert_eq!(service.bootstrap().unwrap().topics, topics_for_age(Some(8)));
    }

    #[test]
    fn log_in_on_a_laptop_with_nobody_saved_asks_for_a_name() {
        let service = service();
        let fresh = service.bootstrap().unwrap();
        assert_eq!((fresh.learner, fresh.saved_learner), (None, None));

        let refused = service.log_in().unwrap_err();
        assert!(matches!(refused, EllaError::Conflict(_)));
        assert_eq!(refused.to_string(), "Tell Ella your name first.");
        let after = service.bootstrap().unwrap();
        assert_eq!((after.learner, after.saved_learner), (None, None), "nobody made up");

        // Log out with nobody saved is harmless too.
        assert_eq!(service.log_out().unwrap().saved_learner, None);

        // The returning-mode name step saves them and goes straight in.
        let kabir = service.save_learner("Kabir", None).unwrap();
        assert_eq!(service.bootstrap().unwrap().learner, Some(kabir));
    }

    #[test]
    fn the_name_step_checks_what_it_is_given() {
        let service = service();
        for (name, age) in [
            (" A ", Some(14)),
            (&*"a".repeat(41), Some(14)),
            ("Asha", Some(2)),
            ("Asha", Some(121)),
        ] {
            assert!(
                matches!(service.save_learner(name, age), Err(EllaError::Validation(_))),
                "{name:?} aged {age:?} must be refused"
            );
        }
        assert_eq!(service.bootstrap().unwrap().saved_learner, None);
        assert_eq!(service.save_learner(&"a".repeat(40), Some(3)).unwrap().age, Some(3));
        assert_eq!(service.save_learner("Al", Some(120)).unwrap().name, "Al");
    }

    #[test]
    fn progress_and_the_summary_count_every_finished_talk() {
        let service = service();
        service.save_learner("Kabir", Some(16)).unwrap();
        for talk in 1..=7 {
            let session = service.start_session("street-food").unwrap();
            service
                .send_text_turn(&session.id, "I love dosa because it is crispy")
                .unwrap();
            let summary = service.complete_session(&session.id).unwrap();
            let expected = format!(
                "That is {talk} conversation{} finished. Come back and talk to Ella again.",
                if talk == 1 { "" } else { "s" }
            );
            assert_eq!(summary.encouragement, expected);
        }
        let snapshot = service.bootstrap().unwrap();
        assert_eq!(snapshot.recent_sessions.len(), 5, "the home list stays short");
        assert_eq!(snapshot.progress.talks_finished, 7);
        assert_eq!(snapshot.progress.answers, 7);
        // Midnight-proof: however the talks fell across days, each is counted
        // on exactly one of them.
        assert_eq!(
            snapshot.progress.days.iter().map(|day| day.talks).sum::<u32>(),
            7
        );

        // A talk with no answers finishes nothing and says so.
        let silent = service.start_session("restaurant-order").unwrap();
        let summary = service.complete_session(&silent.id).unwrap();
        assert_eq!(summary.turns, 0);
        assert_eq!(
            summary.encouragement,
            "Say a few words next time and it counts. Come back and talk to Ella again."
        );
        let after = service.bootstrap().unwrap().progress;
        assert_eq!(after, snapshot.progress);
        assert!(!after.finished_topics.contains(&"restaurant-order".to_string()));
    }

    #[test]
    fn the_avatar_colour_is_checked_and_kept_with_the_learner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let service = AppService::new(Database::open(&path).unwrap(), Box::new(DemoEngine));

        let signed_out = service.save_avatar_color("#7C5CFF").unwrap_err();
        assert!(matches!(signed_out, EllaError::Conflict(_)));
        assert_eq!(signed_out.to_string(), "Tell Ella your name first.");
        // Signed out comes first, even for a colour that would be refused.
        assert!(matches!(service.save_avatar_color("red"), Err(EllaError::Conflict(_))));

        let asha = service.save_learner("Asha", Some(14)).unwrap();
        assert_eq!(asha.avatar_color, None);
        for bad in [
            "7C5CFF", "#7C5CF", "#7C5CFFF", "#7C5CFG", "red", "", "#é1234", " #7C5CFF",
        ] {
            let refused = service.save_avatar_color(bad).unwrap_err();
            assert!(matches!(refused, EllaError::Validation(_)), "{bad:?} must be refused");
            assert_eq!(
                refused.to_string(),
                "Please choose one of the colours on the profile screen."
            );
        }
        assert_eq!(service.bootstrap().unwrap().learner.unwrap().avatar_color, None);
        let saved = service.save_avatar_color("#7c5CfF").unwrap();
        assert_eq!(
            saved,
            Learner {
                avatar_color: Some("#7c5CfF".into()),
                ..asha
            }
        );
        drop(service);

        // The next launch, from the same file.
        let service = AppService::new(Database::open(&path).unwrap(), Box::new(DemoEngine));
        let snapshot = service.bootstrap().unwrap();
        assert_eq!(snapshot.learner, Some(saved.clone()));
        assert_eq!(snapshot.saved_learner, profile("Asha", Some("#7c5CfF")));
        service.log_out().unwrap();
        drop(service);

        // Logged out, relaunched and logged back in, the colour is still theirs.
        let service = AppService::new(Database::open(&path).unwrap(), Box::new(DemoEngine));
        assert_eq!(
            service.bootstrap().unwrap().saved_learner,
            profile("Asha", Some("#7c5CfF"))
        );
        assert_eq!(service.log_in().unwrap(), saved);
    }

    #[test]
    fn the_snapshot_uses_the_field_names_the_window_reads() {
        // The TypeScript types mirror these names exactly; a rename here
        // would leave the window reading `undefined` without a compile error.
        fn keys(value: &serde_json::Value) -> Vec<&str> {
            let mut keys = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            keys.sort();
            keys
        }
        let snapshot_keys = vec![
            "engine_status",
            "learner",
            "progress",
            "recent_sessions",
            "saved_learner",
            "standing",
            "topics",
        ];
        let service = service();
        let fresh = serde_json::to_value(service.bootstrap().unwrap()).unwrap();
        assert_eq!(keys(&fresh), snapshot_keys, "no `learners` any more");
        assert_eq!(fresh["learner"], serde_json::Value::Null);
        assert_eq!(fresh["saved_learner"], serde_json::Value::Null);
        assert_eq!(fresh["standing"], serde_json::Value::Null);
        assert_eq!(
            fresh["progress"],
            serde_json::json!({
                "days": [],
                "talks_finished": 0,
                "answers": 0,
                "finished_topics": [],
                "chores_met": [],
            })
        );

        service.save_learner("Asha", Some(14)).unwrap();
        service.save_avatar_color("#7C5CFF").unwrap();
        answered_talk(&service, "street-food");
        let snapshot = serde_json::to_value(service.bootstrap().unwrap()).unwrap();
        assert_eq!(keys(&snapshot), snapshot_keys);
        assert_eq!(
            keys(&snapshot["learner"]),
            vec!["age", "avatar_color", "created_at", "level_name", "name"],
            "no `id`: there is only ever one learner"
        );
        assert_eq!(snapshot["learner"]["name"], "Asha");
        assert_eq!(snapshot["learner"]["age"], 14);
        assert_eq!(snapshot["learner"]["avatar_color"], "#7C5CFF");
        assert_eq!(
            snapshot["saved_learner"],
            serde_json::json!({ "name": "Asha", "avatar_color": "#7C5CFF" })
        );
        let day = &snapshot["progress"]["days"][0];
        assert_eq!(day["day"].as_str().unwrap().len(), "YYYY-MM-DD".len());
        assert_eq!(day["talks"], 1);
        assert_eq!(day["answers"], 1);
        assert_eq!(snapshot["progress"]["talks_finished"], 1);
        assert_eq!(snapshot["progress"]["answers"], 1);
        assert_eq!(snapshot["progress"]["finished_topics"], serde_json::json!(["street-food"]));
        assert!(snapshot["recent_sessions"].is_array());
        assert!(snapshot["topics"].is_array());
        assert!(snapshot["engine_status"].is_object());
        assert_eq!(
            snapshot["standing"],
            serde_json::json!({
                "level_number": 3,
                "level_count": 6,
                "level_name": "Finding My Voice",
                "step": 1,
                "step_count": 5,
                "step_title": curriculum::step(&curriculum::start()).unwrap().title,
                "percent": 0,
                "placed": false,
            }),
            "no CEFR code: the window shows a level's name and number"
        );

        // Signed out: the saved learner, and nothing of their history.
        let signed_out = serde_json::to_value(service.log_out().unwrap()).unwrap();
        assert_eq!(keys(&signed_out), snapshot_keys);
        assert_eq!(signed_out["learner"], serde_json::Value::Null);
        assert_eq!(signed_out["saved_learner"], snapshot["saved_learner"]);
        assert_eq!(signed_out["recent_sessions"], serde_json::json!([]));
        assert_eq!(signed_out["progress"], fresh["progress"]);

        let learner = serde_json::to_value(service.log_in().unwrap()).unwrap();
        assert_eq!(learner, snapshot["learner"]);
    }

    #[test]
    fn shutdown_can_run_twice() {
        let service = service();
        service.save_learner("Asha", Some(14)).unwrap();
        service.shutdown();
        service.shutdown();
        assert_eq!(service.bootstrap().unwrap().learner.unwrap().name, "Asha");
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
        let engine = LocalEngine::from_environment(EnginePaths::default());
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
        let engine = LocalEngine::from_environment(EnginePaths::default());
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

/// The placement chat and the curriculum, end to end through the service, with
/// a scripted model standing in for llama.cpp wherever one is asked to judge.
#[cfg(test)]
mod curriculum_flow_tests {
    use super::*;
    use crate::{
        domain::{Confidence, EngineStatus},
        infrastructure::{
            database::Database,
            engines::{DemoEngine, SynthesizedAudio},
            stt::Transcription,
        },
    };

    /// What the scripted model has been asked, for the tests to read back.
    #[derive(Default)]
    struct Heard {
        pitches: Vec<Pitch>,
        requests: Vec<TutorRequest>,
        checks: Vec<usize>,
        scored: Vec<Vec<String>>,
        /// The answers each correction was asked for.
        corrected: Vec<Vec<String>>,
    }

    /// A model that always answers the same: ready or not, one level, and the
    /// same confidence for every skill it is shown.
    struct Judge {
        heard: Arc<Mutex<Heard>>,
        readiness: Readiness,
        reading: PlacementReading,
        confidence: f64,
        /// Set to make the next reply fail, as a model that falls over does.
        fail_next_reply: Arc<std::sync::atomic::AtomicBool>,
        /// Whether a correction comes back unreadable.
        correct_fails: bool,
    }

    impl Judge {
        fn new(heard: &Arc<Mutex<Heard>>) -> Self {
            Self {
                heard: Arc::clone(heard),
                readiness: Readiness { ready: false, confidence: Confidence::Low },
                reading: PlacementReading { level: "B1".into(), closing: Some("Lovely to meet you, Asha!".into()) },
                confidence: 0.95,
                fail_next_reply: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                correct_fails: false,
            }
        }
    }

    impl TutorEngine for Judge {
        fn status(&self) -> EngineStatus {
            DemoEngine.status()
        }
        fn opening(&self, topic: &Topic, learner_name: &str, pitch: &Pitch) -> EllaResult<String> {
            self.heard.lock().unwrap().pitches.push(pitch.clone());
            Ok(format!("Hi {learner_name}, let us talk about {}.", topic.label))
        }
        fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply> {
            if self.fail_next_reply.swap(false, std::sync::atomic::Ordering::SeqCst) {
                return Err(EllaError::Engine("the model fell over".into()));
            }
            self.heard.lock().unwrap().requests.push(request.clone());
            let text = match &request.placement {
                Some(brief) if brief.closing => "Thank you, I loved hearing about your family.",
                _ => "That sounds lovely. What else do you enjoy?",
            };
            Ok(GeneratedReply::plain(text.into(), 1.0, 1.0))
        }
        fn judges(&self) -> bool {
            true
        }
        fn placement_readiness(&self, _: &str, _: Option<u8>, messages: &[Message]) -> EllaResult<Option<Readiness>> {
            let answers = messages.iter().filter(|message| message.speaker == Speaker::Learner).count();
            self.heard.lock().unwrap().checks.push(answers);
            Ok(Some(self.readiness))
        }
        fn place(&self, _: &str, _: &[Message]) -> EllaResult<Option<PlacementReading>> {
            Ok(Some(self.reading.clone()))
        }
        fn score(&self, skills: &[Scorable], _: &[Message]) -> EllaResult<Option<HashMap<String, f64>>> {
            self.heard.lock().unwrap().scored.push(skills.iter().map(|skill| skill.key.clone()).collect());
            Ok(Some(skills.iter().map(|skill| (skill.key.clone(), self.confidence)).collect()))
        }
        /// Knows one mistake, and corrects every "it have".
        fn correct(&self, answers: &[&str]) -> EllaResult<Option<Vec<String>>> {
            self.heard.lock().unwrap().corrected.push(answers.iter().map(|answer| (*answer).to_owned()).collect());
            if self.correct_fails {
                return Err(EllaError::Engine("The model could not correct the answers: an answer that could not be read".into()));
            }
            Ok(Some(answers.iter().map(|answer| answer.replace("it have", "it has")).collect()))
        }
        fn uses_native_stt(&self) -> bool {
            false
        }
        fn transcribe(&self, _: &[i16], _: u32) -> EllaResult<Transcription> {
            Err(EllaError::Engine("no microphone in tests".into()))
        }
        fn synthesize(&self, _: &str) -> EllaResult<SynthesizedAudio> {
            DemoEngine.synthesize("")
        }
    }

    fn judged(judge: Judge) -> AppService {
        let service = AppService::new(Database::in_memory().unwrap(), Box::new(judge));
        service.save_learner("Asha", Some(14)).unwrap();
        service
    }

    fn demo() -> AppService {
        let service = AppService::new(Database::in_memory().unwrap(), Box::new(DemoEngine));
        service.save_learner("Asha", Some(14)).unwrap();
        service
    }

    /// Answers until the chat closes itself, and says how many that took.
    fn answer_placement(service: &AppService, session_id: &str) -> (u32, TurnResult) {
        for answer in 1..=progress::PLACEMENT_MAX_TURNS + 1 {
            let result = service
                .send_text_turn(session_id, "I live with my mother and two sisters in Pune")
                .unwrap();
            if result.session_summary.is_some() {
                return (answer, result);
            }
        }
        panic!("the placement chat never closed");
    }

    /// Puts the learner somewhere on the ladder, the way an assessment does.
    fn place_at(service: &AppService, level: &str, step: u8) {
        let session = service.start_placement().unwrap();
        service.complete_session(&session.id).unwrap();
        let position = Position::new(level, step);
        service
            .database
            .commit_assessment(&session.id, |_| {
                Ok(AssessmentWrite {
                    skills: Vec::new(),
                    position: Some(position),
                    placed: true,
                    assessment: "null".into(),
                })
            })
            .unwrap();
    }

    fn talk_on(service: &AppService, topic_id: &str) -> Assessment {
        let session = service.start_session(topic_id).unwrap();
        service
            .send_text_turn(&session.id, "Yesterday I walked to the market and bought red apples")
            .unwrap();
        service.complete_session(&session.id).unwrap();
        service.assess_session(&session.id).unwrap()
    }

    /// A finished talk of these answers, and its summary.
    fn talk_of(service: &AppService, answers: &[&str]) -> (String, SessionSummary) {
        let session = service.start_session("street-food").unwrap();
        for answer in answers {
            service.send_text_turn(&session.id, answer).unwrap();
        }
        let summary = service.complete_session(&session.id).unwrap();
        (session.id, summary)
    }

    const SCHOOL_TALK: [&str; 3] = [
        "My school is very big and it have a big playground",
        "We play football there because it is fun",
        "yes",
    ];

    #[test]
    fn a_talk_of_a_few_words_is_short_and_has_no_notes() {
        let heard = Arc::default();
        let service = judged(Judge::new(&heard));
        let (id, summary) = talk_of(&service, &["I ate poha", "It was good", "yes"]);
        assert!(summary.short, "three answers but only seven words");
        assert_eq!(summary.chore, None);
        assert_eq!(service.assess_session(&id).unwrap().notes, None);
        assert!(heard.lock().unwrap().corrected.is_empty(), "nothing was sent to be corrected");
    }

    #[test]
    fn a_longer_talk_has_notes_and_one_fix_in_the_learners_own_words() {
        let heard = Arc::default();
        let service = judged(Judge::new(&heard));
        let (id, summary) = talk_of(&service, &SCHOOL_TALK);
        assert!(!summary.short);
        let assessment = service.assess_session(&id).unwrap();
        assert_eq!(
            assessment.notes,
            Some(TalkNotes {
                went_well: vec!["Gave reasons".into(), "Full sentences".into()],
                fix: Some(crate::domain::Fix {
                    said: "It have a big playground".into(),
                    better: "It has a big playground".into(),
                }),
                checked: true,
            })
        );
        // Only the answers long enough to have a mistake in were sent.
        assert_eq!(heard.lock().unwrap().corrected, vec![SCHOOL_TALK[..2].iter().map(|answer| (*answer).to_owned()).collect::<Vec<_>>()]);
        // Kept with the rest of the assessment: asking again asks nothing.
        assert_eq!(service.assess_session(&id).unwrap(), assessment);
        assert_eq!(heard.lock().unwrap().corrected.len(), 1);
        assert!(service.speak_fix(&id).is_ok());
    }

    #[test]
    fn a_correction_that_cannot_be_read_still_counts_the_talk_and_claims_no_fix() {
        let heard = Arc::default();
        let service = judged(Judge { correct_fails: true, ..Judge::new(&heard) });
        let (id, _) = talk_of(&service, &SCHOOL_TALK);
        let assessment = service.assess_session(&id).unwrap();
        assert!(assessment.scored);
        let notes = assessment.notes.unwrap();
        assert_eq!(notes.went_well.len(), 2);
        assert_eq!((notes.fix, notes.checked), (None, false), "not the same as nothing to fix");
        assert!(matches!(service.speak_fix(&id), Err(EllaError::NotFound(_))));
    }

    #[test]
    fn without_a_model_the_notes_still_say_what_went_well() {
        let service = demo();
        let (id, _) = talk_of(&service, &SCHOOL_TALK);
        let notes = service.assess_session(&id).unwrap().notes.unwrap();
        assert_eq!(notes.went_well, vec!["Gave reasons", "Full sentences"]);
        assert_eq!((notes.fix, notes.checked), (None, false));
    }

    #[test]
    fn an_assessment_kept_before_notes_existed_reads_without_them() {
        let kept = r#"{"session_id":"s","kind":"talk","standing":{"level_number":3,"level_count":6,
            "level_name":"Finding My Voice","step":1,"step_count":5,"step_title":"Me","percent":0,
            "placed":false},"closing":null,"advanced":null,"skills":[],"scored":true}"#;
        assert_eq!(read_assessment(kept).unwrap().notes, None);
    }

    #[test]
    fn a_chore_recap_says_how_the_ledger_ended_and_counts_every_goal_met() {
        let service = demo();
        let finish = |figure: i32, agreed: bool| {
            let chore = service.start_chore("market-cloth-price").unwrap();
            service.send_text_turn(&chore.id, "I will give you 380 for it").unwrap();
            service.database.save_ledger_state(&chore.id, figure, agreed, &now()).unwrap();
            service.complete_session(&chore.id).unwrap().chore.unwrap()
        };

        let met = finish(380, true);
        assert_eq!(met.chore_id, "market-cloth-price");
        assert_eq!(met.character_id, "stall-owner");
        assert_eq!((met.unit.as_str(), met.direction, met.target), ("Rs", crate::domain::Direction::Down, 400));
        assert_eq!((met.figure, met.agreed, met.met, met.times_met), (380, true, true, 1));
        assert!(met.last_line.is_some_and(|line| !line.is_empty()), "the stall owner's last word");
        assert_eq!(service.bootstrap().unwrap().progress.chores_met, vec!["market-cloth-price"]);

        // Agreed, but above the target; then at it, but never agreed.
        let dear = finish(450, true);
        assert_eq!((dear.met, dear.times_met), (false, 1));
        let unagreed = finish(400, false);
        assert_eq!((unagreed.met, unagreed.times_met), (false, 1));

        let again = finish(400, true);
        assert_eq!((again.met, again.times_met), (true, 2));

        // A free talk and a chore judged on a rubric have no ledger to tell.
        assert_eq!(talk_of(&service, &SCHOOL_TALK).1.chore, None);
        let pen = service.start_chore("sell-me-a-pen").unwrap();
        service.send_text_turn(&pen.id, "This pen never runs out of ink").unwrap();
        assert_eq!(service.complete_session(&pen.id).unwrap().chore, None);
    }

    #[test]
    fn a_new_learner_stands_at_step_1_of_a2_until_a_placement_says_otherwise() {
        let standing = demo().bootstrap().unwrap().standing.unwrap();
        assert_eq!((standing.level_number, standing.level_count), (3, 6));
        assert_eq!(standing.level_name, "Finding My Voice");
        assert_eq!((standing.step, standing.step_count, standing.percent), (1, 5, 0));
        assert!(!standing.placed);
    }

    #[test]
    fn without_a_model_the_placement_runs_five_answers_and_places_where_everyone_starts() {
        let service = demo();
        let session = service.start_placement().unwrap();
        assert_eq!(session.topic_label, "First talk");
        assert_eq!(session.messages[0].content, "So Asha, tell me about your day so far!");

        let (answers, last) = answer_placement(&service, &session.id);
        assert_eq!(answers, progress::PLACEMENT_MIN_TURNS, "the shortest chat allowed");
        assert!(!last.ella_message.content.ends_with('?'), "a goodbye, not a question");
        assert!(last.suggested_complete);
        assert_eq!(service.get_session(&session.id).unwrap().status, "complete");

        let assessment = service.assess_session(&session.id).unwrap();
        assert_eq!(assessment.kind, "placement");
        assert_eq!(assessment.closing.as_deref(), Some("That was lovely, Asha!"));
        assert_eq!(assessment.standing.level_name, "Finding My Voice");
        assert!(assessment.standing.placed, "the chat happened, so the learner is placed");
        assert_eq!(service.bootstrap().unwrap().standing.unwrap(), assessment.standing);
        assert_eq!(service.assess_session(&session.id).unwrap(), assessment, "asked again, answered the same");
    }

    #[test]
    fn a_judged_placement_ends_when_the_judge_is_sure_and_starts_the_learner_at_its_level() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge {
            readiness: Readiness { ready: true, confidence: Confidence::High },
            ..Judge::new(&heard)
        });
        let session = service.start_placement().unwrap();
        let (answers, _) = answer_placement(&service, &session.id);
        assert_eq!(answers, progress::PLACEMENT_MIN_TURNS);
        assert_eq!(heard.lock().unwrap().checks, [4], "checked from the fourth answer, read by the fifth");
        let turns = heard.lock().unwrap().requests.clone();
        assert!(turns.iter().all(|request| request.placement.is_some() && request.chore.is_none()));
        assert_eq!(
            turns.iter().map(|request| request.placement.as_ref().unwrap().closing).collect::<Vec<_>>(),
            [false, false, false, false, true]
        );

        let assessment = service.assess_session(&session.id).unwrap();
        assert_eq!(assessment.closing.as_deref(), Some("Lovely to meet you, Asha!"));
        assert_eq!(assessment.standing.level_name, "Speaking Freely");
        assert_eq!((assessment.standing.level_number, assessment.standing.step), (4, 1));
        assert!(assessment.standing.placed);
        // Every talk from here is pitched at the level the placement found.
        service.start_session("street-food").unwrap();
        assert_eq!(heard.lock().unwrap().pitches.last().unwrap().level, "B1");
    }

    #[test]
    fn a_judge_that_is_never_sure_lets_the_chat_run_to_twelve_answers() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        let session = service.start_placement().unwrap();
        let (answers, _) = answer_placement(&service, &session.id);
        assert_eq!(answers, progress::PLACEMENT_MAX_TURNS);
        assert_eq!(heard.lock().unwrap().checks, (4..12).collect::<Vec<_>>());
    }

    #[test]
    fn a_fairly_sure_judge_ends_the_chat_only_from_the_ninth_answer() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge {
            readiness: Readiness { ready: true, confidence: Confidence::Medium },
            ..Judge::new(&heard)
        });
        let session = service.start_placement().unwrap();
        assert_eq!(answer_placement(&service, &session.id).0, 9);
    }

    #[test]
    fn once_placed_a_placement_only_ever_moves_the_learner_up() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let lower = judged(Judge {
            reading: PlacementReading { level: "A1".into(), closing: None },
            ..Judge::new(&heard)
        });
        place_at(&lower, "B1", 3);
        let session = lower.start_placement().unwrap();
        answer_placement(&lower, &session.id);
        let kept = lower.assess_session(&session.id).unwrap().standing;
        assert_eq!((kept.level_name.as_str(), kept.step), ("Speaking Freely", 3), "a lower reading takes nothing back");

        let higher = judged(Judge {
            reading: PlacementReading { level: "B2".into(), closing: None },
            ..Judge::new(&heard)
        });
        place_at(&higher, "B1", 3);
        let session = higher.start_placement().unwrap();
        answer_placement(&higher, &session.id);
        let moved = higher.assess_session(&session.id).unwrap().standing;
        assert_eq!((moved.level_name.as_str(), moved.step), ("Almost Fluent", 1));
    }

    #[test]
    fn a_placement_of_one_word_answers_is_held_down_whatever_the_judge_reads() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge {
            readiness: Readiness { ready: true, confidence: Confidence::High },
            ..Judge::new(&heard)
        });
        let session = service.start_placement().unwrap();
        for answer in ["college", "yes", "mother father", "cricket", "good"] {
            service.send_text_turn(&session.id, answer).unwrap();
        }
        let placed = service.assess_session(&session.id).unwrap();
        assert_eq!(
            (placed.standing.level_number, placed.standing.level_name.as_str()),
            (1, "Pre-Beginner"),
            "the judge said B1; a word or two an answer is where everyone begins"
        );
    }

    #[test]
    fn a_placement_ended_before_it_heard_five_answers_places_nobody() {
        // Left open, picked up again from Home, and ended with End talk after
        // two answers: however sure a judge would be, that is not a placement.
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge {
            readiness: Readiness { ready: true, confidence: Confidence::High },
            ..Judge::new(&heard)
        });
        let session = service.start_placement().unwrap();
        service.send_text_turn(&session.id, "yes").unwrap();
        service.send_text_turn(&session.id, "I like cricket").unwrap();
        service.complete_session(&session.id).unwrap();
        let assessment = service.assess_session(&session.id).unwrap();
        assert!(!assessment.scored);
        assert!(!assessment.standing.placed);
        assert_eq!(assessment.standing.level_name, "Finding My Voice");
        assert!(!service.bootstrap().unwrap().standing.unwrap().placed);
    }

    #[test]
    fn a_reply_that_fails_keeps_the_verdict_for_the_answer_tried_again() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let judge = Judge {
            readiness: Readiness { ready: true, confidence: Confidence::High },
            ..Judge::new(&heard)
        };
        let fail = Arc::clone(&judge.fail_next_reply);
        let service = judged(judge);
        let session = service.start_placement().unwrap();
        for _ in 0..4 {
            let turn = service.send_text_turn(&session.id, "I go to college in Pune every day").unwrap();
            assert!(turn.session_summary.is_none());
        }
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(service.send_text_turn(&session.id, "My family runs a small shop").is_err());
        let retried = service.send_text_turn(&session.id, "My family runs a small shop").unwrap();
        assert!(retried.session_summary.is_some(), "the fifth answer is still the last");
        assert_eq!(heard.lock().unwrap().checks, [4], "and the check was not asked twice");
    }

    #[test]
    fn skipping_the_placement_places_nobody() {
        let service = demo();
        let session = service.start_placement().unwrap();
        service.send_text_turn(&session.id, "I am fine").unwrap();
        service.complete_session(&session.id).unwrap();
        assert!(!service.bootstrap().unwrap().standing.unwrap().placed);
    }

    #[test]
    fn every_talk_quietly_aims_at_a_skill_of_the_learners_step() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        place_at(&service, "A1", 2);
        let session = service.start_session("street-food").unwrap();
        let target = service.database.session_curriculum(&session.id).unwrap().target_skill.unwrap();
        assert!(target.starts_with("A1:U2-"), "{target}");

        let pitch = heard.lock().unwrap().pitches.last().unwrap().clone();
        assert_eq!(pitch.level, "A1");
        let focus = pitch.focus.expect("the opening's prompt carries the aim");
        assert_eq!(focus.step_title, "Talking about the past");
        assert_eq!(focus.skill, curriculum::skill_at(&target).unwrap().2.text);

        service.send_text_turn(&session.id, "I watched a film").unwrap();
        let turn = heard.lock().unwrap().requests.last().unwrap().clone();
        assert_eq!(turn.pitch, Pitch { level: "A1".into(), focus: Some(focus) }, "every turn says the same");
        assert!(turn.placement.is_none());

        // The next talk aims somewhere else: the last aim is held back.
        let next = service.start_session("booking-a-cab").unwrap();
        let next_target = service.database.session_curriculum(&next.id).unwrap().target_skill.unwrap();
        assert_ne!(next_target, target);
    }

    #[test]
    fn talks_that_show_every_skill_finish_the_step_and_move_the_learner_on() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        place_at(&service, "A1", 1);

        let first = talk_on(&service, "street-food");
        assert!(first.scored);
        assert_eq!(first.advanced, None, "one talk on one topic owns nothing yet");
        assert_eq!(first.skills.len(), 3);
        assert!(first.skills.iter().all(|skill| skill.count == 1));
        assert_eq!(
            heard.lock().unwrap().scored[0],
            keys_of(&progress::skills_in_play(&Position::new("A1", 1))),
            "scored on the step's skills and the next step's"
        );
        let second = talk_on(&service, "booking-a-cab");
        assert_eq!(second.advanced, None, "mastery 0.62: not owned yet");
        let third = talk_on(&service, "street-food");
        assert_eq!(third.advanced, Some(Advance::Step));
        assert_eq!((third.standing.level_name.as_str(), third.standing.step), ("First Words", 2));
        assert_eq!(service.bootstrap().unwrap().standing.unwrap(), third.standing);
        // Step 2's skills were scored in the same talks, so the new step is
        // already under way.
        assert!(third.standing.percent > 20, "{}", third.standing.percent);
    }

    #[test]
    fn moving_on_through_talks_is_not_a_placement() {
        // Nobody placed this learner, so the level map still offers them the
        // chat, however far their talks take them.
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        let mut last = None;
        for topic in ["street-food", "booking-a-cab", "doctor-clinic"] {
            last = Some(talk_on(&service, topic));
        }
        let last = last.unwrap();
        assert_eq!(last.advanced, Some(Advance::Step));
        assert_eq!((last.standing.level_name.as_str(), last.standing.step), ("Finding My Voice", 2));
        assert!(!last.standing.placed);
        assert!(!service.bootstrap().unwrap().standing.unwrap().placed);
    }

    #[test]
    fn a_talk_keeps_the_level_it_began_at_when_another_moves_the_learner_on() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        place_at(&service, "A1", 5);
        let talk = service.start_session("street-food").unwrap();
        // Another talk's assessment lands meanwhile and moves them up a level.
        place_at(&service, "A2", 1);
        service.send_text_turn(&talk.id, "I ate poha this morning").unwrap();
        let turn = heard.lock().unwrap().requests.last().unwrap().clone();
        assert_eq!(turn.pitch.level, "A1", "the prompt reads the same to the end of the talk");
        // A new talk is pitched at the new level.
        service.start_session("booking-a-cab").unwrap();
        assert_eq!(heard.lock().unwrap().pitches.last().unwrap().level, "A2");
    }

    #[test]
    fn finishing_the_last_step_of_a_level_is_a_level_up() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        place_at(&service, "A2", 5);
        let mut advanced = None;
        for topic in ["street-food", "booking-a-cab", "doctor-clinic"] {
            advanced = talk_on(&service, topic).advanced;
        }
        assert_eq!(advanced, Some(Advance::Level));
        let standing = service.bootstrap().unwrap().standing.unwrap();
        assert_eq!((standing.level_number, standing.level_name.as_str(), standing.step), (4, "Speaking Freely", 1));
        // v0.1.6 reads the level's name off the learner row.
        assert_eq!(service.bootstrap().unwrap().learner.unwrap().level_name, "Speaking Freely");
    }

    #[test]
    fn a_talk_is_counted_once_however_often_it_is_asked_about() {
        let heard = Arc::new(Mutex::new(Heard::default()));
        let service = judged(Judge::new(&heard));
        place_at(&service, "A1", 1);
        let session = service.start_session("street-food").unwrap();
        service.send_text_turn(&session.id, "I like red and blue").unwrap();
        service.complete_session(&session.id).unwrap();
        let first = service.assess_session(&session.id).unwrap();
        let before = service.database.skill_progress(&["A1:U1-VOC-01".to_string()]).unwrap();
        assert_eq!(service.assess_session(&session.id).unwrap(), first);
        assert_eq!(service.database.skill_progress(&["A1:U1-VOC-01".to_string()]).unwrap(), before);
        assert_eq!(heard.lock().unwrap().scored.len(), 1, "the model reads the talk once");
    }

    #[test]
    fn a_talk_is_only_assessed_once_it_is_over_and_counts_nothing_without_a_word() {
        let service = demo();
        let session = service.start_session("street-food").unwrap();
        let still_going = service.assess_session(&session.id).unwrap_err();
        assert_eq!(still_going.to_string(), "This talk is still going.");
        service.complete_session(&session.id).unwrap();
        let assessment = service.assess_session(&session.id).unwrap();
        assert!(!assessment.scored);
        assert_eq!((assessment.advanced, assessment.skills.len()), (None, 0));
    }

    #[test]
    fn without_a_model_a_talk_is_kept_but_never_scored() {
        let service = demo();
        let assessment = talk_on(&service, "street-food");
        assert!(!assessment.scored, "demo mode has no judge");
        assert_eq!(assessment.standing.percent, 0);
    }

    #[test]
    fn the_level_map_marks_each_level_and_every_skill_behind_the_learner() {
        let service = demo();
        place_at(&service, "A2", 3);
        let levels = service.levels().unwrap();
        assert_eq!(levels.len(), 6);
        let states: Vec<LevelState> = levels.iter().map(|level| level.state).collect();
        use LevelState::*;
        assert_eq!(states, [Done, Done, Current, Next, Locked, Locked]);
        assert_eq!(levels.iter().map(|level| level.percent).collect::<Vec<_>>(), [100, 100, 40, 0, 0, 0]);
        let current = &levels[2];
        assert_eq!((current.number, current.name.as_str()), (3, "Finding My Voice"));
        assert!(current.steps[..2].iter().all(|step| step.skills.iter().all(|skill| skill.passed)));
        assert!(current.steps[2..].iter().all(|step| step.skills.iter().all(|skill| !skill.passed)));
        assert!(levels[0].steps.iter().all(|step| step.skills.iter().all(|skill| skill.passed)));
        assert!(current.steps[0].skills[0].text.starts_with("I "));
        // No CEFR code anywhere the window reads.
        let json = serde_json::to_string(&levels).unwrap();
        for code in ["\"A0\"", "\"A2\"", "\"B1\"", "\"C1\""] {
            assert!(!json.contains(code), "{code} leaked");
        }
    }

    /// The whole flow against a real model: a placement chat that the model
    /// ends by its own judgement, the level it reads, and a talk it scores.
    /// Needs a llama-server with Ella's model at `ELLA_LLM_BASE_URL`:
    ///
    /// ELLA_LLM_BASE_URL=http://127.0.0.1:39191/v1 cargo test --lib live_model -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_model_places_the_learner_and_scores_a_talk() {
        use crate::infrastructure::engines::{EnginePaths, LocalEngine};
        assert!(std::env::var("ELLA_LLM_BASE_URL").is_ok(), "point ELLA_LLM_BASE_URL at llama-server");
        std::env::set_var("ELLA_PIPER_DAEMON", "0");
        let service = AppService::new(
            Database::in_memory().unwrap(),
            Box::new(LocalEngine::from_environment(EnginePaths::default())),
        );
        service.save_learner("Asha", Some(16)).unwrap();

        // Answers a confident intermediate speaker might give, climbing as
        // the questions do.
        let answers = [
            "Hi Ella! My day was quite busy. I went to college in the morning and after that I helped my father in his shop.",
            "I live with my parents and my younger brother in Nagpur. My brother is twelve and he is crazy about cricket.",
            "Last weekend we visited my aunt in Pune. We took the train, and in the evening we walked by the river and ate pani puri.",
            "I think online classes are useful, but I prefer the classroom, because I can ask questions and discuss things with my friends.",
            "If I could change one thing in my city, I would build more libraries, because many students have no quiet place to study.",
            "My favourite place is my grandmother's village. There are mango trees everywhere, and in summer the whole house smells of fresh mangoes.",
            "I would like to become a software engineer, because I enjoy solving problems and I want to build apps that help farmers.",
            "When I feel stressed, I usually go for a long walk or listen to old Hindi songs, and that helps me calm down.",
            "Honestly, I think social media has good and bad sides. It connects people, but it also wastes a lot of our time.",
            "Next year I hope to do an internship in Bangalore, so I am practising English to feel confident in interviews.",
            "I have been learning English since school, but speaking was always difficult for me because I felt shy.",
            "Thank you, Ella. I really enjoyed talking with you today.",
        ];
        let placement = service.start_placement().unwrap();
        println!("Ella: {}", placement.messages[0].content);
        let mut closed_after = None;
        for (index, answer) in answers.iter().enumerate() {
            let turn = service.send_text_turn(&placement.id, answer).unwrap();
            println!("Asha: {answer}\nElla: {}", turn.ella_message.content);
            if turn.session_summary.is_some() {
                closed_after = Some(index as u32 + 1);
                break;
            }
        }
        let answers_heard = closed_after.expect("the placement closed within twelve answers");
        println!("-- placement closed after {answers_heard} answers");
        assert!((progress::PLACEMENT_MIN_TURNS..=progress::PLACEMENT_MAX_TURNS).contains(&answers_heard));
        let placed = service.assess_session(&placement.id).unwrap();
        println!("-- placed: {:?}", placed);
        assert!(placed.standing.placed && placed.scored);
        assert_eq!(placed.standing.step, 1);

        let talk = service.start_session("street-food").unwrap();
        let target = service.database.session_curriculum(&talk.id).unwrap().target_skill;
        println!("-- the talk aims at {target:?}\nElla: {}", talk.messages[0].content);
        for answer in [
            "Yesterday I ate vada pav near the railway station. It was spicy and crispy, and it cost only fifteen rupees.",
            "The stall belongs to an old man who has been selling it for twenty years. He always adds extra chutney for students.",
            "I would recommend it to anyone who visits Mumbai, because it tastes better than anything in a fancy restaurant.",
        ] {
            let turn = service.send_text_turn(&talk.id, answer).unwrap();
            println!("Asha: {answer}\nElla: {}", turn.ella_message.content);
        }
        service.complete_session(&talk.id).unwrap();
        let scored = service.assess_session(&talk.id).unwrap();
        println!("-- talk assessed: {:?}", scored);
        assert!(scored.scored, "the model's scores were read");
    }

    /// The recap's notes against a real model: a talk with mistakes in it
    /// gets one fix, quoted from what was said, and a fluent one gets none.
    /// Needs the same llama-server as the test above:
    ///
    /// ELLA_LLM_BASE_URL=http://127.0.0.1:39191/v1 cargo test --lib live_model -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_model_writes_the_recaps_notes() {
        use crate::infrastructure::engines::{EnginePaths, LocalEngine};
        assert!(std::env::var("ELLA_LLM_BASE_URL").is_ok(), "point ELLA_LLM_BASE_URL at llama-server");
        std::env::set_var("ELLA_PIPER_DAEMON", "0");
        let service = AppService::new(
            Database::in_memory().unwrap(),
            Box::new(LocalEngine::from_environment(EnginePaths::default())),
        );
        service.save_learner("Aarav", Some(13)).unwrap();
        let notes_for = |answers: &[&str]| {
            let talk = service.start_session("street-food").unwrap();
            for answer in answers {
                service.send_text_turn(&talk.id, answer).unwrap();
            }
            service.complete_session(&talk.id).unwrap();
            let started = Instant::now();
            let assessment = service.assess_session(&talk.id).unwrap();
            println!("-- assessed in {:?}: {:?}", started.elapsed(), assessment.notes);
            assessment.notes.expect("long enough for notes")
        };

        let answers = [
            "My school is very big and it have a big playground",
            "We plays football in lunch time and my friend Rahul is best player",
            "I like maths because the teacher explain very nicely",
        ];
        let notes = notes_for(&answers);
        assert!(notes.checked, "the correction was read");
        let fix = notes.fix.expect("a talk with four mistakes has a fix");
        assert!(
            answers.iter().any(|answer| answer.to_lowercase().contains(&fix.said.to_lowercase())),
            "{:?} is not what Aarav said",
            fix.said
        );

        let fluent = notes_for(&[
            "Nothing much, just chilled at home and watched a movie with my cousins",
            "It was an old Shah Rukh film, I don't remember the name but it was really funny",
            "Yeah, sometimes. My dad loves them so we end up watching a lot",
        ]);
        assert!(fluent.checked);
        assert_eq!(fluent.fix, None, "nothing to fix in a fluent talk");
    }

    #[test]
    fn nobody_signed_in_has_no_level_map_and_no_placement() {
        let service = demo();
        service.log_out().unwrap();
        assert!(service.levels().is_err());
        assert!(service.start_placement().is_err());
    }
}
