#!/usr/bin/env python3
"""How the week's collection went: what the batch record says about the
batches themselves, not about Ella's latency.

Reads the batch record that ingest keeps (latency-sanitize.py --validate
--data data/ --ledger ledger/): a line for every batch file it saw, with a
valid batch's envelope and outcome, or a refused file's problems. The weekly
report includes this as its "Collection" section; alone, it prints the same.

  npm run telemetry:health -- ledger/ [--week 2026-W43] [--json]

A batch counts in the week, Monday to Sunday in IST, it was received, not
the week of its events. A flag is a reason to look, as in the weekly report:
a refused file; a seq given to two batches; a gap in an install's seqs still
open 7 days after it was seen, now taken as lost; dropped events above 1% of
the events received; a field the policy has no rule for; totals that went
back or counts that do not add up; a clock the app or the record doubts.

Installs are named by the first 8 hex of their `install_id` only.
"""
import argparse
import importlib.util
import json
import re
from collections import Counter, defaultdict
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _sibling(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


sanitize = _sibling("latency-sanitize.py", "latency_sanitize")
pct = sanitize.report.pct
IST = sanitize.IST
REASONS = list(sanitize.POLICY["objects"]["dropped_by_reason"])

HEALTH_VERSION = 1

# A gap in an install's seqs is waited for this long, from when a later
# batch showed it, before it is taken as lost.
GAP_WAIT = timedelta(days=7)
# Dropped events, of the events received, past which the week is flagged.
MAX_DROP_RATE = 0.01
# An install that sent within this long before the week, and not in it.
SILENT_AFTER = timedelta(days=28)
# A batch whose last event is this far after it was received.
CLOCK_AHEAD = timedelta(days=1)


def week_bounds(label):
    year, week = label.split("-W")
    start = datetime.fromisocalendar(int(year), int(week), 1).replace(tzinfo=IST)
    return start, start + timedelta(days=7)


def at(text):
    return sanitize.when({"timestamp": text}) if isinstance(text, str) else None


def short(install_id):
    return (install_id or "?")[:8]


def stats(values):
    return {"n": len(values), "p50": pct(values, 50), "p95": pct(values, 95)}


def ratio(part, whole):
    return round(part / whole, 4) if whole else None


def gaps(batches, start, end):
    """Each install's missing seqs, as of `end`: pending while a later
    batch showed it less than GAP_WAIT ago, else lost; `lost_now` those
    whose wait ran out within the week."""
    shown = defaultdict(dict)  # install -> seq -> first received
    for record in batches:
        seqs = shown[record["install_id"]]
        seqs[record["batch_seq"]] = min(seqs.get(record["batch_seq"], record["_at"]), record["_at"])
    pending, lost_now, lost = [], [], 0
    for install, seqs in sorted(shown.items()):
        for missing in range(1, max(seqs)):
            if missing in seqs:
                continue
            seen = min(received for seq, received in seqs.items() if seq > missing)
            entry = {"install": short(install), "batch_seq": missing, "seen": seen.isoformat(timespec="seconds")}
            if end - seen <= GAP_WAIT:
                pending.append(entry)
            else:
                lost += 1
                if start <= seen + GAP_WAIT < end:
                    lost_now.append(entry)
    return pending, lost_now, lost


def consistency(batches, start, end):
    """Each install's totals never go back, and each batch's counts are
    its totals less its last seq's: checked where both are `new`, and
    shown in the week the later of them was received."""
    issues = []
    by_install = defaultdict(list)
    for record in batches:
        if record["outcome"] == "new":
            by_install[record["install_id"]].append(record)
    for install, records in sorted(by_install.items()):
        records.sort(key=lambda r: r["batch_seq"])
        for earlier, later in zip([None] + records, records):
            received = max(r["_at"] for r in (earlier, later) if r)
            if not start <= received < end:
                continue
            for count, total in (("dropped_event_count", "dropped_total"), ("log_resets", "log_resets_total")):
                now = later["body"][total]
                before = earlier["body"][total] if earlier else 0
                if earlier and now < before:
                    issues.append({"install": short(install), "batch_seq": later["batch_seq"], "check": f"{total}_went_back"})
                elif (not earlier and later["batch_seq"] == 1) or (earlier and earlier["batch_seq"] + 1 == later["batch_seq"]):
                    if later["body"][count] != now - before:
                        issues.append({"install": short(install), "batch_seq": later["batch_seq"], "check": f"{count}_does_not_add_up"})
    return issues


def summary(records, week):
    """The week's collection health, from the batch record."""
    start, end = week_bounds(week)
    upto = []
    for record in records:
        received = at(record.get("received_at"))
        if received is not None and received < end:
            upto.append({**record, "_at": received})
    this = [r for r in upto if start <= r["_at"]]
    batches = [r for r in this if r["record"] == "batch"]
    refused = [r for r in this if r["record"] == "refused"]
    received = [r for r in batches if r["outcome"] != "duplicate"]
    app = [r for r in received if r["origin"] == "app"]
    sequenced = [r for r in upto if r["record"] == "batch" and r["origin"] == "app" and r["outcome"] != "duplicate"]

    outcomes = Counter({name: 0 for name in ("new", "duplicate", "seq_conflict", "manual")})
    outcomes.update(r["outcome"] for r in batches)
    problems = Counter(re.sub(r"line \d+", "line N", p) for r in refused for p in r["problems"])

    events = sum(r["body"]["events"] for r in received)
    dropped = sum(r["body"]["dropped_event_count"] or 0 for r in app)
    by_reason = {name: sum((r["body"]["dropped_by_reason"] or {}).get(name) or 0 for r in app) for name in REASONS}

    reporting = {r["install_id"] for r in batches}
    lately = {
        r["install_id"] for r in upto
        if r["record"] == "batch" and start - SILENT_AFTER <= r["_at"] < start
    }
    pending, lost_now, lost = gaps(sequenced, start, end)

    clock = []
    for r in app:
        last = at(r["body"]["last_event_at"])
        if r["send"].get("clock_suspect") or (last and last - r["_at"] > CLOCK_AHEAD):
            clock.append({"install": short(r["install_id"]), "batch_seq": r["batch_seq"]})

    delays = [
        round((r["_at"] - last).total_seconds() * 1000)
        for r in app for last in [at(r["body"]["last_event_at"])] if last
    ]
    attempts = Counter()
    for r in app:
        tries = r["send"].get("attempts")
        attempts["3+" if tries and tries >= 3 else str(tries or "?")] += 1

    result = {
        "health_version": HEALTH_VERSION,
        "week": week,
        "batch_files": sum(outcomes.values()) + len(refused),
        "batches": {**outcomes, "refused": len(refused)},
        "refused_problems": dict(problems.most_common(10)),
        "installs": {
            "reporting": len(reporting),
            "silent": sorted(short(i) for i in lately - reporting),
        },
        "events_received": events,
        "events_new": sum(r.get("events_new") or 0 for r in batches),
        "sequence": {
            "pending_gaps": pending,
            "lost_this_week": lost_now,
            "lost_total": lost,
            "conflicts": [{"install": short(r["install_id"]), "batch_seq": r["batch_seq"]} for r in batches if r["outcome"] == "seq_conflict"],
        },
        "loss": {
            "dropped": dropped,
            "dropped_by_reason": by_reason,
            "drop_rate": ratio(dropped, events),
            "log_resets": sum(r["body"]["log_resets"] or 0 for r in app),
            "policy_excluded": sum(r["body"]["policy_excluded_count"] or 0 for r in received),
            "values_replaced": sum(r["body"]["values_replaced"] or 0 for r in received),
            "unknown_fields_dropped": sum(r["body"]["unknown_fields_dropped"] or 0 for r in received),
            "unmeasured_batches": len(received) - len(app),
        },
        "delivery": {
            "delay_ms": stats(delays),
            "attempts": dict(sorted(attempts.items())),
            "oldest_queued_age_ms": stats([r["send"]["oldest_queued_event_age_ms"] for r in app if r["send"].get("oldest_queued_event_age_ms") is not None]),
            "backlog_after_events_max": max((r["send"].get("backlog_after_events") or 0 for r in app), default=None),
            "clock_suspect": clock,
        },
        "app_versions": dict(Counter(r["body"]["app_version"] or "unknown" for r in received).most_common()),
        "consistency": consistency(sequenced, start, end),
    }
    result["flags"] = flags_of(result)
    return result


def flags_of(health):
    flags = []

    def flag(check, count, **extra):
        if count:
            flags.append({"check": check, "count": count, **extra})

    loss = health["loss"]
    flag("refused", health["batches"]["refused"])
    flag("seq_conflict", len(health["sequence"]["conflicts"]))
    flag("probably_lost", len(health["sequence"]["lost_this_week"]))
    if loss["dropped"] and (loss["drop_rate"] is None or loss["drop_rate"] > MAX_DROP_RATE):
        flag("dropped", loss["dropped"], drop_rate=loss["drop_rate"])
    flag("unknown_fields", loss["unknown_fields_dropped"])
    flag("inconsistent_counts", len(health["consistency"]))
    flag("clock_suspect", len(health["delivery"]["clock_suspect"]))
    return flags


FLAG_TEXT = {
    "refused": "{count} batch file(s) refused",
    "seq_conflict": "{count} seq(s) given to two batches",
    "probably_lost": "{count} batch(es) taken as lost: a gap open {days} days",
    "dropped": "{count} events dropped on the laptops ({rate} of those received)",
    "unknown_fields": "{count} field(s) the policy has no rule for: the app writes something new",
    "inconsistent_counts": "{count} batch(es) whose totals went back or counts do not add up",
    "clock_suspect": "{count} batch(es) with a clock in doubt",
}


def flag_line(flag):
    rate = flag.get("drop_rate")
    return "▲ " + FLAG_TEXT[flag["check"]].format(
        count=flag["count"], days=GAP_WAIT.days, rate="–" if rate is None else f"{rate:.1%}"
    )


def markdown_lines(health):
    """The "Collection" section, for the weekly report or alone."""
    lines = ["## Collection", ""]
    if health is None:
        return lines + ["No batch record given (`--ledger`): nothing to say about collection.", ""]
    lines.append("Batches by the week they were received. Flags are reasons to look, not faults found.")
    lines.append("")
    lines += [f"- {flag_line(flag)}" for flag in health["flags"]] or ["- Nothing flagged."]
    b, loss, d, s = health["batches"], health["loss"], health["delivery"], health["sequence"]

    def listed(counts):
        return ", ".join(f"{name} {count}" for name, count in counts.items()) or "none"

    def named(entries):
        return ", ".join(f"{e['install']}#{e['batch_seq']}" for e in entries) or "none"

    def ms(value):
        return "–" if value is None else f"{value:,.0f}"

    def hours(value):
        return "–" if value is None else f"{value / 3_600_000:,.1f}"

    def share(value):
        return "–" if value is None else f"{value:.1%}"

    checks = ", ".join(f"{e['install']}#{e['batch_seq']} {e['check']}" for e in health["consistency"])

    rows = [
        ("Batch files", health["batch_files"]),
        ("New / duplicate / manual", f"{b['new']} / {b['duplicate']} / {b['manual']}"),
        ("Seq conflicts", b["seq_conflict"]),
        ("Refused", b["refused"]),
        ("Installs reporting", health["installs"]["reporting"]),
        ("Events received (new)", f"{health['events_received']} ({health['events_new']})"),
        ("Dropped on the laptops", f"{loss['dropped']} ({share(loss['drop_rate'])})"),
        ("Log resets", loss["log_resets"]),
        ("Left behind by the policy (events / values / fields)",
         f"{loss['policy_excluded']} / {loss['values_replaced']} / {loss['unknown_fields_dropped']}"),
        ("Last event → received, P50 / P95 (hours)", f"{hours(d['delay_ms']['p50'])} / {hours(d['delay_ms']['p95'])}"),
        ("Oldest queued event when sent, P95 (hours)", hours(d["oldest_queued_age_ms"]["p95"])),
        ("Largest backlog left (events)", ms(d["backlog_after_events_max"])),
    ]
    lines += ["", "| | this week |", "|---|--:|"] + [f"| {name} | {value} |" for name, value in rows]
    lines += [
        "",
        f"- Dropped by reason: {listed(loss['dropped_by_reason'])}"
        + (f"; {loss['unmeasured_batches']} manual batch(es) do not measure it" if loss["unmeasured_batches"] else ""),
        f"- Sent at try: {', '.join(f'{tries}: {count}' for tries, count in d['attempts'].items()) or 'none'}",
        f"- Gaps pending (up to {GAP_WAIT.days} days): {named(s['pending_gaps'])}; "
        f"lost this week: {named(s['lost_this_week'])}; lost in all: {s['lost_total']}",
        f"- Seq conflicts: {named(s['conflicts'])}",
        f"- Counts: {checks or 'consistent'}",
        f"- Clock in doubt: {named(d['clock_suspect'])}",
        f"- Silent installs (sent in the {SILENT_AFTER.days} days before, not this week): {', '.join(health['installs']['silent']) or 'none'}",
        f"- Batches by build: {listed(health['app_versions'])}",
    ]
    if health["refused_problems"]:
        top = dict(list(health["refused_problems"].items())[:5])
        lines.append(f"- Refused for (the first 5): {listed(top)}")
    return lines + [""]


def read(paths):
    records = []
    for path in paths:
        records.extend(sanitize.read_ledger(path) if path.is_dir() else [
            r for r in sanitize.read_events(path) if isinstance(r, dict) and "record" in r
        ])
    return records


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("paths", nargs="+", type=Path, help="the batch record: its folder or files")
    parser.add_argument("--week", help="such as 2026-W43 (default: the last whole week, IST)")
    parser.add_argument("--json", action="store_true", help="print the JSON, not the Markdown")
    args = parser.parse_args(argv)
    week = args.week
    if not week:
        year, number, _ = (datetime.now(IST) - timedelta(days=7)).isocalendar()
        week = f"{year}-W{number:02d}"
    week_bounds(week)
    health = summary(read(args.paths), week)
    if args.json:
        print(json.dumps(health, indent=2))
    else:
        print(f"# Ella collection health: {week}\n")
        print("\n".join(markdown_lines(health)[2:]), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
