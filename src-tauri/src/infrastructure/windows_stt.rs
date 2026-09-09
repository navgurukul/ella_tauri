//! Windows Built-in Speech-to-Text (STT) Engine
//!
//! Utilizes Windows native Speech Recognition capabilities (System.Speech / SAPI)
//! to transcribe captured speech audio without external cloud API dependencies.

#[cfg(target_os = "windows")]
use std::time::Instant;

use crate::{
    error::{EllaError, EllaResult},
    infrastructure::stt::{SpeechToTextEngine, SttStatus, Transcription},
};

#[cfg(target_os = "windows")]
use crate::infrastructure::audio::pcm16_wav;

pub struct WindowsStt {
    language: String,
}

impl WindowsStt {
    pub fn new(language: Option<String>) -> Self {
        Self {
            language: language.unwrap_or_else(|| "en-IN".into()),
        }
    }
}

impl Default for WindowsStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SpeechToTextEngine for WindowsStt {
    fn status(&self) -> SttStatus {
        // Off Windows there is no SAPI to drive and `transcribe` says so on
        // every call, so a green light here would be a lie. Only reachable
        // that way through `ELLA_STT_ENGINE=windows`, since the router picks
        // Canary everywhere else.
        let ready = cfg!(target_os = "windows");
        SttStatus {
            name: "windows-speech".into(),
            ready,
            detail: if ready {
                format!("Windows Built-in Speech Recognition ({})", self.language)
            } else {
                "Windows Built-in Speech Recognition is only available on Windows.".into()
            },
        }
    }

    #[cfg(target_os = "windows")]
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        let started = Instant::now();

        if samples.is_empty() {
            return Err(EllaError::Validation(
                "I did not hear enough speech. Please speak for at least a quarter second.".into(),
            ));
        }

        // Generate standard WAV buffer from PCM samples
        let wav_bytes = pcm16_wav(samples, sample_rate, 1);

        // Perform Windows Speech Recognition
        let recognized_text = transcribe_windows_audio(&wav_bytes, &self.language)?;
        let text = recognized_text.trim().to_owned();

        // SAPI exits 0 and prints nothing when it heard the audio and made no
        // sense of it, which for an accented or code-switching learner is the
        // likely outcome rather than the rare one. Canary and Whisper both
        // report that as an error so `SttRouter` reaches for the fallback;
        // returning Ok("") here would hand the learner silence and leave the
        // better engine unused.
        if text.is_empty() {
            return Err(EllaError::Validation(
                "Windows Speech Recognition received audio but found no words. Move closer to the microphone and try again."
                    .into(),
            ));
        }

        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "[LATENCY]     stt> windows-speech transcribed {} samples in {:.1}ms: \"{}\"",
            samples.len(),
            elapsed_ms,
            text
        );

        Ok(Transcription {
            text,
            engine: "windows-speech".into(),
            backend: "windows-native".into(),
            elapsed_ms,
            fallback_from: None,
            mel_ms: None,
            encode_ms: None,
            decode_ms: None,
        })
    }

    #[cfg(not(target_os = "windows"))]
    fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> EllaResult<Transcription> {
        Err(EllaError::Engine(
            "Windows Speech Recognition is only available on Windows OS.".into(),
        ))
    }
}

#[cfg(target_os = "windows")]
fn transcribe_windows_audio(wav_data: &[u8], language: &str) -> EllaResult<String> {
    use std::env;
    use std::fs;
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use uuid::Uuid;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // Use a temporary WAV file in system temp dir
    let temp_file_name = format!("ella_stt_{}.wav", Uuid::new_v4());
    let temp_path_buf = env::temp_dir().join(temp_file_name);
    let temp_path = temp_path_buf.to_string_lossy().to_string();

    let mut file = fs::File::create(&temp_path_buf)
        .map_err(|e| EllaError::Engine(format!("Failed to create temporary WAV file: {e}")))?;

    file.write_all(wav_data)
        .map_err(|e| EllaError::Engine(format!("Failed to write WAV data: {e}")))?;
    drop(file);

    // PowerShell System.Speech SAPI recognition: loop through sentences until audio ends
    let ps_script = format!(
        r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
try {{
    $culture = New-Object System.Globalization.CultureInfo('{lang}')
    $recognizer = New-Object System.Speech.Recognition.SpeechRecognitionEngine($culture)
}} catch {{
    $recognizer = New-Object System.Speech.Recognition.SpeechRecognitionEngine
}}
$grammar = New-Object System.Speech.Recognition.DictationGrammar
$recognizer.LoadGrammar($grammar)
$recognizer.SetInputToWaveFile('{path}')

$sentences = [System.Collections.Generic.List[string]]::new()
while ($true) {{
    try {{
        $result = $recognizer.Recognize()
    }} catch [System.InvalidOperationException] {{
        break
    }}
    if ($null -eq $result) {{ break }}
    if (![string]::IsNullOrWhiteSpace($result.Text)) {{
        $sentences.Add($result.Text.Trim())
    }}
}}

if ($sentences.Count -gt 0) {{
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output ($sentences -join ' ')
}}
"#,
        lang = language.replace('\'', "''"),
        path = temp_path.replace('\'', "''")
    );

    let mut cmd = Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script]);
    let output = cmd.output();

    // Clean up temporary file
    let _ = fs::remove_file(&temp_path_buf);

    let output = output
        .map_err(|e| EllaError::Engine(format!("Failed to execute Windows speech recognizer: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(EllaError::Engine(format!(
            "Windows Speech Recognition failed: {stderr}"
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(text)
}

