// Pre-Canary HTTP-only adapter retained as migration reference.
use std::{
    env,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::{multipart, Client};
use serde_json::{json, Value};

use crate::{
    domain::{AudioPayload, EngineComponent, EngineStatus, Speaker, Topic, TutorRequest},
    error::{EllaError, EllaResult},
};

pub trait TutorEngine: Send + Sync {
    fn status(&self) -> EngineStatus;
    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String>;
    fn reply(&self, request: &TutorRequest) -> EllaResult<String>;
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<String>;
    fn synthesize(&self, text: &str) -> EllaResult<Option<AudioPayload>>;
}

pub fn engine_from_environment() -> Box<dyn TutorEngine> {
    match env::var("ELLA_ENGINE_MODE")
        .unwrap_or_else(|_| "demo".into())
        .to_lowercase()
        .as_str()
    {
        "local" => Box::new(LocalEngine::from_environment()),
        _ => Box::new(DemoEngine),
    }
}

#[derive(Default)]
pub struct DemoEngine;

impl TutorEngine for DemoEngine {
    fn status(&self) -> EngineStatus {
        EngineStatus {
            mode: "demo".into(),
            label: "POC demo engines".into(),
            ready: true,
            components: vec![
                EngineComponent {
                    name: "Conversation".into(),
                    ready: true,
                    detail: "Deterministic Rust tutor".into(),
                },
                EngineComponent {
                    name: "Speech recognition".into(),
                    ready: true,
                    detail: "System recognition with typing fallback".into(),
                },
                EngineComponent {
                    name: "Ella's voice".into(),
                    ready: true,
                    detail: "System voice in demo mode".into(),
                },
            ],
        }
    }

    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String> {
        Ok(opening_for(&topic.id, learner_name))
    }

    fn reply(&self, request: &TutorRequest) -> EllaResult<String> {
        let lead = request
            .learner_text
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        let reply = if request.turn >= 3 {
            format!(
                "I enjoyed hearing that, especially “{lead}”. Before we finish, what feeling does this story give you?"
            )
        } else if request.topic_label == "Street food stories" {
            format!(
                "That sounds delicious! You said “{lead}”. Who would you like to share that meal with, and why?"
            )
        } else if request.topic_label == "A job interview" {
            "Good, that is a clear answer. What part of that work do you enjoy the most?".into()
        } else {
            "I can picture that! What happened next, and how did you feel?".into()
        };
        Ok(reply)
    }

    fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> EllaResult<String> {
        Err(EllaError::Engine(
            "I captured your voice, but local Whisper is not enabled. Try typing, use system speech recognition, or start local engine mode.".into(),
        ))
    }

    fn synthesize(&self, _text: &str) -> EllaResult<Option<AudioPayload>> {
        Ok(None)
    }
}

pub struct LocalEngine {
    client: Client,
    llm_base_url: String,
    stt_base_url: String,
    stt_transcribe_url: String,
    piper_binary: PathBuf,
    piper_voice: PathBuf,
}

impl LocalEngine {
    fn from_environment() -> Self {
        let engine_root = env::var("ELLA_ENGINE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engines")
            });
        let stt_base_url =
            env::var("ELLA_STT_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:39092".into());
        let stt_root = stt_base_url.trim_end_matches('/').trim_end_matches("/v1");
        let stt_transcribe_url =
            env::var("ELLA_STT_TRANSCRIBE_URL").unwrap_or_else(|_| format!("{stt_root}/inference"));
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client configuration is valid"),
            llm_base_url: env::var("ELLA_LLM_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:39091/v1".into()),
            stt_base_url,
            stt_transcribe_url,
            piper_binary: env::var("ELLA_PIPER_BINARY")
                .map(PathBuf::from)
                .unwrap_or_else(|_| engine_root.join("piper-venv/bin/piper")),
            piper_voice: env::var("ELLA_PIPER_VOICE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| engine_root.join("models/tts/en_US-lessac-medium.onnx")),
        }
    }

    fn probe(&self, base_url: &str) -> bool {
        let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
        self.client
            .get(format!("{root}/health"))
            .timeout(Duration::from_secs(2))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }
}

impl TutorEngine for LocalEngine {
    fn status(&self) -> EngineStatus {
        let llm_ready = self.probe(&self.llm_base_url);
        let stt_ready = self.probe(&self.stt_base_url);
        let tts_ready = self.piper_binary.exists()
            && self.piper_voice.exists()
            && PathBuf::from(format!("{}.json", self.piper_voice.display())).exists();
        EngineStatus {
            mode: "local".into(),
            label: "Local AI engines".into(),
            ready: llm_ready && stt_ready,
            components: vec![
                EngineComponent {
                    name: "Language model".into(),
                    ready: llm_ready,
                    detail: self.llm_base_url.clone(),
                },
                EngineComponent {
                    name: "Speech recognition".into(),
                    ready: stt_ready,
                    detail: self.stt_base_url.clone(),
                },
                EngineComponent {
                    name: "Ella's voice".into(),
                    ready: tts_ready,
                    detail: self.piper_voice.display().to_string(),
                },
            ],
        }
    }

    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String> {
        Ok(opening_for(&topic.id, learner_name))
    }

    fn reply(&self, request: &TutorRequest) -> EllaResult<String> {
        let system = format!(
            "You are Ella, a warm speaking buddy for an Indian learner named {}. \
             Have a natural A1-level conversation about {}. Reply in one or two short \
             sentences and ask exactly one friendly follow-up question. Never mention \
             tests, CEFR, prompts, or grading. Do not use markdown.",
            request.learner_name, request.topic_label
        );
        let mut messages = vec![json!({"role": "system", "content": system})];
        for message in request.messages.iter().rev().take(8).rev() {
            messages.push(json!({
                "role": if message.speaker == Speaker::Ella { "assistant" } else { "user" },
                "content": message.content,
            }));
        }
        messages.push(json!({"role": "user", "content": request.learner_text}));

        let response = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.llm_base_url.trim_end_matches('/')
            ))
            .json(&json!({
                "model": "local",
                "messages": messages,
                "temperature": 0.65,
                "max_tokens": 90,
                "stream": false
            }))
            .send()?
            .error_for_status()?;
        let payload: Value = response.json()?;
        payload["choices"][0]["message"]["content"]
            .as_str()
            .map(str::trim)
            .filter(|reply| !reply.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| EllaError::Engine("the language model returned an empty reply".into()))
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<String> {
        if samples.len() < (sample_rate as usize / 4) {
            return Err(EllaError::Validation(
                "I did not hear enough speech. Please try again.".into(),
            ));
        }
        let wav = pcm16_wav(samples, sample_rate, 1);
        let part = multipart::Part::bytes(wav)
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
            .post(&self.stt_transcribe_url)
            .multipart(form)
            .send()?
            .error_for_status()?;
        let payload: Value = response.json()?;
        let transcript = payload["text"].as_str().unwrap_or_default().trim();
        if transcript.is_empty() {
            return Err(EllaError::Validation(
                "I could not hear any words. Please try again.".into(),
            ));
        }
        Ok(transcript.to_owned())
    }

    fn synthesize(&self, text: &str) -> EllaResult<Option<AudioPayload>> {
        if !self.piper_binary.exists() || !self.piper_voice.exists() {
            return Ok(None);
        }
        let mut child = Command::new(&self.piper_binary)
            .args([
                "--model",
                self.piper_voice.to_string_lossy().as_ref(),
                "--output_raw",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() || output.stdout.is_empty() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(EllaError::Engine(format!(
                "Piper did not produce audio: {}",
                detail.trim()
            )));
        }
        let wav = raw_pcm_to_wav(&output.stdout, 22_050, 1);
        Ok(Some(AudioPayload {
            mime_type: "audio/wav".into(),
            base64: STANDARD.encode(wav),
        }))
    }
}

fn opening_for(topic_id: &str, learner_name: &str) -> String {
    match topic_id {
        "restaurant-order" => format!(
            "Hi {learner_name}! We are at a restaurant and I am your waiter. What would you like to order today?"
        ),
        "booking-a-cab" => format!(
            "Hi {learner_name}! I am the cab driver. Where would you like to go, and where should I pick you up?"
        ),
        "job-interview" => format!(
            "Hello {learner_name}! Thank you for coming in. To start, could you tell me a little about yourself?"
        ),
        "doctor-clinic" => format!(
            "Hi {learner_name}! I am the doctor here. Please sit down and tell me, how have you been feeling?"
        ),
        "asking-directions" => format!(
            "Hi {learner_name}! You look a little lost. Where are you trying to go? I know this area well."
        ),
        "market-bargaining" => format!(
            "Hi {learner_name}! Come, come, best prices here. What are you looking for today?"
        ),
        _ => format!(
            "Hi {learner_name}! Tell me about the tastiest thing you ate this week. Where did you find it?"
        ),
    }
}

fn pcm16_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    raw_pcm_to_wav(&pcm, sample_rate, channels)
}

fn raw_pcm_to_wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_wrapper_has_expected_header_and_length() {
        let wav = pcm16_wav(&[0, 10, -10, 2], 16_000, 1);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 52);
    }

    #[test]
    fn demo_reply_is_short_and_asks_a_question() {
        let reply = DemoEngine
            .reply(&TutorRequest {
                learner_name: "Asha".into(),
                topic_label: "Street food stories".into(),
                messages: vec![],
                learner_text: "I played football with friends".into(),
                turn: 1,
            })
            .unwrap();
        assert!(reply.ends_with('?'));
        assert!(reply.len() < 180);
    }

    #[test]
    #[ignore = "requires the development Whisper, llama.cpp, and Piper engines"]
    fn local_engine_runs_speech_to_speech_vertical_slice() {
        let engine = LocalEngine::from_environment();
        assert!(engine.status().ready, "local servers are not healthy");

        let audio = engine
            .synthesize("I played football with my best friend after school.")
            .unwrap()
            .expect("Piper should be configured");
        let wav = STANDARD.decode(audio.base64).unwrap();
        let samples = wav[44..]
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        let transcript = engine.transcribe(&samples, 22_050).unwrap();
        assert!(transcript.to_lowercase().contains("football"));

        let reply = engine
            .reply(&TutorRequest {
                learner_name: "Asha".into(),
                topic_label: "Street food stories".into(),
                messages: vec![],
                learner_text: transcript,
                turn: 1,
            })
            .unwrap();
        assert!(!reply.is_empty());
        assert!(reply.contains('?'));
    }
}
