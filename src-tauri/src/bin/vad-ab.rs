// Temporary A/B harness: does applying the energy VAD to the streamed-chunk
// audio turn Canary "no words" failures into transcripts?
use std::{env, fs, path::PathBuf, process::ExitCode, time::Instant};

use ella_tauri_lib::infrastructure::{
    audio::trim_to_speech,
    stt::{CanaryStt, SpeechToTextEngine, CANARY_FILE_NAME},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vad-ab failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let dir = PathBuf::from(env::args().nth(1).ok_or("usage: vad-ab <dir-of-wavs>")?);
    let engine_root = env::var("ELLA_ENGINE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engines"));
    let canary = CanaryStt::new(
        &engine_root.join("models/stt").join(CANARY_FILE_NAME),
        0,
        false,
    );
    let status = canary.status();
    if !status.ready {
        return Err(status.detail);
    }

    let mut wavs = fs::read_dir(&dir)
        .map_err(|e| format!("read dir: {e}"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect::<Vec<_>>();
    wavs.sort();

    println!("file\tms_in\tms_vad\tspeech\tA_asis\tB_vad\tA_ms\tB_ms");
    let (mut a_ok, mut b_ok, mut n) = (0usize, 0usize, 0usize);
    for path in &wavs {
        let Ok((samples, rate)) = read_wav(path) else {
            continue;
        };
        n += 1;
        let in_ms = samples.len() as f64 * 1_000.0 / rate as f64;

        let started = Instant::now();
        let a = canary.transcribe(&samples, rate);
        let a_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let a_text = a.map(|t| t.text.trim().to_string()).unwrap_or_default();

        let vad = trim_to_speech(&samples, rate).map_err(|e| format!("vad: {e}"))?;
        let started = Instant::now();
        let b = canary.transcribe(&vad.samples, rate);
        let b_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let b_text = b.map(|t| t.text.trim().to_string()).unwrap_or_default();

        if !a_text.is_empty() {
            a_ok += 1;
        }
        if !b_text.is_empty() {
            b_ok += 1;
        }
        println!(
            "{}\t{:.0}\t{:.0}\t{}\t{}\t{}\t{:.0}\t{:.0}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            in_ms,
            vad.speech_ms,
            vad.speech_detected,
            quote(&a_text),
            quote(&b_text),
            a_ms,
            b_ms
        );
    }
    println!("\n== {n} failure WAVs ==");
    println!("A (as-is, current streamed path): {a_ok}/{n} produced words");
    println!("B (VAD trimmed):                  {b_ok}/{n} produced words");
    Ok(())
}

fn quote(text: &str) -> String {
    if text.is_empty() {
        "-".into()
    } else {
        format!("\"{}\"", text.replace('\t', " "))
    }
}

fn read_wav(path: &PathBuf) -> Result<(Vec<i16>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| format!("open: {e}"))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err("not mono 16-bit".into());
    }
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("decode: {e}"))?;
    Ok((samples, spec.sample_rate))
}
