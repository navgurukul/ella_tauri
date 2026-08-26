use std::{env, fs, path::PathBuf, process::ExitCode};

use ella_tauri_lib::infrastructure::stt::{
    CanaryStt, SpeechToTextEngine, WhisperHttpStt, CANARY_FILE_NAME,
};
use serde_json::{json, Value};

struct Config {
    audio: PathBuf,
    canary_model: PathBuf,
    whisper_url: Option<String>,
    iterations: usize,
    warmup: usize,
    output: Option<PathBuf>,
    whisper_baseline_ms: f64,
    verify_sha256: bool,
    duration_ms: Option<u64>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("benchmark failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let config = parse_args()?;
    let (mut samples, sample_rate) = read_wav(&config.audio)?;
    if let Some(duration_ms) = config.duration_ms {
        let requested_samples = sample_rate as usize * duration_ms as usize / 1_000;
        if requested_samples > samples.len() {
            return Err(format!(
                "--duration-ms {duration_ms} exceeds the {:.1} ms fixture",
                samples.len() as f64 * 1_000.0 / sample_rate as f64
            ));
        }
        samples.truncate(requested_samples);
    }
    let duration_ms = samples.len() as f64 * 1_000.0 / sample_rate as f64;
    println!(
        "Fixture: {} ({duration_ms:.1} ms, {sample_rate} Hz mono)",
        config.audio.display()
    );

    let canary = CanaryStt::new(&config.canary_model, 0, config.verify_sha256);
    let canary_status = canary.status();
    if !canary_status.ready {
        return Err(canary_status.detail);
    }
    println!("Canary: {}", canary_status.detail);
    let canary_result = measure(
        "canary-180m-flash-q8_0",
        &canary,
        &samples,
        sample_rate,
        config.warmup,
        config.iterations,
    )?;

    let whisper_result = if let Some(base_url) = &config.whisper_url {
        let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
        let whisper = WhisperHttpStt::new(base_url.clone(), format!("{root}/inference"));
        Some(measure(
            "whisper-small-live",
            &whisper,
            &samples,
            sample_rate,
            config.warmup,
            config.iterations,
        )?)
    } else {
        None
    };

    let canary_median = canary_result["median_ms"].as_f64().unwrap_or_default();
    println!("\n--- comparison ---");
    println!("Canary median:                 {canary_median:8.1} ms");
    if let Some(whisper) = &whisper_result {
        println!(
            "Whisper small live median:      {:8.1} ms",
            whisper["median_ms"].as_f64().unwrap_or_default()
        );
    }
    println!(
        "Whisper fixed baseline median:   {:8.1} ms",
        config.whisper_baseline_ms
    );
    println!(
        "Canary vs fixed baseline:        {:8.1} ms ({:.1}x faster)",
        canary_median - config.whisper_baseline_ms,
        config.whisper_baseline_ms / canary_median.max(0.001)
    );

    let report = json!({
        "schema_version": 1,
        "fixture": config.audio,
        "audio_duration_ms": duration_ms,
        "iterations": config.iterations,
        "warmup": config.warmup,
        "canary": canary_result,
        "whisper_live": whisper_result,
        "whisper_fixed_baseline_ms": config.whisper_baseline_ms,
        "canary_delta_vs_fixed_baseline_ms": canary_median - config.whisper_baseline_ms,
        "canary_speedup_vs_fixed_baseline": config.whisper_baseline_ms / canary_median.max(0.001),
    });
    if let Some(output) = config.output {
        fs::write(
            &output,
            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|error| format!("write {}: {error}", output.display()))?;
        println!("JSON report: {}", output.display());
    }
    Ok(())
}

fn measure(
    name: &str,
    engine: &dyn SpeechToTextEngine,
    samples: &[i16],
    sample_rate: u32,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    for index in 0..warmup {
        let result = engine
            .transcribe(samples, sample_rate)
            .map_err(|error| error.to_string())?;
        println!("{name} warmup {}: {:.1} ms", index + 1, result.elapsed_ms);
    }
    let mut latencies = Vec::with_capacity(iterations);
    let mut transcript = String::new();
    let mut backend = String::new();
    for index in 0..iterations {
        let result = engine
            .transcribe(samples, sample_rate)
            .map_err(|error| error.to_string())?;
        println!("{name} run {}: {:.1} ms", index + 1, result.elapsed_ms);
        latencies.push(result.elapsed_ms);
        transcript = result.text;
        backend = result.backend;
    }
    latencies.sort_by(f64::total_cmp);
    let median = if latencies.len() % 2 == 0 {
        (latencies[latencies.len() / 2 - 1] + latencies[latencies.len() / 2]) / 2.0
    } else {
        latencies[latencies.len() / 2]
    };
    println!("{name} transcript: {transcript}");
    Ok(json!({
        "engine": name,
        "backend": backend,
        "latencies_ms": latencies,
        "median_ms": median,
        "transcript": transcript,
    }))
}

fn read_wav(path: &PathBuf) -> Result<(Vec<i16>, u32), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|error| format!("open WAV {}: {error}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err(format!(
            "{} must be mono 16-bit PCM WAV (got {} channels, {} bits)",
            path.display(),
            spec.channels,
            spec.bits_per_sample
        ));
    }
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("decode WAV {}: {error}", path.display()))?;
    Ok((samples, spec.sample_rate))
}

fn parse_args() -> Result<Config, String> {
    let engine_root = env::var("ELLA_ENGINE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../ella_app/build/engines")
        });
    let mut audio = None;
    let mut canary_model = engine_root.join("models/stt").join(CANARY_FILE_NAME);
    let mut whisper_url = None;
    let mut iterations = 5;
    let mut warmup = 1;
    let mut output = None;
    let mut whisper_baseline_ms = 3_387.0;
    let mut verify_sha256 = true;
    let mut duration_ms = None;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--audio" => audio = Some(PathBuf::from(value(&mut args, "--audio")?)),
            "--canary-model" => canary_model = PathBuf::from(value(&mut args, "--canary-model")?),
            "--whisper-url" => whisper_url = Some(value(&mut args, "--whisper-url")?),
            "--iterations" => {
                iterations = value(&mut args, "--iterations")?
                    .parse()
                    .map_err(|_| "--iterations must be an integer".to_string())?
            }
            "--warmup" => {
                warmup = value(&mut args, "--warmup")?
                    .parse()
                    .map_err(|_| "--warmup must be an integer".to_string())?
            }
            "--output" => output = Some(PathBuf::from(value(&mut args, "--output")?)),
            "--whisper-baseline-ms" => {
                whisper_baseline_ms = value(&mut args, "--whisper-baseline-ms")?
                    .parse()
                    .map_err(|_| "--whisper-baseline-ms must be a number".to_string())?
            }
            "--skip-sha256" => verify_sha256 = false,
            "--duration-ms" => {
                duration_ms = Some(
                    value(&mut args, "--duration-ms")?
                        .parse()
                        .map_err(|_| "--duration-ms must be an integer".to_string())?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "Usage: stt-benchmark --audio FILE [--canary-model FILE] [--whisper-url URL] \
                     [--iterations 5] [--warmup 1] [--output report.json] \
                     [--whisper-baseline-ms 3387] [--duration-ms 4214] [--skip-sha256]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}; use --help")),
        }
    }
    if iterations == 0 {
        return Err("--iterations must be at least 1".into());
    }
    Ok(Config {
        audio: audio.ok_or_else(|| "--audio is required".to_string())?,
        canary_model,
        whisper_url,
        iterations,
        warmup,
        output,
        whisper_baseline_ms,
        verify_sha256,
        duration_ms,
    })
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
