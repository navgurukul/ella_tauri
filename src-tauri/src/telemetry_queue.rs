//! The upload queue: what of `latency.jsonl` has left the laptop, in
//! batches the telemetry policy allows (`telemetry_policy`), and what was
//! lost on the way.
//!
//! The log is never changed. A cursor in `telemetry/upload/state.json` says
//! how far into it the queue has read; a batch is made from the lines after
//! it, frozen in `upload/outbox/<batch_id>.jsonl`, and sent until a
//! `Transport` acknowledges it. The state file keeps, in one atomic write,
//! the install's id, the cursor, the batch being sent, the last batch
//! acknowledged, and the counts of what was dropped: so a crash at any point
//! sends the same batch again, or makes it again from the same lines, and
//! the team's store, which keeps each `batch_id` and `event_id` once, counts
//! it once.
//!
//! Off unless `ELLA_TELEMETRY_EXPORT=1`, and then it only exports: each batch
//! is written to `telemetry/export/`, for a tester to hand over. Nothing is
//! sent over a network here; a relay would be another `Transport`.
//!
//! A new install starts at the end of the log: what was there before the
//! queue first ran stays on the laptop.

use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write as _},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, RecvTimeoutError, SyncSender},
        OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    telemetry,
    telemetry_policy::{self as policy, Body, Reasons, Sanitizer, SendFields},
};

/// When the queue looks at the log, and how much it sends at a time. The app
/// uses `Config::default()`, fixed; tests make their own, shorter ones.
/// There is no setting or environment variable for any of it.
///
/// Two clocks, kept apart: `idle` says when the queue may work at all (no
/// turn for 2 minutes), and says nothing of talks; `talk_quiet` says when a
/// talk with no end recorded counts as over, for `due_talks`. A pause in a
/// talk longer than `idle` lets the queue look, never ends the talk.
#[derive(Debug, Clone)]
pub struct Config {
    /// The most of the log one batch is made from. A longer line goes alone.
    pub batch_bytes: u64,
    /// A batch is due once this many talks are over,
    pub due_talks: usize,
    /// or the oldest event waiting is this old,
    pub due_age: TimeDelta,
    /// or this much of the log is waiting.
    pub due_bytes: u64,
    /// A talk with no turn for this long is over, for counting talks only.
    pub talk_quiet: TimeDelta,
    /// The most of the log that waits: past it, or past `cap_age`, the
    /// oldest lines are dropped, and counted (`queue_full`).
    pub cap_bytes: u64,
    pub cap_age: TimeDelta,
    /// The queue works only once no turn has been written for this long.
    pub idle: Duration,
    /// And looks at the log this often anyway.
    pub every: Duration,
    /// A batch that could not be sent waits this long, doubling each time,
    /// up to `backoff_max`, a fifth either way.
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// The most batches one look sends.
    pub batches_per_run: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            batch_bytes: 256 * 1024,
            due_talks: 3,
            due_age: TimeDelta::days(3),
            due_bytes: 256 * 1024,
            talk_quiet: TimeDelta::minutes(30),
            cap_bytes: 32 * 1024 * 1024,
            cap_age: TimeDelta::days(180),
            idle: Duration::from_secs(120),
            every: Duration::from_secs(30 * 60),
            backoff_min: Duration::from_secs(60),
            backoff_max: Duration::from_secs(6 * 60 * 60),
            batches_per_run: 20,
        }
    }
}

/// A line longer than this is never a whole event; it is skipped.
const MAX_LINE: usize = 1024 * 1024;
/// The bytes of the log's start the cursor remembers, to tell a log that
/// was replaced from one that grew.
const HEAD: u64 = 256;
/// Batches a transport refused, kept for a person to look at.
const REFUSED_KEPT: usize = 10;

// Where a batch goes.

/// Where a batch goes once it is made. Phase 2 has one, `ExportTransport`.
pub trait Transport: Send {
    fn send(&self, batch: &[u8], batch_id: &str) -> Result<Ack, SendError>;
}

// Export only ever stores; a relay gives the other answers.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    Stored,
    /// Kept already, from a try whose answer was lost.
    Duplicate,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// Tried again later.
    Unreachable,
    Failed(String),
    /// Never tried again: the batch is put aside.
    Refused(String),
}

/// Writes each batch to a folder, `<batch_id>.jsonl`, for a tester to hand
/// over: what `--validate --data` takes in.
pub struct ExportTransport {
    pub dir: PathBuf,
}

impl Transport for ExportTransport {
    fn send(&self, batch: &[u8], batch_id: &str) -> Result<Ack, SendError> {
        fs::create_dir_all(&self.dir)
            .and_then(|()| write_atomically(&self.dir.join(format!("{batch_id}.jsonl")), batch))
            .map(|()| Ack::Stored)
            .map_err(|error| SendError::Failed(error.kind().to_string()))
    }
}

// The state.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub state_version: u32,
    pub install_id: String,
    pub cursor: Cursor,
    pub batch_seq_acked: u64,
    pub pending: Option<Pending>,
    pub health: Health,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub backoff_s: u64,
}

/// How far into the log the queue has read: every line before `offset` is in
/// a batch, or was dropped. `head_hash` is of the log's first `head_len`
/// bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub offset: u64,
    pub head_len: u64,
    pub head_hash: String,
}

/// The batch being sent, frozen in the outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub batch_id: String,
    pub batch_seq: u64,
    pub events: u64,
    pub prepared_at: String,
    pub first_event_at: Option<String>,
    pub attempts: u64,
    pub first_attempt_at: Option<String>,
    /// The totals its body counted up to: acknowledged with it.
    pub dropped_total: Reasons,
    pub log_resets_total: u64,
    /// The log's bytes it was made from, and the lines among them that were
    /// no event: to make it again, should its outbox file be lost.
    pub start: u64,
    pub end: u64,
    pub unreadable: u64,
}

/// What the queue lost, counted since the install began and never reset. A
/// batch carries what was lost since the last batch acknowledged: its total
/// less `acked_through`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub dropped_total: Reasons,
    pub log_resets_total: u64,
    pub acked_through: Acked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acked {
    pub dropped: Reasons,
    pub log_resets: u64,
}

impl State {
    fn whole(&self) -> bool {
        let hex = |text: &str, len| text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        self.state_version == 1
            && hex(&self.install_id, 16)
            && self.pending.as_ref().is_none_or(|pending| hex(&pending.batch_id, 32))
    }
}

/// What one look at the log did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Another copy of Ella had the queue: nothing was done.
    pub locked: bool,
    pub sent: u32,
    /// A batch is waiting to be tried again then.
    pub retry_at: Option<DateTime<Utc>>,
}

pub struct Queue {
    log: PathBuf,
    dir: PathBuf,
    config: Config,
    transport: Box<dyn Transport>,
}

impl Queue {
    /// The queue of the log in `telemetry_dir`, its state beside it in `upload/`.
    pub fn new(telemetry_dir: &Path, config: Config, transport: Box<dyn Transport>) -> Self {
        Self {
            log: telemetry_dir.join("latency.jsonl"),
            dir: telemetry_dir.join("upload"),
            config,
            transport,
        }
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    fn outbox(&self) -> PathBuf {
        self.dir.join("outbox")
    }

    fn outbox_file(&self, batch_id: &str) -> PathBuf {
        self.outbox().join(format!("{batch_id}.jsonl"))
    }

    /// One look at the log: send the batch waiting, or make and send new
    /// ones once they are due. `write_failures`, the events the log could
    /// not take since the last look, are counted when this returns `Ok`
    /// and not `locked`; else the caller keeps them for the next look.
    pub fn run(&self, now: DateTime<Utc>, write_failures: u64) -> io::Result<Outcome> {
        fs::create_dir_all(self.outbox())?;
        let lock = File::options().create(true).truncate(false).write(true).open(self.dir.join("upload.lock"))?;
        if lock.try_lock().is_err() {
            return Ok(Outcome { locked: true, ..Outcome::default() });
        }
        let mut state = self.load(now)?;
        state.health.dropped_total.write_failed += write_failures;
        self.check_log(&mut state)?;
        self.cap(&mut state, now)?;
        self.clear_orphans(&state);
        self.save(&state)?;

        let mut outcome = Outcome::default();
        let mut made = false;
        for _ in 0..self.config.batches_per_run {
            if state.pending.is_some() {
                match self.send(&mut state, now)? {
                    Sent::Stored => outcome.sent += 1,
                    // Made again at once, due or not.
                    Sent::Lost => made = true,
                    Sent::Waiting(at) => {
                        outcome.retry_at = Some(at);
                        break;
                    }
                    Sent::Refused => break,
                }
            } else if (made || outcome.sent > 0 || self.due(&state, now)?) && self.prepare(&mut state, now)? {
                // Once a batch is due, the rest of the log follows it.
                made = true;
            } else {
                break;
            }
        }
        Ok(outcome)
    }

    fn load(&self, now: DateTime<Utc>) -> io::Result<State> {
        let path = self.state_path();
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<State>(&bytes) {
                Ok(state) if state.whole() => Ok(state),
                _ => {
                    // Unreadable: kept for a person to look at, and the
                    // laptop starts again as a new install.
                    let aside = self.dir.join(format!("state.json.corrupt-{}", now.format("%Y%m%dT%H%M%SZ")));
                    eprintln!("[TELEMETRY] the upload state could not be read; kept as {}", aside.display());
                    fs::rename(&path, aside)?;
                    self.new_install()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.new_install(),
            Err(error) => Err(error),
        }
    }

    /// A new install's state: a new id, and the cursor at the end of the
    /// log's whole lines.
    fn new_install(&self) -> io::Result<State> {
        let (offset, head_len, head_hash) = match File::open(&self.log) {
            Ok(mut file) => {
                let len = file.metadata()?.len();
                let head_len = len.min(HEAD);
                (whole_lines_end(&mut file, len)?, head_len, head_of(&mut file, head_len)?)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (0, 0, hash16(b"")),
            Err(error) => return Err(error),
        };
        Ok(State {
            state_version: 1,
            install_id: new_install_id(),
            cursor: Cursor { offset, head_len, head_hash },
            batch_seq_acked: 0,
            pending: None,
            health: Health::default(),
            next_attempt_at: None,
            backoff_s: 0,
        })
    }

    fn save(&self, state: &State) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(state).expect("the state serializes");
        write_atomically(&self.state_path(), &bytes)
    }

    fn log_len(&self) -> io::Result<u64> {
        match fs::metadata(&self.log) {
            Ok(meta) => Ok(meta.len()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error),
        }
    }

    fn open_log(&self) -> io::Result<Option<File>> {
        match File::open(&self.log) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A log shorter than the cursor, or whose start is not the one the
    /// cursor knew, was replaced or cut: it is read again from its start,
    /// and counted. Its events that were sent already are the same events,
    /// with the same ids, and are kept once.
    fn check_log(&self, state: &mut State) -> io::Result<()> {
        let len = self.log_len()?;
        let cursor = &mut state.cursor;
        let mut file = self.open_log()?;
        let same_head = match file.as_mut() {
            Some(file) if len >= cursor.head_len => head_of(file, cursor.head_len)? == cursor.head_hash,
            None => cursor.head_len == 0,
            Some(_) => false,
        };
        if len < cursor.offset || !same_head {
            state.health.log_resets_total += 1;
            cursor.offset = 0;
            cursor.head_len = 0;
            cursor.head_hash = hash16(b"");
            eprintln!("[TELEMETRY] latency.jsonl was replaced or cut; reading it from its start");
        }
        if let Some(file) = file.as_mut() {
            if cursor.head_len < HEAD && len > cursor.head_len {
                cursor.head_len = len.min(HEAD);
                cursor.head_hash = head_of(file, cursor.head_len)?;
            }
        }
        Ok(())
    }

    /// Drops the oldest lines waiting while more than `cap_bytes` waits, or
    /// while they are older than `cap_age`, and counts them.
    fn cap(&self, state: &mut State, now: DateTime<Utc>) -> io::Result<()> {
        let Some(file) = self.open_log()? else { return Ok(()) };
        let len = file.metadata()?.len();
        if len.saturating_sub(state.cursor.offset) <= self.config.cap_bytes
            && !self.oldest_waiting(state)?.is_some_and(|at| at < micros(now - self.config.cap_age))
        {
            return Ok(());
        }
        let cutoff = micros(now - self.config.cap_age);
        let mut lines = Lines::new(file, state.cursor.offset)?;
        let mut dropped = 0;
        while let Some(line) = lines.next_line()? {
            let too_big = len.saturating_sub(line.start) > self.config.cap_bytes;
            let too_old = line.event().as_ref().and_then(time_of).is_some_and(|at| at < cutoff);
            if !too_big && !too_old {
                break;
            }
            dropped += u64::from(!line.blank());
            state.cursor.offset = line.end;
        }
        if dropped > 0 {
            eprintln!("[TELEMETRY] the queue was full: {dropped} of its oldest events dropped");
        }
        state.health.dropped_total.queue_full += dropped;
        Ok(())
    }

    fn oldest_waiting(&self, state: &State) -> io::Result<Option<i64>> {
        let Some(file) = self.open_log()? else { return Ok(None) };
        let mut lines = Lines::new(file, state.cursor.offset)?;
        while let Some(line) = lines.next_line()? {
            if let Some(at) = line.event().as_ref().and_then(time_of) {
                return Ok(Some(at));
            }
        }
        Ok(None)
    }

    /// Outbox files that are no batch waiting: made by a look that stopped
    /// before it kept its state, or sent and acknowledged already.
    fn clear_orphans(&self, state: &State) {
        let waiting = state.pending.as_ref().map(|pending| format!("{}.jsonl", pending.batch_id));
        let Ok(entries) = fs::read_dir(self.outbox()) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if entry.path().is_file() && waiting.as_deref() != name.to_str() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// Whether what waits is worth a batch: `due_talks` talks over, the
    /// oldest event `due_age` old, or `due_bytes` of the log.
    fn due(&self, state: &State, now: DateTime<Utc>) -> io::Result<bool> {
        let Some(file) = self.open_log()? else { return Ok(false) };
        let len = file.metadata()?.len();
        if len.saturating_sub(state.cursor.offset) >= self.config.due_bytes {
            return Ok(true);
        }
        let now = micros(now);
        let mut lines = Lines::new(file, state.cursor.offset)?;
        let mut oldest_seen = false;
        // A talk's last turn, and whether it is over.
        let mut talks: HashMap<String, (i64, bool)> = HashMap::new();
        while let Some(line) = lines.next_line()? {
            let Some(event) = line.event() else { continue };
            let Some(at) = time_of(&event) else { continue };
            if !oldest_seen {
                oldest_seen = true;
                if now - at > self.config.due_age.num_microseconds().unwrap_or(i64::MAX) {
                    return Ok(true);
                }
            }
            let Some(talk) = event.get("session_id").and_then(Value::as_str) else { continue };
            let entry = talks.entry(talk.to_owned()).or_insert((at, false));
            match event.get("event").and_then(Value::as_str) {
                Some("ella_turn_latency") => {
                    entry.0 = entry.0.max(at);
                    entry.1 |= event.get("talk_over") == Some(&Value::Bool(true));
                }
                Some("ella_assessment") => entry.1 = true,
                _ => {}
            }
        }
        let quiet = self.config.talk_quiet.num_microseconds().unwrap_or(i64::MAX);
        let over = talks.values().filter(|(last, over)| *over || now - last > quiet).count();
        Ok(over >= self.config.due_talks)
    }

    /// Makes a batch of the lines after the cursor, freezes it in the outbox,
    /// and keeps it as the batch waiting, the cursor after its lines. False
    /// when no whole line waits.
    fn prepare(&self, state: &mut State, now: DateTime<Utc>) -> io::Result<bool> {
        let Some(file) = self.open_log()? else { return Ok(false) };
        let mut lines = Lines::new(file, state.cursor.offset)?;
        let mut sanitizer = Sanitizer::default();
        let mut events = Vec::new();
        let mut ids = std::collections::HashSet::new();
        let (start, mut end, mut read, mut unreadable) = (state.cursor.offset, state.cursor.offset, 0, 0);
        while read < self.config.batch_bytes {
            let Some(line) = lines.next_line()? else { break };
            read += line.end - line.start;
            end = line.end;
            if line.blank() {
                continue;
            }
            match line.event() {
                Some(raw) => {
                    if let Some(event) = sanitizer.event(&raw) {
                        // One event twice in a log is sent once.
                        if ids.insert(event["event_id"].as_str().unwrap_or_default().to_owned()) {
                            events.push(event);
                        }
                    }
                }
                None => unreadable += 1,
            }
        }
        if end == start {
            return Ok(false);
        }
        state.health.dropped_total.unreadable_line += unreadable;
        policy::oldest_first(&mut events);
        let health = &state.health;
        let dropped = health.dropped_total.since(&health.acked_through.dropped);
        let log_resets = health.log_resets_total.saturating_sub(health.acked_through.log_resets);
        let counts = sanitizer.counts();
        if events.is_empty() && dropped.sum() == 0 && log_resets == 0 && counts.is_zero() {
            // Nothing to say: the cursor moves on, and no batch is made.
            state.cursor.offset = end;
            self.save(state)?;
            return Ok(true);
        }
        let (first_event_at, last_event_at) = policy::span_of(&events);
        let body = Body {
            policy_version: policy::policy().version,
            install_id: state.install_id.clone(),
            batch_seq: state.batch_seq_acked + 1,
            origin: "app".into(),
            app_version: telemetry::APP_VERSION.into(),
            events: events.len() as u64,
            talks: policy::talks_in(&events),
            first_event_at: first_event_at.clone(),
            last_event_at,
            dropped_event_count: dropped.sum(),
            dropped_by_reason: dropped,
            log_resets,
            dropped_total: health.dropped_total.sum(),
            log_resets_total: health.log_resets_total,
            policy_excluded_count: counts.policy_excluded_count,
            values_replaced: counts.values_replaced,
            unknown_fields_dropped: counts.unknown_fields_dropped,
        };
        let event_lines = policy::event_lines(&events);
        let batch_id = policy::batch_id_of(&body, &event_lines);
        let prepared_at = stamp(now);
        let send = SendFields { prepared_at: prepared_at.clone(), ..SendFields::default() };
        // The batch first, then the state that names it: a look that stops
        // between the two leaves an outbox file no state names, cleared at
        // the next look, and the same lines to make it from again.
        write_atomically(&self.outbox_file(&batch_id), &policy::batch_bytes(&body, &batch_id, &send, &event_lines))?;
        state.pending = Some(Pending {
            batch_id,
            batch_seq: body.batch_seq,
            events: body.events,
            prepared_at,
            first_event_at,
            attempts: 0,
            first_attempt_at: None,
            dropped_total: state.health.dropped_total,
            log_resets_total: state.health.log_resets_total,
            start,
            end,
            unreadable,
        });
        state.cursor.offset = end;
        self.save(state)?;
        Ok(true)
    }

    /// Tries the batch waiting once, unless it is to wait longer.
    fn send(&self, state: &mut State, now: DateTime<Utc>) -> io::Result<Sent> {
        if let Some(at) = state.next_attempt_at.filter(|at| now < *at) {
            return Ok(Sent::Waiting(at));
        }
        let pending = state.pending.clone().expect("a batch is waiting");
        let path = self.outbox_file(&pending.batch_id);
        let frozen = match fs::read(&path) {
            Ok(data) => policy::read_batch(&data).map(|(body, _, _, lines)| (body, lines.to_vec())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let Some((body, lines)) = frozen.filter(|(body, lines)| policy::batch_id_of(body, lines) == pending.batch_id)
        else {
            // The batch's file is gone, or not whole: no transport is
            // involved, only the laptop's disk, or a person. While the log
            // still has its lines, the batch is made again from them and
            // nothing is lost or counted; the next batch has its number.
            // Only when the log was replaced or cut since, or the cap has
            // moved the cursor on, are its events gone, and counted as
            // `write_failed`: events the app could not keep to send.
            let lines_kept = state.cursor.offset == pending.end
                && state.health.log_resets_total == pending.log_resets_total;
            if lines_kept {
                eprintln!("[TELEMETRY] the batch waiting was lost; making it again from the log");
                state.cursor.offset = pending.start;
                // Counted again when its lines are read again.
                let unreadable = &mut state.health.dropped_total.unreadable_line;
                *unreadable = unreadable.saturating_sub(pending.unreadable);
            } else {
                eprintln!("[TELEMETRY] the batch waiting was lost with its lines; {} events counted as dropped", pending.events);
                state.health.dropped_total.write_failed += pending.events;
            }
            state.pending = None;
            state.backoff_s = 0;
            state.next_attempt_at = None;
            let _ = fs::remove_file(&path);
            self.save(state)?;
            return Ok(Sent::Lost);
        };

        // The try is counted before it is made, so one cut short counts.
        let pending = state.pending.as_mut().expect("a batch is waiting");
        pending.attempts += 1;
        pending.first_attempt_at.get_or_insert_with(|| stamp(now));
        let pending = pending.clone();
        self.save(state)?;

        let (age_ms, clock_suspect) = age_of(pending.first_event_at.as_deref(), &pending.prepared_at, now);
        let (backlog_events, backlog_bytes) = self.backlog(state.cursor.offset)?;
        let send = SendFields {
            prepared_at: pending.prepared_at,
            sent_at: Some(stamp(now)),
            attempts: Some(pending.attempts),
            first_attempt_at: pending.first_attempt_at,
            oldest_queued_event_age_ms: Some(age_ms),
            clock_suspect: Some(clock_suspect),
            backlog_after_events: Some(backlog_events),
            backlog_after_bytes: Some(backlog_bytes),
        };
        let batch_id = pending.batch_id;
        let data = policy::batch_bytes(&body, &batch_id, &send, &lines);
        match self.transport.send(&data, &batch_id) {
            Ok(_) => {
                let pending = state.pending.take().expect("a batch is waiting");
                state.batch_seq_acked = pending.batch_seq;
                state.health.acked_through = Acked {
                    dropped: pending.dropped_total,
                    log_resets: pending.log_resets_total,
                };
                state.backoff_s = 0;
                state.next_attempt_at = None;
                self.save(state)?;
                let _ = fs::remove_file(&path);
                Ok(Sent::Stored)
            }
            Err(SendError::Refused(why)) => {
                // Only a relay refuses (Phase 4); export never does, so this
                // is a placeholder for it, to be settled with the relay. Sent
                // again it would be refused again: put aside, and its number
                // goes to the next batch, which carries its drops (they were
                // never acknowledged). Its events are not counted as dropped
                // here: the relay knows what it refused.
                eprintln!("[TELEMETRY] batch {batch_id} was refused ({why}); put aside");
                self.put_aside(&path, &batch_id);
                state.pending = None;
                state.backoff_s = 0;
                state.next_attempt_at = None;
                self.save(state)?;
                Ok(Sent::Refused)
            }
            Err(error) => {
                let wait = self.backoff(state.backoff_s);
                state.backoff_s = wait.as_secs();
                let at = now + TimeDelta::from_std(jittered(wait)).expect("a wait of hours fits");
                state.next_attempt_at = Some(at);
                eprintln!("[TELEMETRY] batch {batch_id} not sent ({error:?}); trying again at {at}");
                self.save(state)?;
                Ok(Sent::Waiting(at))
            }
        }
    }

    fn backoff(&self, last_s: u64) -> Duration {
        let wait = if last_s == 0 { self.config.backoff_min } else { Duration::from_secs(last_s.saturating_mul(2)) };
        wait.clamp(self.config.backoff_min, self.config.backoff_max)
    }

    fn put_aside(&self, path: &Path, batch_id: &str) {
        let refused = self.dir.join("refused");
        if fs::create_dir_all(&refused).is_err() || fs::rename(path, refused.join(format!("{batch_id}.jsonl"))).is_err() {
            let _ = fs::remove_file(path);
            return;
        }
        let mut kept: Vec<_> = fs::read_dir(&refused)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        kept.sort();
        for (_, old) in kept.iter().rev().skip(REFUSED_KEPT) {
            let _ = fs::remove_file(old);
        }
    }

    /// The whole lines after `offset`, and their bytes: what waits once the
    /// batch being sent is.
    fn backlog(&self, offset: u64) -> io::Result<(u64, u64)> {
        let Some(mut file) = self.open_log()? else { return Ok((0, 0)) };
        file.seek(SeekFrom::Start(offset))?;
        let (mut lines, mut bytes, mut since_newline) = (0, 0, 0);
        let mut buffer = vec![0; 64 * 1024];
        let mut blank = true;
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            for &byte in &buffer[..read] {
                since_newline += 1;
                if byte == b'\n' {
                    lines += u64::from(!blank);
                    bytes += since_newline;
                    since_newline = 0;
                    blank = true;
                } else if !byte.is_ascii_whitespace() {
                    blank = false;
                }
            }
        }
        Ok((lines, bytes))
    }
}

enum Sent {
    Stored,
    Lost,
    Refused,
    Waiting(DateTime<Utc>),
}

/// How old the oldest event waiting is, the batch's first, and whether the
/// clock looks wrong: an event, or the batch, from the future.
fn age_of(first_event_at: Option<&str>, prepared_at: &str, now: DateTime<Utc>) -> (u64, bool) {
    let now = micros(now);
    let prepared_late = policy::instant_micros(prepared_at).is_some_and(|prepared| prepared > now);
    match first_event_at.and_then(policy::instant_micros) {
        Some(first) if first > now => (0, true),
        Some(first) => (((now - first) / 1000) as u64, prepared_late),
        None => (0, prepared_late),
    }
}

/// One line of the log, from `start` to `end` (after its newline).
struct Line {
    start: u64,
    end: u64,
    /// None for a line too long to be an event.
    bytes: Option<Vec<u8>>,
}

impl Line {
    fn blank(&self) -> bool {
        self.bytes.as_ref().is_some_and(|bytes| bytes.iter().all(u8::is_ascii_whitespace))
    }

    /// The event the line holds, or None for one that is no JSON object.
    fn event(&self) -> Option<Map<String, Value>> {
        let bytes = self.bytes.as_deref()?;
        // The log's first line may start with a byte-order mark.
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        serde_json::from_slice::<Map<String, Value>>(bytes.trim_ascii()).ok()
    }
}

/// The log's whole lines from an offset; a last line still being written
/// is not one.
struct Lines {
    reader: BufReader<File>,
    position: u64,
}

impl Lines {
    fn new(mut file: File, offset: u64) -> io::Result<Self> {
        file.seek(SeekFrom::Start(offset))?;
        Ok(Self { reader: BufReader::new(file), position: offset })
    }

    fn next_line(&mut self) -> io::Result<Option<Line>> {
        let mut bytes = Vec::new();
        let read = self.reader.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut bytes)?;
        if read == 0 {
            return Ok(None);
        }
        let start = self.position;
        if bytes.ends_with(b"\n") {
            self.position += read as u64;
            return Ok(Some(Line { start, end: self.position, bytes: Some(bytes) }));
        }
        if bytes.len() <= MAX_LINE {
            // Not whole yet: read again next time, from its start.
            self.reader.seek(SeekFrom::Start(start))?;
            return Ok(None);
        }
        // Too long to be an event: skipped to its end, if it has one.
        let mut skipped = read as u64;
        loop {
            let buffer = self.reader.fill_buf()?;
            if buffer.is_empty() {
                self.reader.seek(SeekFrom::Start(start))?;
                return Ok(None);
            }
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(at) => {
                    self.reader.consume(at + 1);
                    skipped += at as u64 + 1;
                    self.position += skipped;
                    return Ok(Some(Line { start, end: self.position, bytes: None }));
                }
                None => {
                    let len = buffer.len();
                    self.reader.consume(len);
                    skipped += len as u64;
                }
            }
        }
    }
}

fn time_of(event: &Map<String, Value>) -> Option<i64> {
    event.get("timestamp").and_then(Value::as_str).and_then(policy::instant_micros)
}

fn micros(at: DateTime<Utc>) -> i64 {
    at.timestamp_micros()
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn hash16(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().take(8).map(|byte| format!("{byte:02x}")).collect()
}

fn head_of(file: &mut File, len: u64) -> io::Result<String> {
    let mut head = vec![0; len as usize];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut head)?;
    Ok(hash16(&head))
}

/// Where the log's last whole line ends.
fn whole_lines_end(file: &mut File, len: u64) -> io::Result<u64> {
    let mut end = len;
    let mut buffer = vec![0; 64 * 1024];
    while end > 0 {
        let from = end.saturating_sub(buffer.len() as u64);
        let chunk = &mut buffer[..(end - from) as usize];
        file.seek(SeekFrom::Start(from))?;
        file.read_exact(chunk)?;
        if let Some(at) = chunk.iter().rposition(|byte| *byte == b'\n') {
            return Ok(from + at as u64 + 1);
        }
        end = from;
    }
    Ok(0)
}

/// 16 hex from the operating system's random generator (a v4 UUID's
/// random bytes, its version and variant bits left out).
fn new_install_id() -> String {
    let bytes = *Uuid::new_v4().as_bytes();
    [bytes[0], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `wait`, a fifth longer or shorter at random, so laptops that failed
/// together do not all try again together.
fn jittered(wait: Duration) -> Duration {
    let random = u16::from_le_bytes([Uuid::new_v4().as_bytes()[0], Uuid::new_v4().as_bytes()[15]]);
    let factor = 0.8 + 0.4 * f64::from(random) / f64::from(u16::MAX);
    wait.mul_f64(factor)
}

/// Writes `bytes` to a file beside `path`, flushes it to the disk, and moves
/// it over `path`: a reader sees the old file or the new one, never half.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    {
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}

// The worker.

/// What tells the queue to look sooner than its timer.
#[derive(Debug, Clone, Copy)]
pub enum Nudge {
    EngineReady,
    RecapDone,
    CloudUp,
}

static NUDGES: OnceLock<SyncSender<Nudge>> = OnceLock::new();

/// Asks the queue to look; never waits, and does nothing when it is off.
pub fn nudge(why: Nudge) {
    if let Some(nudges) = NUDGES.get() {
        // One waiting is enough.
        let _ = nudges.try_send(why);
    }
}

pub fn enabled() -> bool {
    std::env::var("ELLA_TELEMETRY_EXPORT").is_ok_and(|value| value == "1")
}

/// Starts the queue of the log in `telemetry_dir`, exporting to `export/`
/// beside it, when `ELLA_TELEMETRY_EXPORT=1`.
pub fn start(telemetry_dir: PathBuf) {
    if !enabled() {
        return;
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    if NUDGES.set(sender).is_err() {
        return;
    }
    let transport = ExportTransport { dir: telemetry_dir.join("export") };
    let queue = Queue::new(&telemetry_dir, Config::default(), Box::new(transport));
    eprintln!("[TELEMETRY] exporting batches to {}", telemetry_dir.join("export").display());
    let spawned = thread::Builder::new()
        .name("telemetry-queue".into())
        .spawn(move || work(&queue, &receiver));
    if let Err(error) = spawned {
        eprintln!("[TELEMETRY] the upload queue could not start: {error}");
    }
}

fn work(queue: &Queue, nudges: &mpsc::Receiver<Nudge>) {
    let idle = queue.config.idle;
    let mut wake_at = Instant::now() + idle;
    loop {
        // A nudge looks now; the timer, when it comes.
        if let Err(RecvTimeoutError::Disconnected) = nudges.recv_timeout(wake_at.saturating_duration_since(Instant::now())) {
            return;
        }
        if let Some(wait) = still_busy(telemetry::quiet_for(), idle) {
            wake_at = Instant::now() + wait;
            continue;
        }
        let failures = telemetry::take_write_failures();
        let outcome = queue.run(Utc::now(), failures);
        if !matches!(outcome, Ok(Outcome { locked: false, .. })) {
            telemetry::restore_write_failures(failures);
        }
        if let Err(error) = &outcome {
            eprintln!("[TELEMETRY] the upload queue could not look at the log: {error}");
        }
        let mut next = Instant::now() + queue.config.every;
        if let Ok(Outcome { retry_at: Some(at), .. }) = outcome {
            let wait = (at - Utc::now()).to_std().unwrap_or_default();
            next = next.min(Instant::now() + wait);
        }
        wake_at = next;
    }
}

/// How much longer to wait for the app to be idle: None once no turn has
/// been written for `idle`.
fn still_busy(quiet: Duration, idle: Duration) -> Option<Duration> {
    (quiet < idle).then(|| idle - quiet)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const LAUNCH: &str = "d0ec3333-119c-4ae0-ac10-b69071ab640a";

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    fn t0() -> DateTime<Utc> {
        at("2026-10-27T06:00:00Z")
    }

    fn turn(talk: &str, when: DateTime<Utc>, over: bool) -> String {
        serde_json::json!({
            "event": "ella_turn_latency", "schema_version": 2, "timestamp": when.to_rfc3339(),
            "app_version": "0.1.26", "launch_id": LAUNCH, "interaction_id": Uuid::new_v4().to_string(),
            "status": "ok", "kind": "voice", "session_id": talk, "talk": "free", "topic": "market",
            "level": "A2", "turn": 1, "talk_over": over, "total_ms": 900,
        })
        .to_string()
    }

    fn recap(talk: &str, when: DateTime<Utc>) -> String {
        serde_json::json!({
            "event": "ella_assessment", "schema_version": 2, "timestamp": when.to_rfc3339(),
            "app_version": "0.1.26", "launch_id": LAUNCH, "status": "ok", "kind": "talk",
            "session_id": talk, "answers": 3, "total_ms": 2000,
        })
        .to_string()
    }

    fn launch(when: DateTime<Utc>) -> String {
        serde_json::json!({
            "event": "ella_launch", "schema_version": 1, "timestamp": when.to_rfc3339(),
            "app_version": "0.1.26", "launch_id": Uuid::new_v4().to_string(), "os": "macOS 15.6",
            "arch": "aarch64", "hostname": "ASHA-MAC",
        })
        .to_string()
    }

    /// Three talks, each over, from `when`.
    fn three_talks(when: DateTime<Utc>) -> Vec<String> {
        (0..3)
            .flat_map(|n| {
                let talk = format!("talk-{n}-{}", when.timestamp());
                let start = when + TimeDelta::minutes(n);
                vec![turn(&talk, start, false), turn(&talk, start + TimeDelta::seconds(20), true), recap(&talk, start + TimeDelta::seconds(30))]
            })
            .collect()
    }

    /// A transport that keeps what it is sent, and answers as told.
    #[derive(Clone, Default)]
    struct Fake {
        sent: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
        answers: Arc<Mutex<Vec<Result<Ack, SendError>>>>,
    }

    impl Fake {
        fn answer(&self, answer: Result<Ack, SendError>) {
            self.answers.lock().unwrap().push(answer);
        }

        fn batches(&self) -> Vec<(Value, Vec<Value>)> {
            self.sent.lock().unwrap().iter().map(|(_, data)| parse(data)).collect()
        }

        fn ids(&self) -> Vec<String> {
            self.sent.lock().unwrap().iter().map(|(id, _)| id.clone()).collect()
        }
    }

    impl Transport for Fake {
        fn send(&self, batch: &[u8], batch_id: &str) -> Result<Ack, SendError> {
            self.sent.lock().unwrap().push((batch_id.to_owned(), batch.to_vec()));
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                Ok(Ack::Stored)
            } else {
                answers.remove(0)
            }
        }
    }

    fn parse(data: &[u8]) -> (Value, Vec<Value>) {
        let text = std::str::from_utf8(data).unwrap();
        let mut lines = text.lines().map(|line| serde_json::from_str(line).unwrap());
        let envelope = lines.next().unwrap();
        (envelope, lines.collect())
    }

    struct Laptop {
        dir: tempfile::TempDir,
        fake: Fake,
    }

    impl Laptop {
        fn new() -> Self {
            let laptop = Self { dir: tempfile::tempdir().unwrap(), fake: Fake::default() };
            fs::write(laptop.log(), "").unwrap();
            laptop
        }

        fn log(&self) -> PathBuf {
            self.dir.path().join("latency.jsonl")
        }

        fn write(&self, lines: &[String]) {
            let mut file = File::options().append(true).create(true).open(self.log()).unwrap();
            for line in lines {
                writeln!(file, "{line}").unwrap();
            }
        }

        fn queue(&self, config: Config) -> Queue {
            Queue::new(self.dir.path(), config, Box::new(self.fake.clone()))
        }

        fn run(&self, now: DateTime<Utc>) -> Outcome {
            self.queue(Config::default()).run(now, 0).unwrap()
        }

        fn state(&self) -> State {
            serde_json::from_slice(&fs::read(self.dir.path().join("upload/state.json")).unwrap()).unwrap()
        }

        fn outbox(&self) -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(self.dir.path().join("upload/outbox"))
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }

        /// A first look, so the install exists and its cursor is at the end.
        fn installed(self) -> Self {
            self.run(t0());
            self
        }
    }

    #[test]
    fn a_new_install_starts_at_the_end_of_the_log() {
        let laptop = Laptop::new();
        laptop.write(&three_talks(t0() - TimeDelta::days(10)));
        laptop.write(&[launch(t0())]);
        let outcome = laptop.run(t0());
        assert_eq!(outcome.sent, 0, "what was there before stays on the laptop");
        let state = laptop.state();
        assert_eq!(state.cursor.offset, fs::metadata(laptop.log()).unwrap().len());
        assert_eq!(state.install_id.len(), 16);
        assert_eq!(state.batch_seq_acked, 0);

        laptop.write(&three_talks(t0() + TimeDelta::hours(1)));
        assert_eq!(laptop.run(t0() + TimeDelta::hours(2)).sent, 1);
        let (envelope, events) = laptop.fake.batches().remove(0);
        assert_eq!(envelope["events"], 9, "only what came after");
        assert_eq!(events.len(), 9);
    }

    #[test]
    fn a_new_install_does_not_start_inside_a_line_being_written() {
        let laptop = Laptop::new();
        laptop.write(&[launch(t0())]);
        let whole = fs::metadata(laptop.log()).unwrap().len();
        fs::OpenOptions::new().append(true).open(laptop.log()).unwrap().write_all(b"{\"event\":\"ella_cl").unwrap();
        laptop.run(t0());
        assert_eq!(laptop.state().cursor.offset, whole);
    }

    #[test]
    fn a_batch_is_prepared_sent_and_committed() {
        let laptop = Laptop::new().installed();
        let install_id = laptop.state().install_id;
        laptop.write(&three_talks(t0()));
        let outcome = laptop.run(t0() + TimeDelta::hours(1));
        assert_eq!(outcome, Outcome { locked: false, sent: 1, retry_at: None });

        let (envelope, events) = laptop.fake.batches().remove(0);
        assert_eq!(envelope["event"], "ella_batch");
        assert_eq!(envelope["install_id"], install_id.as_str());
        assert_eq!(envelope["batch_seq"], 1);
        assert_eq!(envelope["origin"], "app");
        assert_eq!(envelope["policy_version"], 2);
        assert_eq!(envelope["talks"], 3);
        assert_eq!(envelope["events"], 9);
        assert_eq!(envelope["attempts"], 1);
        assert_eq!(envelope["dropped_event_count"], 0);
        assert_eq!(envelope["backlog_after_events"], 0);
        assert_eq!(envelope["oldest_queued_event_age_ms"], 3_600_000);
        assert_eq!(envelope["clock_suspect"], false);
        assert_eq!(envelope["first_event_at"], events[0]["timestamp"]);
        assert!(events.iter().all(|event| event.get("topic").is_none() && event.get("level").is_none()));
        let data = &laptop.fake.sent.lock().unwrap()[0].1;
        let (body, id, _, lines) = policy::read_batch(data).unwrap();
        assert_eq!(policy::batch_id_of(&body, lines), id);

        let state = laptop.state();
        assert_eq!(state.batch_seq_acked, 1);
        assert_eq!(state.pending, None);
        assert_eq!(state.cursor.offset, fs::metadata(laptop.log()).unwrap().len());
        assert!(laptop.outbox().is_empty());
        assert_eq!(laptop.run(t0() + TimeDelta::hours(2)).sent, 0, "nothing new");
    }

    #[test]
    fn a_batch_waits_for_three_talks_over() {
        let laptop = Laptop::new().installed();
        let lines: Vec<_> = three_talks(t0()).into_iter().take(6).collect();
        laptop.write(&lines);
        laptop.write(&[turn("open", t0() + TimeDelta::minutes(5), false)]);
        assert_eq!(laptop.run(t0() + TimeDelta::minutes(10)).sent, 0, "two over, one still open");
        let later = t0() + TimeDelta::minutes(36);
        assert_eq!(laptop.run(later).sent, 1, "the open one has been quiet for half an hour");
    }

    #[test]
    fn a_pause_in_a_talk_lets_the_queue_look_but_does_not_end_the_talk() {
        let laptop = Laptop::new().installed();
        let lines: Vec<_> = three_talks(t0()).into_iter().take(6).collect();
        laptop.write(&lines);
        let last_turn = t0() + TimeDelta::minutes(5);
        laptop.write(&[turn("open", last_turn, false)]);
        let config = Config::default();
        // Three minutes on: past the idle wait, so the queue may look...
        let paused = Duration::from_secs(3 * 60);
        assert_eq!(still_busy(paused, config.idle), None);
        // ...but the talk is not over, so nothing is due.
        let now = last_turn + TimeDelta::minutes(3);
        assert!(!laptop.queue(config.clone()).due(&laptop.state(), now).unwrap());
        assert!(!laptop.queue(config.clone()).due(&laptop.state(), last_turn + TimeDelta::minutes(29)).unwrap());
        assert_eq!(laptop.run(now).sent, 0);
        // Only the 30 minutes, or its end, makes it over.
        assert!(laptop.queue(config.clone()).due(&laptop.state(), last_turn + TimeDelta::minutes(31)).unwrap());
        laptop.write(&[turn("open", now, true)]);
        assert!(laptop.queue(config).due(&laptop.state(), now).unwrap());
    }

    #[test]
    fn a_batch_is_due_when_its_oldest_event_is_three_days_old() {
        let laptop = Laptop::new().installed();
        laptop.write(&[launch(t0())]);
        assert_eq!(laptop.run(t0() + TimeDelta::days(2)).sent, 0);
        assert_eq!(laptop.run(t0() + TimeDelta::days(3) + TimeDelta::minutes(1)).sent, 1);
        assert_eq!(laptop.fake.batches()[0].0["talks"], 0);
    }

    #[test]
    fn a_batch_is_due_when_enough_of_the_log_waits() {
        let laptop = Laptop::new().installed();
        let config = Config { due_bytes: 2_000, batch_bytes: 1_000, ..Config::default() };
        laptop.write(&[launch(t0()), launch(t0())]);
        assert_eq!(laptop.queue(config.clone()).run(t0(), 0).unwrap().sent, 0);
        laptop.write(&(0..10).map(|_| launch(t0())).collect::<Vec<_>>());
        let outcome = laptop.queue(config).run(t0(), 0).unwrap();
        assert!(outcome.sent >= 2, "the whole log follows, in batches: {outcome:?}");
        assert_eq!(laptop.state().cursor.offset, fs::metadata(laptop.log()).unwrap().len());
    }

    #[test]
    fn batches_are_numbered_without_gaps() {
        let laptop = Laptop::new().installed();
        let config = Config { batch_bytes: 1, ..Config::default() };
        laptop.write(&three_talks(t0()));
        laptop.queue(config.clone()).run(t0() + TimeDelta::hours(1), 0).unwrap();
        laptop.write(&three_talks(t0() + TimeDelta::hours(2)));
        laptop.queue(config).run(t0() + TimeDelta::hours(3), 0).unwrap();
        let numbers: Vec<_> = laptop.fake.batches().iter().map(|(envelope, _)| envelope["batch_seq"].as_u64().unwrap()).collect();
        assert_eq!(numbers, (1..=18).collect::<Vec<_>>());
        let ids = laptop.fake.ids();
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn a_batch_not_sent_is_tried_again_the_same_batch() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.fake.answer(Err(SendError::Unreachable));
        laptop.fake.answer(Err(SendError::Failed("disk full".into())));
        let now = t0() + TimeDelta::hours(1);
        let outcome = laptop.run(now);
        assert_eq!(outcome.sent, 0);
        let first_wait = outcome.retry_at.unwrap() - now;
        assert!((48..=72).contains(&first_wait.num_seconds()), "{first_wait}");
        let state = laptop.state();
        assert_eq!(state.backoff_s, 60);
        let pending = state.pending.as_ref().unwrap();
        assert_eq!(pending.attempts, 1);
        // The cursor is past the batch's lines once it is in the outbox;
        // the outbox, not the log, is what is sent again.
        assert_eq!(state.cursor.offset, fs::metadata(laptop.log()).unwrap().len());
        assert_eq!(laptop.outbox(), [format!("{}.jsonl", pending.batch_id)]);

        // Not before its time.
        assert_eq!(laptop.run(now + TimeDelta::seconds(30)).retry_at, outcome.retry_at);
        assert_eq!(laptop.fake.ids().len(), 1);

        let second = laptop.run(now + TimeDelta::minutes(2));
        assert_eq!(laptop.state().backoff_s, 120, "the wait doubles");
        let third = laptop.run(second.retry_at.unwrap());
        assert_eq!(third.sent, 1);

        let batches = laptop.fake.batches();
        let ids = laptop.fake.ids();
        assert!(ids.iter().all(|id| *id == ids[0]), "one batch, three tries");
        assert_eq!(batches[2].0["attempts"], 3);
        assert_eq!(batches[2].0["first_attempt_at"], batches[0].0["sent_at"]);
        assert_eq!(batches[2].0["batch_seq"], 1);
        let state = laptop.state();
        assert_eq!((state.backoff_s, state.next_attempt_at, state.batch_seq_acked), (0, None, 1));
    }

    #[test]
    fn the_wait_doubles_up_to_six_hours() {
        let queue = Laptop::new().queue(Config::default());
        let mut wait = 0;
        let mut waits = Vec::new();
        for _ in 0..12 {
            wait = queue.backoff(wait).as_secs();
            waits.push(wait);
        }
        assert_eq!(waits[..4], [60, 120, 240, 480]);
        assert_eq!(*waits.last().unwrap(), 6 * 60 * 60);
        for _ in 0..50 {
            let jittered = jittered(Duration::from_secs(100)).as_secs_f64();
            assert!((80.0..=120.0).contains(&jittered), "{jittered}");
        }
    }

    #[test]
    fn a_restart_after_a_batch_is_made_sends_that_batch() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        let now = t0() + TimeDelta::hours(1);
        // The look stops after the batch is kept, before it is sent.
        let queue = laptop.queue(Config::default());
        let mut state = queue.load(now).unwrap();
        assert!(queue.prepare(&mut state, now).unwrap());
        let made = state.pending.clone().unwrap();
        drop(queue);

        assert_eq!(laptop.run(now + TimeDelta::minutes(5)).sent, 1);
        assert_eq!(laptop.fake.ids(), [made.batch_id]);
        assert_eq!(laptop.fake.batches()[0].0["batch_seq"], made.batch_seq);
        assert_eq!(laptop.fake.batches()[0].0["prepared_at"], made.prepared_at.as_str());
    }

    #[test]
    fn a_restart_before_the_state_is_kept_makes_the_same_batch_again() {
        let laptop = Laptop::new().installed();
        let before = laptop.state();
        laptop.write(&three_talks(t0()));
        let now = t0() + TimeDelta::hours(1);
        let queue = laptop.queue(Config::default());
        let mut state = queue.load(now).unwrap();
        queue.prepare(&mut state, now).unwrap();
        let made = state.pending.unwrap().batch_id;
        // As if the look had stopped before it kept its state: the batch is
        // in the outbox, and no state names it.
        queue.save(&before).unwrap();
        assert_eq!(laptop.outbox(), [format!("{made}.jsonl")]);

        assert_eq!(laptop.run(now + TimeDelta::minutes(5)).sent, 1);
        assert_eq!(laptop.fake.ids(), [made], "the same lines, the same batch");
        assert!(laptop.outbox().is_empty(), "the orphan is cleared");
    }

    #[test]
    fn a_batch_whose_answer_was_lost_is_sent_again_and_counted_once() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        // Kept by the far side, but the answer never came.
        laptop.fake.answer(Err(SendError::Unreachable));
        laptop.fake.answer(Ok(Ack::Duplicate));
        let now = t0() + TimeDelta::hours(1);
        laptop.run(now);
        assert_eq!(laptop.run(now + TimeDelta::minutes(2)).sent, 1);
        let ids = laptop.fake.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ids[1]);
        assert_eq!(laptop.state().batch_seq_acked, 1, "a duplicate is acknowledged once");
    }

    #[test]
    fn a_refused_batch_is_put_aside_and_its_number_goes_to_the_next() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.write(&["not json".into()]);
        laptop.fake.answer(Err(SendError::Refused("policy_version".into())));
        laptop.run(t0() + TimeDelta::hours(1));
        let refused = laptop.fake.ids()[0].clone();
        assert!(laptop.dir.path().join(format!("upload/refused/{refused}.jsonl")).exists());
        let state = laptop.state();
        assert_eq!((state.pending, state.batch_seq_acked), (None, 0));

        laptop.write(&three_talks(t0() + TimeDelta::hours(2)));
        laptop.run(t0() + TimeDelta::hours(3));
        let (envelope, _) = laptop.fake.batches().pop().unwrap();
        assert_eq!(envelope["batch_seq"], 1);
        assert_eq!(envelope["dropped_by_reason"]["unreadable_line"], 1, "its drops come with the next");
    }

    #[test]
    fn a_lost_outbox_file_is_made_again_from_the_log_and_nothing_counted_lost() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.write(&["junk".into()]);
        laptop.fake.answer(Err(SendError::Unreachable));
        laptop.run(t0() + TimeDelta::hours(1));
        let lost = laptop.state().pending.unwrap();
        fs::remove_file(laptop.dir.path().join(format!("upload/outbox/{}.jsonl", lost.batch_id))).unwrap();

        assert_eq!(laptop.run(t0() + TimeDelta::hours(2)).sent, 1);
        let (envelope, events) = laptop.fake.batches().pop().unwrap();
        assert_eq!(envelope["batch_seq"], 1);
        assert_eq!(events.len(), 9, "the same lines");
        assert_eq!(
            envelope["dropped_by_reason"],
            serde_json::json!({"queue_full": 0, "unreadable_line": 1, "write_failed": 0}),
            "the junk line counted once, nothing counted lost"
        );
        assert_eq!(envelope["dropped_total"], 1);
        assert_eq!(laptop.state().cursor.offset, fs::metadata(laptop.log()).unwrap().len());
    }

    #[test]
    fn a_lost_outbox_file_whose_lines_are_gone_counts_its_events_as_dropped() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.fake.answer(Err(SendError::Unreachable));
        laptop.run(t0() + TimeDelta::hours(1));
        let lost = laptop.state().pending.unwrap();
        fs::remove_file(laptop.dir.path().join(format!("upload/outbox/{}.jsonl", lost.batch_id))).unwrap();
        // And the log replaced: its lines are nowhere now.
        fs::write(laptop.log(), "").unwrap();
        laptop.write(&three_talks(t0() + TimeDelta::hours(2)));
        laptop.run(t0() + TimeDelta::hours(3));
        let (envelope, events) = laptop.fake.batches().pop().unwrap();
        assert_eq!(envelope["batch_seq"], 1);
        assert_eq!(events.len(), 9);
        assert_eq!(envelope["dropped_by_reason"]["write_failed"], 9);
        assert_eq!(envelope["log_resets"], 1);
    }

    #[test]
    fn a_log_replaced_or_cut_is_read_from_its_start_and_counted() {
        let laptop = Laptop::new();
        laptop.write(&[launch(t0())]);
        let laptop = laptop.installed();
        // Replaced: a new log, longer than the old one.
        fs::write(laptop.log(), "").unwrap();
        laptop.write(&three_talks(t0()));
        laptop.run(t0() + TimeDelta::hours(1));
        let (envelope, events) = laptop.fake.batches().pop().unwrap();
        assert_eq!(envelope["log_resets"], 1);
        assert_eq!(envelope["log_resets_total"], 1);
        assert_eq!(events.len(), 9);

        // Cut: shorter than the cursor.
        fs::write(laptop.log(), "").unwrap();
        laptop.write(&[launch(t0() + TimeDelta::hours(2))]);
        laptop.run(t0() + TimeDelta::days(4));
        let (envelope, events) = laptop.fake.batches().pop().unwrap();
        assert_eq!((envelope["log_resets"].as_u64(), envelope["log_resets_total"].as_u64()), (Some(1), Some(2)));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn a_log_that_grew_from_empty_is_no_reset() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.run(t0() + TimeDelta::hours(1));
        assert_eq!(laptop.fake.batches()[0].0["log_resets"], 0);
        assert_eq!(laptop.state().cursor.head_len, HEAD);
    }

    #[test]
    fn the_oldest_lines_are_dropped_past_the_cap_and_counted() {
        let laptop = Laptop::new().installed();
        let config = Config { cap_bytes: 1_500, due_bytes: 100, ..Config::default() };
        laptop.write(&three_talks(t0()));
        assert!(fs::metadata(laptop.log()).unwrap().len() > 1_500);
        laptop.queue(config).run(t0() + TimeDelta::hours(1), 0).unwrap();
        let dropped = laptop.state().health.dropped_total.queue_full;
        assert!(dropped > 0);
        let (envelope, events) = laptop.fake.batches().remove(0);
        assert_eq!(envelope["dropped_by_reason"]["queue_full"], dropped);
        assert_eq!(events.len() as u64 + dropped, 9, "the oldest went, the newest stayed");
        assert_eq!(events.last().unwrap()["event"], "ella_assessment");
    }

    #[test]
    fn events_older_than_the_cap_are_dropped_and_counted() {
        let laptop = Laptop::new().installed();
        laptop.write(&[launch(t0()), launch(t0() + TimeDelta::days(200))]);
        laptop.run(t0() + TimeDelta::days(200) + TimeDelta::hours(1));
        // The new one is not yet due on its own: the old one went.
        let state = laptop.state();
        assert_eq!(state.health.dropped_total.queue_full, 1);
        laptop.run(t0() + TimeDelta::days(204));
        let (envelope, events) = laptop.fake.batches().pop().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(envelope["dropped_event_count"], 1);
    }

    #[test]
    fn unreadable_lines_and_failed_writes_are_counted() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.write(&["{not json".into(), "[1,2]".into(), "".into()]);
        laptop.queue(Config::default()).run(t0() + TimeDelta::hours(1), 2).unwrap();
        let (envelope, _) = laptop.fake.batches().remove(0);
        assert_eq!(
            envelope["dropped_by_reason"],
            serde_json::json!({"queue_full": 0, "unreadable_line": 2, "write_failed": 2})
        );
        assert_eq!(envelope["dropped_event_count"], 4);
        assert_eq!(envelope["dropped_total"], 4);
    }

    #[test]
    fn a_batch_counts_only_what_was_lost_since_the_last_one() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        laptop.write(&["junk".into()]);
        laptop.run(t0() + TimeDelta::hours(1));
        laptop.write(&three_talks(t0() + TimeDelta::hours(2)));
        laptop.write(&["junk".into(), "junk".into()]);
        laptop.queue(Config::default()).run(t0() + TimeDelta::hours(3), 1).unwrap();
        let batches = laptop.fake.batches();
        let counts: Vec<_> = batches
            .iter()
            .map(|(envelope, _)| (envelope["dropped_event_count"].as_u64().unwrap(), envelope["dropped_total"].as_u64().unwrap()))
            .collect();
        assert_eq!(counts, [(1, 1), (3, 4)]);
    }

    #[test]
    fn an_old_builds_events_and_unknown_events_stay_behind_and_are_counted() {
        let laptop = Laptop::new().installed();
        let old = serde_json::json!({"event": "ella_turn_latency", "schema_version": 1, "timestamp": t0().to_rfc3339(), "status": "ok", "total_ms": 4000});
        let new_kind = serde_json::json!({"event": "ella_new_thing", "timestamp": t0().to_rfc3339(), "launch_id": LAUNCH});
        laptop.write(&[old.to_string(), new_kind.to_string()]);
        laptop.write(&three_talks(t0()));
        laptop.run(t0() + TimeDelta::hours(1));
        let (envelope, events) = laptop.fake.batches().remove(0);
        assert_eq!(envelope["policy_excluded_count"], 2);
        assert_eq!(envelope["unknown_fields_dropped"], 0);
        assert_eq!(events.len(), 9);
    }

    #[test]
    fn a_clock_behind_the_events_is_suspect() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0() + TimeDelta::days(1)));
        laptop.write(&[launch(t0() - TimeDelta::days(4))]);
        laptop.run(t0());
        let (envelope, _) = laptop.fake.batches().remove(0);
        assert_eq!(envelope["clock_suspect"], false, "the oldest event is in the past");
        assert_eq!(envelope["oldest_queued_event_age_ms"], 4 * 86_400_000);

        assert_eq!(age_of(Some("2026-10-28T06:00:00Z"), "2026-10-27T05:00:00.000Z", t0()), (0, true));
        assert_eq!(age_of(None, "2026-10-27T07:00:00.000Z", t0()), (0, true));
        assert_eq!(age_of(None, "2026-10-27T05:00:00.000Z", t0()), (0, false));
    }

    #[test]
    fn a_state_file_that_cannot_be_read_starts_a_new_install() {
        let laptop = Laptop::new().installed();
        let before = laptop.state();
        laptop.write(&[launch(t0())]);
        fs::write(laptop.dir.path().join("upload/state.json"), "{ half a sta").unwrap();
        laptop.run(t0() + TimeDelta::hours(1));
        let after = laptop.state();
        assert_ne!(after.install_id, before.install_id);
        assert_eq!(after.cursor.offset, fs::metadata(laptop.log()).unwrap().len());
        assert_eq!(after.batch_seq_acked, 0);
        let kept = fs::read_dir(laptop.dir.path().join("upload"))
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("state.json.corrupt-"));
        assert!(kept);
    }

    #[test]
    fn another_copy_holding_the_queue_is_left_alone() {
        let laptop = Laptop::new().installed();
        laptop.write(&three_talks(t0()));
        let lock = File::options().write(true).open(laptop.dir.path().join("upload/upload.lock")).unwrap();
        lock.lock().unwrap();
        let outcome = laptop.run(t0() + TimeDelta::hours(1));
        assert!(outcome.locked);
        assert!(laptop.fake.ids().is_empty());
        lock.unlock().unwrap();
        assert_eq!(laptop.run(t0() + TimeDelta::hours(1)).sent, 1);
    }

    #[test]
    fn a_line_too_long_to_be_an_event_is_skipped_and_counted() {
        let laptop = Laptop::new().installed();
        laptop.write(&["x".repeat(MAX_LINE + 10)]);
        laptop.write(&three_talks(t0()));
        assert_eq!(laptop.run(t0() + TimeDelta::hours(1)).sent, 2, "the long line fills a batch alone");
        let batches = laptop.fake.batches();
        assert_eq!(batches[0].0["dropped_by_reason"]["unreadable_line"], 1);
        assert_eq!(batches[0].1.len(), 0);
        assert_eq!(batches[1].1.len(), 9);
        assert_eq!(batches[1].0["dropped_event_count"], 0);
    }

    #[test]
    fn the_queue_waits_for_the_app_to_be_idle() {
        let idle = Duration::from_secs(120);
        assert_eq!(still_busy(Duration::from_secs(30), idle), Some(Duration::from_secs(90)));
        assert_eq!(still_busy(Duration::from_secs(120), idle), None);
        assert_eq!(still_busy(Duration::from_secs(3600), idle), None);
    }

    #[test]
    fn exported_batches_pass_the_scripts_validator() {
        let laptop = Laptop::new().installed();
        let export = laptop.dir.path().join("export");
        let queue = Queue::new(laptop.dir.path(), Config { batch_bytes: 2_000, ..Config::default() }, Box::new(ExportTransport { dir: export.clone() }));
        laptop.write(&three_talks(t0()));
        laptop.write(&["junk".into()]);
        laptop.write(&[launch(t0() + TimeDelta::minutes(9))]);
        let outcome = queue.run(t0() + TimeDelta::hours(1), 1).unwrap();
        assert!(outcome.sent >= 2, "{outcome:?}");
        let mut files: Vec<_> = fs::read_dir(&export).unwrap().flatten().map(|entry| entry.path()).collect();
        files.sort();
        assert_eq!(files.len() as u32, outcome.sent);
        if let Some(out) = std::env::var_os("TELEMETRY_BATCH_OUT") {
            fs::create_dir_all(&out).unwrap();
            for file in &files {
                fs::copy(file, Path::new(&out).join(file.file_name().unwrap())).unwrap();
            }
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/latency-sanitize.py");
        let checked = std::process::Command::new("python3").arg(script).arg("--validate").args(&files).output();
        match checked {
            Ok(output) => assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) => eprintln!("python3 not found ({error}): the batches were not checked by the scripts"),
        }
    }

    /// A real log, batched as the app would: `ELLA_SMOKE_LOG=<copy of a
    /// latency.jsonl> ELLA_SMOKE_OUT=<dir> cargo test -- --ignored smoke`.
    /// The cursor starts at 0 here, to batch the whole copy.
    #[test]
    #[ignore]
    fn smoke_a_real_log() {
        let (Some(log), Some(out)) = (std::env::var_os("ELLA_SMOKE_LOG"), std::env::var_os("ELLA_SMOKE_OUT")) else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        fs::copy(log, dir.path().join("latency.jsonl")).unwrap();
        let queue = Queue::new(
            dir.path(),
            Config { batches_per_run: usize::MAX, ..Config::default() },
            Box::new(ExportTransport { dir: PathBuf::from(&out) }),
        );
        let mut state = queue.new_install().unwrap();
        state.cursor.offset = 0;
        fs::create_dir_all(dir.path().join("upload")).unwrap();
        queue.save(&state).unwrap();
        let outcome = queue.run(Utc::now(), 0).unwrap();
        eprintln!("{outcome:?}; state: {}", fs::read_to_string(dir.path().join("upload/state.json")).unwrap());
    }
}
