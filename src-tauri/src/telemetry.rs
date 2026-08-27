use std::{
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
                total_ms: 0,
            },
        }
    }

    pub fn record_vad(&mut self, elapsed_ms: f64, input_ms: f64, speech_ms: f64) {
        self.timings.vad_ms = Some(round_ms(elapsed_ms));
        self.timings.audio_input_ms = Some(round_ms(input_ms));
        self.timings.audio_after_vad_ms = Some(round_ms(speech_ms));
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

    pub fn finish(mut self, status: &str, error: Option<&str>) -> TurnTimings {
        self.timings.total_ms = round_ms(self.started.elapsed().as_secs_f64() * 1_000.0);
        let fmt = |value: Option<u64>| {
            value.map_or_else(|| "-".to_string(), |ms| format!("{ms}ms"))
        };
        eprintln!(
            "[LATENCY] ── turn summary ({}) status={} ──\n\
             [LATENCY]   audio: input={} after_vad={} | vad={}\n\
             [LATENCY]   stt:   {} (engine={} backend={}{})\n\
             [LATENCY]   llm:   ttft={} completion={}\n\
             [LATENCY]   tts:   first_audio={} completion={}\n\
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
