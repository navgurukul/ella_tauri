# Ella desktop

A usable Ella vertical slice built with Tauri 2, React/TypeScript, Rust,
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

The default development engine root is `engines/` in this repository — a symlink
or a staged copy of a built engine tree (`bin/`, `models/`, `piper-venv/`). Set
`ELLA_ENGINE_ROOT` to use another location. If you already have the tree that
`ella_flutter` builds, point at it once:

```bash
ln -s /path/to/ella_flutter/ella_app/build/engines engines
```

## Install and validate the STT models

From the repository root:

```bash
npm install
npm run models:install
npm run models:validate
```

`models:install` downloads the pinned Canary Q8_0 GGUF plus Whisper small from
`tooling/models.json`; `models:validate` checks the files' sizes and Canary SHA-256.
Native startup additionally checks the GGUF header, architecture, English/16 kHz
capabilities, and session creation. Failures name the bad file and print a repair
command. The model can also be checked without starting Ella:

```bash
cargo run --release --manifest-path src-tauri/Cargo.toml --bin stt-benchmark -- \
  --audio bench/fixtures/jfk.wav --duration-ms 4214 --iterations 1 --warmup 0
```

## User interface

The UI implements the **Ella v6** design system (Claude Design project
`549e2295-3339-4b07-9009-1ac4475aad21`, file `Ella v6.dc.html`).

- Tokens, type and geometry live in [`src/styles.css`](src/styles.css). Fonts are
  bundled under `public/assets/fonts` because the Tauri CSP is `font-src 'self'`
  — nothing is fetched from Google Fonts at runtime.
- Ella herself is drawn in CSS, not illustrated: see
  [`src/components/EllaMascot.tsx`](src/components/EllaMascot.tsx). One 660x450
  blob is the single source of truth for her face; every other size is that blob
  under a CSS scale.
- Onboarding is the five-step flow from v6 — welcome, name, age, mic check,
  placement talk — in
  [`src/components/OnboardingFlow.tsx`](src/components/OnboardingFlow.tsx). The
  mic check and the placement talk open the real microphone; the level the
  placement talk reports is still the placeholder band, because nothing scores
  the recording yet.
- Then four screens: home, talk, summary and garden. The design file covers
  home, talk and garden; the post-conversation summary is built from the same
  parts. A conversation hides the sidebar and fills the window.
- The design shows curriculum framing the Rust backend does not model yet — a
  CEFR band, a talking streak, named garden units, per-topic category and
  duration, a weekly digest. All of it is resolved in
  [`src/lib/presentation.ts`](src/lib/presentation.ts), which derives what it can
  from `AppSnapshot` and marks the rest `PLACEHOLDER`. When the backend grows a
  field, delete the constant and read the snapshot instead.

The intended window is 1440x900. Below 1280px wide the home bento reflows from
the design's four columns to two.

## Run the complete local voice POC

macOS development host:

```bash
cd /path/to/ella_tauri
npm run local
```

Or keep sidecar logs in a separate terminal:

```bash
# Terminal 1
npm run engines:local

# Terminal 2
npm run desktop:dev:local
```

`desktop:dev` is the UI-only launch: it leaves `ELLA_ENGINE_MODE` unset, so the
app runs the demo engine and **has no speech recognition** — the desktop webview
does not provide the Web Speech API, so voice turns can only fail with
"native speech recognition is not enabled in demo mode". Use
`desktop:dev:local` (or `npm run local`, which starts the engines too) whenever
you need to speak to Ella. The mode is no longer surfaced in the UI — read it
back from `ELLA_ENGINE_MODE` or the startup log.

Windows x86-64 development host (PowerShell):

```powershell
cd C:\path\to\ella_tauri
npm install
npm run models:install
npm run models:validate
.\scripts\run-local-poc.ps1 -EngineRoot C:\path\to\ella_tauri\engines
```

The native voice route is:

```text
WebAudio PCM -> stop-time energy VAD -> SpeechToTextEngine
                                      -> Canary/transcribe.cpp (primary)
                                      -> Whisper HTTP (fallback)
             -> streamed llama.cpp -> Piper (per sentence) -> WebView playback
```

`SpeechToTextEngine` and `SttRouter` keep the STT boundary independent of the
Canary adapter, so a future Parakeet streaming adapter does not change the
application or UI contracts.

Piper runs a sentence behind the language model rather than after it. Whole
sentences leave the token stream as they complete and are synthesized on a
worker thread, so Piper's time overlaps the model's instead of following it —
about 300 ms off a turn. They are held back until the whole reply is ready:
released sentence by sentence, Ella started talking sooner still, but the text
then arrived in pieces, and a reply that gains words cannot be centred without
the words being read jumping as each piece lands. Ella's opening is the
exception and does stream, because its text is on screen from the moment the
conversation opens, so there is nothing to stage — `speak_opening` is a separate
command from `start_session` so the screen appears before Piper is asked for
anything.

A turn that could be thrown away is safe either way: a ledger chore may
regenerate its reply when the character breaks its own limit, and the audio is
only reused when it says exactly what the reply says. Nothing the learner hears
is ever retracted.

The reply carries word timings, so the word being spoken is highlighted and the
words ahead of it are dimmed. Piper hands back audio without timings, so
`infrastructure/speech_timing.rs` estimates them from syllables, characters and
punctuation, anchored to the sentence's exact duration and to the leading and
trailing silence measured off the PCM. Fitted and measured against real Piper
phoneme alignments: onset error mean ~70 ms, p90 ~160 ms, roughly three quarters
of words inside 100 ms. Swapping in exact alignments is a change to that module
alone — it costs a 68 MB `onnx` dependency, ~30 ms per sentence, and still needs
this estimator as a fallback, because espeak merges words ("in the" becomes one
phoneme group) in about 8% of sentences and leaves no safe positional mapping
back to the text.

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
ELLA_ENGINE_ROOT="$PWD/engines" \
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
ELLA_ENGINE_ROOT="$PWD/engines" \
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
