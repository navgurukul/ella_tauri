//! Times the per-turn work whose cost depends on the operating system, through
//! the same code the app runs, so a Windows machine and a Mac can be compared
//! and a change can be measured before and after.
//!
//!   * `piper` — the resident Piper, one reply's sentences at a time. On
//!     Windows that is `piper.exe` writing each sentence to a WAV in %TEMP%
//!     that Ella reads back and removes; elsewhere the daemon script's pipe.
//!   * `tempfs` — a sentence-sized WAV written, read back and removed, in the
//!     temp directory and beside the database: the file work alone.
//!   * `sqlite` — `Database::persist_turn`, the commit a reply waits on before
//!     its first sentence is released.
//!   * `spawn` — starting a process: `piper --version`, and on Windows
//!     PowerShell, which the Windows speech fallback starts for every answer.
//!   * `sapi` — that fallback (`WindowsStt`) on a WAV, Windows only.
//!
//! ```bash
//! ELLA_ENGINE_ROOT="$PWD/engines" cargo run --release --bin latency-probe -- \
//!   --rounds 5 --audio ../bench/fixtures/jfk.wav --output ../bench/results/probe.json
//! ```

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::Instant,
};

use ella_tauri_lib::{
    domain::{Message, Session, Speaker},
    infrastructure::{
        database::{Database, LessonMark},
        engines::probe,
        stt::SpeechToTextEngine,
        windows_stt::WindowsStt,
    },
};
use serde_json::{json, Map, Value};

/// A reply the size Ella's usually are, split the way the speech pipeline
/// splits it.
const SENTENCES: [&str; 4] = [
    "That sounds like a lovely place to grow up.",
    "I like how you described the market near your house.",
    "What do people in your village usually buy there on Sundays?",
    "Tell me about one thing you would change about it.",
];

struct Config {
    engine_root: PathBuf,
    voice: Option<PathBuf>,
    rounds: usize,
    audio: Option<PathBuf>,
    output: Option<PathBuf>,
    only: Option<Vec<String>>,
    force_binary: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("latency-probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let config = parse_args()?;
    let wants = |name: &str| config.only.as_ref().map_or(true, |only| only.iter().any(|o| o == name));
    let mut results = Map::new();
    results.insert("os".into(), json!(env::consts::OS));
    results.insert("arch".into(), json!(env::consts::ARCH));
    results.insert("rounds".into(), json!(config.rounds));
    results.insert("temp_dir".into(), json!(env::temp_dir().display().to_string()));

    let work = env::temp_dir().join(format!("ella-latency-probe-{}", std::process::id()));
    fs::create_dir_all(&work).map_err(|error| format!("{}: {error}", work.display()))?;
    // Beside the database, i.e. somewhere that is not %TEMP%.
    let data_dir = env::current_dir()
        .map_err(|error| error.to_string())?
        .join("target")
        .join("latency-probe-data");
    fs::create_dir_all(&data_dir).map_err(|error| format!("{}: {error}", data_dir.display()))?;

    if wants("piper") {
        results.insert("piper".into(), piper(&config)?);
    }
    if wants("tempfs") {
        results.insert("tempfs".into(), tempfs(&work, &data_dir, config.rounds));
    }
    if wants("sqlite") {
        results.insert("sqlite".into(), sqlite(&data_dir, config.rounds)?);
    }
    if wants("spawn") {
        results.insert("spawn".into(), spawn(&config));
    }
    if wants("sapi") && cfg!(target_os = "windows") {
        if let Some(audio) = &config.audio {
            results.insert("sapi".into(), sapi(audio, config.rounds.min(3))?);
        }
    }
    let _ = fs::remove_dir_all(&work);

    let report = Value::Object(results);
    println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    if let Some(output) = &config.output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output, serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?)
            .map_err(|error| format!("{}: {error}", output.display()))?;
    }
    Ok(())
}

fn piper(config: &Config) -> Result<Value, String> {
    let voice = config
        .voice
        .clone()
        .unwrap_or_else(|| config.engine_root.join("models/tts/en_US-lessac-medium.onnx"));
    if !voice.is_file() {
        return Err(format!("no Piper voice at {}", voice.display()));
    }
    let daemon = probe::resident_piper(&config.engine_root, &voice, config.force_binary)
        .ok_or_else(|| format!("no Piper under {}", config.engine_root.display()))?;
    let path = if probe::is_binary(&daemon) { "binary" } else { "script" };
    eprintln!("piper: resident {path}, voice {}", voice.display());

    // The app warms Piper in the background before the first reply.
    let warm_started = Instant::now();
    probe::speak(&daemon, "Hello.").map_err(|error| error.to_string())?;
    let warm_ms = ms(warm_started);

    let (mut answer, mut handoff, mut total, mut reply) = (vec![], vec![], vec![], vec![]);
    let mut audio_ms = 0.0;
    for _ in 0..config.rounds {
        let reply_started = Instant::now();
        for sentence in SENTENCES {
            let timing = probe::speak(&daemon, sentence).map_err(|error| error.to_string())?;
            answer.push(timing.answer_ms);
            handoff.push(timing.total_ms - timing.answer_ms);
            total.push(timing.total_ms);
            audio_ms = timing.pcm_bytes as f64 / 2.0 / timing.sample_rate as f64 * 1_000.0;
        }
        reply.push(ms(reply_started));
    }
    Ok(json!({
        "path": path,
        "warm_ms": warm_ms,
        "last_sentence_audio_ms": audio_ms,
        "sentence_answer_ms": stats(&answer),
        "sentence_handoff_ms": stats(&handoff),
        "sentence_total_ms": stats(&total),
        "reply_4_sentences_ms": stats(&reply),
    }))
}

/// A WAV the size of one spoken sentence (~3 s at 22.05 kHz) written, read
/// back and removed, as the Windows Piper path does with every sentence.
fn tempfs(temp: &Path, data_dir: &Path, rounds: usize) -> Value {
    let bytes = vec![7_u8; 132_344];
    let mut out = Map::new();
    for (name, dir) in [("temp_dir", temp), ("data_dir", data_dir)] {
        let (mut write, mut read, mut remove, mut whole) = (vec![], vec![], vec![], vec![]);
        for index in 0..rounds * 20 {
            let path = dir.join(format!("probe-{index}.wav"));
            let started = Instant::now();
            let _ = fs::write(&path, &bytes);
            let written = Instant::now();
            let _ = fs::read(&path);
            let was_read = Instant::now();
            let _ = fs::remove_file(&path);
            let done = Instant::now();
            write.push((written - started).as_secs_f64() * 1_000.0);
            read.push((was_read - written).as_secs_f64() * 1_000.0);
            remove.push((done - was_read).as_secs_f64() * 1_000.0);
            whole.push((done - started).as_secs_f64() * 1_000.0);
        }
        out.insert(
            name.into(),
            json!({
                "write_ms": stats(&write),
                "read_ms": stats(&read),
                "remove_ms": stats(&remove),
                "round_trip_ms": stats(&whole),
            }),
        );
    }
    Value::Object(out)
}

fn sqlite(data_dir: &Path, rounds: usize) -> Result<Value, String> {
    let path = data_dir.join("probe.sqlite3");
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let database = Database::open(&path).map_err(|error| error.to_string())?;
    let now = "2026-10-19T00:00:00Z";
    let message = |speaker, turn, content: &str| Message {
        id: uuid::Uuid::new_v4().to_string(),
        speaker,
        content: content.into(),
        turn,
        created_at: now.into(),
    };
    let session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        topic_id: "a2-city-or-village".into(),
        topic_label: "City or village".into(),
        status: "active".into(),
        started_at: now.into(),
        completed_at: None,
        messages: vec![],
    };
    database
        .create_session(
            &session,
            &message(Speaker::Ella, 0, "Hi!"),
            None,
            None,
            None,
            &LessonMark { ready: false, strengths: None },
        )
        .map_err(|error| error.to_string())?;
    let mut commit = vec![];
    for turn in 1..=(rounds * 10) as u32 {
        let learner = message(Speaker::Learner, turn, "I live in a small village near the river.");
        let ella = message(Speaker::Ella, turn, SENTENCES.join(" ").as_str());
        let started = Instant::now();
        database
            .persist_turn(&session.id, &learner, &ella, Some(2_400), None)
            .map_err(|error| error.to_string())?;
        commit.push(ms(started));
    }
    drop(database);
    Ok(json!({ "persist_turn_ms": stats(&commit) }))
}

fn spawn(config: &Config) -> Value {
    let mut out = Map::new();
    let piper = if cfg!(target_os = "windows") {
        config.engine_root.join("bin/piper/piper.exe")
    } else {
        config.engine_root.join("bin/piper/piper")
    };
    let mut programs: Vec<(&str, Command)> = vec![];
    if piper.is_file() {
        let mut command = Command::new(&piper);
        command.arg("--version");
        programs.push(("piper_version", command));
    }
    if cfg!(target_os = "windows") {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"]);
        programs.push(("powershell_exit", command));
        let mut command = Command::new("powershell");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Speech; exit 0",
        ]);
        programs.push(("powershell_add_type_speech", command));
    }
    for (name, mut command) in programs {
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let mut times = vec![];
        for _ in 0..config.rounds.max(3) {
            let started = Instant::now();
            if command.status().is_err() {
                break;
            }
            times.push(ms(started));
        }
        out.insert(name.into(), json!(stats(&times)));
    }
    Value::Object(out)
}

fn sapi(audio: &Path, rounds: usize) -> Result<Value, String> {
    let (samples, sample_rate) = read_wav(audio)?;
    let engine = WindowsStt::default();
    let mut times = vec![];
    let mut text = String::new();
    for _ in 0..rounds.max(1) {
        let started = Instant::now();
        match engine.transcribe(&samples, sample_rate) {
            Ok(result) => text = result.text,
            Err(error) => text = format!("error: {error}"),
        }
        times.push(ms(started));
    }
    Ok(json!({
        "audio_ms": samples.len() as f64 * 1_000.0 / sample_rate as f64,
        "transcribe_ms": stats(&times),
        "text": text,
    }))
}

fn read_wav(path: &Path) -> Result<(Vec<i16>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err(format!("{} must be mono 16-bit PCM", path.display()));
    }
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok((samples, spec.sample_rate))
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

fn stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    let round = |value: f64| (value * 100.0).round() / 100.0;
    json!({
        "n": sorted.len(),
        "min": round(sorted[0]),
        "p50": round(at(0.5)),
        "p95": round(at(0.95)),
        "max": round(sorted[sorted.len() - 1]),
        "mean": round(sorted.iter().sum::<f64>() / sorted.len() as f64),
    })
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config {
        engine_root: env::var_os("ELLA_ENGINE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("../engines")),
        voice: None,
        rounds: 5,
        audio: None,
        output: None,
        only: None,
        force_binary: false,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--engine-root" => config.engine_root = PathBuf::from(value()?),
            "--voice" => config.voice = Some(PathBuf::from(value()?)),
            "--rounds" => config.rounds = value()?.parse().map_err(|_| "--rounds takes a number")?,
            "--audio" => config.audio = Some(PathBuf::from(value()?)),
            "--output" => config.output = Some(PathBuf::from(value()?)),
            "--only" => config.only = Some(value()?.split(',').map(str::to_owned).collect()),
            "--force-binary" => config.force_binary = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if config.rounds == 0 {
        return Err("--rounds must be at least 1".into());
    }
    Ok(config)
}
