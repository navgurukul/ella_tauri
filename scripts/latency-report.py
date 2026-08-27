#!/usr/bin/env python3
"""Summarize Ella's persisted turn telemetry (latency + errors) by day.

Reads latency.jsonl from the app data dir (or a path passed as argv[1]) and
prints per-day medians/p95s per pipeline stage, error counts, and STT
fallback rates, so improvements can be compared over time.
"""
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

DEFAULT_PATH = (
    Path.home()
    / "Library/Application Support/org.navgurukul.ella.desktop/telemetry/latency.jsonl"
)


def pct(values, p):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round(p / 100 * (len(ordered) - 1))))
    return ordered[index]


def fmt(value):
    return "-" if value is None else f"{value:.0f}"


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_PATH
    if not path.exists():
        print(f"No telemetry found at {path}")
        print("Have a conversation first, or pass the .jsonl path explicitly.")
        return 1

    days = defaultdict(list)
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("event") != "ella_turn_latency":
            continue
        days[event.get("timestamp", "?")[:10]].append(event)

    if not days:
        print(f"{path} contains no turn events yet.")
        return 1

    print(f"Telemetry: {path}\n")
    header = (
        f"{'day':10} {'turns':>5} {'errors':>6} {'fallback':>8} "
        f"{'stt p50/p95':>12} {'ttft p50/p95':>13} {'llm p50/p95':>12} "
        f"{'tts p50/p95':>12} {'total p50/p95':>14}"
    )
    print(header)
    print("-" * len(header))
    for day in sorted(days):
        events = days[day]
        ok = [e for e in events if e.get("status") == "ok"]
        errors = [e for e in events if e.get("status") != "ok"]
        fallbacks = [
            e
            for e in ok
            if e.get("stt_fallback_from")
            or (e.get("stt_engine") == "whisper-small")
        ]
        stt = [e["stt_ms"] for e in ok if e.get("stt_ms") is not None]
        ttft = [e["llm_ttft_ms"] for e in ok if e.get("llm_ttft_ms") is not None]
        llm = [e["llm_completion_ms"] for e in ok if e.get("llm_completion_ms") is not None]
        tts = [e["tts_first_audio_ms"] for e in ok if e.get("tts_first_audio_ms") is not None]
        total = [e["total_ms"] for e in ok if e.get("total_ms") is not None]
        rate = f"{len(fallbacks)}/{len(ok)}" if ok else "-"
        print(
            f"{day:10} {len(events):>5} {len(errors):>6} {rate:>8} "
            f"{fmt(pct(stt, 50)):>5}/{fmt(pct(stt, 95)):>6} "
            f"{fmt(pct(ttft, 50)):>6}/{fmt(pct(ttft, 95)):>6} "
            f"{fmt(pct(llm, 50)):>5}/{fmt(pct(llm, 95)):>6} "
            f"{fmt(pct(tts, 50)):>5}/{fmt(pct(tts, 95)):>6} "
            f"{fmt(pct(total, 50)):>6}/{fmt(pct(total, 95)):>7}"
        )

    errors = [e for events in days.values() for e in events if e.get("status") != "ok"]
    if errors:
        print("\nRecent errors:")
        for event in errors[-8:]:
            print(f"  {event.get('timestamp','?')[:19]}  {event.get('error','?')}")

    if len(sys.argv) <= 1:
        failures = path.parent.parent / "stt-failures"
        if failures.is_dir():
            wavs = sorted(failures.glob("*.wav"))
            if wavs:
                print(f"\nCanary failure recordings: {len(wavs)} in {failures}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
