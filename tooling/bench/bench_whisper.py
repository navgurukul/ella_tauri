#!/usr/bin/env python3
"""base vs small on this machine — the §3 experiment, and it decides a lot.

The plan calls this "a 30-minute experiment that decides transcription quality
for the whole project". `small` is meaningfully better on Indian-accented
English and Hindi code-switching; the question is only whether it fits the
0.8-1.2 s budget in §5 on an Iris Xe with the OpenVINO encoder.

Point it at two whisper-server instances (one per model) and a directory of
real recordings. Synthetic audio tells you nothing about accent handling, so
use clips from actual students if you possibly can.

    python desktop/bench/bench_whisper.py \\
        --small http://127.0.0.1:8081/v1 \\
        --base  http://127.0.0.1:8082/v1 \\
        --audio samples/ --reference samples/transcripts.json

`transcripts.json` maps filename -> the words actually spoken. With it you get
word error rate as well as latency, which is the half that actually matters.
"""
from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
import time
import urllib.error
import urllib.request
import uuid
import wave
from pathlib import Path


def wav_duration(path: Path) -> float:
    try:
        with wave.open(str(path), "rb") as w:
            return w.getnframes() / float(w.getframerate() or 1)
    except (wave.Error, OSError):
        return 0.0


def transcribe(base_url: str, path: Path, timeout: float) -> tuple[str, float]:
    """POST one file. Returns (text, seconds)."""
    boundary = uuid.uuid4().hex
    data = path.read_bytes()
    parts = [
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="file"; filename="{path.name}"\r\n'
        f"Content-Type: audio/wav\r\n\r\n".encode(),
        data,
        b"\r\n",
    ]
    for key, value in (
        ("model", "whisper"),
        ("response_format", "json"),
        ("language", "en"),
        ("temperature", "0"),
    ):
        parts.append(
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="{key}"\r\n\r\n'
            f"{value}\r\n".encode()
        )
    parts.append(f"--{boundary}--\r\n".encode())
    body = b"".join(parts)

    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/audio/transcriptions",
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        payload = json.loads(response.read())
    return (payload.get("text") or "").strip(), time.perf_counter() - started


_PUNCT = re.compile(r"[^\w\s']")


def normalize(text: str) -> list[str]:
    """Lowercase, strip punctuation. WER should not punish a missing comma."""
    return _PUNCT.sub(" ", text.lower()).split()


def word_error_rate(reference: str, hypothesis: str) -> float:
    """Levenshtein over words, divided by reference length."""
    ref, hyp = normalize(reference), normalize(hypothesis)
    if not ref:
        return 0.0 if not hyp else 1.0

    previous = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        current = [i]
        for j, h in enumerate(hyp, 1):
            current.append(min(
                previous[j] + 1,        # deletion
                current[j - 1] + 1,     # insertion
                previous[j - 1] + (r != h),  # substitution
            ))
        previous = current
    return previous[-1] / len(ref)


def evaluate(name: str, base_url: str, files: list[Path],
             references: dict[str, str], timeout: float) -> dict | None:
    print(f"\n=== {name} ({base_url}) ===")
    latencies, ratios, wers = [], [], []

    for path in files:
        duration = wav_duration(path)
        try:
            text, elapsed = transcribe(base_url, path, timeout)
        except (urllib.error.URLError, OSError) as exc:
            print(f"  ✗ {path.name}: {exc}", file=sys.stderr)
            continue

        latencies.append(elapsed)
        # Real-time factor: below 1.0 means faster than the audio is long.
        rtf = elapsed / duration if duration else 0.0
        ratios.append(rtf)

        line = f"  {path.name:32} {elapsed:5.2f} s  rtf {rtf:4.2f}"
        reference = references.get(path.name)
        if reference:
            wer = word_error_rate(reference, text)
            wers.append(wer)
            line += f"  wer {wer*100:5.1f}%"
        print(line)
        print(f"      → {text[:100]}")

    if not latencies:
        return None
    return {
        "name": name,
        "median_latency": statistics.median(latencies),
        "median_rtf": statistics.median(ratios) if ratios else 0.0,
        "median_wer": statistics.median(wers) if wers else None,
        "n": len(latencies),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--small", help="whisper-server running ggml-small")
    parser.add_argument("--base", help="whisper-server running ggml-base")
    parser.add_argument("--audio", required=True, help="directory of .wav clips")
    parser.add_argument("--reference", help="JSON map of filename -> transcript")
    parser.add_argument("--timeout", type=float, default=180.0)
    args = parser.parse_args()

    if not args.small and not args.base:
        print("give at least one of --small / --base", file=sys.stderr)
        return 2

    files = sorted(Path(args.audio).glob("*.wav"))
    if not files:
        print(f"no .wav files in {args.audio}", file=sys.stderr)
        return 2

    references: dict[str, str] = {}
    if args.reference:
        references = json.loads(Path(args.reference).read_text())
        print(f"{len(references)} reference transcripts loaded")
    else:
        print("no --reference given: latency only, no accuracy. "
              "Latency alone cannot decide this question.")

    print(f"{len(files)} clips, total "
          f"{sum(wav_duration(f) for f in files):.1f} s of audio")

    results = []
    for name, url in (("small", args.small), ("base", args.base)):
        if url:
            result = evaluate(name, url, files, references, args.timeout)
            if result:
                results.append(result)

    print("\n--- verdict ---")
    for r in results:
        wer = f"{r['median_wer']*100:5.1f}%" if r["median_wer"] is not None else "  n/a"
        print(f"{r['name']:6} median {r['median_latency']:5.2f} s  "
              f"rtf {r['median_rtf']:4.2f}  wer {wer}  (n={r['n']})")

    small = next((r for r in results if r["name"] == "small"), None)
    base = next((r for r in results if r["name"] == "base"), None)

    if small:
        # §5 budgets 0.8-1.2 s for ~5 s of audio.
        if small["median_latency"] > 1.5:
            print(f"\n✗ small at {small['median_latency']:.2f} s is outside the "
                  f"0.8-1.2 s budget in §5.")
            print("  Check the OpenVINO encoder is actually being used — that "
                  "is the 2-3x that makes small viable at all (§3).")
        else:
            print(f"\n✓ small fits the §5 budget at {small['median_latency']:.2f} s")

    if small and base and small["median_wer"] is not None \
            and base["median_wer"] is not None:
        gain = (base["median_wer"] - small["median_wer"]) * 100
        cost = small["median_latency"] - base["median_latency"]
        print(f"\nsmall vs base: {gain:+.1f} pp WER for {cost:+.2f} s per turn")
        if gain < 2.0:
            print("→ small is not buying much accuracy here. base is the "
                  "cheaper choice.")
        elif cost > 1.0:
            print("→ real accuracy gain, but it costs more than a second a "
                  "turn. Weigh against §5.")
        else:
            print("→ small is the right call: better accuracy, affordable cost.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
