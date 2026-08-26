use std::{env, fs, path::PathBuf};

fn main() {
    // transcribe.cpp's Windows dynamic-backend posture publishes the runtime
    // and module directories to downstream build scripts. Stage every DLL for
    // Tauri to place beside the executable: Vulkan can then fail to load on an
    // unsupported machine while the CPU module remains usable.
    let destination = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transcribe-libs");
    for key in [
        "DEP_TRANSCRIBE_CPP_RUNTIME_DIR",
        "DEP_TRANSCRIBE_CPP_MODULE_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let Some(source) = env::var_os(key).map(PathBuf::from) else {
            continue;
        };
        fs::create_dir_all(&destination).expect("create transcribe-libs staging directory");
        let mut copied = 0;
        for entry in fs::read_dir(&source)
            .unwrap_or_else(|error| panic!("read {}: {error}", source.display()))
        {
            let path = entry.expect("read transcribe.cpp artifact").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name.to_ascii_lowercase().ends_with(".dll") {
                fs::copy(&path, destination.join(name)).unwrap_or_else(|error| {
                    panic!("copy {} into bundle staging: {error}", path.display())
                });
                copied += 1;
            }
        }
        assert!(
            copied > 0,
            "{key} pointed at {} but contained no DLLs",
            source.display()
        );
    }
    tauri_build::build()
}
