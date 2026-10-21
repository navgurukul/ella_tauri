#!/usr/bin/env python3
"""One week of Ella's latency, against the week before: the weekly report.

Reads sanitized events (scripts/latency-sanitize.py) from files or folders,
or a laptop's own latency.jsonl, and writes the week's KPIs as Markdown, for
people, and JSON, whose keys stay put for anything that reads them later.
Weeks run Monday to Sunday in IST; the default is the last whole one.

  npm run telemetry:weekly -- data/ [--week 2026-W43] [--out reports/]

Latency is over the voice turns that succeeded: P50/P95/P99, nearest rank,
as `telemetry:report` counts them. The headline is the wait, from the learner
stopping to Ella starting to speak (`speech_ms`). A flag is a triage signal,
a reason to look at the raw events, not a measured regression: a P50 or P95
at least 15% and 200 ms slower than the week before, with 30 or more turns in
each week; or a system error rate 2 points higher. "Didn't hear you"
(NO_SPEECH_DETECTED) is counted apart, not as a system error.

With --out, writes <week>.md and <week>.json there. Given the folder of past
reports (--reports, by default --out), the report says how many of last
week's events arrived after last week's report was written.
"""
import argparse
import importlib.util
import json
import os
from collections import Counter
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _sibling(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


sanitize = _sibling("latency-sanitize.py", "latency_sanitize")
pct = sanitize.report.pct
by_cloud = sanitize.report.by_cloud
IST = sanitize.IST

REPORT_VERSION = 1

# What a week must hold, and how far it must move, to be flagged.
MIN_N = 30
MIN_PCT = 0.15
MIN_MS = 200
MIN_ERROR_RISE = 0.02
# Shifts in what the turns were that move the numbers by themselves.
MIX_CLOUD_SHARE = 0.20

NO_SPEECH = "NO_SPEECH_DETECTED"

# Key, label. The keys are the JSON's, and stay put.
LATENCY = [
    ("wait_ms", "Wait: learner stops → Ella speaks"),
    ("wait_ms_local", "↳ answered on the laptop"),
    ("wait_ms_cloud", "↳ answered by the cloud"),
    ("total_ms", "Total turn"),
    ("total_ms_local", "↳ answered on the laptop"),
    ("total_ms_cloud", "↳ answered by the cloud"),
    ("stt_ms", "Speech to text"),
    ("llm_ttft_ms", "LLM first token"),
    ("llm_completion_ms", "LLM completion"),
    ("tts_first_audio_ms", "TTS first audio"),
    ("recap_total_ms", "Recap total"),
    ("cold_start_ms", "Cold start: launch → engine ready"),
]


def week_bounds(label):
    year, week = label.split("-W")
    start = datetime.fromisocalendar(int(year), int(week), 1).replace(tzinfo=IST)
    return start, start + timedelta(days=7)


def label_of(at):
    year, week, _ = at.astimezone(IST).isocalendar()
    return f"{year}-W{week:02d}"


def last_whole_week(now):
    return label_of(now - timedelta(days=7))


def read_all(paths):
    """Every event, once, whichever files it came in."""
    files = []
    for path in paths:
        files.extend(sorted(path.rglob("*.jsonl")) if path.is_dir() else [path])
    events = {}
    for path in files:
        for event in sanitize.read_events(path):
            events.setdefault(sanitize.event_id(event), event)
    return list(events.values())


def code(event):
    return event.get("error_code") or sanitize.error_code(event.get("error"))


def talk(event):
    return event.get("talk_id") or event.get("session_id")


def launch(event):
    return event.get("launch") or event.get("launch_id")


def stats(values):
    return {"n": len(values), "p50": pct(values, 50), "p95": pct(values, 95), "p99": pct(values, 99)}


def ratio(part, whole):
    return round(part / whole, 4) if whole else None


def summarize(events):
    turns = [e for e in events if e.get("event") == "ella_turn_latency"]
    voice = [e for e in turns if e.get("kind", "voice") == "voice"]
    ok = [e for e in voice if e.get("status") == "ok"]
    errors = [e for e in turns if e.get("status") != "ok"]
    no_speech = [e for e in errors if code(e) == NO_SPEECH]
    system = [e for e in errors if code(e) != NO_SPEECH]
    cloud = [e for e in ok if by_cloud(e)]
    local = [e for e in ok if not by_cloud(e)]
    recaps = [e for e in events if e.get("event") == "ella_assessment"]
    engines = [e for e in events if e.get("event") == "ella_engine" and not e.get("error") and not e.get("error_code")]

    def wait(e):
        # A log from before `speech_ms` was kept: a reply played whole starts
        # at the total anyway, as telemetry:report counts it.
        return e.get("speech_ms", e.get("total_ms"))

    def series(rows, field):
        return [field(e) if callable(field) else e.get(field) for e in rows]

    def values(rows, field):
        return [v for v in series(rows, field) if v is not None]

    latency = {
        "wait_ms": stats(values(ok, wait)),
        "wait_ms_local": stats(values(local, wait)),
        "wait_ms_cloud": stats(values(cloud, wait)),
        "total_ms": stats(values(ok, "total_ms")),
        "total_ms_local": stats(values(local, "total_ms")),
        "total_ms_cloud": stats(values(cloud, "total_ms")),
        "stt_ms": stats(values(ok, "stt_ms")),
        "llm_ttft_ms": stats(values(ok, "llm_ttft_ms")),
        "llm_completion_ms": stats(values(ok, "llm_completion_ms")),
        "tts_first_audio_ms": stats(values(ok, "tts_first_audio_ms")),
        "recap_total_ms": stats(values([e for e in recaps if e.get("status") == "ok"], "total_ms")),
        "cold_start_ms": stats(values(engines, "since_launch_ms")),
    }
    volume = {
        "events": len(events),
        "turns": len(turns),
        "voice_turns": len(voice),
        "ok_turns": len([e for e in turns if e.get("status") == "ok"]),
        "errors": len(errors),
        "no_speech": len(no_speech),
        "system_errors": len(system),
        "system_error_rate": ratio(len(system), len(turns)),
        "no_speech_rate": ratio(len(no_speech), len(turns)),
        "system_errors_by_stage": dict(Counter(e.get("failed_at") or "?" for e in system).most_common()),
        "top_error_codes": dict(Counter(code(e) or "?" for e in errors).most_common(5)),
        "talks": len({talk(e) for e in turns if talk(e)}),
        "launches": len({launch(e) for e in events if launch(e)}),
        "laptops": len({e.get("source") for e in events if e.get("source")}),
        "app_versions": dict(Counter(e.get("app_version") or "?" for e in turns).most_common()),
        "cloud_turns": len(cloud),
        "cloud_share": ratio(len(cloud), len(ok)),
        "cloud_failed_runs": sum(
            1 for e in turns for run in e.get("llm_runs") or [] if run.get("backend") == "cloud" and run.get("failed")
        ),
        "stt_fallbacks": sum(
            1 for e in ok if e.get("stt_fallback_from") or e.get("stt_engine") == "whisper-small"
        ),
        "recaps": len(recaps),
        "recap_errors": sum(1 for e in recaps if e.get("status") != "ok"),
        "failed_judges": sum(1 for e in recaps for j in e.get("judges") or [] if j.get("status") == "error"),
    }
    return {"volume": volume, "latency": latency}


def flags_of(this, last):
    flags = []
    for key, _ in LATENCY:
        now, before = this["latency"][key], last["latency"][key]
        if now["n"] < MIN_N or before["n"] < MIN_N:
            continue
        for stat in ("p50", "p95"):
            if not before[stat]:
                continue
            delta = now[stat] - before[stat]
            if delta >= MIN_MS and delta / before[stat] >= MIN_PCT:
                flags.append({
                    "kpi": key, "stat": stat, "this": now[stat], "previous": before[stat],
                    "delta_ms": delta, "delta_pct": round(delta / before[stat], 4),
                })
    now, before = this["volume"], last["volume"]
    if (
        now["turns"] >= MIN_N
        and before["turns"] >= MIN_N
        and now["system_error_rate"] - before["system_error_rate"] >= MIN_ERROR_RISE
    ):
        flags.append({
            "kpi": "system_error_rate", "this": now["system_error_rate"],
            "previous": before["system_error_rate"],
            "delta_pp": round((now["system_error_rate"] - before["system_error_rate"]) * 100, 2),
        })
    return flags


def notes_of(this, last, late):
    notes = []
    now, before = this["volume"], last["volume"]
    if now["cloud_share"] is not None and before["cloud_share"] is not None:
        if abs(now["cloud_share"] - before["cloud_share"]) > MIX_CLOUD_SHARE:
            notes.append(
                f"Mix changed: the cloud answered {now['cloud_share']:.0%} of turns, "
                f"against {before['cloud_share']:.0%} the week before."
            )
    def top(versions):
        return max(versions, key=versions.get) if versions else None

    if top(now["app_versions"]) and top(before["app_versions"]) and top(now["app_versions"]) != top(before["app_versions"]):
        notes.append(
            f"Mix changed: most turns ran {top(now['app_versions'])}, "
            f"against {top(before['app_versions'])} the week before."
        )
    if late:
        notes.append(f"{late} of last week's events arrived after its report; last week is counted with them here.")
    if not now["turns"]:
        notes.append("No turns this week.")
    return notes


def build(events, week, previous_report=None, tools_ref=None, now=None):
    start, end = week_bounds(week)
    previous = label_of(start - timedelta(days=7))
    this = summarize([e for e in events if sanitize.week_of(e) == week])
    last = summarize([e for e in events if sanitize.week_of(e) == previous])
    late = None
    if previous_report:
        late = last["volume"]["events"] - previous_report["this"]["volume"]["events"]
    stamps = [(sanitize.when(e), e["timestamp"]) for e in events if sanitize.when(e)]
    return {
        "report_version": REPORT_VERSION,
        "week": week,
        "week_start": start.isoformat(),
        "week_end": end.isoformat(),
        "timezone": "Asia/Kolkata (UTC+05:30)",
        "previous_week": previous,
        "generated_at": (now or datetime.now(timezone.utc)).isoformat(timespec="seconds"),
        "tools_ref": tools_ref,
        "data_through": max(stamps)[1] if stamps else None,
        "thresholds": {
            "min_n": MIN_N, "min_pct": MIN_PCT, "min_ms": MIN_MS, "min_error_rise": MIN_ERROR_RISE,
        },
        "this": this,
        "previous": last,
        "late_events_previous_week": late if late and late > 0 else 0,
        "flags": flags_of(this, last),
        "notes": notes_of(this, last, late if late and late > 0 else 0),
    }


def ms(value, n=None, rare=None):
    if value is None:
        return "–"
    return f"{value:,.0f}" + ("~" if n is not None and rare is not None and n < rare else "")


def delta(now, before, stat):
    if now["n"] < MIN_N or before["n"] < MIN_N:
        return "n<30"
    if now[stat] is None or not before[stat]:
        return "–"
    change = now[stat] - before[stat]
    flagged = change >= MIN_MS and change / before[stat] >= MIN_PCT
    return f"{change:+,.0f} ({change / before[stat]:+.0%})" + (" ▲" if flagged else "")


def share(value):
    return "–" if value is None else f"{value:.1%}"


def markdown(report):
    start = datetime.fromisoformat(report["week_start"])
    end = datetime.fromisoformat(report["week_end"]) - timedelta(days=1)
    this, last = report["this"], report["previous"]
    lines = [
        f"# Ella latency report: {report['week']}",
        "",
        f"{start:%a %d %b} – {end:%a %d %b %Y} (IST), against {report['previous_week']}. "
        f"Data through {report['data_through'] or '–'}; generated {report['generated_at']}"
        + (f" by tools at `{report['tools_ref']}`" if report["tools_ref"] else "")
        + ".",
        "",
    ]
    flags = report["flags"]
    if flags:
        lines.append(f"**Triage: {len(flags)} flagged.** Flags are reasons to look at the raw events, not measured regressions.")
        lines.append("")
        labels = dict(LATENCY)
        for flag in flags:
            if flag["kpi"] == "system_error_rate":
                lines.append(
                    f"- ▲ System error rate {share(flag['this'])}, against {share(flag['previous'])} "
                    f"(+{flag['delta_pp']} points)"
                )
            else:
                lines.append(
                    f"- ▲ {labels[flag['kpi']].lstrip('↳ ')} ({flag['kpi']}) {flag['stat'].upper()} "
                    f"{flag['this']:,.0f} ms, against {flag['previous']:,.0f} ms "
                    f"({flag['delta_ms']:+,.0f} ms, {flag['delta_pct']:+.0%})"
                )
    else:
        lines.append("**Triage: nothing flagged.** Flags are reasons to look at the raw events, not measured regressions.")
    lines += [
        "",
        "## Latency (ms, voice turns that succeeded)",
        "",
        "| | n | P50 | P95 | P99 | last P50 | last P95 | Δ P50 | Δ P95 |",
        "|---|--:|--:|--:|--:|--:|--:|--:|--:|",
    ]
    for key, label in LATENCY:
        now, before = this["latency"][key], last["latency"][key]
        lines.append(
            f"| {label} | {now['n']} | {ms(now['p50'])} | {ms(now['p95'], now['n'], 20)} | "
            f"{ms(now['p99'], now['n'], 100)} | {ms(before['p50'])} | {ms(before['p95'], before['n'], 20)} | "
            f"{delta(now, before, 'p50')} | {delta(now, before, 'p95')} |"
        )
    v, w = this["volume"], last["volume"]
    rows = [
        ("Turns", v["turns"], w["turns"]),
        ("Talks", v["talks"], w["talks"]),
        ("Launches", v["launches"], w["launches"]),
        ("Laptops", v["laptops"], w["laptops"]),
        ("Succeeded", v["ok_turns"], w["ok_turns"]),
        ("System error rate", share(v["system_error_rate"]), share(w["system_error_rate"])),
        ("No-speech rate", share(v["no_speech_rate"]), share(w["no_speech_rate"])),
        ("Cloud answered", share(v["cloud_share"]), share(w["cloud_share"])),
        ("Cloud runs that failed", v["cloud_failed_runs"], w["cloud_failed_runs"]),
        ("STT fallbacks", v["stt_fallbacks"], w["stt_fallbacks"]),
        ("Recaps", v["recaps"], w["recaps"]),
        ("Recap errors", v["recap_errors"], w["recap_errors"]),
        ("Failed judges", v["failed_judges"], w["failed_judges"]),
    ]
    lines += ["", "## Volume and errors", "", "| | this week | last week |", "|---|--:|--:|"]
    lines += [f"| {name} | {now} | {before} |" for name, now, before in rows]
    lines.append("")

    def listed(counts):
        return ", ".join(f"{name} {count}" for name, count in counts.items()) or "none"

    lines.append(f"- System errors by stage: {listed(v['system_errors_by_stage'])}")
    lines.append(f"- Error codes: {listed(v['top_error_codes'])}")
    lines.append(f"- Turns by build: {listed(v['app_versions'])}")
    if report["notes"]:
        lines += ["", "## Notes", ""] + [f"- {note}" for note in report["notes"]]
    lines += [
        "",
        f"`~`: P95 of fewer than 20, or P99 of fewer than 100, values: close to the slowest. "
        f"▲: at least {MIN_PCT:.0%} and {MIN_MS} ms slower, with {MIN_N}+ turns in both weeks. "
        "Percentiles are nearest-rank.",
    ]
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--week", help="such as 2026-W43 (default: the last whole week, IST)")
    parser.add_argument("--out", type=Path, help="write <week>.md and <week>.json here")
    parser.add_argument("--reports", type=Path, help="past reports, for late events (default: --out)")
    parser.add_argument("--tools-ref", default=os.environ.get("ELLA_TOOLS_REF"), help="the scripts' commit")
    args = parser.parse_args(argv)

    week = args.week or last_whole_week(datetime.now(IST))
    week_bounds(week)  # a malformed week fails here, before anything is read
    reports = args.reports or args.out
    previous_report = None
    if reports:
        past = reports / f"{label_of(week_bounds(week)[0] - timedelta(days=7))}.json"
        if past.exists():
            previous_report = json.loads(past.read_text(encoding="utf-8"))

    result = build(read_all(args.paths), week, previous_report, args.tools_ref)
    text = markdown(result)
    if args.out:
        args.out.mkdir(parents=True, exist_ok=True)
        (args.out / f"{week}.md").write_text(text, encoding="utf-8")
        (args.out / f"{week}.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
