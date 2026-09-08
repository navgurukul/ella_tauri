use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::{multipart, Client};
use serde_json::Value;
use sha2::{Digest, Sha256};
use transcribe_cpp::{Model, Pnc, RunOptions, Session, SessionOptions, TimestampKind};

use crate::{
    error::{EllaError, EllaResult},
    infrastructure::audio::pcm16_wav,
};

pub use super::windows_stt::WindowsStt;

pub const CANARY_FILE_NAME: &str = "canary-180m-flash-Q8_0.gguf";
pub const CANARY_SHA256: &str = "e13c7f5d0952b056a027cfffec13e3a3a134d1608babed24f983568f141e297c";
const CANARY_MIN_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SttStatus {
    pub name: String,
    pub ready: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct Transcription {
    pub text: String,
    pub engine: String,
    pub backend: String,
    pub elapsed_ms: f64,
    pub fallback_from: Option<String>,
    pub mel_ms: Option<f64>,
    pub encode_ms: Option<f64>,
    pub decode_ms: Option<f64>,
}

pub trait SpeechToTextEngine: Send + Sync {
    fn status(&self) -> SttStatus;
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription>;
}

pub struct SttRouter {
    primary: Box<dyn SpeechToTextEngine>,
    fallback: Option<Box<dyn SpeechToTextEngine>>,
}

impl SttRouter {
    pub fn new(
        primary: Box<dyn SpeechToTextEngine>,
        fallback: Option<Box<dyn SpeechToTextEngine>>,
    ) -> Self {
        Self { primary, fallback }
    }

    pub fn status(&self) -> (SttStatus, Option<SttStatus>) {
        (
            self.primary.status(),
            self.fallback.as_ref().map(|fallback| fallback.status()),
        )
    }

    pub fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        let primary_name = self.primary.status().name;
        eprintln!("[LATENCY]     stt> trying primary engine: {primary_name}");
        match self.primary.transcribe(samples, sample_rate) {
            Ok(result) => Ok(result),
            Err(primary_error) => {
                eprintln!(
                    "[LATENCY]     stt> primary {primary_name} FAILED ({primary_error}), trying fallback"
                );
                let Some(fallback) = &self.fallback else {
                    return Err(EllaError::Engine(format!(
                        "{primary_name} failed: {primary_error}. No STT fallback is configured."
                    )));
                };
                match fallback.transcribe(samples, sample_rate) {
                    Ok(mut result) => {
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "event": "stt_fallback",
                                "primary": primary_name,
                                "fallback": result.engine,
                                "reason": primary_error.to_string(),
                            })
                        );
                        result.fallback_from = Some(primary_name);
                        Ok(result)
                    }
                    Err(fallback_error) => Err(EllaError::Engine(format!(
                        "Primary STT ({primary_name}) failed: {primary_error}. Fallback STT ({}) also failed: {fallback_error}",
                        fallback.status().name
                    ))),
                }
            }
        }
    }
}

struct CanaryRuntime {
    model: Model,
    session: Mutex<Session>,
}

pub struct CanaryStt {
    model_path: PathBuf,
    runtime: Result<CanaryRuntime, String>,
}

impl CanaryStt {
    pub fn new(model_path: impl Into<PathBuf>, n_threads: i32, verify_sha256: bool) -> Self {
        let model_path = model_path.into();
        let runtime = Self::load(&model_path, n_threads, verify_sha256);
        Self {
            model_path,
            runtime,
        }
    }

    fn load(path: &Path, n_threads: i32, verify_sha256: bool) -> Result<CanaryRuntime, String> {
        validate_canary_model(path, verify_sha256)?;
        transcribe_cpp::init_backends_default()
            .map_err(|error| format!("could not initialize transcribe.cpp backends: {error}"))?;
        let model = Model::load(path).map_err(|error| {
            format!("transcribe.cpp could not load {}: {error}", path.display())
        })?;
        let architecture = model.arch();
        let variant = model.variant();
        if architecture != "canary" || !variant.contains("180m-flash") {
            return Err(format!(
                "{} is a GGUF, but it is {architecture}/{variant}, not Canary-180M-Flash Q8_0",
                path.display()
            ));
        }
        let capabilities = model.capabilities();
        if capabilities.native_sample_rate != 16_000
            || !capabilities
                .languages
                .iter()
                .any(|language| language == "en")
        {
            return Err(format!(
                "{} has incompatible Canary metadata (sample rate {}, languages {:?})",
                path.display(),
                capabilities.native_sample_rate,
                capabilities.languages
            ));
        }
        let session = model
            .session_with(&SessionOptions {
                n_threads,
                ..SessionOptions::default()
            })
            .map_err(|error| format!("could not create a Canary session: {error}"))?;
        Ok(CanaryRuntime {
            model,
            session: Mutex::new(session),
        })
    }
}

impl SpeechToTextEngine for CanaryStt {
    fn status(&self) -> SttStatus {
        match &self.runtime {
            Ok(runtime) => SttStatus {
                name: "Canary-180M-Flash Q8_0".into(),
                ready: true,
                detail: format!(
                    "Native transcribe.cpp batch STT on {} ({})",
                    runtime.model.backend(),
                    self.model_path.display()
                ),
            },
            Err(error) => SttStatus {
                name: "Canary-180M-Flash Q8_0".into(),
                ready: false,
                detail: format!(
                    "{error}. Install/repair: npm run models:install"
                ),
            },
        }
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        let runtime = self.runtime.as_ref().map_err(|error| {
            EllaError::Engine(format!(
                "Canary is unavailable: {error}. Run `npm run models:install` and restart Ella."
            ))
        })?;
        let resample_started = Instant::now();
        let mut pcm = pcm16_to_16k_f32(samples, sample_rate)?;
        eprintln!(
            "[LATENCY]     stt> canary resample to 16k f32 took {:.1}ms ({} samples, ~{:.0} ms audio)",
            resample_started.elapsed().as_secs_f64() * 1_000.0,
            pcm.len(),
            pcm.len() as f64 / 16.0
        );
        let peak = pcm.iter().fold(0.0_f32, |max, value| max.max(value.abs()));
        let rms = (pcm.iter().map(|value| value * value).sum::<f32>()
            / pcm.len().max(1) as f32)
            .sqrt();
        eprintln!("[LATENCY]     stt> canary input stats: peak={peak:.3} rms={rms:.4}");
        if pcm.len() < 4_000 {
            return Err(EllaError::Validation(
                "I did not hear enough speech. Please speak for at least a quarter second.".into(),
            ));
        }
        // Canary's attention decoder emits an instant EOS ("no words") when
        // speech starts or ends flush at the utterance boundary, which the
        // tight VAD trim produces (verified against captured failure WAVs).
        // Half a second of real silence at both ends makes decoding reliable
        // and costs only ~60 ms of extra encode time.
        pad_with_silence(&mut pcm, CANARY_EDGE_SILENCE_SAMPLES);
        let started = Instant::now();
        let options = RunOptions {
            language: Some("en".into()),
            timestamps: TimestampKind::None,
            pnc: Pnc::On,
            ..RunOptions::default()
        };
        let result = runtime
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .run(&pcm, &options)
            .map_err(|error| {
                let dump = dump_canary_failure(samples, sample_rate);
                EllaError::Engine(format!(
                    "Canary transcription failed: {error}{}",
                    dump_note(&dump)
                ))
            })?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "[LATENCY]     stt> canary inference took {:.1}ms (mel={:.1}ms encode={:.1}ms decode={:.1}ms, backend={})",
            elapsed_ms,
            result.timings.mel_ms,
            result.timings.encode_ms,
            result.timings.decode_ms,
            runtime.model.backend()
        );
        let text = result.text.trim().to_owned();
        if text.is_empty() {
            let dump = dump_canary_failure(samples, sample_rate);
            eprintln!(
                "[LATENCY]     stt> canary heard no words (peak={peak:.3} rms={rms:.4}){}",
                dump_note(&dump)
            );
            return Err(EllaError::Validation(
                "Canary received audio but found no words. Move closer to the microphone and try again."
                    .into(),
            ));
        }
        Ok(Transcription {
            text,
            engine: "canary-180m-flash-q8_0".into(),
            backend: runtime.model.backend(),
            elapsed_ms,
            fallback_from: None,
            mel_ms: nonzero(result.timings.mel_ms),
            encode_ms: nonzero(result.timings.encode_ms),
            decode_ms: nonzero(result.timings.decode_ms),
        })
    }
}

pub struct WhisperHttpStt {
    client: Client,
    base_url: String,
    transcribe_url: String,
}

impl WhisperHttpStt {
    pub fn new(base_url: String, transcribe_url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client configuration is valid"),
            base_url,
            transcribe_url,
        }
    }

    fn probe(&self) -> bool {
        let root = self.base_url.trim_end_matches('/').trim_end_matches("/v1");
        self.client
            .get(format!("{root}/health"))
            .timeout(Duration::from_secs(2))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }
}

impl SpeechToTextEngine for WhisperHttpStt {
    fn status(&self) -> SttStatus {
        let ready = self.probe();
        SttStatus {
            name: "Whisper small fallback".into(),
            ready,
            detail: if ready {
                format!("Ready at {}", self.transcribe_url)
            } else {
                format!(
                    "Not reachable at {}. Start it with `npm run engines:local`.",
                    self.base_url
                )
            },
        }
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        if samples.len() < (sample_rate as usize / 4) {
            return Err(EllaError::Validation(
                "I did not hear enough speech. Please try again.".into(),
            ));
        }
        eprintln!(
            "[LATENCY]     stt> whisper HTTP request to {} ({} samples)",
            self.transcribe_url,
            samples.len()
        );
        let started = Instant::now();
        let part = multipart::Part::bytes(pcm16_wav(samples, sample_rate, 1))
            .file_name("speech.wav")
            .mime_str("audio/wav")?;
        let form = multipart::Form::new()
            .part("file", part)
            .text("model", "whisper-small")
            .text("language", "en")
            .text("temperature", "0")
            .text("response_format", "json");
        let response = self
            .client
            .post(&self.transcribe_url)
            .multipart(form)
            .send()?
            .error_for_status()?;
        eprintln!(
            "[LATENCY]     stt> whisper HTTP response after {:.1}ms",
            started.elapsed().as_secs_f64() * 1_000.0
        );
        let payload: Value = response.json()?;
        let text = payload["text"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if text.is_empty() {
            return Err(EllaError::Validation(
                "Whisper received audio but found no words. Please try again.".into(),
            ));
        }
        Ok(Transcription {
            text,
            engine: "whisper-small".into(),
            backend: "http-sidecar".into(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
            fallback_from: None,
            mel_ms: None,
            encode_ms: None,
            decode_ms: None,
        })
    }
}

pub fn validate_canary_model(path: &Path, verify_sha256: bool) -> Result<(), String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("Canary model is missing at {} ({error})", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "Canary model path is not a file: {}",
            path.display()
        ));
    }
    if metadata.len() < CANARY_MIN_BYTES {
        return Err(format!(
            "Canary model at {} is only {:.1} MiB and appears truncated (expected about 208 MiB)",
            path.display(),
            metadata.len() as f64 / 1024.0 / 1024.0
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| format!("cannot open Canary model {}: {error}", path.display()))?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .map_err(|error| format!("cannot read Canary model header: {error}"))?;
    if &magic != b"GGUF" {
        return Err(format!(
            "Canary model {} has an invalid header; expected a GGUF file",
            path.display()
        ));
    }
    if verify_sha256 {
        let mut digest = Sha256::new();
        digest.update(magic);
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("cannot verify Canary model: {error}"))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != CANARY_SHA256 {
            return Err(format!(
                "Canary model checksum mismatch at {} (got {actual}, expected {CANARY_SHA256})",
                path.display()
            ));
        }
    }
    Ok(())
}

/// 500 ms at 16 kHz, applied to both ends of the audio Canary decodes.
const CANARY_EDGE_SILENCE_SAMPLES: usize = 8_000;

fn pad_with_silence(pcm: &mut Vec<f32>, samples_each_side: usize) {
    let mut padded = Vec::with_capacity(pcm.len() + samples_each_side * 2);
    padded.extend(std::iter::repeat(0.0_f32).take(samples_each_side));
    padded.append(pcm);
    padded.extend(std::iter::repeat(0.0_f32).take(samples_each_side));
    *pcm = padded;
}

/// Save the exact audio that made Canary fail so it can be replayed offline
/// with `stt-benchmark --audio <file>`. Best-effort; never fails the turn.
fn dump_canary_failure(samples: &[i16], sample_rate: u32) -> Option<PathBuf> {
    let dir = env::var("ELLA_STT_DEBUG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::temp_dir().join("ella-stt-failures"));
    std::fs::create_dir_all(&dir).ok()?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let path = dir.join(format!("canary-fail-{millis}.wav"));
    std::fs::write(&path, pcm16_wav(samples, sample_rate, 1)).ok()?;
    Some(path)
}

fn dump_note(dump: &Option<PathBuf>) -> String {
    dump.as_ref()
        .map(|path| format!(" [failing audio saved to {}]", path.display()))
        .unwrap_or_default()
}

fn pcm16_to_16k_f32(samples: &[i16], sample_rate: u32) -> EllaResult<Vec<f32>> {
    if sample_rate == 0 {
        return Err(EllaError::Validation("Invalid audio sample rate.".into()));
    }
    if sample_rate == 16_000 {
        return Ok(samples
            .iter()
            .map(|sample| *sample as f32 / 32_768.0)
            .collect());
    }
    let output_len = (samples.len() as u64 * 16_000 / sample_rate as u64) as usize;
    let ratio = sample_rate as f64 / 16_000.0;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let position = index as f64 * ratio;
        let left = position.floor() as usize;
        let right = (left + 1).min(samples.len().saturating_sub(1));
        let fraction = (position - left as f64) as f32;
        let value = samples.get(left).copied().unwrap_or_default() as f32 * (1.0 - fraction)
            + samples.get(right).copied().unwrap_or_default() as f32 * fraction;
        output.push(value / 32_768.0);
    }
    Ok(output)
}

fn nonzero(value: f32) -> Option<f64> {
    (value > 0.0).then_some(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeStt {
        name: &'static str,
        fails: bool,
    }

    impl SpeechToTextEngine for FakeStt {
        fn status(&self) -> SttStatus {
            SttStatus {
                name: self.name.into(),
                ready: true,
                detail: "test engine".into(),
            }
        }

        fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> EllaResult<Transcription> {
            if self.fails {
                return Err(EllaError::Engine("deliberate primary failure".into()));
            }
            Ok(Transcription {
                text: "fallback transcript".into(),
                engine: self.name.into(),
                backend: "test".into(),
                elapsed_ms: 1.0,
                fallback_from: None,
                mel_ms: None,
                encode_ms: None,
                decode_ms: None,
            })
        }
    }

    #[test]
    fn canary_audio_is_padded_with_edge_silence() {
        let mut pcm = vec![0.5_f32; 1_000];
        pad_with_silence(&mut pcm, 8_000);
        assert_eq!(pcm.len(), 17_000);
        assert_eq!(pcm[..8_000], vec![0.0; 8_000][..]);
        assert_eq!(pcm[16_000..], vec![0.0; 1_000][..]);
        assert_eq!(pcm[8_000], 0.5);
    }

    #[test]
    fn resampling_produces_sixteen_khz_pcm() {
        let source = vec![1_000_i16; 48_000];
        let output = pcm16_to_16k_f32(&source, 48_000).unwrap();
        assert_eq!(output.len(), 16_000);
        assert!((output[0] - 1_000.0 / 32_768.0).abs() < 0.0001);
    }

    #[test]
    fn missing_model_error_includes_the_path() {
        let path = Path::new("/definitely/missing/canary.gguf");
        let error = validate_canary_model(path, false).unwrap_err();
        assert!(error.contains("/definitely/missing/canary.gguf"));
        assert!(error.contains("missing"));
    }

    #[test]
    fn router_uses_fallback_and_records_the_failed_primary() {
        let router = SttRouter::new(
            Box::new(FakeStt {
                name: "canary-test",
                fails: true,
            }),
            Some(Box::new(FakeStt {
                name: "whisper-test",
                fails: false,
            })),
        );

        let transcription = router.transcribe(&[1; 8_000], 16_000).unwrap();
        assert_eq!(transcription.engine, "whisper-test");
        assert_eq!(transcription.fallback_from.as_deref(), Some("canary-test"));
    }
}
