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
cd /path/to/ella_tauri
npm run engines:local
```

In a second terminal:

```bash
ELLA_ENGINE_ROOT="$PWD/engines" \
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

# Latency probe (OS-dependent turn work)

`latency-probe` times, through the code the app runs, the work whose cost
depends on the operating system: the resident Piper per sentence (on Windows,
`piper.exe` writing a WAV to `%TEMP%` that Ella reads back), a sentence-sized
file round trip, `Database::persist_turn`, process starts (PowerShell, which the
Windows speech fallback starts per answer), the SAPI fallback itself, Canary's
load with and without its launch checksum, and — with `--only launch` — the
local engine's whole start-up as the setup screen waits for it.

```bash
ELLA_ENGINE_ROOT="$PWD/engines" \
cargo run --release --manifest-path src-tauri/Cargo.toml --bin latency-probe -- \
  --rounds 5 --audio bench/fixtures/jfk.wav --output bench/results/probe.json
```

A development Mac cannot build the Windows target, so the **Latency probe**
workflow runs it on `windows-latest` for every push to a `perf/**` branch,
with Defender's real-time scanning switched on, and posts each result as a
notice on the run. Hosted runners land on different CPUs from run to run
(EPYC 7763 and 9V74 both seen, ±20% on the same commit), so a change is
measured inside one job: the workflow builds the probe at `PROBE_BEFORE`
(default `HEAD~1`) and at `HEAD` and alternates their launch timings.

## Windows findings (2026-10-19)

From [`results/windows-ci-2026-10-19.json`](results/windows-ci-2026-10-19.json)
and [`results/probe-mac-m1pro.json`](results/probe-mac-m1pro.json):

| Probe | Windows runner | M1 Pro |
|---|---:|---:|
| Piper, after it answers until Ella has the PCM (WAV read + remove on Windows, pipe on Mac) | 0.45 ms | 0.03 ms |
| Piper sentence, total | 212 ms | 132 ms |
| Sentence-sized WAV write/read/remove in `%TEMP%`, Defender on | 0.56 ms | 0.18 ms |
| `persist_turn` commit | 0.2–0.4 ms | 4.2 ms |
| PowerShell start / + `Add-Type System.Speech` | 133 / 190 ms | – |
| SAPI fallback, 11 s of audio | 1.7–2.0 s | – |
| Canary load, verified / unverified | 328 / 184 ms | 724 / 80 ms |
| Canary, 4.2 s fixture, threads default / 2 / 4 | 351 / 353 / 326 ms | 211 ms (Intel Mac, CPU) |
| Engine start-up, Canary loaded before / beside llama-server | 2777 → 2527 ms | 1502 → 773 ms |
