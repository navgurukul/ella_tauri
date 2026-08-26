# ZoSpeak desktop POC

A usable Ella/ZoSpeak vertical slice built with Tauri 2, React/TypeScript, Rust,
SQLite, and local AI engines. In local mode, the primary speech recognizer is
Canary-180M-Flash Q8_0 running in-process through `transcribe.cpp`; Whisper small
remains an HTTP fallback.

Canary is batch speech recognition, not true streaming. Ella runs a lightweight
stop-time VAD and optimizes the interval from microphone stop to final transcript.

## Prerequisites

- Node.js/npm, stable Rust, CMake, and the normal Tauri 2 platform prerequisites.
- Local llama.cpp and Piper assets under an engine root.
- The Canary and Whisper model files installed by the checked-in manifest.
- On the Windows x86-64 build host, Visual Studio C++ Build Tools and a Vulkan
  SDK providing `glslc`; on target machines, a current GPU driver for optional
  Vulkan acceleration. The packaged native runtime tries the Vulkan module
  automatically and retains the CPU module when Vulkan is unavailable.

The default development engine root is `../../ella_app/build/engines` relative
to this directory. Set `ELLA_ENGINE_ROOT` to use another location.

## Install and validate the STT models

From `desktop/ella_tauri`:

```bash
npm install
npm run models:install
npm run models:validate
```

`models:install` downloads the pinned Canary Q8_0 GGUF plus Whisper small from
`../models.json`; `models:validate` checks the files' sizes and Canary SHA-256.
Native startup additionally checks the GGUF header, architecture, English/16 kHz
capabilities, and session creation. Failures name the bad file and print a repair
command. The model can also be checked without starting Ella:

```bash
cargo run --release --manifest-path src-tauri/Cargo.toml --bin stt-benchmark -- \
  --audio bench/fixtures/jfk.wav --duration-ms 4214 --iterations 1 --warmup 0
```

## Run the complete local voice POC

macOS development host:

```bash
cd /Users/sama/Desktop/ella_flutter/desktop/ella_tauri
npm run local
```

Or keep sidecar logs in a separate terminal:

```bash
# Terminal 1
npm run engines:local

# Terminal 2
ELLA_ENGINE_MODE=local \
ELLA_ENGINE_ROOT=/Users/sama/Desktop/ella_flutter/ella_app/build/engines \
npm run desktop:dev
```

Windows x86-64 development host (PowerShell):

```powershell
cd C:\path\to\ella_flutter\desktop\ella_tauri
npm install
npm run models:install
npm run models:validate
.\scripts\run-local-poc.ps1 -EngineRoot C:\path\to\ella_flutter\ella_app\build\engines
```

The native voice route is:

```text
WebAudio PCM -> stop-time energy VAD -> SpeechToTextEngine
                                      -> Canary/transcribe.cpp (primary)
                                      -> Whisper HTTP (fallback)
             -> streamed llama.cpp -> Piper -> WebView playback
```

`SpeechToTextEngine` and `SttRouter` keep the STT boundary independent of the
Canary adapter, so a future Parakeet streaming adapter does not change the
application or UI contracts.

Environment overrides:

- `ELLA_ENGINE_ROOT`
- `ELLA_LLM_BASE_URL`
- `ELLA_STT_BASE_URL` and `ELLA_STT_TRANSCRIBE_URL` (Whisper fallback)
- `ELLA_PIPER_BINARY` and `ELLA_PIPER_VOICE`
- `ELLA_CANARY_MODEL`, `ELLA_STT_THREADS`, `ELLA_CANARY_VERIFY_SHA256`

## Timing telemetry

Every native turn writes one JSON line with `event: "ella_turn_latency"` and a
correlation ID. Voice lines contain audio input/after-VAD duration, VAD, STT,
Canary mel/encode/decode, STT engine/backend/fallback, LLM TTFT/completion, Piper
first-audio/completion, total latency, and success/error status. The WebView also
logs `ella_voice_playback_ready` after audio reaches the browser playback path.

## Repeatable Canary/Whisper benchmark

Start the Whisper fallback, then run the fixed 4.214-second same-input test:

```bash
npm run engines:local
```

```bash
ELLA_ENGINE_ROOT=/Users/sama/Desktop/ella_flutter/ella_app/build/engines \
npm run benchmark:stt -- \
  --audio bench/fixtures/jfk.wav \
  --duration-ms 4214 \
  --whisper-url http://127.0.0.1:39092 \
  --warmup 1 \
  --iterations 5 \
  --output bench/results/local.json
```

The tool reports sorted samples, medians, transcripts, the live same-input
comparison, and the supplied 3387 ms Whisper baseline comparison. Details and
the development-host result are in [`bench/README.md`](bench/README.md).

## Test and build

```bash
npm run check

# Full local voice path: VAD -> native Canary -> llama.cpp -> SQLite -> Piper
ELLA_ENGINE_MODE=local \
ELLA_ENGINE_ROOT=/Users/sama/Desktop/ella_flutter/ella_app/build/engines \
cargo test --release --manifest-path src-tauri/Cargo.toml \
  application::tests::complete_local_voice_turn_uses_canary_and_returns_playable_audio \
  -- --ignored --nocapture

# Native application build without an installer
npm run desktop:build
```

The ignored end-to-end test requires healthy llama.cpp and Whisper sidecars;
Whisper is present to verify the fallback is ready, while the assertion confirms
the turn actually used Canary and returned playable Piper audio.

## Windows packaging

Collect Windows x86-64 llama.cpp, whisper.cpp, and Piper executables plus models
into an engine root, then stage and build on Windows:

```powershell
.\scripts\stage-windows-engines.ps1 -EngineRoot C:\ella\engines
rustup target add x86_64-pc-windows-msvc
npm run tauri -- build --target x86_64-pc-windows-msvc
```

The Windows Cargo target enables `transcribe-cpp`'s `vulkan` and
`dynamic-backends` features. `build.rs` stages the produced runtime, CPU, and
Vulkan DLLs next to the packaged executable; Tauri also bundles the staged
engine tree. Run the resulting build on both a Vulkan-capable and CPU-only
Windows 11 x86-64 machine before distribution.

## POC boundaries

The automated full-turn test feeds a real WAV fixture into the application
service, rather than operating physical microphone/speaker hardware. Windows
cross-compilation and GPU execution are not available on the current Intel Mac,
so Windows installer, driver fallback, microphone permission, and hardware
playback still require the two-machine checks above. Model update UI, signed
installers, native capture, true streaming STT, and Parakeet remain outside this
POC.
