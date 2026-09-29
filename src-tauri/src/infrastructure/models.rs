//! First-run and post-update model delivery.
//!
//! Weights are the one part of Ella that cannot ride inside the installer:
//! they are ~2.3 GB against a 2 GB cap on a release asset, and shipping them
//! in the bundle would make every code fix a multi-gigabyte re-download. So
//! the installer carries the binaries and this module fetches the weights into
//! app data on first launch.
//!
//! That split is also what makes a model change deployable. The manifest is
//! compiled into the binary, and every downloaded file is recorded next to the
//! weights with the variant and URL it came from. A release that points `llm`
//! at a different GGUF therefore arrives as an ordinary app update: the
//! recorded variant no longer matches the manifest, and the new file is
//! fetched on the next launch. Nothing has to be versioned by hand.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{EllaError, EllaResult};

/// The same manifest the development tooling reads, compiled in so that an
/// installed build can never disagree with the release it came from.
const MANIFEST: &str = include_str!("../../../tooling/models.json");

/// What the app cannot start without. `stt_fallback` is deliberately absent:
/// Whisper only covers a Canary failure, and 488 MB is a poor trade on a
/// classroom connection when the primary engine is in-process. `vad` and
/// `scorer` are absent because nothing reads them yet.
const REQUIRED: [&str; 2] = ["llm", "stt"];

/// Records what actually landed on disk, so a manifest change is detectable.
const STATE_FILE: &str = ".ella-models.json";

/// Hugging Face rate-limits by address, and a school or office puts every
/// machine behind one. A first run that gives up on the first 429 would leave
/// a classroom of installs stuck at the same moment, so a transfer is retried
/// with a widening pause before it is called a failure.
const DOWNLOAD_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Manifest group: `llm`, `stt`, and so on.
    pub key: String,
    /// The chosen variant name, recorded so a swap is visible after the fact.
    pub variant: String,
    /// Where the file has to end up, relative to the models root.
    pub target: PathBuf,
    pub url: String,
    pub sha256: Option<String>,
    /// Manifest figure, used only to draw a progress bar before the server
    /// sends a length.
    pub approximate_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct InstalledState {
    /// Keyed by the target path as written in the manifest.
    files: std::collections::BTreeMap<String, InstalledFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InstalledFile {
    variant: String,
    url: String,
    sha256: Option<String>,
}

/// Why a download stopped, in the terms the setup screen needs: whether to
/// blame the connection, and whether trying again by itself can help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trouble {
    /// The connection failed, dropped or stalled.
    Network,
    /// The server is rate-limiting or struggling (429, 5xx).
    Busy,
    /// The network refuses the download outright: a school filter or proxy
    /// answering 403, a 404. Waiting will not change it.
    Blocked,
    /// There is no room left on the laptop.
    Disk,
    /// The file arrived, but not intact.
    Broken,
}

impl Trouble {
    pub fn of(error: &EllaError) -> Self {
        match error {
            EllaError::Validation(_) => Self::Broken,
            EllaError::HttpStatus { status, .. } => match status {
                408 | 429 | 500..=599 => Self::Busy,
                _ => Self::Blocked,
            },
            EllaError::Io(reason) if disk_full(reason) => Self::Disk,
            _ => Self::Network,
        }
    }

    /// Whether waiting and trying the same request again can succeed.
    fn passes(self) -> bool {
        matches!(self, Self::Network | Self::Busy)
    }
}

fn disk_full(reason: &std::io::Error) -> bool {
    // ENOSPC, and ERROR_HANDLE_DISK_FULL / ERROR_DISK_FULL on Windows.
    let code = reason.raw_os_error();
    reason.kind() == std::io::ErrorKind::StorageFull
        || (cfg!(unix) && code == Some(28))
        || (cfg!(windows) && matches!(code, Some(39) | Some(112)))
}

/// A download that gave up, and why.
#[derive(Debug)]
pub struct DownloadFailure {
    pub trouble: Trouble,
    pub error: EllaError,
}

impl std::fmt::Display for DownloadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl From<EllaError> for DownloadFailure {
    fn from(error: EllaError) -> Self {
        Self {
            trouble: Trouble::of(&error),
            error,
        }
    }
}

/// One step of progress, reported to the window so a 2 GB first run is not a
/// frozen screen.
#[derive(Debug, Clone, Serialize)]
pub struct ModelProgress {
    pub key: String,
    pub variant: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// Position in this launch's work list, 1-based, and how long it is.
    pub index: usize,
    pub of: usize,
    /// 1 on the first try. Above that the connection dropped or the host
    /// pushed back, and the screen should say so rather than look frozen.
    pub attempt: u32,
    /// `total_bytes` is the manifest's guess rather than the server's figure,
    /// as it is before a response arrives, and should not replace a figure
    /// the server has already given.
    pub total_is_estimate: bool,
    /// On a retry, what went wrong with the attempt before it.
    pub trouble: Option<Trouble>,
}

/// The models this build wants, in download order.
pub fn required_models() -> EllaResult<Vec<ModelSpec>> {
    let manifest: Value = serde_json::from_str(MANIFEST)?;
    let models = manifest
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| EllaError::Engine("Model manifest has no `models` object".into()))?;

    let mut specs = Vec::new();
    for key in REQUIRED {
        let group = models
            .get(key)
            .ok_or_else(|| EllaError::Engine(format!("Model manifest has no `{key}` group")))?;
        let variant_name = group
            .get("default")
            .and_then(Value::as_str)
            .ok_or_else(|| EllaError::Engine(format!("`{key}` names no default variant")))?;
        let variant = group
            .get("variants")
            .and_then(|variants| variants.get(variant_name))
            .ok_or_else(|| {
                EllaError::Engine(format!("`{key}` default `{variant_name}` has no entry"))
            })?;
        // `target` is written relative to the engine root in the manifest
        // ("models/llm/model.gguf"); the models root is that `models`
        // directory, so the first component comes off.
        let target = group
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| EllaError::Engine(format!("`{key}` names no target path")))?;
        let target = target.strip_prefix("models/").unwrap_or(target);

        let Some(url) = variant_url(variant) else {
            // `source: local` entries ship inside the installer. Nothing to
            // fetch, and not an error.
            continue;
        };

        specs.push(ModelSpec {
            key: key.to_string(),
            variant: variant_name.to_string(),
            target: PathBuf::from(target),
            url,
            sha256: variant
                .get("sha256")
                .and_then(Value::as_str)
                .map(str::to_string),
            approximate_bytes: variant
                .get("size_mb")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_mul(1024 * 1024),
        });
    }
    Ok(specs)
}

/// An explicit `url` wins; otherwise a Hugging Face repo and file name are
/// enough to build one. Anything else — `source: local` — is bundled, not
/// fetched.
fn variant_url(variant: &Value) -> Option<String> {
    if let Some(url) = variant.get("url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    let repo = variant.get("repo").and_then(Value::as_str)?;
    let file = variant.get("file").and_then(Value::as_str)?;
    Some(format!("https://huggingface.co/{repo}/resolve/main/{file}"))
}

/// What this launch has to download: anything absent, and anything whose
/// recorded origin no longer matches the manifest.
pub fn outstanding(models_root: &Path) -> EllaResult<Vec<ModelSpec>> {
    let state = read_state(models_root);
    Ok(required_models()?
        .into_iter()
        .filter(|spec| {
            let key = spec.target.to_string_lossy().replace('\\', "/");
            let present = models_root.join(&spec.target).exists();
            let recorded = state.files.get(&key);
            let unchanged = recorded.is_some_and(|file| {
                file.variant == spec.variant && file.url == spec.url
            });
            !(present && unchanged)
        })
        .collect())
}

/// Downloads whatever `outstanding` reports, reporting progress as it goes.
///
/// Files land under a `.part` name and are renamed only once complete and
/// verified, so an interrupted download can never be mistaken for a usable
/// model — and can be resumed rather than restarted, which matters on the
/// connections this app is meant for.
pub fn ensure(
    models_root: &Path,
    progress: &mut dyn FnMut(ModelProgress),
) -> Result<Vec<ModelSpec>, DownloadFailure> {
    let work = outstanding(models_root)?;
    let total_steps = work.len();
    for (index, spec) in work.iter().enumerate() {
        let mut retrying = None;
        for attempt in 1..=DOWNLOAD_ATTEMPTS {
            match download(models_root, spec, index + 1, total_steps, attempt, retrying, progress) {
                Ok(()) => break,
                // A file that fails its checksum will fail it again, a full
                // disk stays full, and a network that refuses the download
                // will refuse it again: retrying only makes the learner wait
                // longer to hear why.
                Err(reason) if !Trouble::of(&reason).passes() => return Err(reason.into()),
                Err(reason) if attempt < DOWNLOAD_ATTEMPTS => {
                    // The partial file survives, so a retry resumes from where
                    // the connection dropped rather than starting over.
                    let pause = Duration::from_secs(5 * 2_u64.pow(attempt - 1));
                    eprintln!(
                        "[setup] {} download attempt {attempt} failed ({reason}); retrying in {}s",
                        spec.key,
                        pause.as_secs()
                    );
                    // Said before the pause rather than after it, so the
                    // screen owns up to the dropped connection straight away
                    // instead of sitting still for the length of the wait.
                    let fetched = fetched_so_far(models_root, spec);
                    retrying = Some(Trouble::of(&reason));
                    progress(ModelProgress {
                        key: spec.key.clone(),
                        variant: spec.variant.clone(),
                        downloaded_bytes: fetched,
                        total_bytes: spec.approximate_bytes.max(fetched),
                        index: index + 1,
                        of: total_steps,
                        attempt: attempt + 1,
                        total_is_estimate: true,
                        trouble: retrying,
                    });
                    std::thread::sleep(pause);
                }
                Err(reason) => return Err(reason.into()),
            }
        }
        record(models_root, spec)?;
    }
    Ok(work)
}

/// How much of a file an earlier, interrupted transfer left on disk, which the
/// next one resumes from.
pub fn fetched_so_far(models_root: &Path, spec: &ModelSpec) -> u64 {
    partial_path(models_root, spec)
        .metadata()
        .map(|meta| meta.len())
        .unwrap_or(0)
}

fn partial_path(models_root: &Path, spec: &ModelSpec) -> PathBuf {
    models_root.join(&spec.target).with_extension("part")
}

/// How often a transfer that is moving reports, however slowly it moves.
const REPORT_EVERY: Duration = Duration::from_secs(1);

/// Without a deadline on each read, a connection that stays open but stops
/// sending — a captive portal, a proxy that has given up, a stalled edge —
/// blocks the download for good, and the setup screen with it. In the
/// blocking client this bounds every separate read, not the whole transfer,
/// so a slow link that keeps moving is never cut off.
const STALLED_AFTER: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(60)
};

fn download(
    models_root: &Path,
    spec: &ModelSpec,
    index: usize,
    of: usize,
    attempt: u32,
    retrying: Option<Trouble>,
    progress: &mut dyn FnMut(ModelProgress),
) -> EllaResult<()> {
    let destination = models_root.join(&spec.target);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = partial_path(models_root, spec);
    let already = partial.metadata().map(|meta| meta.len()).unwrap_or(0);

    // Before the request, which can take up to the connect timeout to answer,
    // so the screen shows the download from the moment it begins.
    progress(ModelProgress {
        key: spec.key.clone(),
        variant: spec.variant.clone(),
        downloaded_bytes: already,
        total_bytes: spec.approximate_bytes.max(already),
        index,
        of,
        attempt,
        total_is_estimate: true,
        trouble: retrying,
    });

    let client = reqwest::blocking::Client::builder()
        // No overall deadline: this is gigabytes over a connection that may be
        // slow without being broken. What is caught is a connect that never
        // lands and a transfer that stops moving, and either resumes from the
        // partial file on the next attempt.
        .timeout(STALLED_AFTER)
        .connect_timeout(Duration::from_secs(30))
        .build()?;
    let mut request = client.get(&spec.url);
    if already > 0 {
        request = request.header("Range", format!("bytes={already}-"));
    }
    let mut response = request.send()?;

    // The partial file is already the whole file, or longer: a transfer that
    // finished but was never checked and renamed, because Ella closed in
    // between or a scanner held the file. Asking for the rest of it will
    // never succeed, so it starts over clean rather than failing the same
    // way on every attempt and every launch.
    if already > 0 && response.status().as_u16() == 416 {
        // `bytes */N` is the file's real length. When the partial file is
        // exactly that long it is finished, and only the checks and the
        // rename remain; otherwise it is not this file, and starts over.
        let length = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().strip_prefix("bytes */"))
            .and_then(|length| length.trim().parse::<u64>().ok());
        drop(response);
        if length == Some(already) {
            return finish(models_root, spec, index, of, attempt, already, retrying, progress);
        }
        fs::remove_file(&partial)?;
        return download(models_root, spec, index, of, attempt, retrying, progress);
    }

    // A server that ignores the range restarts the file; anything else is a
    // real failure worth surfacing with its status.
    let resuming = response.status().as_u16() == 206;
    if !response.status().is_success() {
        return Err(EllaError::HttpStatus {
            status: response.status().as_u16(),
            message: format!("Downloading {} failed with HTTP {}", spec.key, response.status()),
        });
    }
    let mut written = if resuming { already } else { 0 };
    let server_total = response.content_length().map(|length| length + written);
    let total = server_total.unwrap_or(spec.approximate_bytes);
    let total_is_estimate = server_total.is_none();

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&partial)?;

    let mut buffer = vec![0_u8; 1024 * 256];
    let mut since_report = 0_u64;
    let mut reported_at = Instant::now();
    progress(ModelProgress {
        key: spec.key.clone(),
        variant: spec.variant.clone(),
        downloaded_bytes: written,
        total_bytes: total,
        index,
        of,
        attempt,
        total_is_estimate,
        trouble: retrying,
    });
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        written += read as u64;
        since_report += read as u64;
        // Every 4 MB, which keeps the IPC channel from being the bottleneck
        // on a fast link, or every second, so a slow one — a classroom
        // sharing a connection — still moves the bar and never reads as
        // stalled while bytes are arriving.
        if since_report >= 4 * 1024 * 1024 || reported_at.elapsed() >= REPORT_EVERY {
            since_report = 0;
            reported_at = Instant::now();
            progress(ModelProgress {
                key: spec.key.clone(),
                variant: spec.variant.clone(),
                downloaded_bytes: written,
                total_bytes: total,
                index,
                of,
                attempt,
                total_is_estimate,
                trouble: retrying,
            });
        }
    }
    file.flush()?;
    drop(file);

    finish(models_root, spec, index, of, attempt, written, retrying, progress)
}

/// A transfer that is complete on disk: check it, and move it into place.
fn finish(
    models_root: &Path,
    spec: &ModelSpec,
    index: usize,
    of: usize,
    attempt: u32,
    length: u64,
    retrying: Option<Trouble>,
    progress: &mut dyn FnMut(ModelProgress),
) -> EllaResult<()> {
    let destination = models_root.join(&spec.target);
    let partial = partial_path(models_root, spec);

    if let Some(expected) = &spec.sha256 {
        let actual = sha256_of(&partial)?;
        if !actual.eq_ignore_ascii_case(expected) {
            // Keeping a corrupt file would make the next launch resume from
            // the end of it and fail identically, forever.
            let _ = fs::remove_file(&partial);
            return Err(EllaError::Validation(format!(
                "{} failed its checksum. Expected {expected}, got {actual}.",
                spec.key
            )));
        }
    }

    rename_when_free(&partial, &destination)?;
    progress(ModelProgress {
        key: spec.key.clone(),
        variant: spec.variant.clone(),
        downloaded_bytes: length,
        total_bytes: length,
        index,
        of,
        attempt,
        total_is_estimate: false,
        trouble: retrying,
    });
    Ok(())
}

/// A virus scanner, or the search indexer, opens a freshly written file the
/// moment it closes, and on Windows that refuses the rename until it lets go
/// — usually within a second or two. Waiting that out beats failing a whole
/// 2 GB download at its last step.
fn rename_when_free(from: &Path, to: &Path) -> EllaResult<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(reason) if tries < 10 && held_by_another_process(&reason) => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(reason) => return Err(reason.into()),
        }
    }
}

fn held_by_another_process(reason: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION, which the standard
    // library does not fold into `PermissionDenied` as it does access denied.
    let locked = cfg!(windows) && matches!(reason.raw_os_error(), Some(32) | Some(33));
    locked || reason.kind() == std::io::ErrorKind::PermissionDenied
}

fn sha256_of(path: &Path) -> EllaResult<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn read_state(models_root: &Path) -> InstalledState {
    fs::read_to_string(models_root.join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn record(models_root: &Path, spec: &ModelSpec) -> EllaResult<()> {
    let mut state = read_state(models_root);
    state.files.insert(
        spec.target.to_string_lossy().replace('\\', "/"),
        InstalledFile {
            variant: spec.variant.clone(),
            url: spec.url.clone(),
            sha256: spec.sha256.clone(),
        },
    );
    fs::create_dir_all(models_root)?;
    // Written beside the real file and renamed over it, so a quit or a power
    // cut mid-write leaves the old record rather than half a new one. A
    // record that does not parse reads as empty, and an empty record sends an
    // offline laptop off to download weights it already has.
    let temporary = models_root.join(format!("{STATE_FILE}.part"));
    let mut file = fs::File::create(&temporary)?;
    file.write_all(serde_json::to_string_pretty(&state)?.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, models_root.join(STATE_FILE))?;
    Ok(())
}

/// Whether every model the app needs has a file at its place on disk, whatever
/// the record says about where it came from. When a download cannot happen —
/// no internet — weights that are already there are worth loading: they are
/// what Ella ran on last time, and a laptop that is offline should still talk.
pub fn all_on_disk(models_root: &Path) -> EllaResult<bool> {
    Ok(required_models()?
        .iter()
        .all(|spec| models_root.join(&spec.target).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_yields_the_two_models_the_app_cannot_start_without() {
        let specs = required_models().unwrap();
        let keys: Vec<&str> = specs.iter().map(|spec| spec.key.as_str()).collect();
        assert_eq!(keys, vec!["llm", "stt"]);
    }

    #[test]
    fn targets_are_relative_to_the_models_root_not_the_engine_root() {
        let specs = required_models().unwrap();
        let llm = specs.iter().find(|spec| spec.key == "llm").unwrap();
        assert_eq!(llm.target, PathBuf::from("llm/model.gguf"));
        assert!(llm.url.starts_with("https://huggingface.co/"));
    }

    #[test]
    fn canary_carries_the_checksum_the_manifest_pins() {
        let specs = required_models().unwrap();
        let stt = specs.iter().find(|spec| spec.key == "stt").unwrap();
        assert_eq!(
            stt.sha256.as_deref(),
            Some("e13c7f5d0952b056a027cfffec13e3a3a134d1608babed24f983568f141e297c")
        );
    }

    #[test]
    fn everything_is_outstanding_when_nothing_has_been_downloaded() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(outstanding(root.path()).unwrap().len(), 2);
    }

    #[test]
    fn a_recorded_file_stops_being_outstanding_and_a_changed_variant_starts_again() {
        let root = tempfile::tempdir().unwrap();
        let spec = required_models()
            .unwrap()
            .into_iter()
            .find(|spec| spec.key == "stt")
            .unwrap();
        fs::create_dir_all(root.path().join("stt")).unwrap();
        fs::write(root.path().join(&spec.target), b"weights").unwrap();
        record(root.path(), &spec).unwrap();
        assert!(!outstanding(root.path())
            .unwrap()
            .iter()
            .any(|outstanding| outstanding.key == "stt"));

        // A release that repoints the manifest is exactly this: same path,
        // different origin. The file on disk is stale and must be refetched.
        let moved = ModelSpec {
            url: "https://example.invalid/canary-v2.gguf".into(),
            ..spec
        };
        record(root.path(), &moved).unwrap();
        assert!(outstanding(root.path())
            .unwrap()
            .iter()
            .any(|outstanding| outstanding.key == "stt"));
    }

    #[test]
    fn weights_on_disk_are_usable_offline_whatever_the_record_says() {
        let root = tempfile::tempdir().unwrap();
        assert!(!all_on_disk(root.path()).unwrap());
        for spec in required_models().unwrap() {
            let target = root.path().join(&spec.target);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, b"weights").unwrap();
        }
        // No record at all, as after a lost or corrupt `.ella-models.json`:
        // both are "outstanding", but a laptop without internet can still
        // load them.
        assert_eq!(outstanding(root.path()).unwrap().len(), 2);
        assert!(all_on_disk(root.path()).unwrap());

        let stt = required_models().unwrap().into_iter().find(|spec| spec.key == "stt").unwrap();
        fs::remove_file(root.path().join(&stt.target)).unwrap();
        assert!(!all_on_disk(root.path()).unwrap());
    }

    #[test]
    fn the_record_is_replaced_whole_and_leaves_nothing_half_written() {
        let root = tempfile::tempdir().unwrap();
        let specs = required_models().unwrap();
        for spec in &specs {
            record(root.path(), spec).unwrap();
        }
        assert_eq!(read_state(root.path()).files.len(), 2);
        assert!(!root.path().join(format!("{STATE_FILE}.part")).exists());
    }

    /// A one-connection-at-a-time HTTP server on loopback. `respond` gets the
    /// request's `Range` header, if any, and writes the whole response.
    fn serve(
        respond: impl Fn(Option<String>, &mut std::net::TcpStream) + Send + 'static,
    ) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut range = None;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range:") {
                        range = Some(value.trim().to_string());
                    }
                }
                respond(range, &mut stream);
            }
        });
        format!("http://{address}/model.bin")
    }

    fn spec_at(url: String) -> ModelSpec {
        ModelSpec {
            key: "llm".into(),
            variant: "test".into(),
            target: PathBuf::from("llm/model.bin"),
            url,
            sha256: None,
            approximate_bytes: 11,
        }
    }

    #[test]
    fn a_partial_file_that_is_already_whole_starts_over_instead_of_failing_forever() {
        let url = serve(|range, stream| {
            let reply = if range.is_some() {
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world".to_string()
            };
            let _ = stream.write_all(reply.as_bytes());
        });
        let root = tempfile::tempdir().unwrap();
        let spec = spec_at(url);
        let partial = partial_path(root.path(), &spec);
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        fs::write(&partial, b"hello world").unwrap();

        download(root.path(), &spec, 1, 1, 1, None, &mut |_| {}).unwrap();

        assert_eq!(fs::read(root.path().join(&spec.target)).unwrap(), b"hello world");
        assert!(!partial.exists());
    }

    #[test]
    fn a_partial_file_the_server_says_is_whole_is_finished_not_fetched_again() {
        let url = serve(|range, stream| {
            let reply = if range.is_some() {
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */11\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
            } else {
                // Only a fresh download would ever see this body.
                "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nWRONG WORLD".to_string()
            };
            let _ = stream.write_all(reply.as_bytes());
        });
        let root = tempfile::tempdir().unwrap();
        let spec = spec_at(url);
        let partial = partial_path(root.path(), &spec);
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        fs::write(&partial, b"hello world").unwrap();

        let mut last = None;
        download(root.path(), &spec, 1, 1, 1, None, &mut |progress| last = Some(progress)).unwrap();

        assert_eq!(fs::read(root.path().join(&spec.target)).unwrap(), b"hello world");
        let last = last.unwrap();
        assert_eq!((last.downloaded_bytes, last.total_bytes), (11, 11));
        assert!(!last.total_is_estimate);
    }

    #[test]
    fn what_stopped_a_download_decides_whether_waiting_can_help() {
        let status = |status| EllaError::HttpStatus { status, message: String::new() };
        assert_eq!(Trouble::of(&status(403)), Trouble::Blocked);
        assert_eq!(Trouble::of(&status(404)), Trouble::Blocked);
        assert_eq!(Trouble::of(&status(429)), Trouble::Busy);
        assert_eq!(Trouble::of(&status(503)), Trouble::Busy);
        assert_eq!(Trouble::of(&EllaError::Validation("checksum".into())), Trouble::Broken);
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert_eq!(Trouble::of(&EllaError::Io(full)), Trouble::Disk);
        let reset = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        assert_eq!(Trouble::of(&EllaError::Io(reset)), Trouble::Network);

        assert!(Trouble::Network.passes() && Trouble::Busy.passes());
        assert!(!Trouble::Blocked.passes() && !Trouble::Disk.passes() && !Trouble::Broken.passes());
    }

    #[test]
    fn a_refused_download_says_which_status_refused_it() {
        let url = serve(|_, stream| {
            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let root = tempfile::tempdir().unwrap();
        let error = download(root.path(), &spec_at(url), 1, 1, 1, None, &mut |_| {}).unwrap_err();
        assert!(matches!(error, EllaError::HttpStatus { status: 403, .. }), "{error:?}");
        assert_eq!(Trouble::of(&error), Trouble::Blocked);
    }

    #[test]
    fn a_slow_transfer_still_reports_every_second() {
        let url = serve(|_, stream| {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 30\r\nConnection: close\r\n\r\n");
            // 30 bytes over about three seconds: far below the 4 MB step.
            for _ in 0..10 {
                let _ = stream.write_all(b"abc");
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        let root = tempfile::tempdir().unwrap();
        let spec = spec_at(url);

        let mut during = 0;
        download(root.path(), &spec, 1, 1, 1, None, &mut |progress| {
            if progress.downloaded_bytes > 0 && progress.downloaded_bytes < 30 {
                during += 1;
            }
        })
        .unwrap();

        assert!(during >= 2, "only {during} reports while the bytes trickled in");
    }

    #[test]
    fn a_transfer_that_stops_moving_fails_and_keeps_what_it_fetched() {
        let url = serve(|_, stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nhello",
            );
            let _ = stream.flush();
            // Open, and silent: the connection a captive portal leaves.
            std::thread::sleep(Duration::from_secs(30));
        });
        let root = tempfile::tempdir().unwrap();
        let spec = spec_at(url);

        let started = std::time::Instant::now();
        let outcome = download(root.path(), &spec, 1, 1, 1, None, &mut |_| {});

        assert!(outcome.is_err(), "a stalled transfer must end in an error");
        assert!(started.elapsed() < Duration::from_secs(20), "and not wait on the silence");
        assert_eq!(fetched_so_far(root.path(), &spec), 5, "the bytes that arrived are kept to resume from");
    }

    #[test]
    fn the_screen_hears_about_a_download_before_the_server_answers() {
        let url = serve(|_, stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world",
            );
        });
        let root = tempfile::tempdir().unwrap();
        let spec = spec_at(url);
        let partial = partial_path(root.path(), &spec);
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        fs::write(&partial, b"hel").unwrap();

        let mut reports = Vec::new();
        // The server ignores the range and sends the whole file, which the
        // download restarts from.
        download(root.path(), &spec, 1, 1, 1, None, &mut |progress| reports.push(progress.downloaded_bytes)).unwrap();

        assert_eq!(reports.first(), Some(&3), "the first report is what was already on disk");
        assert_eq!(reports.last(), Some(&11));
    }
}
