//! First-run setup: fetching the weights, then bringing the local engine up.
//!
//! Nobody gets past the window's setup screen until this says `ready`, because
//! a talk before then can only fail. So the window has to know, at any moment,
//! where setup has got to. Events carry each step as it happens, but one sent
//! before the window subscribed is simply gone, and a window reloaded in the
//! middle of a download has missed them all. So the latest announcement is also
//! kept here, numbered, for the window to ask for: of the one it was sent and
//! the one it asked for, the higher number is the current one.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::domain::EngineStatus;
use crate::infrastructure::{
    engine_manager::EngineSlot,
    engines::{engine_from_environment, EnginePaths},
    models::{self, Trouble},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupStage {
    Downloading,
    Loading,
    Ready,
    Failed,
}

/// Which half of setup a failure happened in, because they read differently:
/// a download that stopped keeps what it fetched and picks up from there, a
/// model that would not load has nothing left to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupFailure {
    Download,
    Load,
}

#[derive(Clone, Debug, Serialize)]
pub struct SetupProgress {
    pub stage: SetupStage,
    /// What is happening, or for a failure what went wrong, in the backend's
    /// words: the screen writes its own sentence and shows this as the detail.
    pub message: String,
    /// Across every file this launch fetches, not just the current one, so
    /// the bar only ever moves forward.
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// Which file of how many, for a first run that fetches more than one.
    pub index: usize,
    pub of: usize,
    /// 1 on the first try; above that the transfer is being retried.
    pub attempt: u32,
    /// True once this launch has had anything to download. A launch that has
    /// not is an ordinary start, and its model load is a moment's wait rather
    /// than the end of an install.
    pub first_run: bool,
    /// An update's download could not run, and Ella is loading the models she
    /// already had: nothing new arrived, and the next launch tries again.
    pub fell_back: bool,
    pub failure: Option<SetupFailure>,
    /// Why a download is being retried, or why it stopped.
    pub trouble: Option<Trouble>,
    /// Counts every announcement, starting from 1.
    pub seq: u64,
}

impl SetupProgress {
    fn at(stage: SetupStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            downloaded_bytes: 0,
            total_bytes: 0,
            index: 0,
            of: 0,
            attempt: 1,
            first_run: false,
            fell_back: false,
            failure: None,
            trouble: None,
            seq: 0,
        }
    }

    fn failed(failure: SetupFailure, message: impl Into<String>) -> Self {
        Self {
            failure: Some(failure),
            ..Self::at(SetupStage::Failed, message)
        }
    }
}

/// Where each announcement goes besides `Setup` itself: the window, in the app.
pub trait SetupSink: Send + Sync {
    fn announce(&self, progress: &SetupProgress);
}

/// One run of setup, kept so "Try again" can start it over.
type SetupJob = Arc<dyn Fn(&Setup) + Send + Sync>;

pub struct Setup {
    latest: Mutex<SetupProgress>,
    running: AtomicBool,
    job: Option<SetupJob>,
    sink: Box<dyn SetupSink>,
}

impl Setup {
    /// An engine that needs no setup — demo mode, or someone else's server —
    /// is ready from the start, and there is never anything to retry.
    pub fn ready(sink: Box<dyn SetupSink>) -> Arc<Self> {
        Arc::new(Self {
            latest: Mutex::new(SetupProgress {
                seq: 1,
                ..SetupProgress::at(SetupStage::Ready, "Ella is ready.")
            }),
            running: AtomicBool::new(false),
            job: None,
            sink,
        })
    }

    /// Setup that has yet to run. Until `start` finds out whether there is
    /// anything to download, it reads as the model loading, which is what an
    /// ordinary launch does next.
    pub fn deferred(
        sink: Box<dyn SetupSink>,
        slot: EngineSlot,
        paths: EnginePaths,
        models_root: PathBuf,
    ) -> Arc<Self> {
        Self::with_job(
            sink,
            Arc::new(move |setup: &Setup| {
                setup.prepare(slot.clone(), paths.clone(), models_root.clone())
            }),
        )
    }

    fn with_job(sink: Box<dyn SetupSink>, job: SetupJob) -> Arc<Self> {
        Arc::new(Self {
            latest: Mutex::new(SetupProgress {
                seq: 1,
                ..SetupProgress::at(SetupStage::Loading, "Getting Ella ready")
            }),
            running: AtomicBool::new(false),
            job: Some(job),
            sink,
        })
    }

    /// The last announcement, whether or not the window heard it.
    pub fn latest(&self) -> SetupProgress {
        self.lock().clone()
    }

    /// Runs setup on its own thread. False, and nothing happens, when there is
    /// nothing to set up or a run is already under way.
    pub fn start(self: &Arc<Self>) -> bool {
        if self.job.is_none() || self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.spawn();
        true
    }

    /// "Try again" on the setup screen. Only a failed setup starts over; while
    /// one is running, or once Ella is ready, the press does nothing. Returns
    /// whether a new run began.
    ///
    /// The failure is replaced before this returns, so the answer to the press
    /// already shows the new attempt: a download picks up where it stopped,
    /// with its bar where it was, rather than sitting on the old failure until
    /// the first bytes arrive.
    pub fn retry(self: &Arc<Self>) -> bool {
        if self.job.is_none() {
            return false;
        }
        let mut latest = self.lock();
        if latest.stage != SetupStage::Failed || self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        let again = if latest.failure == Some(SetupFailure::Download) {
            SetupProgress {
                downloaded_bytes: latest.downloaded_bytes,
                total_bytes: latest.total_bytes,
                index: latest.index,
                of: latest.of,
                ..SetupProgress::at(SetupStage::Downloading, "Trying the download again")
            }
        } else {
            SetupProgress::at(SetupStage::Loading, "Trying again")
        };
        self.announce_to(&mut latest, again);
        drop(latest);
        self.spawn();
        true
    }

    /// Only called with `running` just set, by `start` or `retry`.
    fn spawn(self: &Arc<Self>) {
        let Some(job) = self.job.clone() else { return };
        let setup = Arc::clone(self);
        thread::spawn(move || {
            let _finished = Finished(&setup);
            job(&setup);
        });
    }

    /// For exit: gives a run that is loading the model a moment to drop what
    /// it built. Canary holds GPU buffers that must be freed before the
    /// process tears down, or macOS reports the quit as a crash. A run that
    /// is downloading has nothing like that, and is not waited for.
    pub fn wait_while_loading(&self, limit: Duration) {
        let deadline = std::time::Instant::now() + limit;
        while self.running.load(Ordering::SeqCst)
            && self.latest().stage == SetupStage::Loading
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn lock(&self) -> MutexGuard<'_, SetupProgress> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn announce(&self, progress: SetupProgress) {
        let mut latest = self.lock();
        self.announce_to(&mut latest, progress);
    }

    fn announce_to(&self, latest: &mut SetupProgress, mut progress: SetupProgress) {
        progress.seq = latest.seq + 1;
        // A launch that downloaded stays a first run through the load and any
        // retry after it, so the screen does not change its story halfway.
        progress.first_run |= latest.first_run;
        progress.fell_back |= latest.fell_back;
        *latest = progress;
        // Under the lock, so events leave in the order they are numbered.
        self.sink.announce(latest);
    }

    /// A download failure, with the bar left where it stopped: what was
    /// fetched is kept on disk, and "Try again" carries on from it.
    fn download_failed(&self, reason: String, trouble: Trouble) {
        let mut latest = self.lock();
        let failed = SetupProgress {
            downloaded_bytes: latest.downloaded_bytes,
            total_bytes: latest.total_bytes,
            index: latest.index,
            of: latest.of,
            trouble: Some(trouble),
            ..SetupProgress::failed(SetupFailure::Download, reason)
        };
        self.announce_to(&mut latest, failed);
    }

    /// Fetch whatever weights this build needs, then bring the local engine
    /// up and hand it to the service.
    ///
    /// Everything here is reported to the window and nothing here panics: a
    /// failed download or a model that will not load leaves the app open and
    /// explaining itself, with a way to try again.
    fn prepare(&self, slot: EngineSlot, paths: EnginePaths, models_root: PathBuf) {
        // A retry after a model that would not load starts from nothing, so
        // two copies of a 2 GB model never sit in memory at once.
        slot.clear("Ella is getting set up.");

        match models::outstanding(&models_root) {
            Ok(work) if !work.is_empty() => {
                let approximate: u64 = work.iter().map(|spec| spec.approximate_bytes).sum();
                slot.waiting_on(format!(
                    "Ella is downloading her voice and language models ({} MB). This happens once.",
                    approximate / (1024 * 1024)
                ));
                // Each file's size starts as the manifest's guess and becomes
                // the server's figure once it sends one; what an earlier launch
                // fetched counts from the start.
                let mut totals: Vec<u64> = work.iter().map(|spec| spec.approximate_bytes).collect();
                // Once the server has said how big a file is, a later guess
                // (a reconnect, before the server answers again) does not
                // move the bar back.
                let mut known = vec![false; work.len()];
                let mut fetched: Vec<u64> = work
                    .iter()
                    .map(|spec| models::fetched_so_far(&models_root, spec))
                    .collect();
                // Said now rather than with the first bytes, which can be a
                // connect timeout away: a first run shows its download from
                // the first moment, and never the ordinary start's screen.
                self.announce(SetupProgress {
                    downloaded_bytes: fetched.iter().sum(),
                    total_bytes: totals.iter().zip(&fetched).map(|(total, got)| *total.max(got)).sum(),
                    index: 1,
                    of: work.len(),
                    first_run: true,
                    ..SetupProgress::at(SetupStage::Downloading, "Starting the download")
                });
                let mut report = |progress: models::ModelProgress| {
                    if let Some(position) = progress.index.checked_sub(1).filter(|at| *at < work.len()) {
                        if !progress.total_is_estimate || !known[position] {
                            totals[position] = progress.total_bytes;
                            known[position] |= !progress.total_is_estimate;
                        }
                        fetched[position] = progress.downloaded_bytes;
                    }
                    self.announce(SetupProgress {
                        downloaded_bytes: fetched.iter().sum(),
                        total_bytes: totals.iter().sum(),
                        index: progress.index,
                        of: progress.of,
                        attempt: progress.attempt,
                        first_run: true,
                        trouble: progress.trouble,
                        ..SetupProgress::at(
                            SetupStage::Downloading,
                            format!("Downloading {} ({} of {})", progress.key, progress.index, progress.of),
                        )
                    });
                };
                if let Err(reason) = models::ensure(&models_root, &mut report) {
                    if models::all_on_disk(&models_root).unwrap_or(false) {
                        // Only the download failed, and every model is already
                        // on disk: its record was lost, or an update named a
                        // newer file while the laptop is offline. Ella loads
                        // what she ran on last time and tries the download
                        // again next launch — and the screen says so, rather
                        // than that everything arrived.
                        eprintln!(
                            "[setup] could not refresh the models ({reason}); loading the ones already on disk"
                        );
                        slot.waiting_on("Ella is loading her language model.");
                        self.announce(SetupProgress {
                            fell_back: true,
                            ..SetupProgress::at(SetupStage::Loading, "Loading the models already on this laptop")
                        });
                    } else {
                        // The partial file is kept, so "Try again" — or the
                        // next launch — resumes rather than starting over.
                        slot.waiting_on(format!("Ella could not finish downloading her models: {reason}"));
                        self.download_failed(reason.to_string(), reason.trouble);
                        return;
                    }
                }
            }
            Ok(_) => {}
            Err(reason) => eprintln!("[setup] model manifest unreadable: {reason}"),
        }

        slot.waiting_on("Ella is loading her language model.");
        self.announce(SetupProgress::at(SetupStage::Loading, "Loading the language model"));

        let engine = engine_from_environment(paths);
        // Ella closed while it loaded: its server is already stopped, and
        // what is left of the engine is dropped here and now, while exit
        // waits for it (see `wait_while_loading`).
        if slot.is_closed() {
            return;
        }
        // A server that has only just loaded can miss one short health probe
        // on a laptop still busy paging the model in, and a false "could not
        // load" costs the learner the whole load again.
        let mut status = engine.status();
        for _ in 0..2 {
            if status.ready || slot.is_closed() {
                break;
            }
            thread::sleep(Duration::from_secs(1));
            status = engine.status();
        }
        if !slot.fill(engine) {
            // Ella is closing; there is no window left to tell.
            return;
        }
        if status.ready {
            self.announce(SetupProgress::at(SetupStage::Ready, status.label));
        } else {
            self.announce(SetupProgress::failed(SetupFailure::Load, not_ready_reason(&status)));
        }
    }
}

/// Marks the end of a run however it ends. A run that stopped without saying
/// so — a panic somewhere below — would otherwise leave the screen showing a
/// download that is never going to finish, with no way to try again.
struct Finished<'a>(&'a Setup);

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        let stage = self.0.latest().stage;
        if stage != SetupStage::Ready && stage != SetupStage::Failed {
            self.0.announce(SetupProgress::failed(
                SetupFailure::Load,
                "Setup stopped unexpectedly.",
            ));
        }
        self.0.running.store(false, Ordering::SeqCst);
    }
}

/// Why an engine that loaded is still not ready, from the parts that are not:
/// their own details are what a bug report needs.
fn not_ready_reason(status: &EngineStatus) -> String {
    let missing: Vec<String> = status
        .components
        .iter()
        .filter(|component| !component.ready)
        .map(|component| format!("{}: {}", component.name, component.detail))
        .collect();
    if missing.is_empty() {
        status.label.clone()
    } else {
        missing.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::EngineComponent;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    #[derive(Default, Clone)]
    struct Recorder(Arc<Mutex<Vec<SetupProgress>>>);

    impl SetupSink for Recorder {
        fn announce(&self, progress: &SetupProgress) {
            self.0.lock().unwrap().push(progress.clone());
        }
    }

    /// A setup whose runs only count themselves and then fail, so no test
    /// ever reaches the network or loads a model.
    /// Each run also waits for as long as `hold` is set, so a test can keep
    /// one under way for as long as it needs.
    fn counted(recorder: &Recorder, hold: Arc<AtomicBool>) -> (Arc<Setup>, Arc<AtomicUsize>) {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let setup = Setup::with_job(
            Box::new(recorder.clone()),
            Arc::new(move |setup: &Setup| {
                counter.fetch_add(1, Ordering::SeqCst);
                while hold.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(5));
                }
                setup.announce(SetupProgress::failed(SetupFailure::Load, "test run"));
            }),
        );
        (setup, runs)
    }

    fn deferred(recorder: &Recorder) -> Arc<Setup> {
        counted(recorder, Arc::new(AtomicBool::new(false))).0
    }

    /// Waits for a run the test started to finish and say so.
    fn settle(setup: &Setup) {
        for _ in 0..200 {
            if !setup.running.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the run never finished");
    }

    #[test]
    fn an_engine_with_nothing_to_set_up_is_ready_from_the_start() {
        let setup = Setup::ready(Box::new(Recorder::default()));
        assert_eq!(setup.latest().stage, SetupStage::Ready);
        assert!(!setup.start(), "there is nothing to run");
        assert!(!setup.retry(), "and nothing to retry");
    }

    #[test]
    fn deferred_setup_reads_as_loading_until_it_has_said_anything() {
        let setup = deferred(&Recorder::default());
        let latest = setup.latest();
        assert_eq!(latest.stage, SetupStage::Loading);
        assert_eq!(latest.seq, 1);
        assert!(!latest.first_run);
    }

    #[test]
    fn every_announcement_is_kept_numbered_and_sent_in_order() {
        let recorder = Recorder::default();
        let setup = deferred(&recorder);
        setup.announce(SetupProgress::at(SetupStage::Downloading, "one"));
        setup.announce(SetupProgress::at(SetupStage::Loading, "two"));

        let sent = recorder.0.lock().unwrap().clone();
        assert_eq!(sent.iter().map(|each| each.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(setup.latest().message, "two");
        assert_eq!(setup.latest().seq, 3);
    }

    #[test]
    fn a_launch_that_downloaded_stays_a_first_run_through_the_load() {
        let setup = deferred(&Recorder::default());
        setup.announce(SetupProgress {
            first_run: true,
            ..SetupProgress::at(SetupStage::Downloading, "fetching")
        });
        setup.announce(SetupProgress::at(SetupStage::Loading, "loading"));
        assert!(setup.latest().first_run);
        setup.announce(SetupProgress::failed(SetupFailure::Load, "no"));
        assert!(setup.latest().first_run);
    }

    #[test]
    fn try_again_replaces_the_failure_before_it_answers() {
        let recorder = Recorder::default();
        let setup = deferred(&recorder);
        setup.announce(SetupProgress {
            downloaded_bytes: 700,
            total_bytes: 1000,
            index: 1,
            of: 2,
            first_run: true,
            ..SetupProgress::at(SetupStage::Downloading, "fetching")
        });
        setup.download_failed("connection reset".into(), Trouble::Network);
        let failed = setup.latest();
        assert_eq!(failed.stage, SetupStage::Failed);
        assert_eq!((failed.downloaded_bytes, failed.total_bytes), (700, 1000), "the bar stays where it stopped");

        assert!(setup.retry());
        settle(&setup);
        let resumed = recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|each| each.seq == failed.seq + 1)
            .cloned()
            .unwrap();
        assert_eq!(resumed.stage, SetupStage::Downloading);
        assert_eq!((resumed.downloaded_bytes, resumed.total_bytes), (700, 1000));
        assert!(resumed.first_run);
    }

    #[test]
    fn a_load_that_failed_is_retried_as_a_load() {
        let recorder = Recorder::default();
        let setup = deferred(&recorder);
        setup.announce(SetupProgress::failed(SetupFailure::Load, "no server"));
        let failed = setup.latest().seq;
        assert!(setup.retry());
        settle(&setup);
        let resumed = recorder.0.lock().unwrap().iter().find(|each| each.seq == failed + 1).cloned().unwrap();
        assert_eq!(resumed.stage, SetupStage::Loading);
    }

    #[test]
    fn many_presses_at_once_start_one_run() {
        // The run lasts until every press has landed, however they are
        // scheduled, so each of them meets a run already under way.
        let hold = Arc::new(AtomicBool::new(true));
        let (setup, runs) = counted(&Recorder::default(), Arc::clone(&hold));
        setup.announce(SetupProgress::failed(SetupFailure::Download, "dropped"));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let presses: Vec<_> = (0..8)
            .map(|_| {
                let setup = Arc::clone(&setup);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    setup.retry()
                })
            })
            .collect();
        let started = presses.into_iter().map(|press| press.join().unwrap()).filter(|began| *began).count();
        hold.store(false, Ordering::SeqCst);
        settle(&setup);
        assert_eq!(started, 1);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn only_a_failed_setup_can_be_retried() {
        let setup = deferred(&Recorder::default());
        assert!(!setup.retry(), "still loading");
        setup.announce(SetupProgress::at(SetupStage::Ready, "ready"));
        assert!(!setup.retry(), "already ready");
    }

    #[test]
    fn a_second_run_cannot_start_while_one_is_under_way() {
        let setup = deferred(&Recorder::default());
        setup.running.store(true, Ordering::SeqCst);
        setup.announce(SetupProgress::failed(SetupFailure::Download, "dropped"));
        assert!(!setup.retry());
        assert!(!setup.start());
    }

    #[test]
    fn a_run_that_stops_without_a_word_reads_as_failed_and_can_be_retried() {
        let recorder = Recorder::default();
        let setup = deferred(&recorder);
        setup.running.store(true, Ordering::SeqCst);
        setup.announce(SetupProgress::at(SetupStage::Downloading, "fetching"));
        drop(Finished(&setup));

        let latest = setup.latest();
        assert_eq!(latest.stage, SetupStage::Failed);
        assert_eq!(latest.failure, Some(SetupFailure::Load));
        assert!(!setup.running.load(Ordering::SeqCst));
    }

    #[test]
    fn exit_waits_for_a_run_that_is_loading_but_not_for_a_download() {
        let hold = Arc::new(AtomicBool::new(true));
        let (setup, _) = counted(&Recorder::default(), Arc::clone(&hold));
        assert!(setup.start());
        // Still reads as loading: the run has not announced anything yet.
        let releaser = {
            let hold = Arc::clone(&hold);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                hold.store(false, Ordering::SeqCst);
            })
        };
        setup.wait_while_loading(Duration::from_secs(5));
        assert!(!setup.running.load(Ordering::SeqCst), "it waited for the run to end");
        releaser.join().unwrap();

        let (downloading, _) = counted(&Recorder::default(), Arc::new(AtomicBool::new(true)));
        downloading.announce(SetupProgress::at(SetupStage::Downloading, "fetching"));
        downloading.running.store(true, Ordering::SeqCst);
        let started = std::time::Instant::now();
        downloading.wait_while_loading(Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_millis(500), "a download is not waited for");
    }

    #[test]
    fn a_run_that_ended_properly_is_left_as_it_ended() {
        let setup = deferred(&Recorder::default());
        setup.announce(SetupProgress::at(SetupStage::Ready, "ready"));
        drop(Finished(&setup));
        assert_eq!(setup.latest().stage, SetupStage::Ready);
    }

    #[test]
    fn the_stage_and_failure_reach_the_window_in_lower_case() {
        let json = serde_json::to_value(SetupProgress::failed(SetupFailure::Download, "x")).unwrap();
        assert_eq!(json["stage"], "failed");
        assert_eq!(json["failure"], "download");
        let json = serde_json::to_value(SetupProgress::at(SetupStage::Ready, "x")).unwrap();
        assert_eq!(json["failure"], serde_json::Value::Null);
        let json = serde_json::to_value(SetupProgress {
            trouble: Some(Trouble::Blocked),
            ..SetupProgress::at(SetupStage::Failed, "x")
        })
        .unwrap();
        assert_eq!(json["trouble"], "blocked");
    }

    #[test]
    fn an_engine_that_is_not_ready_says_which_parts_are_missing() {
        let status = EngineStatus {
            mode: "local".into(),
            label: "Local AI".into(),
            ready: false,
            components: vec![
                EngineComponent {
                    name: "Language model".into(),
                    ready: false,
                    detail: "llama-server did not start".into(),
                },
                EngineComponent {
                    name: "Ella's voice".into(),
                    ready: true,
                    detail: "fine".into(),
                },
            ],
        };
        assert_eq!(not_ready_reason(&status), "Language model: llama-server did not start");
        assert_eq!(
            not_ready_reason(&EngineStatus {
                components: Vec::new(),
                ..status
            }),
            "Local AI"
        );
    }
}
