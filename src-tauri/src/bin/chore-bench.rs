//! End-to-end chore harness.
//!
//! Drives a scripted conversation through the real `AppService` — the same
//! `start_chore` / `send_text_turn` path the UI calls — against live
//! `llama-server` and Piper. It exists to answer three questions that unit
//! tests cannot:
//!
//!   * **Latency** — where the wall-clock actually goes, per turn and per stage.
//!   * **Flow** — does the ledger move, does the character concede sanely, does
//!     `[DEAL]` ever fire, how often does the limit get broken.
//!   * **Relevance** — the transcript is printed in full, because whether a
//!     reply is any good is a judgement no assertion makes for you.
//!
//! Text in, so the run is repeatable and STT variance is out of the numbers.
//! `stt-benchmark` already covers the audio front end.
//!
//! ```bash
//! npm run engines:local                     # llama-server + whisper sidecars
//! ELLA_ENGINE_MODE=local ELLA_ENGINE_ROOT="$PWD/engines" \
//!   ELLA_LLM_BASE_URL=http://127.0.0.1:39091/v1 \
//!   npm run bench:chore -- --chore market-cloth-price
//! ```

use std::{
    env, fs,
    path::PathBuf,
    process,
    sync::{Arc, Mutex},
    time::Instant,
};

use ella_tauri_lib::{
    application::{AppService, SpeechBroadcast},
    domain::{find_chore, SpeechStreamEvent, TurnSignal, WinCondition},
    infrastructure::{database::Database, engines::{engine_from_environment, EnginePaths}},
};
use serde_json::json;

/// Stands in for the window: records when each sentence would have reached the
/// speaker, which is the number sentence streaming exists to lower.
#[derive(Default)]
struct SegmentRecorder {
    ready_ms: Mutex<Vec<f64>>,
    /// One line per sentence describing its word timings, so the highlight the
    /// window will draw can be read off a bench run.
    words: Mutex<Vec<String>>,
}

impl SegmentRecorder {
    /// Milliseconds into the generation that the first sentence became
    /// audible, and how many sentences the turn produced.
    fn take(&self) -> (Option<f64>, usize) {
        let mut ready = self
            .ready_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let taken = std::mem::take(&mut *ready);
        (taken.first().copied(), taken.len())
    }

    fn take_words(&self) -> Vec<String> {
        let mut words = self
            .words
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *words)
    }
}

impl SpeechBroadcast for SegmentRecorder {
    fn speak(&self, event: SpeechStreamEvent) {
        self.ready_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event.ready_ms);
        let spans = event
            .words
            .iter()
            .map(|word| format!("{}@{:.0}", word.text, word.start_ms))
            .collect::<Vec<_>>()
            .join(" ");
        self.words
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(spans);
    }
}

/// One learner utterance and what it is for, so a failed run reads as a story
/// rather than a list of strings.
struct ScriptLine {
    intent: &'static str,
    text: &'static str,
}

/// A haggling script that tries the things a real learner would: an opening
/// ask, a reason, a counter, a lowball under the floor (which the ledger must
/// refuse), and a close.
const LEDGER_DOWN_SCRIPT: &[ScriptLine] = &[
    ScriptLine { intent: "open", text: "Hello. How much is this shirt?" },
    ScriptLine { intent: "first push", text: "That is too much for me. Can you make it less?" },
    ScriptLine { intent: "give a reason", text: "I am a student and I only have a little money today." },
    ScriptLine { intent: "counter", text: "Can you do it for four hundred and fifty rupees?" },
    ScriptLine { intent: "lowball under the floor", text: "I will give you one hundred rupees, final." },
    ScriptLine { intent: "reasonable close", text: "Okay, four hundred rupees and I will buy it right now." },
    ScriptLine { intent: "confirm", text: "So we agree? Four hundred rupees." },
];

const LEDGER_UP_SCRIPT: &[ScriptLine] = &[
    ScriptLine { intent: "open", text: "Hello sir. I moved out last week. Can I get my deposit back?" },
    ScriptLine { intent: "ask for all of it", text: "The flat was clean when I left. I would like the full five thousand." },
    ScriptLine { intent: "concede a little", text: "I understand there is some cleaning. What can you return?" },
    ScriptLine { intent: "specific point", text: "I painted the bedroom myself before leaving, so there is no paint cost." },
    ScriptLine { intent: "overreach past the ceiling", text: "Then give me six thousand rupees." },
    ScriptLine { intent: "reasonable close", text: "Three thousand five hundred rupees and we are finished." },
    // The down script has always confirmed at the end; without the same line
    // here the run stopped on the landlord's offer and scored a chore that
    // reached its target as a failure, because nobody ever shook on it.
    ScriptLine { intent: "confirm", text: "Yes, three thousand five hundred is fine. Thank you, sir." },
];

const RUBRIC_SCRIPT: &[ScriptLine] = &[
    ScriptLine { intent: "open", text: "Hello. Can I show you this pen?" },
    ScriptLine { intent: "ask about needs", text: "What do you write with mostly, at work?" },
    ScriptLine { intent: "name a benefit", text: "This pen never leaks in your pocket, so your shirt stays clean." },
    ScriptLine { intent: "handle the objection", text: "It costs a little more, but it lasts three times longer than a cheap one." },
    ScriptLine { intent: "close", text: "So shall I pack one for you?" },
];

fn script_for(chore_id: &str) -> &'static [ScriptLine] {
    match chore_id {
        "deposit-refund" => LEDGER_UP_SCRIPT,
        "sell-me-a-pen" => RUBRIC_SCRIPT,
        _ => LEDGER_DOWN_SCRIPT,
    }
}

struct Options {
    chore_id: String,
    /// Set instead of `chore_id` to drive a free conversation on a topic. That
    /// is the path the app takes today, and the only one where sentences reach
    /// the speaker live — a ledger chore holds them until the figure is checked.
    topic_id: Option<String>,
    learner_name: String,
    output: Option<PathBuf>,
    max_turns: usize,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        chore_id: "market-cloth-price".into(),
        topic_id: None,
        learner_name: "Souvik".into(),
        output: None,
        max_turns: usize::MAX,
    };
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = |flag: &str| -> Result<String, String> {
            args.next()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--chore" => options.chore_id = value("--chore")?,
            "--topic" => options.topic_id = Some(value("--topic")?),
            "--name" => options.learner_name = value("--name")?,
            "--output" => options.output = Some(PathBuf::from(value("--output")?)),
            "--max-turns" => {
                options.max_turns = value("--max-turns")?
                    .parse()
                    .map_err(|_| "--max-turns needs a number".to_string())?
            }
            "--list" => {
                for chore in ella_tauri_lib::domain::chores() {
                    println!(
                        "{:<24} {:<12} {}",
                        chore.id,
                        chore.track.as_str(),
                        chore.title
                    );
                }
                process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "chore-bench [--chore <id> | --topic <id>] [--name <learner>] \
                     [--max-turns <n>] [--output <file.json>] [--list]"
                );
                process::exit(0);
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(options)
}

fn main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("chore-bench: {error}");
            process::exit(2);
        }
    };
    if let Err(error) = run(&options) {
        eprintln!("\nchore-bench failed: {error}");
        process::exit(1);
    }
}

/// What a learner actually says on a free topic: short answers, a question
/// back, a longer story. Enough turns for a median that means something.
const TOPIC_SCRIPT: &[ScriptLine] = &[
    ScriptLine { intent: "short answer", text: "I ate bhel puri near my college yesterday." },
    ScriptLine { intent: "detail", text: "It was spicy and a little sweet, with onion and green chutney on top." },
    ScriptLine { intent: "question back", text: "Have you ever eaten bhel puri? What did you think of it?" },
    ScriptLine { intent: "longer story", text: "My mother used to make it at home on Sunday evenings, and my sister and I would eat it on the balcony while it was still light outside." },
    ScriptLine { intent: "opinion", text: "I think the street version tastes better than the one in restaurants." },
    ScriptLine { intent: "close", text: "Next time I go, I will try the one with extra lemon." },
];

/// Builds the throwaway database and live engine both harness modes need.
fn open_service(learner_name: &str) -> Result<AppService, String> {
    let scratch = env::temp_dir().join("ella-chore-bench.sqlite3");
    let _ = fs::remove_file(&scratch);
    let _ = fs::remove_file(scratch.with_extension("sqlite3-wal"));
    let _ = fs::remove_file(scratch.with_extension("sqlite3-shm"));
    let database = Database::open(&scratch).map_err(|error| error.to_string())?;
    let engine = engine_from_environment(EnginePaths::default());
    let status = engine.status();
    println!("engine: {} ({})", status.label, status.mode);
    for component in &status.components {
        println!(
            "  {:<22} {}  {}",
            component.name,
            if component.ready { "ready" } else { "DOWN " },
            component.detail
        );
    }
    if !status.ready {
        return Err(
            "engines are not ready. Start them with `npm run engines:local` and set \
             ELLA_ENGINE_MODE=local ELLA_ENGINE_ROOT=$PWD/engines \
             ELLA_LLM_BASE_URL=http://127.0.0.1:39091/v1"
                .into(),
        );
    }
    let service = AppService::new(database, engine);
    service
        .save_learner(learner_name, Some(22))
        .map_err(|error| error.to_string())?;
    Ok(service)
}

/// Free conversation on a topic, measuring the thing the learner actually
/// waits for: not when the turn returns, but when Ella starts talking.
fn run_topic(options: &Options, topic_id: &str) -> Result<(), String> {
    let service = open_service(&options.learner_name)?;
    let recorder = Arc::new(SegmentRecorder::default());
    service.set_speech_broadcast(recorder.clone());

    let session = service
        .start_session(topic_id)
        .map_err(|error| error.to_string())?;
    println!("\ntopic   {}  [{}]", session.topic_label, session.topic_id);
    println!("\n─── conversation ───");
    if let Some(opening) = session.messages.first() {
        println!("  ella     {}", opening.content);
    }
    // The opening is spoken through the same pipeline as a reply, so it should
    // stream and carry timings too.
    let opening_started = Instant::now();
    match service.speak_opening(&session.id) {
        Ok(line) => {
            let (first_ms, segments) = recorder.take();
            println!(
                "  opening  {} sentence(s), {} word timings, whole line ready in {:.0}ms{}",
                segments,
                line.speech_words.len(),
                opening_started.elapsed().as_secs_f64() * 1_000.0,
                first_ms
                    .map(|ms| format!(" | starts talking at {ms:.0}ms"))
                    .unwrap_or_default(),
            );
            for line in recorder.take_words() {
                println!("  words    {line}");
            }
        }
        Err(error) => println!("  opening  could not be spoken: {error}"),
    }

    let mut turns = Vec::new();
    for (index, line) in TOPIC_SCRIPT.iter().take(options.max_turns).enumerate() {
        let started = Instant::now();
        let result = service
            .send_text_turn(&session.id, line.text)
            .map_err(|error| format!("turn {} failed: {error}", index + 1))?;
        let total_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let timings = result.timings.as_ref();
        // Only the opening streams now; a reply is released whole, so the
        // recorder stays empty here and `first_speech_ms` reads as "-".
        let (first_segment_ms, segments) = recorder.take();
        // Both clocks start within a millisecond of each other at the top of
        // the generation, so they compare directly: how long until Ella starts
        // talking, against how long until she has finished writing.
        let completion_ms = timings.and_then(|t| t.llm_completion_ms).unwrap_or(0) as f64;
        let saved = first_segment_ms.map(|ms| completion_ms - ms);

        println!("\n  learner  {}", line.text);
        println!("           ({})", line.intent);
        println!("  ella     {}", result.ella_message.content);
        println!(
            "  timing   total {total_ms:.0}ms | ttft {}ms | reply written at {completion_ms:.0}ms | \
             {segments} sentence(s)",
            timings.and_then(|t| t.llm_ttft_ms).unwrap_or(0),
        );
        println!(
            "  speech   {}",
            match (first_segment_ms, saved) {
                (Some(ms), Some(saved)) => format!(
                    "starts talking at {ms:.0}ms \u{2014} {saved:.0}ms before the reply is written",
                ),
                _ => "nothing streamed".into(),
            }
        );
        for line in recorder.take_words() {
            println!("  words    {line}");
        }

        turns.push(json!({
            "turn": index + 1,
            "intent": line.intent,
            "learner": line.text,
            "reply": result.ella_message.content,
            "total_ms": total_ms.round(),
            "llm_ttft_ms": timings.and_then(|t| t.llm_ttft_ms),
            "llm_completion_ms": timings.and_then(|t| t.llm_completion_ms),
            "first_speech_ms": first_segment_ms.map(|ms| ms.round()),
            "saved_ms": saved.map(|ms| ms.round()),
        }));
    }

    let totals: Vec<f64> = turns
        .iter()
        .filter_map(|turn| turn["total_ms"].as_f64())
        .collect();
    let first_speech: Vec<f64> = turns
        .iter()
        .filter_map(|turn| turn["first_speech_ms"].as_f64())
        .collect();
    let written: Vec<f64> = turns
        .iter()
        .filter_map(|turn| turn["llm_completion_ms"].as_f64())
        .collect();
    let saved: Vec<f64> = turns
        .iter()
        .filter_map(|turn| turn["saved_ms"].as_f64())
        .collect();

    println!("\n─── result ───");
    println!("turns                    {}", turns.len());
    println!("median turn              {:.0} ms", median(&totals));
    println!("median reply written at  {:.0} ms", median(&written));
    if first_speech.is_empty() {
        println!("median time to speech    - (no sentence streamed; is the resident Piper up?)");
    } else {
        println!(
            "median time to speech    {:.0} ms   ({} of {} turns streamed)",
            median(&first_speech),
            first_speech.len(),
            turns.len()
        );
        println!(
            "median head start        {:.0} ms   (Ella starts talking this much earlier)",
            median(&saved)
        );

    }

    if let Some(path) = &options.output {
        let report = json!({
            "mode": "topic",
            "topic_id": topic_id,
            "turns": turns,
            "median_total_ms": median(&totals).round(),
            "median_first_speech_ms": (!first_speech.is_empty())
                .then(|| median(&first_speech).round()),
        });
        fs::write(path, serde_json::to_string_pretty(&report).unwrap())
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        println!("\nreport written to {}", path.display());
    }
    Ok(())
}

fn run(options: &Options) -> Result<(), String> {
    if let Some(topic_id) = &options.topic_id {
        return run_topic(options, topic_id);
    }
    let chore = find_chore(&options.chore_id)
        .ok_or_else(|| format!("no chore called {} (try --list)", options.chore_id))?;

    // A throwaway database per run, so attempts and ledgers never accumulate
    // across benchmarks. `Database::in_memory` is test-gated, so this uses a
    // scratch file and removes it on the way in.
    let scratch = env::temp_dir().join("ella-chore-bench.sqlite3");
    let _ = fs::remove_file(&scratch);
    let _ = fs::remove_file(scratch.with_extension("sqlite3-wal"));
    let _ = fs::remove_file(scratch.with_extension("sqlite3-shm"));
    let database = Database::open(&scratch).map_err(|error| error.to_string())?;
    let engine = engine_from_environment(EnginePaths::default());
    let status = engine.status();
    println!("engine: {} ({})", status.label, status.mode);
    for component in &status.components {
        println!(
            "  {:<22} {}  {}",
            component.name,
            if component.ready { "ready" } else { "DOWN " },
            component.detail
        );
    }
    if !status.ready {
        return Err(
            "engines are not ready. Start them with `npm run engines:local` and set \
             ELLA_ENGINE_MODE=local ELLA_ENGINE_ROOT=$PWD/engines \
             ELLA_LLM_BASE_URL=http://127.0.0.1:39091/v1"
                .into(),
        );
    }

    let service = AppService::new(database, engine);
    service
        .save_learner(&options.learner_name, Some(22))
        .map_err(|error| error.to_string())?;

    println!("\nchore   {}  [{}]", chore.title, chore.id);
    println!("setting {}", chore.setting);
    println!("goal    {}", chore.learner_goal);
    match &chore.win {
        WinCondition::Ledger(spec) => println!(
            "ledger  {} opening {} -> target {} (limit {}, max step {}, {:?})",
            spec.unit, spec.opening, spec.target, spec.limit, spec.max_step, spec.direction
        ),
        WinCondition::Rubric { criteria, pass_at } => println!(
            "rubric  {} of {} criteria to pass",
            pass_at,
            criteria.len()
        ),
    }

    // ---- session start (off any turn clock, but worth knowing) ----
    let opening_started = Instant::now();
    let session = service
        .start_chore(&chore.id)
        .map_err(|error| error.to_string())?;
    let opening_ms = opening_started.elapsed().as_secs_f64() * 1_000.0;
    let opening_text = session
        .messages
        .first()
        .map(|message| message.content.clone())
        .unwrap_or_default();
    println!("\n─── conversation ───");
    println!("  {:>7}  {} {}", format!("{opening_ms:.0}ms"), chore.character_id, opening_text);

    // ---- turns ----
    let script = script_for(&chore.id);
    let mut turns = Vec::new();
    let mut ended_with = None;

    for (index, line) in script.iter().take(options.max_turns).enumerate() {
        let started = Instant::now();
        let result = match service.send_text_turn(&session.id, line.text) {
            Ok(result) => result,
            Err(error) => {
                println!("  turn {} failed: {error}", index + 1);
                return Err(format!("turn {} failed: {error}", index + 1));
            }
        };
        let total_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let timings = result.timings.as_ref();
        let ttft = timings.and_then(|t| t.llm_ttft_ms).unwrap_or(0);
        let completion = timings.and_then(|t| t.llm_completion_ms).unwrap_or(0);
        let tts_first = timings.and_then(|t| t.tts_first_audio_ms);

        println!("\n  learner  {}", line.text);
        println!("           ({})", line.intent);
        println!("  {:<8} {}", chore.character_id, result.ella_message.content);

        if let Some(ledger) = &result.ledger {
            println!(
                "  ledger   {} {} → target {} | {:.0}% {}{}",
                ledger.unit,
                ledger.current,
                ledger.target,
                ledger.progress * 100.0,
                if ledger.agreed { "AGREED " } else { "" },
                if ledger.regenerated { "[regenerated]" } else { "" },
            );
        }
        println!(
            "  timing   total {total_ms:.0}ms | ttft {ttft}ms | llm {completion}ms | tts first {}",
            tts_first
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "-".into())
        );

        if let Some(signal) = result.signal {
            ended_with = Some(signal);
        }

        turns.push(json!({
            "turn": index + 1,
            "intent": line.intent,
            "learner": line.text,
            "reply": result.ella_message.content,
            "total_ms": total_ms.round(),
            "llm_ttft_ms": ttft,
            "llm_completion_ms": completion,
            "tts_first_audio_ms": tts_first,
            "tts_completion_ms": timings.and_then(|t| t.tts_completion_ms),
            "ledger": result.ledger.as_ref().map(|ledger| json!({
                "current": ledger.current,
                "target": ledger.target,
                "progress": ledger.progress,
                "agreed": ledger.agreed,
                "reached_target": ledger.reached_target,
                "regenerated": ledger.regenerated,
            })),
            "signal": result.signal.map(|signal| match signal {
                TurnSignal::Deal => "deal",
                TurnSignal::Walk => "walk",
            }),
            "audio_bytes": result.audio.as_ref().map(|audio| audio.base64.len()),
        }));

        if ended_with.is_some() {
            println!("\n  character ended the conversation: {:?}", ended_with.unwrap());
            break;
        }
    }

    // ---- summary ----
    let totals: Vec<f64> = turns
        .iter()
        .filter_map(|turn| turn["total_ms"].as_f64())
        .collect();
    let ttfts: Vec<f64> = turns
        .iter()
        .filter_map(|turn| turn["llm_ttft_ms"].as_f64())
        .collect();
    let regenerations = turns
        .iter()
        .filter(|turn| turn["ledger"]["regenerated"] == json!(true))
        .count();
    let last_ledger = turns
        .iter()
        .rev()
        .find_map(|turn| turn["ledger"].as_object().cloned());

    println!("\n─── result ───");
    println!("turns              {}", turns.len());
    println!("median turn        {:.0} ms", median(&totals));
    println!("slowest turn       {:.0} ms", totals.iter().cloned().fold(0.0, f64::max));
    println!("median ttft        {:.0} ms", median(&ttfts));
    println!("ledger breaks      {regenerations} (regenerated)");

    let outcome = match (&chore.win, &last_ledger) {
        (WinCondition::Ledger(_), Some(ledger)) => {
            let reached = ledger["reached_target"] == json!(true);
            let agreed = ledger["agreed"] == json!(true);
            println!(
                "final figure       {} (target reached: {reached}, agreed: {agreed})",
                ledger["current"]
            );
            if reached && agreed {
                "passed"
            } else {
                "failed"
            }
        }
        (WinCondition::Rubric { .. }, _) => {
            println!("rubric chore       needs the grading pass (phase 2) to decide");
            "ungraded"
        }
        _ => "ungraded",
    };
    println!("outcome            {outcome}");

    if outcome == "failed" {
        println!(
            "\nnote: a failed chore is a valid result, not a harness error — it is what a\n\
             learner who did not get there would see. Check the transcript above for whether\n\
             the character conceded sensibly or just stonewalled."
        );
    }

    if let Some(path) = &options.output {
        let report = json!({
            "schema_version": 1,
            "chore": chore.id,
            "title": chore.title,
            "learner": options.learner_name,
            "opening_ms": opening_ms.round(),
            "opening": opening_text,
            "turns": turns,
            "median_turn_ms": median(&totals),
            "median_ttft_ms": median(&ttfts),
            "ledger_breaks": regenerations,
            "outcome": outcome,
        });
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, serde_json::to_string_pretty(&report).unwrap())
            .map_err(|error| error.to_string())?;
        println!("\nwrote {}", path.display());
    }

    Ok(())
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}
