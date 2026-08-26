# STT benchmark

`stt-benchmark` measures final-transcript latency through the same native
`SpeechToTextEngine` implementations used by Ella. It is deliberately a
stop-to-final benchmark; Canary-180M-Flash is not a streaming model.

The checked-in WAV is the public `samples/jfk.wav` fixture from whisper.cpp.
The runner truncates it in memory to exactly 4.214 seconds, so both engines see
the same mono PCM samples and the duration matches the existing Ella baseline.
It is not the original Ella baseline recording, which was not present in this
checkout; compare transcript quality only within this same-input run.

## Run

Start the Whisper small fallback server first:

```bash
cd desktop/ella_tauri
npm run engines:local
```

In a second terminal:

```bash
ELLA_ENGINE_ROOT=/absolute/path/to/ella_app/build/engines \
npm run benchmark:stt -- \
  --audio bench/fixtures/jfk.wav \
  --duration-ms 4214 \
  --whisper-url http://127.0.0.1:39092 \
  --warmup 1 \
  --iterations 5 \
  --whisper-baseline-ms 3387 \
  --output bench/results/local.json
```

Omit `--whisper-url` for a Canary-only run. Model integrity verification is on
by default; `--skip-sha256` exists only for model-development experiments.

## Intel Mac development result (2026-08-26)

The AMD Radeon Pro 5300M did not provide the matrix feature required by the
Metal backend, so transcribe.cpp selected CPU. After one warm-up:

| Engine | Backend | Runs (ms) | Median |
|---|---|---:|---:|
| Canary-180M-Flash Q8_0 | CPU | 210.8, 210.2, 208.0, 215.8, 214.1 | **210.8 ms** |
| Whisper small | HTTP sidecar / CPU | 3600.1, 4758.2, 3750.7, 3093.1, 3823.5 | **3750.7 ms** |

Canary was 16.1x faster than the supplied 3387 ms Whisper baseline and 17.8x
faster than the live Whisper median on the same fixture. The transcripts differ,
so this small fixture is a latency regression test rather than an accuracy
evaluation. See [`results/intel-mac-2026-08-26.json`](results/intel-mac-2026-08-26.json).
