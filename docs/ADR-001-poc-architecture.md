# ADR-001: Tauri POC architecture

## Status

Accepted for the proof of concept.

## Decision

Use Tauri 2 with a React/TypeScript WebView and a Rust application core.

The trust boundary is deliberately narrow:

```text
React presentation
  └─ typed Tauri commands
       └─ AppService use cases
            ├─ SQLite repository
            └─ TutorEngine trait
                 ├─ DemoEngine (default, deterministic)
                 └─ LocalEngine
                      ├─ SpeechToTextEngine / SttRouter
                      │    ├─ Canary/transcribe.cpp (primary)
                      │    ├─ Whisper HTTP (fallback)
                      │    └─ Parakeet adapter (future seam)
                      ├─ streamed llama.cpp client
                      └─ Piper process adapter
```

- React owns rendering, accessibility, temporary form state, microphone capture, and playback.
- Rust owns validation, conversation state, skill evidence, persistence, and engine selection.
- `TutorEngine` is a port. Demo and local adapters can be replaced without changing domain or UI code.
- `SpeechToTextEngine` is a second, narrower port. Canary and Whisper implement
  the same final-transcript contract, while `SttRouter` owns failover. Parakeet
  can later add streaming events without coupling its model lifecycle to the
  application service.
- Canary-180M-Flash Q8_0 is an in-process, persistent native model/session. It is
  batch STT, not true streaming; recognition starts after stop-time VAD trims
  the captured utterance.
- The Windows x86-64 build uses transcribe.cpp dynamic CPU and Vulkan modules.
  Runtime backend discovery prefers available acceleration and leaves CPU as the
  compatibility path.
- A learner turn and Ella reply are committed to SQLite together. A partial turn is never visible.
- TTS is non-fatal: a valid text turn remains usable if speech synthesis fails.
- One structured `ella_turn_latency` record is emitted for every turn, including
  failures. It correlates VAD, STT and native-stage timing, LLM TTFT/completion,
  TTS first-audio/completion, and total latency.
- The WebView receives only application-specific commands. It has no filesystem, shell, or network plugin permission.

## POC concessions

- WebAudio captures PCM for portability. A production build should move capture/VAD into a native Rust audio adapter.
- Demo mode uses system speech recognition and system TTS when available; typing is always available.
- Local llama.cpp and the Whisper fallback server are started by development
  scripts. Canary itself is linked through the official Rust bindings. A
  production packaged build should use a Rust `EngineManager` to allocate
  ports, supervise sidecars, stream logs, and guarantee shutdown.
- The current stop-time VAD is an energy gate. Silero/native capture and true
  barge-in remain later work.
- Placement, login/sync, background grading, model download/update, pronunciation scoring, and six complete level gardens remain outside this vertical slice.

## Exit criteria for the POC

- A learner can create a local profile, choose a topic, complete a multi-turn conversation, hear Ella, end the session, and see garden growth.
- The same React journey works against both the browser preview adapter and Tauri/Rust adapter.
- Rust persistence and browser preview behavior have automated tests.
- A local integration test exercises VAD -> Canary -> llama.cpp -> persistence
  -> Piper with real model processes and returns playable WAV data.
- A fixed-duration benchmark runs Canary and Whisper against the same PCM and
  emits a machine-readable report.
- The app compiles as a native Tauri executable on the development host.
