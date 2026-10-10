#!/usr/bin/env python3
"""Summarize Ella's persisted turn and recap telemetry (latency + errors).

Reads latency.jsonl from the app data dir (or a path passed as an argument)
and prints per-day medians/p95s per pipeline stage, error counts, and STT
fallback rates, so improvements can be compared over time. Newer builds name
themselves in every event, so a day that ran two builds is split by build, and
their events (`schema_version` 2) say enough to tell a slow computer from slow
code: what the model read and wrote and how fast, how Canary kept up, and how
the computer was powered and how busy it was.

A file saved through PowerShell comes out UTF-16; that is read too.

  npm run telemetry:report -- [path] [--turns]

--turns also prints every turn, talk by talk.
"""
import json
import re
import statistics
import sys
from collections import Counter, defaultdict
from datetime import datetime, timedelta
from pathlib import Path

DEFAULT_PATH = (
    Path.home()
    / "Library/Application Support/org.navgurukul.ella.desktop/telemetry/latency.jsonl"
)

# A gap this long between turns starts a new talk, for events from before
# turns named their talk.
TALK_GAP = timedelta(minutes=4)

# Rust writes up to nanoseconds; Python before 3.11 reads at most micro.
FRACTION = re.compile(r"(\.\d{6})\d+")


def pct(values, p):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round(p / 100 * (len(ordered) - 1))))
    return ordered[index]


def fmt(value):
    return "-" if value is None else f"{value:.0f}"


def secs(ms):
    return "-" if ms is None else f"{ms / 1000:.1f}"


def median(values):
    return statistics.median(values) if values else None


def read_lines(path):
    """The file's lines, whatever it was saved as: UTF-8, with or without a
    byte-order mark, or UTF-16, as PowerShell's `>` writes it."""
    raw = path.read_bytes()
    if raw.startswith((b"\xff\xfe", b"\xfe\xff")):
        text = raw.decode("utf-16")
    elif raw.startswith(b"\xef\xbb\xbf"):
        text = raw.decode("utf-8-sig")
    elif len(raw) > 1 and raw[1:2] == b"\x00":
        text = raw.decode("utf-16-le")
    else:
        text = raw.decode("utf-8", errors="replace")
    return text.splitlines()


def read_events(path):
    events = []
    for line in read_lines(path):
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return events


def when(event):
    stamp = FRACTION.sub(r"\1", event.get("timestamp", "")).replace("Z", "+00:00")
    try:
        return datetime.fromisoformat(stamp)
    except ValueError:
        return None


def local(event):
    at = when(event)
    return at.astimezone() if at else None


def group_of(event):
    """The day, and the build where the event names it; `cloud` after it for a
    turn the cloud answered, or a recap the cloud judged, so neither is
    counted in with what the laptop did itself."""
    day = event.get("timestamp", "?")[:10]
    version = event.get("app_version")
    group = f"{day} {version}" if version else day
    return f"{group} cloud" if by_cloud(event) else group


def runs(event):
    return event.get("llm_runs") or []


def local_runs(event):
    """The generations the laptop's own model wrote: only they say how fast
    the computer is."""
    return [run for run in runs(event) if not run.get("backend")]


def by_cloud(event):
    """Whether the cloud wrote the reply the learner heard, or judged the whole recap."""
    if event.get("event") == "ella_assessment":
        judges = [judge for judge in event.get("judges", []) if judge.get("status") == "ok"]
        return bool(judges) and all(judge.get("backend") == "cloud" for judge in judges)
    written = [run for run in runs(event) if not run.get("failed")]
    return bool(written) and written[-1].get("backend") == "cloud"


def prompt_rate(run):
    """Prompt tokens a second the server evaluated, or None."""
    tokens, ms = run.get("prompt_evaluated"), run.get("prompt_ms")
    # A handful of tokens is too few to time.
    return tokens * 1000 / ms if tokens and ms and tokens >= 16 else None


def write_rate(run):
    tokens, ms = run.get("gen_tokens"), run.get("gen_ms")
    return tokens * 1000 / ms if tokens and ms and tokens >= 4 else None


def canary_factor(piece):
    """Seconds Canary took over each second of speech, waiting excluded."""
    speech = piece.get("speech_ms") or piece.get("audio_ms")
    if not speech or not str(piece.get("engine", "")).startswith("canary"):
        return None
    return max(0, piece.get("ms", 0) - (piece.get("queued_ms") or 0)) / speech


def stage_table(days):
    header = (
        f"{'day / build':17} {'turns':>5} {'errors':>6} {'fallback':>8} "
        f"{'stt p50/p95':>12} {'ttft p50/p95':>13} {'llm p50/p95':>12} "
        f"{'tts p50/p95':>12} {'speaks p50/p95':>15} {'total p50/p95':>14}"
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
        # When Ella started to speak. Logs from before it was kept have only
        # the total, which is when a reply played whole starts anyway.
        speaks = [e.get("speech_ms", e.get("total_ms")) for e in ok if e.get("speech_ms", e.get("total_ms")) is not None]
        total = [e["total_ms"] for e in ok if e.get("total_ms") is not None]
        rate = f"{len(fallbacks)}/{len(ok)}" if ok else "-"
        print(
            f"{day:17} {len(events):>5} {len(errors):>6} {rate:>8} "
            f"{fmt(pct(stt, 50)):>5}/{fmt(pct(stt, 95)):>6} "
            f"{fmt(pct(ttft, 50)):>6}/{fmt(pct(ttft, 95)):>6} "
            f"{fmt(pct(llm, 50)):>5}/{fmt(pct(llm, 95)):>6} "
            f"{fmt(pct(tts, 50)):>5}/{fmt(pct(tts, 95)):>6} "
            f"{fmt(pct(speaks, 50)):>7}/{fmt(pct(speaks, 95)):>7} "
            f"{fmt(pct(total, 50)):>6}/{fmt(pct(total, 95)):>7}"
        )


def detail_summary(days):
    """What `schema_version` 2 events add, build by build."""
    printed = False
    for day in sorted(days):
        events = [e for e in days[day] if e.get("schema_version", 1) >= 2]
        if not events:
            continue
        if not printed:
            print("\nWhy, for builds that say (rates are medians; 'others' is CPU busy with something besides Ella):")
            printed = True
        ok = [e for e in events if e.get("status") == "ok"]
        all_runs = [run for e in ok for run in local_runs(e)]
        prompt = [rate for rate in map(prompt_rate, all_runs) if rate]
        write = [rate for rate in map(write_rate, all_runs) if rate]
        evaluated = [
            local_runs(e)[0].get("prompt_evaluated")
            for e in ok
            if local_runs(e) and local_runs(e)[0].get("prompt_evaluated") is not None
        ]
        cloud_runs = [run for e in ok for run in runs(e) if run.get("backend") == "cloud"]
        cloud_ok = [run for run in cloud_runs if not run.get("failed")]
        cloud_failed = Counter(run["failed"] for run in cloud_runs if run.get("failed"))
        cloud_ttft = [run["ttft_ms"] for run in cloud_ok if run.get("ttft_ms") is not None]
        cloud_wasted = [run["ms"] for run in cloud_runs if run.get("failed") and run.get("ms") is not None]
        cloud_prompt = sum(run.get("prompt_tokens") or 0 for run in cloud_ok)
        cloud_cached = sum((run.get("prompt_tokens") or 0) - (run.get("prompt_evaluated") or 0) for run in cloud_ok)
        rewrites = sum(1 for e in ok if any(run.get("why") == "rewrite" for run in runs(e)))
        on_clock = sum(1 for e in ok if e.get("tts_path") == "on_clock")
        firsts = [e for e in ok if e.get("turn") == 1 and e.get("llm_ttft_ms") is not None]
        cold = [e for e in firsts if e["llm_ttft_ms"] >= 5000]
        pieces = [piece for e in events for piece in e.get("stt_pieces") or []]
        factor = [f for f in map(canary_factor, pieces) if f is not None]
        queued = sum(1 for piece in pieces if (piece.get("queued_ms") or 0) >= 100)
        engines = Counter(piece.get("engine") for piece in pieces)
        machines = [e["machine"] for e in events if e.get("machine")]
        battery = sum(1 for m in machines if m.get("on_ac") is False)
        saver = sum(1 for m in machines if m.get("battery_saver"))
        modes = Counter(m.get("power_mode") for m in machines if m.get("power_mode"))
        busy = [m["cpu_busy_pct"] for m in machines if m.get("cpu_busy_pct") is not None]
        others = [
            max(0, m["cpu_busy_pct"] - m["cpu_ella_pct"])
            for m in machines
            if m.get("cpu_busy_pct") is not None and m.get("cpu_ella_pct") is not None
        ]
        limits = [m["cpu_mhz_limit"] for m in machines if m.get("cpu_mhz_limit")]
        tops = [m["cpu_mhz_max"] for m in machines if m.get("cpu_mhz_max")]
        notes = Counter(note for e in ok for note in e.get("notes") or [])

        print(f"{day}  ({len(events)} turns)")
        if cloud_runs:
            print(
                f"  cloud    wrote {len(cloud_ok)} runs, first word {fmt(median(cloud_ttft))} ms "
                f"(slowest {fmt(max(cloud_ttft) if cloud_ttft else None)}); "
                f"{cloud_cached * 100 // cloud_prompt if cloud_prompt else 0}% of its prompt tokens cached; "
                f"failed {', '.join(f'{why} {n}' for why, n in cloud_failed.most_common()) or 'never'}"
                + (f", costing {fmt(median(cloud_wasted))} ms each (median)" if cloud_wasted else "")
            )
        if all_runs:
            print(
                f"  model    reads {fmt(median(prompt))} tok/s (slowest {fmt(min(prompt) if prompt else None)}), "
                f"writes {fmt(median(write))} tok/s; evaluates {fmt(median(evaluated))} prompt tokens a turn"
            )
        print(
            f"  replies  rewritten {rewrites}/{len(ok)}, said on the clock {on_clock}/{len(ok)}; "
            f"first replies {len(firsts)}, cold (ttft >= 5 s) {len(cold)}"
            + (f": {', '.join(secs(e['llm_ttft_ms']) for e in cold)} s" if cold else "")
        )
        if notes:
            print("  notes    " + ", ".join(f"{name} {count}" for name, count in notes.most_common()))
        if pieces:
            speed = f"Canary {median(factor):.2f} s a second of speech, " if factor else ""
            print(
                f"  speech   {speed}{len(pieces)} pieces, {queued} waited for another; "
                + ", ".join(f"{name} {count}" for name, count in engines.most_common())
            )
        if machines:
            print(
                f"  machine  on battery {battery}/{len(machines)}, saver {saver}/{len(machines)}, "
                f"mode {', '.join(f'{mode} {count}' for mode, count in modes.most_common()) or '-'}; "
                f"CPU busy {fmt(median(busy))}%, others {fmt(median(others))}% (most {fmt(max(others) if others else None)}%); "
                f"clock limit {fmt(min(limits) if limits else None)} of {fmt(max(tops) if tops else None)} MHz"
            )


def launches(events):
    started = [e for e in events if e.get("event") == "ella_launch"]
    if not started:
        return
    engines = {e.get("launch_id"): e for e in events if e.get("event") == "ella_engine"}
    print("\nLaunches:")
    for launch in started:
        at = local(launch)
        cores = f"{launch.get('cores_physical', '?')}/{launch.get('cores_logical', '?')} cores"
        ram = f"{launch['ram_mb'] / 1024:.0f} GB" if launch.get("ram_mb") else "-"
        line = (
            f"  {at:%Y-%m-%d %H:%M} {launch.get('app_version', '?'):8} {launch.get('os', '?')}  "
            f"{launch.get('cpu', '?')}  {cores}  {ram}"
            if at
            else f"  {launch.get('app_version', '?')} {launch.get('os', '?')}"
        )
        power = launch.get("machine") or {}
        if power.get("on_ac") is not None:
            line += "  on AC" if power["on_ac"] else f"  on battery {power.get('battery_pct', '?')}%"
        engine = engines.get(launch.get("launch_id"))
        if engine:
            line += (
                f"\n      engine: llama {engine.get('llm_server')}, {engine.get('llama_threads', '-')} threads, "
                f"build {engine.get('llama_build', '-')}, model {engine.get('llm_model_mb', '-')} MB; "
                f"stt {engine.get('stt')}; piper {engine.get('piper')}; "
                f"ready {secs(engine.get('since_launch_ms'))} s after launch"
                + (f"; error: {engine['error']}" if engine.get("error") else "")
            )
        print(line)


def talks(turns):
    """Turns grouped into talks: by the talk they name, or by the gaps between
    them for events from before turns named it."""
    grouped = []
    previous = None
    for event in turns:
        at = when(event)
        session = event.get("session_id")
        new = (
            not grouped
            or (session and session != grouped[-1][0].get("session_id"))
            or (not session and (at is None or previous is None or at - previous > TALK_GAP))
        )
        if new:
            grouped.append([])
        grouped[-1].append(event)
        previous = at
    return grouped


def turn_rows(turns):
    print("\nTurns (wait = from the learner stopping to Ella starting; s unless marked):")
    for talk in talks(turns):
        first = talk[0]
        at = local(first)
        title = " ".join(
            str(part)
            for part in (first.get("talk"), first.get("topic"), first.get("level"), first.get("app_version"))
            if part
        )
        print(f"---- {title or 'talk'} from {at:%a %d %b %H:%M}" if at else f"---- {title or 'talk'}")
        print(
            f"  {'time':8} {'#':>2} {'said':>5} {'stt':>4} {'ttft':>5} {'llm':>5} {'wait':>5} "
            f"{'read':>5} {'p t/s':>5} {'g t/s':>5} {'tts':>10}  notes / machine"
        )
        for event in talk:
            at = local(event)
            first_run = runs(event)[0] if runs(event) else {}
            extra = list(event.get("notes") or [])
            if len(runs(event)) > 1:
                extra.append(f"{len(runs(event))} runs")
            warm_up = event.get("warm_up")
            if warm_up:
                restored = warm_up.get("restored_tokens")
                extra.append(
                    f"warm-up {secs(warm_up.get('ms'))} s, "
                    + (f"restored {restored}, " if restored else "")
                    + f"evaluated {warm_up.get('evaluated_tokens')}"
                    + (f", behind {warm_up['errand']}" if warm_up.get("errand") else "")
                )
            if (first_run.get("wait_ms") or 0) >= 100:
                extra.append(f"waited {secs(first_run['wait_ms'])} s for the warm-up")
            if first_run.get("errand"):
                extra.append(f"behind {first_run['errand']}")
            pieces = event.get("stt_pieces") or []
            late = [p for p in pieces if p.get("part") == "background" and p.get("done_ms", 0) > 0]
            if late:
                extra.append(f"{len(late)} piece(s) done after stop")
            odd = [p.get("engine") for p in pieces if not str(p.get("engine", "")).startswith("canary")]
            if odd:
                extra.append("/".join(odd))
            machine = event.get("machine") or {}
            if machine:
                power = "AC" if machine.get("on_ac") else ("battery" if machine.get("on_ac") is False else "")
                if machine.get("battery_saver"):
                    power += " saver"
                if machine.get("power_mode") and machine["power_mode"] != "balanced":
                    power += f" {machine['power_mode']}"
                busy, mine = machine.get("cpu_busy_pct"), machine.get("cpu_ella_pct")
                if busy is not None and mine is not None:
                    power += f" busy {busy}% others {max(0, busy - mine)}%"
                limit, top = machine.get("cpu_mhz_limit"), machine.get("cpu_mhz_max")
                if limit and top and limit < top:
                    power += f" held to {limit}/{top} MHz"
                extra.append(power.strip())
            if event.get("status") != "ok":
                where = f" after {event['failed_at']}" if event.get("failed_at") else ""
                extra.append(f"ERROR{where}: {event.get('error')}")
            print(
                f"  {f'{at:%H:%M:%S}' if at else '?':8} {event.get('turn', ''):>2} {secs(event.get('audio_input_ms')):>5} "
                f"{secs(event.get('stt_ms')):>4} {secs(event.get('llm_ttft_ms')):>5} "
                f"{secs(event.get('llm_completion_ms')):>5} {secs(event.get('speech_ms')):>5} "
                f"{fmt(first_run.get('prompt_evaluated')):>5} {fmt(prompt_rate(first_run)):>5} "
                f"{fmt(write_rate(first_run)):>5} {event.get('tts_path', '-'):>10}  "
                + "; ".join(part for part in extra if part)
            )


def main() -> int:
    arguments = [argument for argument in sys.argv[1:] if not argument.startswith("--")]
    flags = {argument for argument in sys.argv[1:] if argument.startswith("--")}
    path = Path(arguments[0]) if arguments else DEFAULT_PATH
    if not path.exists():
        print(f"No telemetry found at {path}")
        print("Have a conversation first, or pass the .jsonl path explicitly.")
        return 1

    events = read_events(path)
    days = defaultdict(list)
    recaps = defaultdict(list)
    turns = []
    for event in events:
        if event.get("event") == "ella_turn_latency":
            days[group_of(event)].append(event)
            turns.append(event)
        elif event.get("event") == "ella_assessment":
            recaps[group_of(event)].append(event)

    if not days:
        print(f"{path} contains no turn events yet.")
        return 1

    print(f"Telemetry: {path}\n")
    stage_table(days)

    errors = [e for events in days.values() for e in events if e.get("status") != "ok"]
    if errors:
        print("\nRecent errors:")
        for event in errors[-8:]:
            where = f" (after {event['failed_at']})" if event.get("failed_at") else ""
            print(f"  {event.get('timestamp','?')[:19]}  {event.get('error','?')}{where}")

    detail_summary(days)
    launches(events)

    if recaps:
        # What the recap waits for after a talk: its skills ("scored"), then
        # all of it, the one fix included.
        print()
        header = (
            f"{'day / build':17} {'recaps':>6} {'errors':>6} {'asked p50':>9} "
            f"{'scored p50/p95':>15} {'total p50/p95':>14} {'restored':>8} {'stopped':>7}"
        )
        print(header)
        print("-" * len(header))
        for day in sorted(recaps):
            events = recaps[day]
            ok = [e for e in events if e.get("status") == "ok"]
            asked = [e["since_close_ms"] for e in ok if e.get("since_close_ms") is not None]
            scored = [e["scored_ms"] for e in ok if e.get("scored_ms") is not None]
            total = [e["total_ms"] for e in ok if e.get("total_ms") is not None]
            judges = [judge for e in ok for judge in e.get("judges", [])]
            restored = sum(1 for judge in judges if judge.get("restored_tokens") is not None)
            stopped = sum(1 for judge in judges if judge.get("stopped_early"))
            print(
                f"{day:17} {len(events):>6} {len(events) - len(ok):>6} {fmt(pct(asked, 50)):>9} "
                f"{fmt(pct(scored, 50)):>7}/{fmt(pct(scored, 95)):>7} "
                f"{fmt(pct(total, 50)):>6}/{fmt(pct(total, 95)):>7} "
                f"{restored:>3}/{len(judges):<4} {stopped:>7}"
            )

    if "--turns" in flags:
        turn_rows(turns)

    if not arguments:
        failures = path.parent.parent / "stt-failures"
        if failures.is_dir():
            wavs = sorted(failures.glob("*.wav"))
            if wavs:
                print(f"\nCanary failure recordings: {len(wavs)} in {failures}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
