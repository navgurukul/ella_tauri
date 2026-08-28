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
use std::time::Duration;

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
) -> EllaResult<Vec<ModelSpec>> {
    let work = outstanding(models_root)?;
    let total_steps = work.len();
    for (index, spec) in work.iter().enumerate() {
        for attempt in 1..=DOWNLOAD_ATTEMPTS {
            match download(models_root, spec, index + 1, total_steps, attempt, progress) {
                Ok(()) => break,
                // A file that does not match its checksum will not match it on
                // a second try; retrying would burn gigabytes to fail again.
                Err(EllaError::Validation(reason)) => {
                    return Err(EllaError::Validation(reason))
                }
                Err(reason) if attempt < DOWNLOAD_ATTEMPTS => {
                    // The partial file survives, so a retry resumes from where
                    // the connection dropped rather than starting over.
                    let pause = Duration::from_secs(5 * 2_u64.pow(attempt - 1));
                    eprintln!(
                        "[setup] {} download attempt {attempt} failed ({reason}); retrying in {}s",
                        spec.key,
                        pause.as_secs()
                    );
                    std::thread::sleep(pause);
                }
                Err(reason) => return Err(reason),
            }
        }
        record(models_root, spec)?;
    }
    Ok(work)
}

fn download(
    models_root: &Path,
    spec: &ModelSpec,
    index: usize,
    of: usize,
    attempt: u32,
    progress: &mut dyn FnMut(ModelProgress),
) -> EllaResult<()> {
    let destination = models_root.join(&spec.target);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = destination.with_extension("part");
    let already = partial.metadata().map(|meta| meta.len()).unwrap_or(0);

    let client = reqwest::blocking::Client::builder()
        // No overall deadline: this is gigabytes over a connection that may be
        // slow without being broken. A stalled *connect* is the failure worth
        // catching, and an interrupted transfer resumes on the next launch.
        .timeout(None)
        .connect_timeout(Duration::from_secs(30))
        .build()?;
    let mut request = client.get(&spec.url);
    if already > 0 {
        request = request.header("Range", format!("bytes={already}-"));
    }
    let mut response = request.send()?;

    // A server that ignores the range restarts the file; anything else is a
    // real failure worth surfacing with its status.
    let resuming = response.status().as_u16() == 206;
    if !response.status().is_success() {
        return Err(EllaError::Engine(format!(
            "Downloading {} failed with HTTP {}",
            spec.key,
            response.status()
        )));
    }
    let mut written = if resuming { already } else { 0 };
    let total = response
        .content_length()
        .map(|length| length + written)
        .unwrap_or(spec.approximate_bytes);

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&partial)?;

    let mut buffer = vec![0_u8; 1024 * 256];
    let mut since_report = 0_u64;
    progress(ModelProgress {
        key: spec.key.clone(),
        variant: spec.variant.clone(),
        downloaded_bytes: written,
        total_bytes: total,
        index,
        of,
        attempt,
    });
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        written += read as u64;
        since_report += read as u64;
        // Roughly every 4 MB: often enough for a smooth bar, rarely enough
        // that the IPC channel is not the bottleneck on a fast link.
        if since_report >= 4 * 1024 * 1024 {
            since_report = 0;
            progress(ModelProgress {
                key: spec.key.clone(),
                variant: spec.variant.clone(),
                downloaded_bytes: written,
                total_bytes: total,
                index,
                of,
                attempt,
            });
        }
    }
    file.flush()?;
    drop(file);

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

    fs::rename(&partial, &destination)?;
    progress(ModelProgress {
        key: spec.key.clone(),
        variant: spec.variant.clone(),
        downloaded_bytes: total,
        total_bytes: total,
        index,
        of,
        attempt,
    });
    Ok(())
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
    fs::write(
        models_root.join(STATE_FILE),
        serde_json::to_string_pretty(&state)?,
    )?;
    Ok(())
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
}
