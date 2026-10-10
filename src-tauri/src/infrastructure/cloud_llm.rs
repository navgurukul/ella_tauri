//! Ella's language model over the internet, when there is one.
//!
//! The laptop never holds a model provider's key. It asks Ella Desktop's
//! proxy (`supabase/functions/ella-desktop`), which holds the key, forwards
//! Ella-sized chat requests to DeepSeek and passes the answer back as it
//! streams. The proxy gives each install a token once, kept beside the
//! weights.
//!
//! Every request has the local llama-server behind it. The cloud is asked
//! first only while it is up. A request it fails before its first word goes to
//! the local model as though the cloud had not been asked, and the cloud is
//! then left alone for a while (`Health`), so the requests after it do not wait
//! on it too. Whether it is back is decided by asking the proxy's `/healthz`,
//! never by what the computer says about its network: a school's Wi-Fi that
//! wants a sign-in first looks connected.
//!
//! A request runs on a thread of its own and hands back what it reads, so the
//! caller can stop waiting at a deadline whatever the network does: a
//! connection that goes quiet would otherwise hold a blocking read for as long
//! as the whole request may take.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::telemetry;

/// The proxy an installed build asks: Ella Desktop's Supabase project.
pub const DEFAULT_PROXY: &str =
    "https://xoeydvvslyvslvpaajmi.supabase.co/functions/v1/ella-desktop";

/// How long a talk's reply waits for the cloud's first word before the local
/// model is asked instead. The learner is waiting through it, so it is short:
/// the cloud's first word usually comes in about a second.
pub const REPLY_FIRST_WORD: Duration = Duration::from_secs(5);
/// The same for a judge, which nobody hears as it works: a longer wait for the
/// cloud still beats the local model's on a laptop.
pub const JUDGE_FIRST_WORD: Duration = Duration::from_secs(12);
/// The longest silence between two pieces of an answer already started.
const STALL: Duration = Duration::from_secs(8);
/// All of one request, its stream included.
const REQUEST_LIMIT: Duration = Duration::from_secs(60);
/// A look at `/healthz`, or asking for a token.
const PROBE_LIMIT: Duration = Duration::from_secs(5);
/// After a failure the cloud is left alone this long before it is looked at
/// again, doubled after each look that finds it still down, up to the most.
const BACKOFF_MIN: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Left alone after the proxy says it, or DeepSeek, is too busy.
const BUSY_WAIT: Duration = Duration::from_secs(60);

/// What one request asks for, beyond its messages.
#[derive(Debug, Clone, Copy)]
pub struct Ask {
    pub max_tokens: u32,
    pub temperature: f64,
    /// An answer in JSON (`response_format: json_object`). The prompt must
    /// say so too, as every judge's does.
    pub json: bool,
    /// How long to wait for the first word before giving up on the cloud.
    pub first_word: Duration,
}

/// The tokens one answer took, as DeepSeek counted them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub prompt: u64,
    /// Prompt tokens DeepSeek had cached from an earlier request.
    pub hit: u64,
    /// Prompt tokens it read afresh.
    pub miss: u64,
    pub completion: u64,
}

#[derive(Debug)]
pub struct CloudAnswer {
    pub text: String,
    pub ttft_ms: f64,
    pub ms: f64,
    pub usage: Usage,
    /// The caller had what it needed before the end, and the rest was not
    /// waited for.
    pub stopped_early: bool,
    /// It ran out of `max_tokens` before it was done.
    pub cut_short: bool,
}

/// What came of asking the cloud.
#[derive(Debug)]
pub enum Asked {
    /// The whole answer, or as much of it as the caller wanted.
    Answered(CloudAnswer),
    /// No answer came: ask the local model, as though the cloud had not been
    /// asked. How long it took is in `ms`.
    Unanswered { why: String, ms: f64 },
    /// It answered with nothing at all, as DeepSeek's JSON mode now and then
    /// does. That says nothing about the network, so the cloud stays up, and a
    /// judge asks it again.
    Empty { ms: f64 },
    /// Part of an answer came, then it stopped. Whatever was done with the
    /// part must be undone.
    Cut { why: String, ms: f64 },
}

/// Why the cloud failed, which decides how long it is left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// Out of reach, broken off, or failing.
    Unreachable,
    /// It did not say anything in time.
    Slow,
    /// Too busy, or this install is over its limit: try again in a minute.
    Busy,
    /// It will not answer until someone acts: no key, no balance, the day's
    /// spending cap reached.
    Refused,
    /// The proxy does not know this install's token.
    Unauthorized,
    /// The proxy refused the request itself as malformed: a fault in Ella, not
    /// in the network. This request goes local; the cloud stays up.
    Rejected,
}

#[derive(Debug)]
struct Health {
    up: bool,
    /// Whether it has been looked at at all yet.
    known: bool,
    /// When a cloud that is down may be looked at again.
    retry_at: Instant,
    backoff: Duration,
    /// Why it is down, for the status and the log.
    reason: String,
}

pub struct CloudLlm {
    client: Client,
    base_url: String,
    token_file: Option<PathBuf>,
    token: Mutex<Option<String>>,
    health: Mutex<Health>,
    probing: AtomicBool,
}

impl CloudLlm {
    /// The cloud this launch uses, or `None` when it is off. It is on, with
    /// Ella Desktop's proxy, wherever it answers: there is nothing to switch
    /// on. `ELLA_CLOUD=off` keeps a laptop to its own model, and
    /// `ELLA_CLOUD_URL` names another proxy. A test build, which builds
    /// engines from the environment to try the laptop's own models, reaches
    /// the cloud only when asked with `ELLA_CLOUD=on`. The install token is
    /// kept in `models_root/cloud/install-token`, or `ELLA_CLOUD_TOKEN_FILE`.
    pub fn from_environment(models_root: &Path) -> Option<Arc<Self>> {
        let switch = env::var("ELLA_CLOUD")
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if matches!(switch.as_str(), "off" | "0" | "false" | "no") {
            return None;
        }
        let asked = matches!(switch.as_str(), "on" | "1" | "true" | "yes");
        let url = match env::var("ELLA_CLOUD_URL") {
            Ok(url) if !url.trim().is_empty() => url.trim().to_owned(),
            _ if asked || !cfg!(test) => DEFAULT_PROXY.to_owned(),
            _ => return None,
        };
        let token_file = env::var_os("ELLA_CLOUD_TOKEN_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| models_root.join("cloud").join("install-token"));
        let cloud = Arc::new(Self::new(url, Some(token_file)));
        cloud.probe_now();
        Some(cloud)
    }

    pub fn new(base_url: String, token_file: Option<PathBuf>) -> Self {
        let token = token_file.as_deref().and_then(read_token);
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(4))
                .timeout(REQUEST_LIMIT)
                .build()
                .expect("reqwest client configuration is valid"),
            base_url: base_url.trim_end_matches('/').to_owned(),
            token_file,
            token: Mutex::new(token),
            health: Mutex::new(Health {
                up: false,
                known: false,
                retry_at: Instant::now(),
                backoff: BACKOFF_MIN,
                reason: "not looked at yet".into(),
            }),
            probing: AtomicBool::new(false),
        }
    }

    /// The proxy's host, for the launch's telemetry.
    pub fn host(&self) -> String {
        let rest = self
            .base_url
            .split_once("://")
            .map_or(self.base_url.as_str(), |(_, rest)| rest);
        rest.split('/').next().unwrap_or(rest).to_owned()
    }

    /// Whether the cloud answers now, and in a few words why not.
    pub fn describe(&self) -> (bool, String) {
        let health = self.health();
        if health.up {
            (true, format!("Answering through {}", self.host()))
        } else {
            (false, format!("Not used now: {}", health.reason))
        }
    }

    /// Waits until the first look at the proxy has come back, or `limit`.
    pub fn wait_for_first_look(&self, limit: Duration) {
        let started = Instant::now();
        while !self.health().known && started.elapsed() < limit {
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Whether a request should go to the cloud. While the cloud is down this
    /// says no, and once its wait is over has the proxy looked at in the
    /// background, for the requests after this one.
    pub fn usable(self: &Arc<Self>) -> bool {
        let health = self.health();
        if health.up {
            return true;
        }
        let due = Instant::now() >= health.retry_at;
        drop(health);
        if due {
            self.probe_now();
        }
        false
    }

    /// Asks for one streamed answer to `messages`, handing `more` each piece
    /// as it lands and all of the answer so far. `more` returning false stops
    /// the answer there: the rest is not waited for, and the proxy stops
    /// DeepSeek writing it.
    pub fn ask(
        &self,
        messages: &[Value],
        ask: Ask,
        mut more: impl FnMut(&str, &str) -> bool,
    ) -> Asked {
        let started = Instant::now();
        let token = match self.token() {
            Ok(token) => token,
            Err(why) => {
                self.went_down(&why, Failure::Unreachable);
                return Asked::Unanswered {
                    why,
                    ms: elapsed_ms(started),
                };
            }
        };
        let mut body = json!({
            "messages": messages,
            "max_tokens": ask.max_tokens,
            "temperature": ask.temperature,
            "stream": true,
        });
        if ask.json {
            body["response_format"] = json!({"type": "json_object"});
        }
        let (sender, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.send(token, body, Arc::clone(&cancel), sender);

        let mut text = String::new();
        let mut ttft_ms = None;
        let mut usage = Usage::default();
        let mut cut_short = false;
        let ended = loop {
            let wait = match ttft_ms {
                None => ask.first_word.saturating_sub(started.elapsed()),
                Some(_) => STALL,
            };
            match events.recv_timeout(wait) {
                Ok(Event::Piece(piece)) => {
                    ttft_ms.get_or_insert_with(|| elapsed_ms(started));
                    text.push_str(&piece);
                    if !more(&piece, &text) {
                        cancel.store(true, Ordering::Relaxed);
                        break Ok(true);
                    }
                }
                Ok(Event::Usage(counted)) => usage = counted,
                Ok(Event::OutOfTokens) => cut_short = true,
                Ok(Event::End { clean: true }) => break Ok(false),
                Ok(Event::End { clean: false }) => {
                    break Err(("the answer broke off".to_owned(), Failure::Unreachable));
                }
                Ok(Event::Refused { why, failure }) => break Err((why, failure)),
                Ok(Event::Failed(why)) => break Err((why, Failure::Unreachable)),
                Err(RecvTimeoutError::Timeout) => {
                    cancel.store(true, Ordering::Relaxed);
                    let why = match ttft_ms {
                        None => format!("no first word in {:.0} s", ask.first_word.as_secs_f64()),
                        Some(_) => format!("silent for {} s partway", STALL.as_secs()),
                    };
                    break Err((why, Failure::Slow));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    break Err(("the request stopped".to_owned(), Failure::Unreachable));
                }
            }
        };
        let ms = elapsed_ms(started);
        match ended {
            Ok(stopped_early) if !text.trim().is_empty() => {
                self.came_up();
                Asked::Answered(CloudAnswer {
                    ttft_ms: ttft_ms.unwrap_or(ms),
                    ms,
                    usage,
                    stopped_early,
                    cut_short,
                    text,
                })
            }
            Ok(_) => Asked::Empty { ms },
            Err((why, failure)) => {
                self.went_down(&why, failure);
                if text.is_empty() {
                    Asked::Unanswered { why, ms }
                } else {
                    Asked::Cut { why, ms }
                }
            }
        }
    }

    /// Sends one request on a thread of its own, which hands back what it
    /// reads through `events` and lets the connection go as soon as `cancel`
    /// is set, so the proxy stops DeepSeek too.
    fn send(&self, token: String, body: Value, cancel: Arc<AtomicBool>, events: Sender<Event>) {
        let request = self
            .client
            .post(format!("{}/v1/chat", self.base_url))
            .bearer_auth(token)
            .json(&body);
        thread::spawn(move || {
            let response = match request.send() {
                Ok(response) => response,
                Err(error) => {
                    let _ = events.send(Event::Failed(short(&error)));
                    return;
                }
            };
            let status = response.status().as_u16();
            if !response.status().is_success() {
                let body: Value = response.json().unwrap_or(Value::Null);
                let (failure, why) = refusal(
                    status,
                    body["error"].as_str().unwrap_or_default(),
                    body["message"].as_str().unwrap_or_default(),
                );
                let _ = events.send(Event::Refused { why, failure });
                return;
            }
            let mut finished = false;
            for line in BufReader::new(response).lines() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let line = match line {
                    Ok(line) => line,
                    Err(error) => {
                        // Broken off after the last word is still the whole answer.
                        let _ = events.send(if finished {
                            Event::End { clean: true }
                        } else {
                            Event::Failed(error.to_string())
                        });
                        return;
                    }
                };
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    finished = true;
                    break;
                }
                let Ok(chunk) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(usage) = usage_of(&chunk["usage"]) {
                    let _ = events.send(Event::Usage(usage));
                }
                if let Some(reason) = chunk["choices"][0]["finish_reason"].as_str() {
                    finished = true;
                    if reason == "length" {
                        let _ = events.send(Event::OutOfTokens);
                    }
                }
                if let Some(piece) = chunk["choices"][0]["delta"]["content"].as_str() {
                    if !piece.is_empty() && events.send(Event::Piece(piece.to_owned())).is_err() {
                        return;
                    }
                }
            }
            let _ = events.send(Event::End { clean: finished });
        });
    }

    /// This install's token: the one kept, or a new one asked for and kept.
    fn token(&self) -> Result<String, String> {
        let mut token = self.token.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(token) = token.as_ref() {
            return Ok(token.clone());
        }
        let answer: Value = self
            .client
            .post(format!("{}/v1/installs", self.base_url))
            .timeout(PROBE_LIMIT)
            .json(&json!({"app_version": env!("CARGO_PKG_VERSION")}))
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .map_err(|error| short(&error))?;
        let minted = answer["token"]
            .as_str()
            .filter(|minted| is_token(minted))
            .ok_or("the proxy gave no install token")?
            .to_owned();
        if let Some(file) = &self.token_file {
            keep_token(file, &minted);
        }
        *token = Some(minted.clone());
        Ok(minted)
    }

    /// Looks at the proxy on a thread of its own, unless a look is already
    /// under way.
    fn probe_now(self: &Arc<Self>) {
        if self.probing.swap(true, Ordering::AcqRel) {
            return;
        }
        let cloud = Arc::clone(self);
        thread::spawn(move || {
            match cloud.probe() {
                Ok(()) => cloud.came_up(),
                Err(why) => cloud.went_down(&why, Failure::Unreachable),
            }
            cloud.probing.store(false, Ordering::Release);
        });
    }

    /// Whether the proxy can answer: it is reachable, has a model to ask, and
    /// this install has a token.
    fn probe(&self) -> Result<(), String> {
        let health: Value = self
            .client
            .get(format!("{}/healthz", self.base_url))
            .timeout(PROBE_LIMIT)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .map_err(|error| short(&error))?;
        if health["chat"] != true {
            return Err("the proxy has no model to ask".into());
        }
        self.token().map(|_| ())
    }

    fn came_up(&self) {
        let mut health = self.health();
        if !health.up {
            eprintln!("[cloud] up: answering through {}", self.host());
            telemetry::cloud_changed(true, "");
        }
        health.up = true;
        health.known = true;
        health.backoff = BACKOFF_MIN;
        health.reason.clear();
    }

    fn went_down(&self, why: &str, failure: Failure) {
        if failure == Failure::Rejected {
            eprintln!("[cloud] the proxy refused a request as malformed: {why}");
            return;
        }
        if failure == Failure::Unauthorized {
            self.forget_token();
        }
        let mut health = self.health();
        let wait = match failure {
            // A new token is asked for at the next look, which can be now.
            Failure::Unauthorized => Duration::ZERO,
            Failure::Busy => BUSY_WAIT,
            Failure::Refused => BACKOFF_MAX,
            _ if health.up || !health.known => BACKOFF_MIN,
            _ => (health.backoff * 2).min(BACKOFF_MAX),
        };
        if !wait.is_zero() {
            health.backoff = wait;
        }
        health.retry_at = Instant::now() + wait;
        if health.up || !health.known || health.reason != why {
            eprintln!(
                "[cloud] down ({why}); looking again in {} s",
                wait.as_secs()
            );
        }
        if health.up || !health.known {
            telemetry::cloud_changed(false, why);
        }
        health.up = false;
        health.known = true;
        health.reason = why.to_owned();
    }

    fn forget_token(&self) {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if let Some(file) = &self.token_file {
            let _ = fs::remove_file(file);
        }
    }

    fn health(&self) -> MutexGuard<'_, Health> {
        self.health.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a request's thread hands back as it reads.
enum Event {
    Piece(String),
    Usage(Usage),
    /// The answer stopped because it reached `max_tokens`.
    OutOfTokens,
    /// The stream ended: `clean` when it said it was done.
    End {
        clean: bool,
    },
    /// The proxy answered with a refusal.
    Refused {
        why: String,
        failure: Failure,
    },
    /// The request never got an answer, or its stream broke.
    Failed(String),
}

/// What the proxy's refusal (`{"error": code, "message": ...}`) means here.
fn refusal(status: u16, code: &str, message: &str) -> (Failure, String) {
    let failure = match code {
        "unauthorized" => Failure::Unauthorized,
        "limit" | "upstream_busy" => Failure::Busy,
        "cap" | "not_configured" | "upstream_config" => Failure::Refused,
        "bad_request" | "upstream_rejected" => Failure::Rejected,
        _ if status == 401 => Failure::Unauthorized,
        _ if status == 429 => Failure::Busy,
        _ => Failure::Unreachable,
    };
    let said = if code.is_empty() {
        format!("HTTP {status}")
    } else {
        code.to_owned()
    };
    let why = if message.is_empty() {
        said
    } else {
        format!("{said}: {message}")
    };
    (failure, why)
}

fn usage_of(usage: &Value) -> Option<Usage> {
    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
    let counted = Usage {
        prompt: count("prompt_tokens"),
        hit: count("prompt_cache_hit_tokens"),
        miss: count("prompt_cache_miss_tokens"),
        completion: count("completion_tokens"),
    };
    (counted != Usage::default()).then_some(counted)
}

fn is_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_token(file: &Path) -> Option<String> {
    let token = fs::read_to_string(file).ok()?;
    let token = token.trim();
    is_token(token).then(|| token.to_owned())
}

/// Keeps the token where the next launch finds it, written whole or not at
/// all. Losing it costs nothing but a new one.
fn keep_token(file: &Path, token: &str) {
    let written = file
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| {
            let partial = file.with_extension("partial");
            fs::write(&partial, token)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&partial, fs::Permissions::from_mode(0o600))?;
            }
            fs::rename(&partial, file)
        });
    if let Err(error) = written {
        eprintln!(
            "[cloud] could not keep the install token at {}: {error}",
            file.display()
        );
    }
}

/// A request error in a few words, without the URL reqwest puts in it.
fn short(error: &reqwest::Error) -> String {
    let what = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else if let Some(status) = error.status() {
        return format!("HTTP {}", status.as_u16());
    } else if error.is_decode() {
        "an answer that could not be read"
    } else {
        "the request failed"
    };
    what.to_owned()
}

fn elapsed_ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    /// How the stand-in proxy answers a chat request.
    #[derive(Clone)]
    enum Chat {
        /// Streams these pieces with this pause before each, then usage and `[DONE]`.
        Stream(Vec<&'static str>, Duration),
        /// Streams these pieces, then closes the connection mid-answer.
        Breaks(Vec<&'static str>),
        /// Says nothing for this long, then closes.
        Silent(Duration),
        /// Refuses with this status and error code.
        Refuses(u16, &'static str),
    }

    struct FakeProxy {
        url: String,
        asked: Arc<Mutex<Vec<(String, Value)>>>,
        healthy: Arc<AtomicBool>,
    }

    impl FakeProxy {
        fn start(chat: Chat) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/ella-desktop", listener.local_addr().unwrap());
            let asked = Arc::new(Mutex::new(Vec::new()));
            let chat = Arc::new(Mutex::new(chat));
            let healthy = Arc::new(AtomicBool::new(true));
            let (seen, script, up) = (Arc::clone(&asked), Arc::clone(&chat), Arc::clone(&healthy));
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (seen, script, up) =
                        (Arc::clone(&seen), Arc::clone(&script), Arc::clone(&up));
                    thread::spawn(move || {
                        let _ = serve(stream, &seen, &script, &up);
                    });
                }
            });
            Self {
                url,
                asked,
                healthy,
            }
        }

        fn cloud(&self, token_file: Option<PathBuf>) -> Arc<CloudLlm> {
            Arc::new(CloudLlm::new(self.url.clone(), token_file))
        }

        fn paths(&self) -> Vec<String> {
            self.asked
                .lock()
                .unwrap()
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        }
    }

    fn serve(
        mut stream: TcpStream,
        asked: &Mutex<Vec<(String, Value)>>,
        chat: &Mutex<Chat>,
        healthy: &AtomicBool,
    ) -> std::io::Result<()> {
        let (path, authorization, body) = read_request(&mut stream)?;
        asked.lock().unwrap().push((path.clone(), body.clone()));
        match path.as_str() {
            "/ella-desktop/healthz" => respond(
                &mut stream,
                200,
                &json!({"ok": true, "chat": healthy.load(Ordering::Relaxed)}),
            ),
            "/ella-desktop/v1/installs" => {
                respond(&mut stream, 201, &json!({"token": "ab".repeat(32)}))
            }
            "/ella-desktop/v1/chat" if authorization != format!("Bearer {}", "ab".repeat(32)) => {
                respond(
                    &mut stream,
                    401,
                    &json!({"error": "unauthorized", "message": "no"}),
                )
            }
            "/ella-desktop/v1/chat" => {
                let script = chat.lock().unwrap().clone();
                match script {
                    Chat::Stream(pieces, pause) => {
                        start_stream(&mut stream)?;
                        write!(stream, ": keep-alive\n\n")?;
                        for piece in &pieces {
                            thread::sleep(pause);
                            send_piece(&mut stream, piece)?;
                        }
                        let usage = json!({"choices": [{"delta": {"content": ""}, "finish_reason": "stop"}],
                            "usage": {"prompt_tokens": 120, "prompt_cache_hit_tokens": 100,
                                      "prompt_cache_miss_tokens": 20, "completion_tokens": 9}});
                        write!(stream, "data: {usage}\n\ndata: [DONE]\n\n")?;
                        stream.flush()
                    }
                    Chat::Breaks(pieces) => {
                        start_stream(&mut stream)?;
                        for piece in &pieces {
                            send_piece(&mut stream, piece)?;
                        }
                        // Gone without saying it was done.
                        stream.shutdown(std::net::Shutdown::Both)
                    }
                    Chat::Silent(pause) => {
                        start_stream(&mut stream)?;
                        thread::sleep(pause);
                        Ok(())
                    }
                    Chat::Refuses(status, code) => respond(
                        &mut stream,
                        status,
                        &json!({"error": code, "message": "refused"}),
                    ),
                }
            }
            _ => respond(&mut stream, 404, &json!({"error": "not_found"})),
        }
    }

    fn read_request(stream: &mut TcpStream) -> std::io::Result<(String, String, Value)> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let (mut length, mut authorization) = (0, String::new());
        loop {
            let mut header = String::new();
            reader.read_line(&mut header)?;
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if name.eq_ignore_ascii_case("authorization") {
                    authorization = value.trim().to_owned();
                }
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        Ok((
            path,
            authorization,
            serde_json::from_slice(&body).unwrap_or(Value::Null),
        ))
    }

    fn respond(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
        let body = body.to_string();
        write!(
            stream,
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()
    }

    /// A stream's headers. Its body ends when the connection closes, so a
    /// stream that breaks off and one that finishes differ only in whether it
    /// said it was done.
    fn start_stream(stream: &mut TcpStream) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )?;
        stream.flush()
    }

    fn send_piece(stream: &mut TcpStream, piece: &str) -> std::io::Result<()> {
        write!(
            stream,
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": piece}}]})
        )?;
        stream.flush()
    }

    fn ask(json: bool, first_word_ms: u64) -> Ask {
        Ask {
            max_tokens: 50,
            temperature: 0.65,
            json,
            first_word: Duration::from_millis(first_word_ms),
        }
    }

    fn messages() -> Vec<Value> {
        vec![
            json!({"role": "system", "content": "You are Ella."}),
            json!({"role": "user", "content": "I went to the market."}),
        ]
    }

    /// Waits for the background look the cloud takes at the proxy.
    fn settled(cloud: &Arc<CloudLlm>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while cloud.probing.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn up(cloud: &Arc<CloudLlm>) {
        cloud.probe_now();
        settled(cloud);
        assert!(cloud.usable(), "{:?}", cloud.describe());
    }

    #[test]
    fn a_streamed_answer_comes_piece_by_piece_with_its_usage() {
        let proxy = FakeProxy::start(Chat::Stream(
            vec!["Nice! ", "What did ", "you buy?"],
            Duration::ZERO,
        ));
        let dir = tempfile::tempdir().unwrap();
        let cloud = proxy.cloud(Some(dir.path().join("cloud/install-token")));
        up(&cloud);
        let mut pieces = Vec::new();
        let Asked::Answered(answer) = cloud.ask(&messages(), ask(false, 2_000), |piece, _| {
            pieces.push(piece.to_owned());
            true
        }) else {
            panic!("no answer");
        };
        assert_eq!(answer.text, "Nice! What did you buy?");
        assert_eq!(pieces, ["Nice! ", "What did ", "you buy?"]);
        assert_eq!(
            answer.usage,
            Usage {
                prompt: 120,
                hit: 100,
                miss: 20,
                completion: 9
            }
        );
        assert!(!answer.stopped_early);

        // One token, kept for the next launch, and sent with every request.
        assert_eq!(
            read_token(&dir.path().join("cloud/install-token")),
            Some("ab".repeat(32))
        );
        let chat = proxy
            .asked
            .lock()
            .unwrap()
            .iter()
            .find(|(path, _)| path.ends_with("/v1/chat"))
            .cloned()
            .unwrap();
        assert_eq!(chat.1["max_tokens"], 50);
        assert_eq!(chat.1["stream"], true);
        assert!(chat.1.get("response_format").is_none());
        assert_eq!(
            proxy
                .paths()
                .iter()
                .filter(|path| path.ends_with("/v1/installs"))
                .count(),
            1
        );
    }

    #[test]
    fn a_judge_asks_for_json_and_stops_reading_once_it_has_enough() {
        let proxy = FakeProxy::start(Chat::Stream(
            vec!["{\"lines\":", "[\"a\"", ",\"b\"]}", "tail"],
            Duration::ZERO,
        ));
        let cloud = proxy.cloud(None);
        up(&cloud);
        let Asked::Answered(answer) = cloud.ask(&messages(), ask(true, 2_000), |_, whole| {
            !whole.contains("\"a\"")
        }) else {
            panic!("no answer");
        };
        assert!(answer.stopped_early);
        assert_eq!(answer.text, "{\"lines\":[\"a\"");
        let chat = proxy
            .asked
            .lock()
            .unwrap()
            .iter()
            .find(|(path, _)| path.ends_with("/v1/chat"))
            .cloned()
            .unwrap();
        assert_eq!(chat.1["response_format"], json!({"type": "json_object"}));
    }

    #[test]
    fn no_first_word_in_time_is_unanswered_and_leaves_the_cloud_alone() {
        let proxy = FakeProxy::start(Chat::Silent(Duration::from_secs(3)));
        let cloud = proxy.cloud(None);
        up(&cloud);
        let started = Instant::now();
        let asked = cloud.ask(&messages(), ask(false, 300), |_, _| true);
        assert!(matches!(asked, Asked::Unanswered { .. }), "{asked:?}");
        assert!(
            started.elapsed() < Duration::from_millis(1_500),
            "waited {:?}",
            started.elapsed()
        );
        assert!(!cloud.usable());
        assert!(cloud.describe().1.contains("no first word"));
    }

    #[test]
    fn an_answer_that_breaks_off_partway_is_cut() {
        let proxy = FakeProxy::start(Chat::Breaks(vec!["Nice! ", "What did "]));
        let cloud = proxy.cloud(None);
        up(&cloud);
        let asked = cloud.ask(&messages(), ask(false, 2_000), |_, _| true);
        assert!(matches!(asked, Asked::Cut { .. }), "{asked:?}");
        assert!(!cloud.usable());
    }

    #[test]
    fn a_refusal_says_how_long_to_leave_the_cloud_alone() {
        for (status, code, wait) in [
            (503, "cap", BACKOFF_MAX),
            (503, "not_configured", BACKOFF_MAX),
            (429, "upstream_busy", BUSY_WAIT),
            (502, "upstream", BACKOFF_MIN),
        ] {
            let proxy = FakeProxy::start(Chat::Refuses(status, code));
            let cloud = proxy.cloud(None);
            up(&cloud);
            let asked = cloud.ask(&messages(), ask(false, 2_000), |_, _| true);
            assert!(
                matches!(asked, Asked::Unanswered { .. }),
                "{code}: {asked:?}"
            );
            let health = cloud.health();
            assert!(!health.up, "{code}");
            let left = health.retry_at.saturating_duration_since(Instant::now());
            assert!(
                left > wait - Duration::from_secs(2) && left <= wait,
                "{code}: {left:?}"
            );
        }
    }

    #[test]
    fn a_malformed_request_goes_local_but_the_cloud_stays_up() {
        let proxy = FakeProxy::start(Chat::Refuses(400, "bad_request"));
        let cloud = proxy.cloud(None);
        up(&cloud);
        assert!(matches!(
            cloud.ask(&messages(), ask(false, 2_000), |_, _| true),
            Asked::Unanswered { .. }
        ));
        assert!(cloud.usable());
    }

    #[test]
    fn a_token_the_proxy_no_longer_knows_is_replaced() {
        let proxy = FakeProxy::start(Chat::Stream(vec!["Hello?"], Duration::ZERO));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("install-token");
        fs::write(&file, "cd".repeat(32)).unwrap();
        let cloud = proxy.cloud(Some(file.clone()));
        up(&cloud);
        let asked = cloud.ask(&messages(), ask(false, 2_000), |_, _| true);
        assert!(matches!(asked, Asked::Unanswered { .. }), "{asked:?}");
        assert!(!file.exists(), "the stale token is forgotten");
        // Looked at again straight away, with a new token.
        assert!(!cloud.usable());
        settled(&cloud);
        assert!(cloud.usable());
        assert!(matches!(
            cloud.ask(&messages(), ask(false, 2_000), |_, _| true),
            Asked::Answered(_)
        ));
        assert_eq!(read_token(&file), Some("ab".repeat(32)));
    }

    #[test]
    fn a_proxy_with_no_model_is_down_until_it_has_one() {
        let proxy = FakeProxy::start(Chat::Stream(vec!["Hi"], Duration::ZERO));
        proxy.healthy.store(false, Ordering::Relaxed);
        let cloud = proxy.cloud(None);
        cloud.probe_now();
        settled(&cloud);
        assert!(!cloud.usable());
        assert!(cloud.describe().1.contains("no model"));
        // Its wait over, the next look finds it answering.
        proxy.healthy.store(true, Ordering::Relaxed);
        cloud.health().retry_at = Instant::now();
        assert!(!cloud.usable());
        settled(&cloud);
        assert!(cloud.usable());
    }

    #[test]
    fn a_proxy_out_of_reach_is_down_and_looked_at_less_often() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/ella-desktop", listener.local_addr().unwrap());
        drop(listener);
        let cloud = Arc::new(CloudLlm::new(url, None));
        cloud.probe_now();
        settled(&cloud);
        assert!(!cloud.usable());
        let first = cloud.health().backoff;
        cloud.health().retry_at = Instant::now();
        assert!(!cloud.usable());
        settled(&cloud);
        assert_eq!(first, BACKOFF_MIN);
        assert_eq!(cloud.health().backoff, BACKOFF_MIN * 2);
    }

    #[test]
    fn the_proxy_is_named_by_its_host_and_its_refusals_by_their_code() {
        assert_eq!(
            CloudLlm::new(format!("{DEFAULT_PROXY}/"), None).host(),
            "xoeydvvslyvslvpaajmi.supabase.co"
        );
        assert_eq!(refusal(503, "cap", "").0, Failure::Refused);
        assert_eq!(refusal(401, "", "").0, Failure::Unauthorized);
        assert_eq!(refusal(500, "", "").1, "HTTP 500");
    }
}
