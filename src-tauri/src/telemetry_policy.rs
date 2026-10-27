//! The telemetry policy, `scripts/telemetry-fields.json`, as the app applies
//! it: what of `latency.jsonl` may leave a laptop, and the batch it leaves in.
//! The file is built in, so the scripts and the app read one policy, and
//! `scripts/telemetry-vectors.json` holds both to the same answers.
//!
//! It does what `scripts/latency-sanitize.py` does, with one difference: an
//! event with no `launch_id`, from a build before 2026-10-07, stays behind
//! here (counted as left out), since its `event_id` is a hash of Python's own
//! JSON of it. `--batch-out` sends those, by hand.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    sync::OnceLock,
};

use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const POLICY_JSON: &str = include_str!("../../scripts/telemetry-fields.json");

/// What a value that breaks its rule leaves as, when it is text.
pub const OTHER: &str = "other";
/// Rules whose values must hold exactly: `other` is no hash, time or batch.
const EXACT: [&str; 3] = ["hex16", "hex32", "timestamp"];
/// Enums that are never `other`.
const CLOSED: [&str; 2] = ["host", "origin"];
/// Taken from the event itself, never from its fields.
const IGNORED: [&str; 2] = ["event_id", "source"];

pub struct Policy {
    root: Value,
    patterns: BTreeMap<String, Regex>,
    pub version: u64,
    hardware_max: usize,
    /// Why the cloud went down: `HTTP 503`, or a proxy's `cap: <its message>`.
    /// `\n?\z` is Python's `$`.
    http: Regex,
    proxy: Regex,
    http_code: Regex,
    /// A field name a count may show as it is.
    plain: Regex,
}

/// The policy built into this build.
pub fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| {
        let root: Value = serde_json::from_str(POLICY_JSON).expect("the policy built in is JSON");
        let patterns = root["patterns"]
            .as_object()
            .expect("the policy has patterns")
            .iter()
            .map(|(name, pattern)| {
                let pattern = pattern.as_str().expect("a pattern is text");
                (name.clone(), Regex::new(pattern).expect("the policy's patterns compile"))
            })
            .collect();
        Policy {
            version: root["policy_version"].as_u64().expect("the policy has a version"),
            hardware_max: root["hardware_max"].as_u64().expect("the policy has hardware_max") as usize,
            patterns,
            http: Regex::new(r"^HTTP ([1-5][0-9]{2})(?::|\n?\z)").unwrap(),
            proxy: Regex::new(r"^([a-z_]+)(?::|\n?\z)").unwrap(),
            http_code: Regex::new(r"^http_[1-5][0-9]{2}$").unwrap(),
            plain: Regex::new(r"^[a-z0-9_]{1,40}$").unwrap(),
            root,
        }
    })
}

/// A rule's kind and its argument: `list:object:judge` is `list`, `object:judge`.
fn split(rule: &str) -> (&str, &str) {
    rule.split_once(':').unwrap_or((rule, ""))
}

fn strings(value: &Value) -> impl Iterator<Item = &str> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str)
}

pub fn short_hash(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))[..16].to_owned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Whether Python would take `value` as false: what a coder leaves out.
fn falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(value) => !value,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
    }
}

/// A field name as a count may show it: as it is when it is a plain one,
/// else escaped as Python's JSON escapes it and cut short.
fn shown(name: &str) -> String {
    if policy().plain.is_match(name) {
        return name.to_owned();
    }
    let mut quoted = String::from("\"");
    for char in name.chars() {
        match char {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            char if (char as u32) < 0x20 || (char as u32) > 0x7f => {
                for unit in char.encode_utf16(&mut [0; 2]) {
                    let _ = write!(quoted, "\\u{unit:04x}");
                }
            }
            char => quoted.push(char),
        }
    }
    quoted.push('"');
    if quoted.len() <= 18 {
        quoted
    } else {
        format!("{}…\"", &quoted[..16])
    }
}

/// When a `timestamp` that keeps its pattern was, in microseconds, as Python
/// reads it (the fraction cut to six digits); None for a time no calendar has.
pub fn instant_micros(stamp: &str) -> Option<i64> {
    if !policy().patterns["timestamp"].is_match(stamp) {
        return None;
    }
    let at: DateTime<FixedOffset> = DateTime::parse_from_rfc3339(stamp).ok()?;
    // Python has no year 0 and no leap second.
    if at.year() < 1 || at.nanosecond() >= 1_000_000_000 {
        return None;
    }
    Some(at.timestamp_micros())
}

#[cfg(test)]
pub fn error_code(text: &str) -> Option<String> {
    code_of("error", &Value::String(text.to_owned()))
}

#[cfg(test)]
pub fn reason_code(text: &str) -> Option<String> {
    code_of("reason", &Value::String(text.to_owned()))
}

/// A coder's code for `value`: None for no error at all, and `other` for
/// anything it does not know, never the words.
fn code_of(coder: &str, value: &Value) -> Option<String> {
    if falsy(value) {
        return None;
    }
    // Anything but text: Python's `str` of it starts with no prefix or code.
    let Some(text) = value.as_str() else {
        return Some(OTHER.into());
    };
    let p = policy();
    let spec = &p.root["coders"][coder];
    if coder == "error" {
        let prefixes = spec["prefixes"].as_object().expect("the error coder has prefixes");
        return Some(
            prefixes
                .iter()
                .find(|(prefix, _)| text.starts_with(prefix.as_str()))
                .and_then(|(_, code)| code.as_str())
                .unwrap_or(OTHER)
                .to_owned(),
        );
    }
    if let Some(code) = spec["exact"].get(text).and_then(Value::as_str) {
        return Some(code.to_owned());
    }
    let prefixes = spec["prefix"].as_object().expect("the reason coder has prefixes");
    if let Some(code) = prefixes.iter().find(|(prefix, _)| text.starts_with(prefix.as_str())) {
        return code.1.as_str().map(str::to_owned);
    }
    if let Some(found) = p.http.captures(text) {
        return Some(format!("http_{}", &found[1]));
    }
    if let Some(found) = p.proxy.captures(text) {
        if strings(&spec["proxy"]).any(|code| code == &found[1]) {
            return Some(format!("proxy_{}", &found[1]));
        }
    }
    Some(OTHER.into())
}

fn code_allowed(coder: &str, code: &str) -> bool {
    if code == OTHER {
        return true;
    }
    let p = policy();
    let spec = &p.root["coders"][coder];
    let among = |codes: &Value| codes.as_object().is_some_and(|codes| codes.values().any(|value| value == code));
    if coder == "error" {
        return among(&spec["prefixes"]);
    }
    among(&spec["exact"])
        || among(&spec["prefix"])
        || p.http_code.is_match(code)
        || code.strip_prefix("proxy_").is_some_and(|proxy| strings(&spec["proxy"]).any(|known| known == proxy))
}

fn output_name<'a>(name: &'a str, rule: &'a str) -> &'a str {
    match split(rule) {
        ("hash", out) => out,
        ("code", coder) => policy().root["coders"][coder]["output"].as_str().expect("a coder names its output"),
        _ => name,
    }
}

fn output_rule(rule: &str) -> &str {
    if rule.starts_with("hash:") {
        "hex16"
    } else {
        rule
    }
}

fn in_enum(name: &str, value: &Value) -> bool {
    policy().root["enums"][name].as_array().is_some_and(|values| values.contains(value))
}

/// Whether a value keeps a rule that is not an object's, a list's or a hash's.
fn holds(value: &Value, rule: &str) -> bool {
    let p = policy();
    let (kind, arg) = split(rule);
    match kind {
        _ if value.is_null() => true,
        "num" => value.is_number(),
        "int" => value.is_u64(),
        "bool" => value.is_boolean(),
        "hardware" => value.as_str().is_some_and(|text| {
            (1..=p.hardware_max).contains(&text.len()) && text.bytes().all(|byte| (b' '..=b'~').contains(&byte))
        }),
        "enum" => in_enum(arg, value) || (value == OTHER && !CLOSED.contains(&arg)),
        "host" => in_enum("host", value),
        "code" => value.as_str().is_some_and(|code| code_allowed(arg, code)),
        _ => match p.patterns.get(kind) {
            Some(pattern) => value
                .as_str()
                .is_some_and(|text| pattern.is_match(text) || (text == OTHER && !EXACT.contains(&kind))),
            None => {
                debug_assert!(false, "no such rule: {rule}");
                false
            }
        },
    }
}

/// An event's fields' rules: those of every event, and its own.
fn schema_of(kind: &str) -> Option<Map<String, Value>> {
    let p = policy();
    let spec = p.root["events"].get(kind)?;
    let mut schema = p.root["every_event"].as_object().cloned().unwrap_or_default();
    schema.extend(spec["fields"].as_object().cloned().unwrap_or_default());
    Some(schema)
}

/// The `event_id` the policy's recipe gives `raw`, made before it is
/// sanitized; None for an event with no `launch_id`, which stays behind.
pub fn event_id(raw: &Map<String, Value>) -> Option<String> {
    if let Some(kept) = raw.get("event_id").and_then(Value::as_str) {
        if policy().patterns["hex16"].is_match(kept) {
            return Some(kept.to_owned());
        }
    }
    raw.get("launch_id").and_then(Value::as_str).filter(|launch| !launch.is_empty())?;
    let key = ["launch_id", "timestamp", "event", "interaction_id"]
        .map(|name| raw.get(name).and_then(Value::as_str).unwrap_or(""))
        .join("|");
    Some(short_hash(&key))
}

/// What the policy's own words would print for an event's `event`.
fn kind_name(kind: Option<&Value>) -> String {
    match kind {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Bool(true)) => "True".into(),
        Some(Value::Bool(false)) => "False".into(),
        Some(Value::Number(number)) => number.to_string(),
        Some(_) => OTHER.into(),
    }
}

/// What a batch counts of what its sanitizing left behind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub policy_excluded_count: u64,
    pub values_replaced: u64,
    pub unknown_fields_dropped: u64,
}

impl Counts {
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

/// Applies the policy, and counts what it left behind: `excluded`, whole
/// events; `unknown`, fields it has no rule for; `replaced`, values that
/// broke their rule. As `Sanitizer` in latency-sanitize.py.
#[derive(Debug, Default)]
pub struct Sanitizer {
    pub excluded: BTreeMap<String, u64>,
    pub unknown: BTreeMap<String, u64>,
    pub replaced: BTreeMap<String, u64>,
}

impl Sanitizer {
    /// `raw` as it may leave the laptop, or None.
    pub fn event(&mut self, raw: &Map<String, Value>) -> Option<Map<String, Value>> {
        let kind = raw.get("event");
        let Some((kind, schema)) = kind.and_then(Value::as_str).and_then(|kind| Some((kind, schema_of(kind)?))) else {
            *self.excluded.entry(kind_name(kind)).or_default() += 1;
            return None;
        };
        if raw.get("timestamp").and_then(Value::as_str).and_then(instant_micros).is_none() {
            *self.excluded.entry(format!("{kind} without a time")).or_default() += 1;
            return None;
        }
        let Some(id) = event_id(raw) else {
            *self.excluded.entry(format!("{kind} without a launch_id")).or_default() += 1;
            return None;
        };
        let mut skip: BTreeSet<&str> = IGNORED.into_iter().collect();
        let excluded = &policy().root["events"][kind]["excluded"];
        skip.extend(strings(excluded));
        let kept = self.fields(raw, &schema, &skip, kind);
        let mut event = Map::new();
        event.insert("event_id".into(), Value::String(id));
        event.extend(kept);
        Some(event)
    }

    pub fn counts(&self) -> Counts {
        Counts {
            policy_excluded_count: self.excluded.values().sum(),
            values_replaced: self.replaced.values().sum(),
            unknown_fields_dropped: self.unknown.values().sum(),
        }
    }

    fn fields(
        &mut self,
        value: &Map<String, Value>,
        schema: &Map<String, Value>,
        skip: &BTreeSet<&str>,
        path: &str,
    ) -> Map<String, Value> {
        let rule_of = |name: &String| schema[name].as_str().expect("a rule is text");
        // A field already in the form it leaves in, from a sanitized file.
        let left: BTreeMap<&str, &str> =
            schema.keys().map(|name| (output_name(name, rule_of(name)), output_rule(rule_of(name)))).collect();
        let mut kept = Map::new();
        for (key, item) in value {
            if skip.contains(key.as_str()) {
                continue;
            }
            let field_path = format!("{path}.{key}");
            let (name, result) = if schema.contains_key(key) {
                let rule = rule_of(key);
                (output_name(key, rule), self.value(item, rule, &field_path))
            } else if let Some(rule) = left.get(key.as_str()) {
                (key.as_str(), self.value(item, rule, &field_path))
            } else {
                *self.unknown.entry(format!("{path}.{}", shown(key))).or_default() += 1;
                continue;
            };
            if let Some(result) = result {
                kept.insert(name.to_owned(), result);
            }
        }
        kept
    }

    /// `item` as it leaves under `rule`; None when it does not leave at all.
    fn value(&mut self, item: &Value, rule: &str, path: &str) -> Option<Value> {
        if item.is_null() {
            return Some(Value::Null);
        }
        let (kind, arg) = split(rule);
        match kind {
            "hash" => {
                if let Some(text) = item.as_str() {
                    return Some(Value::String(short_hash(text)));
                }
            }
            "code" if !item.as_str().is_some_and(|code| code_allowed(arg, code)) => {
                // An empty error or reason is no error: left out, as the app does.
                return code_of(arg, item).map(Value::String);
            }
            "host" if !in_enum("host", item) => {
                let default = policy().root["hosts"]["default"].as_array().is_some_and(|hosts| hosts.contains(item));
                return Some(Value::from(if default { "default" } else { "custom" }));
            }
            "object" => {
                if let Some(fields) = item.as_object() {
                    let schema = policy().root["objects"][arg].as_object().expect("the policy has the object");
                    let skip = IGNORED.into_iter().collect();
                    return Some(Value::Object(self.fields(fields, schema, &skip, path)));
                }
            }
            "list" => {
                if let Some(items) = item.as_array() {
                    return Some(Value::Array(items.iter().filter_map(|part| self.value(part, arg, path)).collect()));
                }
            }
            _ => {
                if holds(item, rule) {
                    return Some(item.clone());
                }
            }
        }
        *self.replaced.entry(path.to_owned()).or_default() += 1;
        let textual =
            policy().patterns.contains_key(kind) || matches!(kind, "hardware" | "enum" | "code" | "host");
        (textual && !EXACT.contains(&kind)).then(|| Value::from(OTHER))
    }
}

// Batches.

/// What a batch dropped, by why: the policy's `dropped_by_reason`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reasons {
    pub queue_full: u64,
    pub unreadable_line: u64,
    pub write_failed: u64,
}

impl Reasons {
    pub fn sum(&self) -> u64 {
        self.queue_full + self.unreadable_line + self.write_failed
    }

    /// What `self` has that `before` had not: never below zero.
    pub fn since(&self, before: &Reasons) -> Reasons {
        Reasons {
            queue_full: self.queue_full.saturating_sub(before.queue_full),
            unreadable_line: self.unreadable_line.saturating_sub(before.unreadable_line),
            write_failed: self.write_failed.saturating_sub(before.write_failed),
        }
    }
}

/// A batch's body, frozen when it is prepared: the policy's `batch.body`,
/// in its order. An app's batch measures everything a manual one cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    pub policy_version: u64,
    pub install_id: String,
    pub batch_seq: u64,
    pub origin: String,
    pub app_version: String,
    pub events: u64,
    pub talks: u64,
    pub first_event_at: Option<String>,
    pub last_event_at: Option<String>,
    pub dropped_event_count: u64,
    pub dropped_by_reason: Reasons,
    pub log_resets: u64,
    pub dropped_total: u64,
    pub log_resets_total: u64,
    pub policy_excluded_count: u64,
    pub values_replaced: u64,
    pub unknown_fields_dropped: u64,
}

/// How a batch's sending went: the policy's `batch.send`. Only
/// `prepared_at` is known before the first try.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendFields {
    pub prepared_at: String,
    pub sent_at: Option<String>,
    pub attempts: Option<u64>,
    pub first_attempt_at: Option<String>,
    pub oldest_queued_event_age_ms: Option<u64>,
    pub clock_suspect: Option<bool>,
    pub backlog_after_events: Option<u64>,
    pub backlog_after_bytes: Option<u64>,
}

/// JSON with its keys sorted and no spaces: what a batch's body is hashed as.
pub fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let ordered: BTreeMap<&String, Value> = fields.iter().map(|(key, value)| (key, sorted(value))).collect();
                Value::Object(ordered.into_iter().map(|(key, value)| (key.clone(), value)).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    // Sorted here, whether or not serde_json's `preserve_order` is on (a
    // build dependency turns it on for build scripts, not the app).
    serde_json::to_string(&sorted(value)).expect("JSON values serialize")
}

/// A batch's events as they are hashed and sent: one line each.
pub fn event_lines(events: &[Map<String, Value>]) -> Vec<u8> {
    let mut lines = Vec::new();
    for event in events {
        serde_json::to_writer(&mut lines, event).expect("JSON values serialize");
        lines.push(b'\n');
    }
    lines
}

pub fn batch_id_of(body: &Body, lines: &[u8]) -> String {
    let body = serde_json::to_value(body).expect("a body serializes");
    batch_id_of_value(&body, lines)
}

fn batch_id_of_value(body: &Value, lines: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical(body).as_bytes());
    hasher.update(b"\n");
    hasher.update(lines);
    hex(&hasher.finalize())[..32].to_owned()
}

/// A batch file: the envelope, then the event lines.
pub fn batch_bytes(body: &Body, batch_id: &str, send: &SendFields, lines: &[u8]) -> Vec<u8> {
    #[derive(Serialize)]
    struct Envelope<'a> {
        event: &'static str,
        #[serde(flatten)]
        body: &'a Body,
        batch_id: &'a str,
        #[serde(flatten)]
        send: &'a SendFields,
    }
    let mut data = serde_json::to_vec(&Envelope {
        event: "ella_batch",
        body,
        batch_id,
        send,
    })
    .expect("an envelope serializes");
    data.push(b'\n');
    data.extend_from_slice(lines);
    data
}

/// A batch file read back: its body, id and send fields, and its event
/// lines, or None when it is not one this app made whole.
pub fn read_batch(data: &[u8]) -> Option<(Body, String, SendFields, &[u8])> {
    let end = data.iter().position(|byte| *byte == b'\n')?;
    let (head, lines) = (&data[..end], &data[end + 1..]);
    let body: Body = serde_json::from_slice(head).ok()?;
    let send: SendFields = serde_json::from_slice(head).ok()?;
    let envelope: Map<String, Value> = serde_json::from_slice(head).ok()?;
    let batch_id = envelope.get("batch_id")?.as_str()?.to_owned();
    (envelope.get("event")? == "ella_batch" && batch_id_of(&body, lines) == batch_id)
        .then_some((body, batch_id, send, lines))
}

/// The policy's `talks`: the distinct talk ids among the events.
pub fn talks_in(events: &[Map<String, Value>]) -> u64 {
    events.iter().filter_map(|event| event.get("talk_id")?.as_str()).collect::<BTreeSet<_>>().len() as u64
}

fn micros_of(event: &Map<String, Value>) -> Option<i64> {
    event.get("timestamp").and_then(Value::as_str).and_then(instant_micros)
}

/// The first and last event's `timestamp`, as written.
pub fn span_of(events: &[Map<String, Value>]) -> (Option<String>, Option<String>) {
    let times: Vec<(i64, &str)> = events
        .iter()
        .filter_map(|event| Some((micros_of(event)?, event.get("timestamp")?.as_str()?)))
        .collect();
    let first = times.iter().min().map(|(_, stamp)| (*stamp).to_owned());
    let last = times.iter().max().map(|(_, stamp)| (*stamp).to_owned());
    (first, last)
}

/// The policy's order: by time, and as they were written where two share
/// one (a stable sort).
pub fn oldest_first(events: &mut [Map<String, Value>]) {
    events.sort_by_key(|event| micros_of(event).unwrap_or(i64::MIN));
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: &str = include_str!("../../scripts/telemetry-vectors.json");

    fn vectors() -> Value {
        serde_json::from_str(VECTORS).unwrap()
    }

    fn keys(counts: &BTreeMap<String, u64>) -> Vec<String> {
        counts.keys().cloned().collect()
    }

    fn texts(value: &Value) -> Vec<String> {
        strings(value).map(str::to_owned).collect()
    }

    #[test]
    fn every_vector_leaves_as_the_scripts_say() {
        for vector in vectors()["events"].as_array().unwrap() {
            let name = vector["name"].as_str().unwrap();
            let raw = vector["raw"].as_object().unwrap();
            let mut sanitizer = Sanitizer::default();
            let event = sanitizer.event(raw);
            let has_launch = raw.get("launch_id").and_then(Value::as_str).is_some_and(|id| !id.is_empty());
            if !vector["sanitized"].is_null() && !has_launch {
                // The one difference: an old build's event stays behind.
                assert_eq!(event, None, "{name}");
                let kind = raw["event"].as_str().unwrap();
                assert_eq!(keys(&sanitizer.excluded), [format!("{kind} without a launch_id")], "{name}");
                continue;
            }
            assert_eq!(event.map(Value::Object).unwrap_or(Value::Null), vector["sanitized"], "{name}");
            assert_eq!(keys(&sanitizer.unknown), texts(&vector["unknown"]), "{name}");
            assert_eq!(keys(&sanitizer.replaced), texts(&vector["replaced"]), "{name}");
            assert_eq!(keys(&sanitizer.excluded), texts(&vector["excluded"]), "{name}");
        }
    }

    #[test]
    fn sanitizing_what_left_changes_nothing() {
        for vector in vectors()["events"].as_array().unwrap() {
            let Some(event) = vector["sanitized"].as_object() else { continue };
            if !event.contains_key("launch") {
                continue;
            }
            let mut sanitizer = Sanitizer::default();
            assert_eq!(sanitizer.event(event).as_ref(), Some(event), "{}", vector["name"]);
            assert!(sanitizer.counts().is_zero(), "{}", vector["name"]);
        }
    }

    #[test]
    fn errors_and_reasons_become_the_scripts_codes() {
        let vectors = vectors();
        for (coder, list) in [("error", "error_codes"), ("reason", "reason_codes")] {
            for pair in vectors[list].as_array().unwrap() {
                let text = pair[0].as_str().unwrap();
                let code = if coder == "error" { error_code(text) } else { reason_code(text) };
                assert_eq!(code.map(Value::String).unwrap_or(Value::Null), pair[1], "{text}");
            }
        }
        // Python's `$` lets a line end come before the end.
        assert_eq!(reason_code("HTTP 503\n").as_deref(), Some("http_503"));
        assert_eq!(reason_code("cap\n").as_deref(), Some("proxy_cap"));
        // Not text: never its words.
        assert_eq!(code_of("reason", &Value::from(503)).as_deref(), Some("other"));
        assert_eq!(code_of("error", &Value::from(false)), None);
    }

    #[test]
    fn a_batch_id_is_the_scripts_batch_id() {
        let golden = &vectors()["batch_id"];
        let lines = golden["lines"].as_str().unwrap().as_bytes();
        assert_eq!(batch_id_of_value(&golden["body"], lines), golden["batch_id"].as_str().unwrap());
    }

    #[test]
    fn the_batch_structs_are_the_policys_batch() {
        let batch = &policy().root["batch"];
        let names = |value: &Value| value.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let body = Body {
            policy_version: 2,
            install_id: String::new(),
            batch_seq: 1,
            origin: "app".into(),
            app_version: String::new(),
            events: 0,
            talks: 0,
            first_event_at: None,
            last_event_at: None,
            dropped_event_count: 0,
            dropped_by_reason: Reasons::default(),
            log_resets: 0,
            dropped_total: 0,
            log_resets_total: 0,
            policy_excluded_count: 0,
            values_replaced: 0,
            unknown_fields_dropped: 0,
        };
        let mut ours = names(&serde_json::to_value(&body).unwrap());
        let mut theirs = names(&batch["body"]);
        ours.sort();
        theirs.sort();
        assert_eq!(ours, theirs);
        let mut ours = names(&serde_json::to_value(SendFields::default()).unwrap());
        let mut theirs = names(&batch["send"]);
        ours.sort();
        theirs.sort();
        assert_eq!(ours, theirs);
        let mut ours = names(&serde_json::to_value(Reasons::default()).unwrap());
        let mut theirs = names(&policy().root["objects"]["dropped_by_reason"]);
        ours.sort();
        theirs.sort();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn hardware_is_printable_ascii_only() {
        assert!(holds(&Value::from("Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz"), "hardware"));
        assert!(!holds(&Value::from("Apple M1 Pro™"), "hardware"));
        assert!(!holds(&Value::from("Windows\u{7} 11"), "hardware"));
        assert!(!holds(&Value::from(""), "hardware"));
        assert!(!holds(&Value::from("x".repeat(101)), "hardware"));
    }

    #[test]
    fn a_time_is_read_as_python_reads_it() {
        assert_eq!(
            instant_micros("2026-10-20T06:00:00.123456789+00:00"),
            instant_micros("2026-10-20T06:00:00.123456Z")
        );
        assert!(instant_micros("2026-10-20T11:30:00+05:30") == instant_micros("2026-10-20T06:00:00Z"));
        assert_eq!(instant_micros("2026-13-20T06:00:00Z"), None);
        assert_eq!(instant_micros("2026-10-20T06:00:60Z"), None, "no leap second");
        assert_eq!(instant_micros("0000-10-20T06:00:00Z"), None, "no year 0");
        assert_eq!(instant_micros("2026-10-20T06:00:00.5Z"), None, "the pattern's fractions only");
    }

    #[test]
    fn odd_field_names_are_shown_escaped_and_short() {
        assert_eq!(shown("hostname"), "hostname");
        assert_eq!(shown("Asha"), "\"Asha\"");
        assert_eq!(shown("é"), "\"\\u00e9\"");
        assert_eq!(shown("my name is Asha Kumari"), "\"my name is Asha…\"");
    }

    #[test]
    fn events_are_ordered_and_spanned_as_the_scripts_do() {
        let event = |stamp: &str, n: u64| {
            let mut event = Map::new();
            event.insert("timestamp".into(), Value::from(stamp));
            event.insert("n".into(), Value::from(n));
            event
        };
        let mut events = vec![
            event("2026-10-20T06:00:02Z", 1),
            event("2026-10-20T06:00:01.000000001+00:00", 2),
            event("2026-10-20T06:00:01Z", 3),
        ];
        oldest_first(&mut events);
        // 2 and 3 are one moment to Python, so they keep their order.
        assert_eq!(events.iter().map(|event| event["n"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 3, 1]);
        assert_eq!(
            span_of(&events),
            (Some("2026-10-20T06:00:01.000000001+00:00".into()), Some("2026-10-20T06:00:02Z".into()))
        );
    }
}
