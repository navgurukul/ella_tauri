#!/usr/bin/env python3
"""Make a laptop's latency.jsonl fit to leave it, for the weekly latency report.

Keeps only what scripts/telemetry-fields.json allows: an allowlist, so a field
the app starts writing stays behind until someone names it there. Free-text
errors and the cloud's reasons become codes, and the IDs that would tie an
event to the laptop's own database are hashed. Every event gets an `event_id` that is the same whichever
upload it came in, so the whole cumulative file can be uploaded every week and
nothing is counted twice; and a `source`, the laptop's label, from the file's
name (`lab-win-hp-02.jsonl`), never a person's.

  npm run telemetry:sanitize -- <label>.jsonl...                  # to stdout
  npm run telemetry:sanitize -- <label>.jsonl... --data data/     # ingest

With --data, only events not already under data/ are written, to
data/<week>/<ingest-id>.jsonl: one new file per ingest, never a changed one.
Weeks run Monday to Sunday in IST. The last line printed says what was done.
"""
import argparse
import hashlib
import importlib.util
import json
import re
import sys
from collections import Counter, defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIELDS = HERE / "telemetry-fields.json"

# The team and its testers are in India, which keeps no daylight saving.
IST = timezone(timedelta(hours=5, minutes=30), "IST")

# A laptop's label: what the uploaded file is named.
LABEL = re.compile(r"^[a-z0-9][a-z0-9-]{0,39}$")

# NO_SPEECH_DETECTED: I couldn't hear any words in that recording...
CODE = re.compile(r"^([A-Z][A-Z0-9_]{2,}):")

# Why the cloud went down: `HTTP 503`, or a proxy's `cap: <its message>`.
HTTP = re.compile(r"^HTTP ([1-5][0-9]{2})(?::|$)")
PROXY = re.compile(r"^([a-z_]+)(?::|$)")

# What a sanitized event carries that the app's does not.
ADDED = ("event_id", "source")


def _latency_report():
    """scripts/latency-report.py, to read files the way it does: UTF-16 from
    PowerShell included, and a line cut off mid-write skipped."""
    spec = importlib.util.spec_from_file_location("latency_report", HERE / "latency-report.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


report = _latency_report()
read_events = report.read_events
when = report.when


def short_hash(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


def error_code(text):
    """The word before the colon of an error the app names, else `other`."""
    if not text:
        return None
    match = CODE.match(str(text))
    return match.group(1) if match else "other"


def reason_code(text, reasons):
    """Why the cloud went down, as one of `cloud_reasons` in
    telemetry-fields.json, else `other`: never the text, which can be a
    server's own words."""
    if not text:
        return None
    text = str(text)
    if text in reasons["exact"]:
        return reasons["exact"][text]
    for prefix, code in reasons["prefix"].items():
        if text.startswith(prefix):
            return code
    match = HTTP.match(text)
    if match:
        return f"http_{match.group(1)}"
    match = PROXY.match(text)
    if match and match.group(1) in reasons["proxy"]:
        return f"proxy_{match.group(1)}"
    return "other"


def event_id(event):
    """The same event, from whichever upload, gets the same id. An event of
    a build too old to name its launch is known by all of itself."""
    if event.get("event_id"):
        return event["event_id"]
    if event.get("launch_id"):
        key = "|".join(
            str(event.get(name, "")) for name in ("launch_id", "timestamp", "event", "interaction_id")
        )
    else:
        key = json.dumps(event, sort_keys=True, separators=(",", ":"))
    return short_hash(key)


def week_of(event):
    """The event's week, Monday to Sunday in IST, as `2026-W43`."""
    at = when(event)
    if at is None:
        return None
    year, week, _ = at.astimezone(IST).isocalendar()
    return f"{year}-W{week:02d}"


class Sanitizer:
    def __init__(self, fields):
        self.events = fields["events"]
        self.every_event = fields["every_event"]
        self.hashed = fields["hashed"]
        self.coded = fields["coded"]
        self.objects = fields["objects"]
        reasons = fields["cloud_reasons"]
        self.coders = {"error": error_code, "reason": lambda text: reason_code(text, reasons)}
        # A sanitized event fed back in is left as it is.
        self.outputs = set(self.hashed.values()) | set(self.coded.values()) | set(ADDED)
        self.dropped = Counter()

    def event(self, raw, source):
        """`raw` as it may leave the laptop, or None for a kind of event the
        allowlist does not name."""
        kind = raw.get("event")
        allowed = self.events.get(kind)
        if allowed is None:
            self.dropped[f"{kind} (whole event)"] += 1
            return None
        kept = self._keep(raw, set(self.every_event) | set(allowed), kind)
        return {"event_id": event_id(raw), **kept, "source": raw.get("source") or source}

    def _keep(self, value, names, where):
        kept = {}
        for key, item in value.items():
            if key in ADDED:
                continue
            if key not in names and key not in self.outputs:
                self.dropped[f"{where}.{key}"] += 1
            elif key in self.hashed:
                if item is not None:
                    kept[self.hashed[key]] = short_hash(str(item))
            elif key in self.coded:
                code = self.coders[key](item)
                if code:
                    kept[self.coded[key]] = code
            elif key in self.objects:
                kept[key] = self._nested(item, key, f"{where}.{key}")
            else:
                kept[key] = item
        return kept

    def _nested(self, item, key, where):
        names = set(self.objects[key])
        if isinstance(item, dict):
            return self._keep(item, names, where)
        if isinstance(item, list):
            return [self._keep(part, names, where) if isinstance(part, dict) else part for part in item]
        return item


def label_of(path, given):
    label = given or Path(path).stem
    if not LABEL.match(label) or label == "latency":
        raise SystemExit(
            f"{path}: name the file after its laptop, such as lab-win-hp-02.jsonl "
            "(lowercase letters, digits and dashes; never a person's name), or pass --source."
        )
    return label


def known_ids(data, weeks):
    ids = set()
    for week in weeks:
        for path in sorted((data / week).glob("*.jsonl")):
            ids.update(event_id(event) for event in read_events(path))
    return ids


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("files", nargs="+", type=Path)
    parser.add_argument("--source", help="the laptop's label, for a single file not named after it")
    parser.add_argument("--data", type=Path, help="ingest into this folder instead of printing")
    parser.add_argument("--ingest-id", help="the new files' name (default: the time, UTC)")
    args = parser.parse_args(argv)
    if args.source and len(args.files) > 1:
        parser.error("--source names one file's laptop; name several files after theirs")

    sanitizer = Sanitizer(json.loads(FIELDS.read_text(encoding="utf-8")))
    clean = {}
    read = Counter()
    untimed = 0
    for path in args.files:
        source = label_of(path, args.source)
        for raw in read_events(path):
            read[source] += 1
            event = sanitizer.event(raw, source)
            if event is None:
                continue
            if week_of(event) is None:
                untimed += 1
                continue
            clean.setdefault(event["event_id"], event)

    by_week = defaultdict(list)
    for event in clean.values():
        by_week[week_of(event)].append(event)
    already = known_ids(args.data, by_week) if args.data else set()
    fresh = {week: [e for e in events if e["event_id"] not in already] for week, events in by_week.items()}
    new = sum(len(events) for events in fresh.values())

    if args.data:
        ingest = args.ingest_id or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        for week, events in sorted(fresh.items()):
            if not events:
                continue
            target = args.data / week / f"{ingest}.jsonl"
            if target.exists():
                raise SystemExit(f"{target} exists: every ingest writes files of its own")
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open("w", encoding="utf-8") as out:
                for event in sorted(events, key=lambda e: e.get("timestamp", "")):
                    out.write(json.dumps(event, separators=(",", ":")) + "\n")
            print(f"wrote {len(events)} events to {target}", file=sys.stderr)
    else:
        for events in fresh.values():
            for event in sorted(events, key=lambda e: e.get("timestamp", "")):
                print(json.dumps(event, separators=(",", ":")))

    for source, count in sorted(read.items()):
        print(f"{source}: {count} lines read", file=sys.stderr)
    if untimed:
        print(f"{untimed} events without a readable timestamp left out", file=sys.stderr)
    if sanitizer.dropped:
        print("left behind (not in telemetry-fields.json):", file=sys.stderr)
        for field, count in sanitizer.dropped.most_common():
            print(f"  {count:>6} x {field}", file=sys.stderr)
    print(
        f"{new} new events from {', '.join(sorted(read)) or 'nothing'} "
        f"({len(clean) - new} already had)",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
