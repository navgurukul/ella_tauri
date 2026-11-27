//! The one place that decides what telemetry does on this laptop: whether
//! the local log (`latency.jsonl`, `telemetry::persist_to`) is written, and
//! whether its batches are sent to the telemetry relay, exported for a
//! tester, or kept (`telemetry_queue::start`). Nothing else reads the
//! switch or `ELLA_TELEMETRY_SEND` / `ELLA_TELEMETRY_EXPORT`.
//!
//! In order, the first that holds:
//!
//! 1. `TELEMETRY_ENABLED` is false: nothing. No log, no queue, whatever the
//!    environment says.
//! 2. `ELLA_TELEMETRY_SEND=1`: the log, sent to the relay.
//! 3. `ELLA_TELEMETRY_EXPORT=1`: the log, its batches written to
//!    `telemetry/export/` for a tester to hand over; nothing over a network.
//! 4. `ELLA_TELEMETRY_SEND=0`: the log, kept on the laptop.
//! 5. A release build: the log, sent to the relay. The default for every
//!    installed copy of Ella.
//! 6. A development build: the log, kept on the laptop, so a developer's
//!    runs never reach the fleet's data unless asked with `SEND=1`.
//!
//! Any other value of either variable counts as unset. What is written and
//! what may leave is the same in every mode: `telemetry_policy` decides it.

use std::path::PathBuf;

use crate::{telemetry, telemetry_queue};

/// Telemetry as a whole: the local log and its upload. On. A build with this
/// set to false writes no telemetry and sends none: the one place to turn it
/// all off.
pub const TELEMETRY_ENABLED: bool = true;

pub fn telemetry_enabled() -> bool {
    TELEMETRY_ENABLED
}

/// Where the log's batches go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upload {
    /// To the telemetry relay, over HTTPS.
    Send,
    /// To `telemetry/export/`, for a tester to hand over.
    Export,
    /// Nowhere: the log stays on the laptop.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No telemetry at all: no log, no queue.
    Off,
    /// The local log, and where its batches go.
    On(Upload),
}

/// What telemetry does, from the switch, the kind of build and the
/// environment (`env` reads a variable), as the module's rules say.
pub fn mode(enabled: bool, release_build: bool, env: impl Fn(&str) -> Option<String>) -> Mode {
    if !enabled {
        return Mode::Off;
    }
    let send = env("ELLA_TELEMETRY_SEND");
    let upload = if send.as_deref() == Some("1") {
        Upload::Send
    } else if env("ELLA_TELEMETRY_EXPORT").as_deref() == Some("1") {
        Upload::Export
    } else if send.as_deref() == Some("0") {
        Upload::Off
    } else if release_build {
        Upload::Send
    } else {
        Upload::Off
    };
    Mode::On(upload)
}

/// Starts telemetry for the log in `telemetry_dir`, as `mode` decides for
/// this build and this environment. The app's one call.
pub fn start(telemetry_dir: PathBuf) {
    let decided = mode(telemetry_enabled(), !cfg!(debug_assertions), |name| std::env::var(name).ok());
    start_with(decided, telemetry_dir, telemetry::persist_to, telemetry_queue::start);
}

/// `start`, with what it starts passed in so a test can see what ran.
fn start_with(
    decided: Mode,
    telemetry_dir: PathBuf,
    persist: impl FnOnce(PathBuf),
    queue: impl FnOnce(PathBuf, Upload),
) {
    let Mode::On(upload) = decided else {
        eprintln!("[TELEMETRY] off in this build: nothing is written or sent");
        return;
    };
    persist(telemetry_dir.clone());
    match upload {
        Upload::Send | Upload::Export => queue(telemetry_dir, upload),
        Upload::Off => eprintln!("[TELEMETRY] batches stay on this laptop (ELLA_TELEMETRY_SEND=1 sends them)"),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| vars.get(name).cloned()
    }

    const RELEASE: bool = true;
    const DEV: bool = false;

    #[test]
    fn telemetry_is_on_by_default() {
        assert!(telemetry_enabled(), "telemetry is on by default: turning it off is a product decision");
    }

    #[test]
    fn a_release_build_sends_without_being_asked() {
        assert_eq!(mode(telemetry_enabled(), RELEASE, env(&[])), Mode::On(Upload::Send));
    }

    #[test]
    fn a_development_build_keeps_its_batches_unless_asked_to_send() {
        assert_eq!(mode(true, DEV, env(&[])), Mode::On(Upload::Off));
        assert_eq!(mode(true, DEV, env(&[("ELLA_TELEMETRY_SEND", "1")])), Mode::On(Upload::Send));
    }

    #[test]
    fn the_switch_off_turns_off_everything_whatever_the_environment() {
        for release in [RELEASE, DEV] {
            for vars in [
                &[][..],
                &[("ELLA_TELEMETRY_SEND", "1")][..],
                &[("ELLA_TELEMETRY_EXPORT", "1")][..],
                &[("ELLA_TELEMETRY_SEND", "1"), ("ELLA_TELEMETRY_EXPORT", "1")][..],
            ] {
                assert_eq!(mode(false, release, env(vars)), Mode::Off, "{vars:?}");
            }
        }
    }

    #[test]
    fn export_is_as_it_was() {
        for release in [RELEASE, DEV] {
            assert_eq!(mode(true, release, env(&[("ELLA_TELEMETRY_EXPORT", "1")])), Mode::On(Upload::Export));
            // Send wins over export, as before.
            let both = env(&[("ELLA_TELEMETRY_SEND", "1"), ("ELLA_TELEMETRY_EXPORT", "1")]);
            assert_eq!(mode(true, release, both), Mode::On(Upload::Send));
            // Asked to export and not to send: an export, nothing over a network.
            let export_only = env(&[("ELLA_TELEMETRY_SEND", "0"), ("ELLA_TELEMETRY_EXPORT", "1")]);
            assert_eq!(mode(true, release, export_only), Mode::On(Upload::Export));
        }
    }

    #[test]
    fn send_0_keeps_the_batches_on_the_laptop() {
        for release in [RELEASE, DEV] {
            assert_eq!(mode(true, release, env(&[("ELLA_TELEMETRY_SEND", "0")])), Mode::On(Upload::Off));
        }
    }

    #[test]
    fn other_values_count_as_unset() {
        for value in ["", "yes", "true", "on", " 1", "2"] {
            let vars = [("ELLA_TELEMETRY_SEND", value), ("ELLA_TELEMETRY_EXPORT", value)];
            assert_eq!(mode(true, RELEASE, env(&vars)), Mode::On(Upload::Send), "{value:?}");
            assert_eq!(mode(true, DEV, env(&vars)), Mode::On(Upload::Off), "{value:?}");
        }
    }

    /// What `start_with` ran: the log, and the queue with its upload.
    fn started(decided: Mode) -> (bool, Option<Upload>) {
        let persisted = RefCell::new(false);
        let queued = RefCell::new(None);
        start_with(
            decided,
            PathBuf::from("telemetry"),
            |_| *persisted.borrow_mut() = true,
            |_, upload| *queued.borrow_mut() = Some(upload),
        );
        (persisted.into_inner(), queued.into_inner())
    }

    #[test]
    fn off_starts_neither_the_log_nor_the_queue() {
        assert_eq!(started(Mode::Off), (false, None));
    }

    #[test]
    fn on_starts_the_log_and_the_queue_it_was_given() {
        assert_eq!(started(Mode::On(Upload::Send)), (true, Some(Upload::Send)));
        assert_eq!(started(Mode::On(Upload::Export)), (true, Some(Upload::Export)));
        assert_eq!(started(Mode::On(Upload::Off)), (true, None), "the log, and no queue");
    }
}
