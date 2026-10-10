use std::{
    cell::{Cell, RefCell},
    fs::{self, OpenOptions},
    io::Write as _,
    path::PathBuf,
    sync::OnceLock,
    time::Instant,
};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::{domain::TurnTimings, machine};

/// The build writing the events. Every release sets it. Telemetry used to
/// carry no version, so turns had to be matched to releases by when each was
/// published.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

static TELEMETRY_FILE: OnceLock<PathBuf> = OnceLock::new();
static LAUNCH: OnceLock<Launch> = OnceLock::new();

/// This run of Ella. Every event names it, so the turns of one launch read
/// together, and a restart, with its cold caches, shows.
struct Launch {
    id: String,
    started: Instant,
}

fn launch() -> &'static Launch {
    LAUNCH.get_or_init(|| Launch {
        id: Uuid::new_v4().to_string(),
        started: Instant::now(),
    })
}

/// What every event begins with.
#[derive(Serialize)]
struct Header {
    event: &'static str,
    schema_version: u8,
    timestamp: String,
    app_version: &'static str,
    launch_id: &'static str,
}

impl Header {
    fn new(event: &'static str, schema_version: u8) -> Self {
        Self {
            event,
            schema_version,
            timestamp: Utc::now().to_rfc3339(),
            app_version: APP_VERSION,
            launch_id: &launch().id,
        }
    }
}

/// Persist every turn's latency event to `<dir>/latency.jsonl` so error and
/// latency history survives app restarts and can be reviewed later with
/// `npm run telemetry:report`. Called once at startup, where it also writes
/// the launch's `ella_launch` event: the build, and the computer it runs on.
pub fn persist_to(directory: PathBuf) {
    // The launch is timed from here, at the latest.
    launch();
    match fs::create_dir_all(&directory) {
        Ok(()) => {
            let path = directory.join("latency.jsonl");
            eprintln!("[LATENCY] persisting turn telemetry to {}", path.display());
            let _ = TELEMETRY_FILE.set(path);
        }
        Err(error) => eprintln!("[LATENCY] telemetry dir {} unavailable: {error}", directory.display()),
    }
    write_launch();
}

/// The launch's `ella_launch` event: the build, and the computer it runs on.
fn write_launch() {
    write_event(&LaunchEvent {
        header: Header::new("ella_launch", 1),
        computer: machine::describe(),
        machine: machine::state_now(),
    });
}

fn append_event_line(line: &str) {
    let Some(path) = TELEMETRY_FILE.get() else {
        return;
    };
    let appended = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{line}"));
    if let Err(error) = appended {
        eprintln!("[LATENCY] could not append telemetry to {}: {error}", path.display());
    }
}

fn write_event(event: &impl Serialize) {
    match serde_json::to_string(event) {
        Ok(line) => {
            eprintln!("{line}");
            append_event_line(&line);
            #[cfg(test)]
            WRITTEN.with(|written| written.borrow_mut().push(line));
        }
        Err(error) => eprintln!("[LATENCY] could not write a telemetry event: {error}"),
    }
}

#[cfg(test)]
thread_local! {
    /// Every event written on this thread, for tests to read back.
    static WRITTEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The events written on this thread so far, oldest first.
#[cfg(test)]
pub fn written() -> Vec<serde_json::Value> {
    WRITTEN.with(|written| {
        written
            .borrow()
            .iter()
            .map(|line| serde_json::from_str(line).expect("every event is one JSON object"))
            .collect()
    })
}

#[derive(Serialize)]
struct LaunchEvent {
    #[serde(flatten)]
    header: Header,
    #[serde(flatten)]
    computer: machine::Description,
    #[serde(skip_serializing_if = "Option::is_none")]
    machine: Option<machine::State>,
}

/// How the engine came up, for the launch's `ella_engine` event.
#[derive(Debug, Default, Serialize)]
pub struct EngineDetails {
    /// `started`; `external`, a server someone else runs, named by
    /// `ELLA_LLM_BASE_URL`; or `failed`.
    pub llm_server: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llama_threads: Option<i32>,
    /// What the server says it is: llama.cpp's build, and the context it was
    /// started with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llama_build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_ctx: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_model_mb: Option<u64>,
    /// The speech-to-text engines in the order they are tried.
    pub stt: String,
    /// Canary's threads. 0 leaves the choice to Canary.
    pub stt_threads: i32,
    /// `resident`, Piper kept running between lines, or `one-shot`.
    pub piper: &'static str,
    /// The proxy's host when the cloud language model is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud: Option<String>,
    /// Why the server did not start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
struct EngineEvent<'a> {
    #[serde(flatten)]
    header: Header,
    /// From the launch to the engine being ready, the model loaded.
    since_launch_ms: u64,
    #[serde(flatten)]
    details: &'a EngineDetails,
}

/// Writes the `ella_engine` event, once the engine is up or has failed to
/// come up.
pub fn engine_ready(details: &EngineDetails) {
    write_event(&EngineEvent {
        header: Header::new("ella_engine", 1),
        since_launch_ms: elapsed_ms(launch().started),
        details,
    });
}

/// The cloud language model came up, or went down and why, as an
/// `ella_cloud` event: when a laptop was online for Ella, and what took it off.
pub fn cloud_changed(up: bool, reason: &str) {
    #[derive(Serialize)]
    struct CloudEvent<'a> {
        #[serde(flatten)]
        header: Header,
        up: bool,
        #[serde(skip_serializing_if = "str::is_empty")]
        reason: &'a str,
    }
    write_event(&CloudEvent {
        header: Header::new("ella_cloud", 1),
        up,
        reason,
    });
}

pub struct LatencyTrace {
    started: Instant,
    timings: TurnTimings,
    detail: TurnDetail,
    machine: machine::Sample,
    /// The last stage the turn reached, to say where a failed one stopped.
    stage: Cell<&'static str>,
}

/// What a turn's event says beyond its timings, which the window gets too:
/// the talk the turn was part of, how each part of the work went, and the
/// computer it ran on. Only the event carries it.
#[derive(Debug, Default, Serialize)]
struct TurnDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// `free`, `chore` or `placement`.
    #[serde(skip_serializing_if = "Option::is_none")]
    talk: Option<&'static str>,
    /// The talk's topic, or its chore.
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<String>,
    /// Which of the talk's turns this was, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    turn: Option<u32>,
    /// This turn ended the talk.
    #[serde(skip_serializing_if = "Option::is_none")]
    talk_over: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    learner_words: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_words: Option<usize>,
    /// The last stage a failed turn reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    failed_at: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stt_pieces: Vec<SttPiece>,
    /// From the turn's start to the reply being asked for, so the times of
    /// `llm_runs`, which start there, can be put on the turn's own clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    llm_runs: Vec<LlmRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warm_up: Option<WarmUpReport>,
    /// What happened to the reply on the way, in order: `flagged`,
    /// `no_audio_ahead`, `repeated_question`, `rewrite_repeat_dropped`,
    /// `rewrite_empty`, `repeat_dropped`, `took_offer`,
    /// `said_before_dropped`, `ledger_break`, `ledger_break_twice`,
    /// `audio_ahead_dropped`, `piper_broke`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<&'static str>,
    /// `overlapped`, said from audio made while the reply was written;
    /// `on_clock`, synthesized after it with the learner waiting; or
    /// `failed`, not said at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    tts_path: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tts_sentences: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    machine: Option<machine::State>,
}

/// One piece of a spoken answer and what speech-to-text made of it.
#[derive(Debug, Clone, Serialize)]
pub struct SttPiece {
    /// `background`, sent while the learner was still speaking; `tail`, what
    /// was left when they stopped; or `whole`, a recording sent in one go.
    pub part: &'static str,
    pub audio_ms: u64,
    /// What was left after the silence at its ends was trimmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_ms: Option<u64>,
    /// The engine that gave its words, or `no-words`, `silence` or `none`.
    pub engine: String,
    pub words: usize,
    /// From the piece being handed to speech-to-text to its answer.
    pub ms: u64,
    /// Of `ms`, how long it waited for Canary to finish another piece.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued_ms: Option<u64>,
    /// How many times Canary was asked. More than one when it heard nothing
    /// at first and was asked again without punctuation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    /// When it was done, from the turn's start. Below zero for a piece done
    /// while the learner was still speaking, which kept it off the wait.
    pub done_ms: i64,
}

/// One generation the reply took: the reply itself, and any rewrite.
#[derive(Debug, Clone, Serialize)]
pub struct LlmRun {
    /// `reply`, or `rewrite` for a second generation asked for after the
    /// first got something wrong. `notes` says what.
    pub why: &'static str,
    /// `cloud` when the proxy's model wrote it; left out for the local one.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub backend: &'static str,
    /// For a cloud run that gave no reply: `unanswered`, and the local model
    /// was asked instead, or `cut`, broken off partway. Its `ms` is the time
    /// it cost the turn.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub failed: &'static str,
    /// How long the request waited for the talk's warm-up, or for a slot
    /// being saved, before it went. Part of `ttft_ms`.
    pub wait_ms: u64,
    /// The errand that had the model when the request went, if one did. The
    /// server finishes the piece the errand is on before it starts this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errand: Option<&'static str>,
    pub ttft_ms: u64,
    pub ms: u64,
    /// The prompt's tokens, and how many of them the server evaluated rather
    /// than took from its cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_evaluated: Option<u64>,
    /// The server's own time for evaluating those tokens. With
    /// `prompt_evaluated`, it gives the computer's speed, whatever the
    /// prompt's length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_ms: Option<u64>,
    /// The tokens written, and the server's time for writing them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gen_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gen_ms: Option<u64>,
}

/// The warm-up that got a talk's instructions into the model's slot when the
/// talk opened. Reported by the first reply after it, which waits for it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WarmUpReport {
    /// How long before the reply was asked for it began. This is about when
    /// the talk opened.
    pub began_ms_before: u64,
    pub ms: u64,
    /// Tokens of the instructions restored from a kept slot, if they were.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_tokens: Option<u64>,
    /// Tokens it evaluated: everything not restored.
    pub evaluated_tokens: u64,
    /// The errand that had the model when it began, if one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errand: Option<&'static str>,
}

/// What the engine records about the reply being written on this thread.
#[derive(Default)]
struct ReplyRecord {
    runs: Vec<LlmRun>,
    notes: Vec<&'static str>,
    warm_up: Option<WarmUpReport>,
}

thread_local! {
    /// The reply being written on this thread, for its turn's trace to take
    /// when it finishes. Set up by `LatencyTrace::new`, as an assessment's
    /// judges are by `AssessmentTrace::begin`. A turn runs on one thread from
    /// start to finish, the engine's reply included. Recorded on any other
    /// thread, it is dropped.
    static REPLY: RefCell<Option<ReplyRecord>> = const { RefCell::new(None) };
}

fn with_reply(record: impl FnOnce(&mut ReplyRecord)) {
    REPLY.with(|reply| {
        if let Some(reply) = reply.borrow_mut().as_mut() {
            record(reply);
        }
    });
}

/// One generation for the reply being written on this thread.
pub fn record_llm_run(run: LlmRun) {
    with_reply(|reply| reply.runs.push(run));
}

/// Something that happened to the reply being written on this thread: see
/// `TurnDetail::notes`.
pub fn note(what: &'static str) {
    with_reply(|reply| reply.notes.push(what));
}

/// The warm-up the reply being written on this thread waited for, or came
/// after.
pub fn record_warm_up(report: WarmUpReport) {
    with_reply(|reply| reply.warm_up = Some(report));
}

#[derive(Serialize)]
struct LatencyEvent<'a> {
    #[serde(flatten)]
    header: Header,
    status: &'a str,
    error: Option<&'a str>,
    #[serde(flatten)]
    timings: &'a TurnTimings,
    #[serde(flatten)]
    detail: &'a TurnDetail,
}

impl LatencyTrace {
    /// Console logging: prints one readable line per pipeline stage with the
    /// elapsed time since this turn started, so latency can be watched live.
    pub fn stage(&self, stage: &'static str, detail: &str) {
        self.stage.set(stage);
        eprintln!(
            "[LATENCY] +{:>8.1}ms  {:<18} {}",
            self.started.elapsed().as_secs_f64() * 1_000.0,
            stage,
            detail
        );
    }

    pub fn new(kind: &str) -> Self {
        eprintln!(
            "[LATENCY] ================= new {kind} turn ================="
        );
        REPLY.with(|reply| *reply.borrow_mut() = Some(ReplyRecord::default()));
        Self {
            started: Instant::now(),
            timings: TurnTimings {
                interaction_id: Uuid::new_v4().to_string(),
                kind: kind.into(),
                audio_input_ms: None,
                audio_after_vad_ms: None,
                vad_ms: None,
                stt_ms: None,
                stt_engine: None,
                stt_backend: None,
                stt_fallback_from: None,
                stt_mel_ms: None,
                stt_encode_ms: None,
                stt_decode_ms: None,
                llm_ttft_ms: None,
                llm_completion_ms: None,
                tts_first_audio_ms: None,
                tts_completion_ms: None,
                speech_ms: None,
                total_ms: 0,
            },
            detail: TurnDetail::default(),
            machine: machine::sample(),
            stage: Cell::new("turn:start"),
        }
    }

    pub fn record_vad(&mut self, elapsed_ms: f64, input_ms: f64, speech_ms: f64) {
        self.timings.vad_ms = Some(round_ms(elapsed_ms));
        self.timings.audio_input_ms = Some(round_ms(input_ms));
        self.timings.audio_after_vad_ms = Some(round_ms(speech_ms));
    }

    /// How long a spoken answer lasted: the speech the VAD kept, or the whole
    /// recording for a streamed turn, which is not trimmed. `None` for a typed
    /// turn, which recorded nothing.
    pub fn spoken_ms(&self) -> Option<u64> {
        self.timings.audio_after_vad_ms
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_stt(
        &mut self,
        elapsed_ms: f64,
        engine: String,
        backend: String,
        fallback_from: Option<String>,
        mel_ms: Option<f64>,
        encode_ms: Option<f64>,
        decode_ms: Option<f64>,
    ) {
        self.timings.stt_ms = Some(round_ms(elapsed_ms));
        self.timings.stt_engine = Some(engine);
        self.timings.stt_backend = Some(backend);
        self.timings.stt_fallback_from = fallback_from;
        self.timings.stt_mel_ms = mel_ms.map(round_ms);
        self.timings.stt_encode_ms = encode_ms.map(round_ms);
        self.timings.stt_decode_ms = decode_ms.map(round_ms);
    }

    pub fn record_browser_stt(&mut self) {
        self.timings.stt_ms = Some(0);
        self.timings.stt_engine = Some("browser-web-speech".into());
        self.timings.stt_backend = Some("system-service".into());
    }

    /// The pieces the answer was transcribed in, in the order they were said.
    pub fn record_pieces(&mut self, pieces: Vec<SttPiece>) {
        self.detail.stt_pieces = pieces;
    }

    /// The talk this turn is part of.
    pub fn record_talk(&mut self, session_id: &str, talk: &'static str, topic: &str, level: &str, turn: u32) {
        self.detail.session_id = Some(session_id.to_owned());
        self.detail.talk = Some(talk);
        self.detail.topic = Some(topic.to_owned());
        self.detail.level = Some(level.to_owned());
        self.detail.turn = Some(turn);
    }

    pub fn record_talk_over(&mut self, over: bool) {
        self.detail.talk_over = Some(over);
    }

    /// How long the answer and the reply were, in words. Their words are not
    /// kept.
    pub fn record_words(&mut self, answer: &str, reply: &str) {
        self.detail.learner_words = Some(answer.split_whitespace().count());
        self.detail.reply_words = Some(reply.split_whitespace().count());
    }

    /// The reply is being asked for now.
    pub fn record_llm_start(&mut self) {
        self.detail.llm_start_ms = Some(elapsed_ms(self.started));
    }

    /// Something that happened to this turn's reply: see `TurnDetail::notes`.
    pub fn note(&mut self, what: &'static str) {
        self.detail.notes.push(what);
    }

    pub fn record_llm(&mut self, ttft_ms: f64, completion_ms: f64) {
        self.timings.llm_ttft_ms = Some(round_ms(ttft_ms));
        self.timings.llm_completion_ms = Some(round_ms(completion_ms));
    }

    pub fn record_tts(&mut self, first_audio_ms: Option<f64>, completion_ms: Option<f64>) {
        self.timings.tts_first_audio_ms = first_audio_ms.map(round_ms);
        self.timings.tts_completion_ms = completion_ms.map(round_ms);
    }

    /// How the reply came to be heard: see `TurnDetail::tts_path`.
    pub fn record_tts_path(&mut self, path: &'static str) {
        self.detail.tts_path = Some(path);
    }

    /// How many sentences the window was handed one by one.
    pub fn record_tts_sentences(&mut self, sentences: u32) {
        self.detail.tts_sentences = Some(sentences);
    }

    /// Ella's first sentence went to the window at `at`. A reply played whole
    /// never calls this, and starts when the turn returns.
    pub fn record_speech(&mut self, at: Instant) {
        self.timings.speech_ms = Some(round_ms(
            at.saturating_duration_since(self.started).as_secs_f64() * 1_000.0,
        ));
    }

    /// `at` on the turn's clock: below zero for a moment before it started.
    pub fn offset_ms(&self, at: Instant) -> i64 {
        match at.checked_duration_since(self.started) {
            Some(after) => round_ms(after.as_secs_f64() * 1_000.0) as i64,
            None => -(round_ms(self.started.duration_since(at).as_secs_f64() * 1_000.0) as i64),
        }
    }

    pub fn finish(mut self, status: &str, error: Option<&str>) -> TurnTimings {
        self.timings.total_ms = round_ms(self.started.elapsed().as_secs_f64() * 1_000.0);
        if status == "ok" && self.timings.speech_ms.is_none() {
            self.timings.speech_ms = Some(self.timings.total_ms);
        }
        if let Some(reply) = REPLY.with(|reply| reply.borrow_mut().take()) {
            self.detail.llm_runs = reply.runs;
            self.detail.notes.extend(reply.notes);
            self.detail.warm_up = reply.warm_up;
        }
        if status != "ok" {
            self.detail.failed_at = Some(self.stage.get());
        }
        self.detail.machine = machine::state_since(&self.machine);
        let fmt = |value: Option<u64>| {
            value.map_or_else(|| "-".to_string(), |ms| format!("{ms}ms"))
        };
        eprintln!(
            "[LATENCY] ── turn summary ({}) status={} ──\n\
             [LATENCY]   audio: input={} after_vad={} | vad={}\n\
             [LATENCY]   stt:   {} (engine={} backend={}{})\n\
             [LATENCY]   llm:   ttft={} completion={}\n\
             [LATENCY]   tts:   first_audio={} completion={}\n\
             [LATENCY]   Ella started speaking at {}\n\
             [LATENCY]   TOTAL (Rust side): {}ms{}",
            self.timings.kind,
            status,
            fmt(self.timings.audio_input_ms),
            fmt(self.timings.audio_after_vad_ms),
            fmt(self.timings.vad_ms),
            fmt(self.timings.stt_ms),
            self.timings.stt_engine.as_deref().unwrap_or("-"),
            self.timings.stt_backend.as_deref().unwrap_or("-"),
            self.timings
                .stt_fallback_from
                .as_deref()
                .map(|from| format!(" fallback_from={from}"))
                .unwrap_or_default(),
            fmt(self.timings.llm_ttft_ms),
            fmt(self.timings.llm_completion_ms),
            fmt(self.timings.tts_first_audio_ms),
            fmt(self.timings.tts_completion_ms),
            fmt(self.timings.speech_ms),
            self.timings.total_ms,
            error.map(|detail| format!(" error={detail}")).unwrap_or_default(),
        );
        write_event(&LatencyEvent {
            header: Header::new("ella_turn_latency", 2),
            status,
            error,
            timings: &self.timings,
            detail: &self.detail,
        });
        self.timings
    }
}

fn round_ms(value: f64) -> u64 {
    value.max(0.0).round() as u64
}

fn elapsed_ms(since: Instant) -> u64 {
    round_ms(since.elapsed().as_secs_f64() * 1_000.0)
}

/// One judge's part in an assessment: how long it waited for the model and
/// then had it, and what it read and wrote. The engine records it as the judge
/// answers (`record_judge`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JudgeTiming {
    /// `score`, `correct` or `place`.
    pub judge: &'static str,
    /// `cloud` when the proxy's model judged; left out for the local one.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub backend: &'static str,
    /// `ok`, or `error` for an answer that never came or could not be read.
    pub status: &'static str,
    /// From asking to the model's first piece: what a talk, or a more urgent
    /// errand, held it back.
    pub waited_ms: u64,
    /// From asking to the answer, waits included.
    pub ms: u64,
    /// Times it stopped to let a talk or a more urgent errand go first.
    pub gave_way: u32,
    pub attempts: u32,
    /// Its instructions, restored from where they were kept rather than read.
    pub restored_tokens: Option<u64>,
    /// Prompt tokens the server read for it, every piece and the answer's
    /// last stretch together, as far as the server said.
    pub read_tokens: u64,
    /// Tokens of the answer the server wrote.
    pub written_tokens: u64,
    /// The answer said all that was needed before it was finished, and the
    /// rest of it was not waited for.
    pub stopped_early: bool,
}

thread_local! {
    /// The judges of the assessment being worked out on this thread, if one is.
    static JUDGES: RefCell<Option<Vec<JudgeTiming>>> = const { RefCell::new(None) };
}

/// Keeps `timing` for the assessment being worked out on this thread. A judge
/// runs on the thread that asked for it, so an assessment's judges land in
/// its own `AssessmentTrace`; asked for anywhere else, it is only logged.
pub fn record_judge(timing: JudgeTiming) {
    JUDGES.with(|judges| {
        if let Some(judges) = judges.borrow_mut().as_mut() {
            judges.push(timing);
        }
    });
}

/// One assessment, from the window asking for it to the last of it kept,
/// written to `latency.jsonl` as an `ella_assessment` event: what the recap
/// waits for, as `ella_turn_latency` is what a turn waits for.
pub struct AssessmentTrace {
    started: Instant,
    session_id: String,
    kind: String,
    answers: u32,
    checked_answers: Option<u32>,
    since_close_ms: Option<u64>,
    scored_ms: Option<u64>,
    machine: machine::Sample,
}

#[derive(Serialize)]
struct AssessmentEvent<'a> {
    #[serde(flatten)]
    header: Header,
    status: &'a str,
    error: Option<&'a str>,
    session_id: &'a str,
    /// `talk` or `placement`.
    kind: &'a str,
    /// The learner's answers in the talk.
    answers: u32,
    /// How many of them went to be corrected, for a talk with notes.
    checked_answers: Option<u32>,
    /// From the talk closing to this assessment being asked for.
    since_close_ms: Option<u64>,
    /// From asking to what the talk counted being kept: when the recap's
    /// skills, and any step or level finished, are on screen.
    scored_ms: Option<u64>,
    /// From asking to the whole of it kept, the one fix included.
    total_ms: u64,
    judges: &'a [JudgeTiming],
    #[serde(skip_serializing_if = "Option::is_none")]
    machine: Option<machine::State>,
}

impl AssessmentTrace {
    /// Starts timing an assessment of `session_id`, and collecting the
    /// judges this thread asks until it finishes.
    pub fn begin(session_id: &str, kind: &str, answers: u32, since_close_ms: Option<u64>) -> Self {
        JUDGES.with(|judges| *judges.borrow_mut() = Some(Vec::new()));
        Self {
            started: Instant::now(),
            session_id: session_id.to_owned(),
            kind: kind.to_owned(),
            answers,
            checked_answers: None,
            since_close_ms,
            scored_ms: None,
            machine: machine::sample(),
        }
    }

    /// What the talk counted is kept, and on its way to the window.
    pub fn scored(&mut self) {
        self.scored_ms = Some(elapsed_ms(self.started));
    }

    /// `answers` of the talk's went to be corrected.
    pub fn checked(&mut self, answers: usize) {
        self.checked_answers = Some(answers as u32);
    }

    pub fn finish(self, status: &str, error: Option<&str>) {
        let judges = JUDGES.with(|judges| judges.borrow_mut().take()).unwrap_or_default();
        let total_ms = elapsed_ms(self.started);
        let fmt = |value: Option<u64>| value.map_or_else(|| "-".to_string(), |ms| format!("{ms}ms"));
        eprintln!(
            "[LATENCY] ── assessment summary ({}) status={status} ──\n\
             [LATENCY]   asked {} after the talk closed; scored at {}; all of it at {total_ms}ms",
            self.kind,
            fmt(self.since_close_ms),
            fmt(self.scored_ms),
        );
        for judge in &judges {
            eprintln!(
                "[LATENCY]   {:<8} {} waited={}ms took={}ms gave_way={} attempts={} restored={} read={} wrote={}{}",
                judge.judge,
                judge.status,
                judge.waited_ms,
                judge.ms,
                judge.gave_way,
                judge.attempts,
                judge.restored_tokens.map_or_else(|| "-".to_string(), |tokens| tokens.to_string()),
                judge.read_tokens,
                judge.written_tokens,
                if judge.stopped_early { " stopped early" } else { "" },
            );
        }
        write_event(&AssessmentEvent {
            header: Header::new("ella_assessment", 2),
            status,
            error,
            session_id: &self.session_id,
            kind: &self.kind,
            answers: self.answers,
            checked_answers: self.checked_answers,
            since_close_ms: self.since_close_ms,
            scored_ms: self.scored_ms,
            total_ms,
            judges: &judges,
            machine: machine::state_since(&self.machine),
        });
    }
}

impl Drop for AssessmentTrace {
    /// Whatever ends the assessment, its judges stop landing here.
    fn drop(&mut self) {
        JUDGES.with(|judges| judges.borrow_mut().take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_says_what_it_runs_on_and_the_engine_how_it_came_up() {
        write_launch();
        engine_ready(&EngineDetails {
            llm_server: "started",
            llama_threads: Some(4),
            stt: "canary-180m-flash-q8_0 > windows-speech".into(),
            piper: "resident",
            ..EngineDetails::default()
        });
        let events = written();
        let launch = events.iter().find(|event| event["event"] == "ella_launch").unwrap();
        assert_eq!(launch["app_version"], APP_VERSION);
        assert!(launch["os"].is_string());
        assert!(launch["cores_logical"].as_u64().unwrap() >= 1);
        let engine = events.iter().find(|event| event["event"] == "ella_engine").unwrap();
        assert_eq!(engine["launch_id"], launch["launch_id"], "one launch, one id");
        assert_eq!(engine["llama_threads"], 4);
        assert!(engine["since_launch_ms"].is_u64());
        assert!(engine.get("error").is_none());
    }

    #[test]
    fn a_turn_keeps_only_what_its_own_thread_recorded_while_it_ran() {
        // Before any turn: nowhere to land.
        note("took_offer");
        let trace = LatencyTrace::new("text");
        std::thread::spawn(|| note("repeat_dropped")).join().unwrap();
        note("said_before_dropped");
        trace.finish("ok", None);
        let event = written().pop().unwrap();
        assert_eq!(event["notes"], serde_json::json!(["said_before_dropped"]));

        // And the next turn starts with nothing.
        LatencyTrace::new("text").finish("ok", None);
        assert!(written().pop().unwrap().get("notes").is_none());
    }

    #[test]
    fn successful_trace_has_a_correlation_id_and_total() {
        let mut trace = LatencyTrace::new("voice");
        trace.record_vad(0.4, 4_214.0, 3_900.0);
        let timings = trace.finish("ok", None);
        assert!(!timings.interaction_id.is_empty());
        assert_eq!(timings.vad_ms, Some(0));
        assert_eq!(timings.audio_input_ms, Some(4_214));
    }
}
