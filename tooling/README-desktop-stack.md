# Ella Desktop — build and run

Offline speech-to-speech Ella. The student speaks, Ella answers aloud, and STT,
LLM, TTS and pronunciation scoring all run on the machine. Auth and account
sync stay online.

Architecture and the reasoning behind every choice here:
[`docs/desktop-architecture.md`](../docs/desktop-architecture.md).

---

## What runs where

```
ella_app (Flutter)                     the window, the mic, playback
  └── EngineSupervisor                 spawns and supervises the rest
        ├── ella-orchestrator          backend/ as a sidecar, loopback only
        │     └── piper                spawned per turn, streams PCM
        ├── llama-server               --parallel 2: slot 0 chat, slot 1 grading
        └── whisper-server             OpenAI-compatible transcription
```

The orchestrator is not a new service — it is `backend/` with
`DESKTOP_MODE=true`, which repoints the AI path at localhost. Hosted behaviour
is unchanged when that flag is off.

---

## 1. Native engines

Build or download these into `engines/bin/`:

| Binary | Source | Notes |
|---|---|---|
| `llama-server` | [llama.cpp](https://github.com/ggml-org/llama.cpp) | |
| `whisper-server` | [whisper.cpp](https://github.com/ggml-org/whisper.cpp) | **Build with OpenVINO.** This is the 2-3x on the encoder that makes `small` viable on Iris Xe (§3). Without it, use `base`. |
| `piper` | [piper](https://github.com/rhasspy/piper) | Use the real binary. It links espeak-ng, which fixes the identity-function phonemizer bug on the Kotlin branch *and* is what the scorer needs (§7). |

## 2. Models

```bash
python desktop/fetch_models.py \
    --dest engines \
    --piper-voice /path/to/en_IN-navgurukul-medium.onnx
```

~3 GB. Resumable, and skips anything already present.

The Piper voice is not downloadable — it is the custom NavGurukul voice on
`origin/feature/navgurukul-piper` (commit `bd348be`). Both the `.onnx` and its
`.onnx.json` are required; piper reads the sample rate and phoneme id map from
the sidecar.

`--llm llama-3.2-3b` fetches the bake-off alternative (§11).

## 3. Orchestrator

```bash
cd backend
pip install -r requirements.txt -r requirements-desktop.txt
```

Run it directly during development:

```bash
DESKTOP_MODE=true \
GRADING_ASYNC=true \
DESKTOP_TOKEN=devtoken \
ELLA_BIND_PORT=8000 \
LOCAL_LLM_BASE_URL=http://127.0.0.1:8080/v1 \
LOCAL_STT_BASE_URL=http://127.0.0.1:8081/v1 \
PIPER_BINARY=/usr/local/bin/piper \
PIPER_VOICE_PATH=/path/to/en_IN-navgurukul-medium.onnx \
python desktop_main.py
```

It migrates its own database on start (a packaged app has no terminal) and
refuses to bind anything but loopback while `DESKTOP_MODE` is on.

Check it: `curl http://127.0.0.1:8000/desktop/health` — per-engine readiness,
with the reason each one is not ready.

## 4. Flutter

`ella_app/.env`:

```ini
ELLA_OFFLINE=true
ELLA_ENGINE_ROOT=/absolute/path/to/engines

# Dev overrides — skip these in a packaged build, where everything is
# resolved relative to the executable.
ELLA_ORCHESTRATOR=/path/to/backend/run-orchestrator.sh
ELLA_LLAMA_SERVER=/path/to/llama-server
ELLA_WHISPER_SERVER=/path/to/whisper-server
ELLA_PIPER=/usr/local/bin/piper

# Tuning
ELLA_LLAMA_THREADS=6      # cores - 2; leave room for whisper and piper
ELLA_LLAMA_CTX=8192
ELLA_VAD_SILENCE_MS=450   # trailing silence before a turn ends — a UX knob
ELLA_SCORER_ENABLED=false
```

```bash
cd ella_app
flutter run -d windows      # or -d macos
```

With `ELLA_OFFLINE` unset, the desktop build talks to the hosted API exactly
like the Android build — useful for UI work without 3 GB of models loaded.

---

## Windows quickstart (the reference target)

Prerequisites the Flutter toolchain will not install for you:

- **Visual Studio 2022** with the *Desktop development with C++* workload.
  `flutter doctor -v` must show a green Windows entry; the Build Tools alone are
  not enough for `flutter run -d windows`.
- **Python 3.12** on PATH.
- `git`, and enough disk for ~4 GB of models.

```powershell
git clone https://github.com/navgurukul/ella_flutter.git
cd ella_flutter
git checkout feature/desktop-offline-sts
flutter doctor -v          # confirm the Windows target is green
```

### Engine binaries -> `engines\bin\`

All three ship prebuilt for Windows x64:

| Binary | Where | Windows notes |
|---|---|---|
| `llama-server.exe` | llama.cpp releases, `llama-*-bin-win-*.zip` | Take the AVX2 build for Tiger Lake. Keep the DLLs beside the exe. |
| `whisper-server.exe` | whisper.cpp releases | For the OpenVINO encoder (§3) you need the OpenVINO build **plus** the OpenVINO runtime DLLs on PATH. The plain build works first — get a turn end to end, then chase the 2-3x. |
| `piper.exe` | piper releases, `piper_windows_amd64.zip` | **Extract the whole archive.** piper needs its `espeak-ng-data\` directory sitting next to the exe; without it phonemization fails at synthesis time. |

### Models and config

```powershell
python desktop\fetch_models.py --dest ella_app\build\engines ^
    --piper-voice C:\path\to\en_IN-navgurukul-medium.onnx

cd backend
python -m venv .venv
.venv\Scripts\pip install -r requirements.txt -r requirements-desktop.txt
```

`ella_app\.env`:

```ini
ELLA_OFFLINE=true
ELLA_ENGINE_ROOT=C:\path\to\ella_flutter\ella_app\build\engines
ELLA_ORCHESTRATOR=C:\path\to\ella_flutter\backend\run-orchestrator.cmd
ELLA_LLAMA_SERVER=C:\path\to\engines\bin\llama-server.exe
ELLA_WHISPER_SERVER=C:\path\to\engines\bin\whisper-server.exe
ELLA_PIPER=C:\path\to\engines\bin\piper.exe
ELLA_LLAMA_THREADS=6
```

Use `run-orchestrator.cmd`, not the `.sh` — that one is for macOS and Linux.

### Run

```powershell
cd ella_app
flutter run -d windows
```

The supervisor starts all three engines itself, so nothing needs launching by
hand. The boot screen shows per-engine readiness and prints the failing
engine's own stderr when one does not come up.

Sanity-check the backend on its own first if the boot screen sticks:

```powershell
curl http://127.0.0.1:8000/desktop/health
```

### Windows-specific gotchas

- **Defender will scan the first launch** of `llama-server.exe` reading a 2 GB
  model, which can add tens of seconds. That is Defender, not the engine — the
  health poll allows three minutes for exactly this reason. Consider an
  exclusion for the `engines\` directory while developing.
- **A firewall prompt on first run is a bug, not a normal step.** Everything
  binds `127.0.0.1`; if Windows offers to allow public-network access,
  something is binding the wrong interface — capture it and file it.
- No sandbox on Windows, so unlike macOS there is no entitlement to add for the
  loopback listener.

---

## Week-one experiments

These decide things the architecture currently assumes. Run them before the
design hardens (§11).

```bash
# Is the KV cache actually being reused, and does decode outrun speech? (§4, §5)
python desktop/bench/bench_llm.py --base-url http://127.0.0.1:8080/v1

# small vs base on real student audio. (§3)
python desktop/bench/bench_whisper.py \
    --small http://127.0.0.1:8081/v1 --base http://127.0.0.1:8082/v1 \
    --audio samples/ --reference samples/transcripts.json

# How long is a session report really? (§6, §11)
python tooling/bench/measure_grading_tokens.py --from-db backend/zoe.db
python tooling/bench/measure_grading_tokens.py --live --limit 10
```

`bench_llm.py` is the one to run first. If it prints *"KV CACHE IS NOT BEING
REUSED"*, nothing else matters until that is fixed — every turn is paying a
full prefill and the product is unusable regardless of how good the model is.

---

## Packaging

```bash
# 1. Flutter release
cd ella_app && flutter build windows --release

# 2. Orchestrator
pyinstaller desktop/ella_orchestrator.spec --distpath desktop/dist

# 3. Installer
iscc desktop\installer\ella.iss                  # models download on first run
iscc /DIncludeModels desktop\installer\ella.iss  # ~4 GB, fully offline
```

**Start the code-signing certificate now.** It takes weeks. Unsigned installers
hit SmartScreen and students will not get past it, and PyInstaller bundles draw
antivirus false positives on top of that. This is the underrated risk in the
whole project — the AI is the known quantity (§10).

---

## Security notes

Desktop mode is not hosted mode with a different URL:

- `allow_origins=["*"]` is **not** used. Desktop builds restrict CORS to
  loopback origins, because the orchestrator is a port on the student's own
  machine.
- Every AI-path request needs `X-Ella-Desktop-Token`, generated per launch by
  the supervisor and known only to the two processes.
- The orchestrator refuses to bind a non-loopback interface while
  `DESKTOP_MODE` is on.
- If the Flutter shell crashes, the orchestrator notices its parent is gone and
  exits, rather than leaving a 2 GB model resident with no UI attached.
