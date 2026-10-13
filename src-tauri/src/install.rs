//! Which installer put this copy of Ella here, what kind of place it runs
//! from, and whether another copy sits elsewhere, for the launch's
//! `ella_launch` event.
//!
//! On 2026-10-13 one tester's laptop ran 0.1.16, 0.1.19 and 0.1.20 in a day,
//! and its 0.1.20 drove 0.1.16's llama-server. Windows has two installers:
//! the NSIS one installs for the user under `%LOCALAPPDATA%`, the MSI for the
//! machine under Program Files. Each updates only itself, and both read the
//! same app data, so a laptop that has run both keeps two copies whose
//! telemetry lands in one file and cannot be told apart. This tells them
//! apart. Paths themselves are never written down: they hold the user's name.
//! Only the kind of place is.

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Install {
    /// The installer that made this copy, as the bundler stamped it into the
    /// executable: `nsis`, `msi`, or `app` on macOS. None for a build run
    /// straight from the source tree on Windows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<&'static str>,
    /// `per_user` (the NSIS install), `per_machine` (the MSI), `applications`
    /// or `user_applications` on macOS, `disk_image` for a copy run from its
    /// mounted .dmg (which can never update itself), `source` for a build in
    /// a checkout, or `other`.
    pub place: &'static str,
    /// The kinds of place that hold another copy of Ella.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also_at: Vec<&'static str>,
}

/// This launch's install.
pub fn describe() -> Install {
    let windows = cfg!(target_os = "windows");
    let places = known_places();
    let exe = std::env::current_exe().ok();
    let exe_text = exe
        .as_deref()
        .map(|exe| normalized(&exe.to_string_lossy(), windows));
    let place = exe_text
        .as_deref()
        .map_or("other", |exe| place_of(exe, &places, windows));
    let also_at = places
        .iter()
        .flat_map(|place| place.copies.iter().map(move |copy| (place.kind, copy)))
        .filter(|(_, copy)| copy.exists())
        .filter(|(_, copy)| {
            let copy = normalized(&copy.to_string_lossy(), windows);
            !exe_text
                .as_deref()
                .is_some_and(|exe| within(exe, &copy, windows))
        })
        .map(|(kind, _)| kind)
        .fold(Vec::new(), |mut kinds, kind| {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
            kinds
        });
    Install {
        bundle: bundle(),
        place,
        also_at,
    }
}

fn bundle() -> Option<&'static str> {
    use tauri::utils::config::BundleType;
    Some(match tauri::utils::platform::bundle_type()? {
        BundleType::Nsis => "nsis",
        BundleType::Msi => "msi",
        BundleType::App | BundleType::Dmg => "app",
        BundleType::Deb => "deb",
        BundleType::Rpm => "rpm",
        BundleType::AppImage => "appimage",
    })
}

/// A kind of place Ella is installed in: where it starts, and where a copy
/// installed there would be.
struct Place {
    kind: &'static str,
    root: PathBuf,
    copies: Vec<PathBuf>,
}

#[cfg(target_os = "windows")]
fn known_places() -> Vec<Place> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut places = Vec::new();
    if let Some(local) = var("LOCALAPPDATA") {
        places.push(Place {
            kind: "per_user",
            copies: vec![
                local.join("Ella").join("Ella.exe"),
                local.join("Programs").join("Ella").join("Ella.exe"),
            ],
            root: local,
        });
    }
    for program_files in [var("ProgramFiles"), var("ProgramFiles(x86)")]
        .into_iter()
        .flatten()
    {
        places.push(Place {
            kind: "per_machine",
            copies: vec![program_files.join("Ella").join("Ella.exe")],
            root: program_files,
        });
    }
    places
}

#[cfg(not(target_os = "windows"))]
fn known_places() -> Vec<Place> {
    let mut places = vec![
        Place {
            kind: "applications",
            root: PathBuf::from("/Applications"),
            copies: vec![PathBuf::from("/Applications/Ella.app")],
        },
        Place {
            kind: "disk_image",
            root: PathBuf::from("/Volumes"),
            copies: Vec::new(),
        },
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let applications = home.join("Applications");
        places.push(Place {
            kind: "user_applications",
            copies: vec![applications.join("Ella.app")],
            root: applications,
        });
    }
    places
}

/// What kind of place `exe` (normalized) runs from.
fn place_of(exe: &str, places: &[Place], windows: bool) -> &'static str {
    let sep = if windows { '\\' } else { '/' };
    let built = ["debug", "release"]
        .iter()
        .any(|profile| exe.contains(&format!("{sep}target{sep}{profile}{sep}")));
    if built {
        return "source";
    }
    places
        .iter()
        .find(|place| {
            within(
                exe,
                &normalized(&place.root.to_string_lossy(), windows),
                windows,
            )
        })
        .map_or("other", |place| place.kind)
}

/// A path written the one way two spellings of it can be compared: Windows
/// paths are case-blind, may come with `/` for `\`, and from `current_exe`
/// may carry the `\\?\` prefix.
fn normalized(path: &str, windows: bool) -> String {
    if !windows {
        return path.to_owned();
    }
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    path.replace('/', "\\").to_lowercase()
}

/// Whether `path` is `root` or inside it.
fn within(path: &str, root: &str, windows: bool) -> bool {
    let sep = if windows { '\\' } else { '/' };
    let root = root.trim_end_matches(sep);
    !root.is_empty() && (path == root || path.starts_with(&format!("{root}{sep}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_places() -> Vec<Place> {
        vec![
            Place {
                kind: "per_user",
                root: PathBuf::from(r"C:\Users\Asha\AppData\Local"),
                copies: Vec::new(),
            },
            Place {
                kind: "per_machine",
                root: PathBuf::from(r"C:\Program Files"),
                copies: Vec::new(),
            },
        ]
    }

    #[test]
    fn a_windows_copy_is_placed_whatever_the_spelling_of_its_path() {
        let places = windows_places();
        let at = |exe: &str| place_of(&normalized(exe, true), &places, true);
        assert_eq!(
            at(r"\\?\C:\Users\Asha\AppData\Local\Ella\Ella.exe"),
            "per_user"
        );
        assert_eq!(at(r"c:/users/asha/appdata/local/Ella/Ella.exe"), "per_user");
        assert_eq!(at(r"C:\Program Files\Ella\Ella.exe"), "per_machine");
        assert_eq!(
            at(r"C:\Program Files (x86)\Ella\Ella.exe"),
            "other",
            "not inside Program Files"
        );
        assert_eq!(
            at(r"C:\Users\Asha\Desktop\ella_tauri\src-tauri\target\release\Ella.exe"),
            "source"
        );
        assert_eq!(at(r"D:\Ella\Ella.exe"), "other");
    }

    #[test]
    fn a_mac_copy_is_placed_and_a_dmg_is_told_apart() {
        let places = vec![
            Place {
                kind: "applications",
                root: PathBuf::from("/Applications"),
                copies: Vec::new(),
            },
            Place {
                kind: "disk_image",
                root: PathBuf::from("/Volumes"),
                copies: Vec::new(),
            },
        ];
        let at = |exe: &str| place_of(exe, &places, false);
        assert_eq!(
            at("/Applications/Ella.app/Contents/MacOS/ella-tauri"),
            "applications"
        );
        assert_eq!(
            at("/Volumes/Ella/Ella.app/Contents/MacOS/ella-tauri"),
            "disk_image"
        );
        assert_eq!(
            at("/Applicationsfoo/Ella.app/Contents/MacOS/ella-tauri"),
            "other"
        );
        assert_eq!(
            at("/Users/asha/ella_tauri/src-tauri/target/debug/ella-tauri"),
            "source"
        );
    }

    #[test]
    fn the_launch_says_where_it_runs_and_never_the_path() {
        let install = describe();
        let json = serde_json::to_string(&install).unwrap();
        assert!(
            !json.contains('/') && !json.contains('\\'),
            "no path in {json}"
        );
        // `cargo test` runs from target/debug/deps.
        assert_eq!(install.place, "source");
    }
}
