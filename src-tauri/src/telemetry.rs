use std::{
    cell::RefCell,
    fs::{self, OpenOptions},
    io::Write as _,
    path::PathBuf,
    sync::OnceLock,
    time::Instant,
};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::domain::TurnTimings;

static TELEMETRY_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Persist every turn's latency event to `<dir>/latency.jsonl` so error and
/// latency history survives app restarts and can be reviewed later with
/// `npm run telemetry:report`. Called once at startup.
pub fn persist_to(directory: PathBuf) {
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!("[LATENCY] telemetry dir {} unavailable: {error}", directory.display());
        return;
    }
    let path = directory.join("latency.jsonl");
    eprintln!("[LATENCY] persisting turn telemetry to {}", path.display());
    let _ = TELEMETRY_FILE.set(path);
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

pub struct LatencyTrace {
    started: Instant,
    timings: TurnTimings,
}

#[derive(Serialize)]
struct LatencyEvent<'a> {
    event: &'static str,
    schema_version: u8,
    timestamp: String,
    status: &'a str,
    error: Option<&'a str>,
    #[serde(flatten)]
    timings: &'a TurnTimings,
}

impl LatencyTrace {
    /// Console logging: prints one readable line per pipeline stage with the
    /// elapsed time since this turn started, so latency can be watched live.
    pub fn stage(&self, stage: &str, detail: &str) {
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

    pub fn record_llm(&mut self, ttft_ms: f64, completion_ms: f64) {
        self.timings.llm_ttft_ms = Some(round_ms(ttft_ms));
        self.timings.llm_completion_ms = Some(round_ms(completion_ms));
    }

    pub fn record_tts(&mut self, first_audio_ms: Option<f64>, completion_ms: Option<f64>) {
        self.timings.tts_first_audio_ms = first_audio_ms.map(round_ms);
        self.timings.tts_completion_ms = completion_ms.map(round_ms);
    }

    /// Ella's first sentence went to the window at `at`. A reply played whole
    /// never calls this, and starts when the turn returns.
    pub fn record_speech(&mut self, at: Instant) {
        self.timings.speech_ms = Some(round_ms(
            at.saturating_duration_since(self.started).as_secs_f64() * 1_000.0,
        ));
    }

    pub fn finish(mut self, status: &str, error: Option<&str>) -> TurnTimings {
        self.timings.total_ms = round_ms(self.started.elapsed().as_secs_f64() * 1_000.0);
        if status == "ok" && self.timings.speech_ms.is_none() {
            self.timings.speech_ms = Some(self.timings.total_ms);
        }
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
        let event = LatencyEvent {
            event: "ella_turn_latency",
            schema_version: 1,
            timestamp: Utc::now().to_rfc3339(),
            status,
            error,
            timings: &self.timings,
        };
        match serde_json::to_string(&event) {
            Ok(line) => {
                eprintln!("{line}");
                append_event_line(&line);
            }
            Err(serialization_error) => eprintln!(
                "{{\"event\":\"ella_turn_latency_log_error\",\"error\":{}}}",
                serde_json::to_string(&serialization_error.to_string())
                    .unwrap_or_else(|_| "\"serialization failed\"".into())
            ),
        }
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
}

#[derive(Serialize)]
struct AssessmentEvent<'a> {
    event: &'static str,
    schema_version: u8,
    timestamp: String,
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
        let event = AssessmentEvent {
            event: "ella_assessment",
            schema_version: 1,
            timestamp: Utc::now().to_rfc3339(),
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
        };
        match serde_json::to_string(&event) {
            Ok(line) => {
                eprintln!("{line}");
                append_event_line(&line);
            }
            Err(error) => eprintln!("[LATENCY] could not write the assessment's telemetry: {error}"),
        }
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
    fn successful_trace_has_a_correlation_id_and_total() {
        let mut trace = LatencyTrace::new("voice");
        trace.record_vad(0.4, 4_214.0, 3_900.0);
        let timings = trace.finish("ok", None);
        assert!(!timings.interaction_id.is_empty());
        assert_eq!(timings.vad_ms, Some(0));
        assert_eq!(timings.audio_input_ms, Some(4_214));
    }
}
