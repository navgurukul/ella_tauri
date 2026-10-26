#!/usr/bin/env python3
"""Make a laptop's latency.jsonl fit to leave it, and check what arrives.

Applies the telemetry policy, scripts/telemetry-fields.json: every field that
may leave has a rule, and a field without one stays behind. A value that
breaks its rule leaves as `other` (a number not at all); errors, and why the
cloud went down, leave as one of a fixed set of codes; the IDs that would tie
an event to the laptop's own database are hashed. Every event gets an
`event_id`, the same whichever upload it came in, so nothing is counted twice.

  npm run telemetry:sanitize -- <label>.jsonl... --batch-out out/  # batches
  npm run telemetry:sanitize -- --validate out/*.jsonl              # check them
  npm run telemetry:sanitize -- <label>.jsonl...                    # to stdout

--batch-out writes each file as one batch, `out/<batch_id>.jsonl`: the
envelope the policy's `batch` defines, then the events. A laptop's label is
its file's name (`lab-win-hp-02.jsonl`), never a person's, and the batch's
`install_id` a hash of it. --validate checks batches as strictly as the team's
store will: any field, value or count the policy does not allow is a problem,
named without its value. --unsent accepts an app's batch not yet sent.

Before batches, the Stage-1 store took events stamped with their laptop
(`source`): to stdout, or with --data, the events not already under data/,
to data/<week>/<ingest-id>.jsonl. Weeks run Monday to Sunday in IST.
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
POLICY = json.loads(FIELDS.read_text(encoding="utf-8"))
PATTERNS = {name: re.compile(pattern) for name, pattern in POLICY["patterns"].items()}
BODY = POLICY["batch"]["body"]
SEND = POLICY["batch"]["send"]

# The team and its testers are in India, which keeps no daylight saving.
IST = timezone(timedelta(hours=5, minutes=30), "IST")

# A laptop's label: what the uploaded file is named.
LABEL = re.compile(r"^[a-z0-9][a-z0-9-]{0,39}$")

# What a value that breaks its rule leaves as, when it is text.
OTHER = "other"
# Rules whose values must hold exactly: `other` is no hash, time or batch.
EXACT = {"hex16", "hex32", "timestamp"}
# Enums that are never `other`.
CLOSED = {"host", "origin"}

# Why the cloud went down: `HTTP 503`, or a proxy's `cap: <its message>`.
HTTP = re.compile(r"^HTTP ([1-5][0-9]{2})(?::|$)")
PROXY = re.compile(r"^([a-z_]+)(?::|$)")
HTTP_CODE = re.compile(r"^http_[1-5][0-9]{2}$")

# Taken from the event itself, never from its fields: a Stage-1 file's
# `source` included.
IGNORED = {"event_id", "source"}

_DROP = object()


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
    """The code of an error the app names before its own words, else
    `other`: never the words."""
    if not text:
        return None
    text = str(text)
    for prefix, code in POLICY["coders"]["error"]["prefixes"].items():
        if text.startswith(prefix):
            return code
    return OTHER


def reason_code(text):
    """Why the cloud went down, as one of the reason coder's codes, else
    `other`: never the text, which can be a server's own words."""
    if not text:
        return None
    reasons = POLICY["coders"]["reason"]
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
    return OTHER


CODERS = {"error": error_code, "reason": reason_code}


def code_allowed(coder, code):
    if code == OTHER:
        return True
    spec = POLICY["coders"][coder]
    if coder == "error":
        return code in spec["prefixes"].values()
    return (
        code in spec["exact"].values()
        or code in spec["prefix"].values()
        or bool(HTTP_CODE.match(code))
        or (code.startswith("proxy_") and code[len("proxy_"):] in spec["proxy"])
    )


def output_name(name, rule):
    kind, _, arg = rule.partition(":")
    if kind == "hash":
        return arg
    if kind == "code":
        return POLICY["coders"][arg]["output"]
    return name


def output_rule(rule):
    """The rule a field's value keeps once it has left."""
    return "hex16" if rule.startswith("hash:") else rule


def printable(text):
    return 0 < len(text) <= POLICY["hardware_max"] and text.isprintable()


def check(value, rule, path, problems):
    """Whether `value`, as it leaves, keeps `rule`; problems name the field
    and the rule, never the value."""
    if value is None:
        return

    def broke():
        problems.append(f"{path}: {type(value).__name__} breaks `{rule}`")

    kind, _, arg = rule.partition(":")
    if kind == "num":
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            broke()
    elif kind == "int":
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            broke()
    elif kind == "bool":
        if not isinstance(value, bool):
            broke()
    elif kind in PATTERNS:
        if not isinstance(value, str) or not (
            PATTERNS[kind].match(value) or (value == OTHER and kind not in EXACT)
        ):
            broke()
    elif kind == "hardware":
        if not isinstance(value, str) or not printable(value):
            broke()
    elif kind == "enum":
        if value not in POLICY["enums"][arg] and (value != OTHER or arg in CLOSED):
            broke()
    elif kind == "host":
        if value not in POLICY["enums"]["host"]:
            broke()
    elif kind == "code":
        if not isinstance(value, str) or not code_allowed(arg, value):
            broke()
    elif kind == "object":
        if isinstance(value, dict):
            check_fields(value, POLICY["objects"][arg], path, problems)
        else:
            broke()
    elif kind == "list":
        if isinstance(value, list):
            for item in value:
                check(item, arg, f"{path}[]", problems)
        else:
            broke()
    else:
        raise ValueError(f"no such rule: {rule}")


def check_fields(value, schema, path, problems):
    allowed = {output_name(name, rule): output_rule(rule) for name, rule in schema.items()}
    for key, item in value.items():
        if key not in allowed:
            problems.append(f"{path}.{key}: not in the policy")
        else:
            check(item, allowed[key], f"{path}.{key}", problems)


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


def schema_of(kind):
    spec = POLICY["events"][kind]
    return {**POLICY["every_event"], **spec["fields"]}


class Sanitizer:
    """Applies the policy, and counts what it left behind: `excluded`, whole
    events; `unknown`, fields it has no rule for; `replaced`, values that
    broke their rule."""

    def __init__(self):
        self.excluded = Counter()
        self.unknown = Counter()
        self.replaced = Counter()

    def event(self, raw, source=None):
        """`raw` as it may leave the laptop, or None. `source`, a laptop's
        label, is only for the Stage-1 store's files."""
        kind = raw.get("event")
        spec = POLICY["events"].get(kind)
        if spec is None:
            self.excluded[str(kind)] += 1
            return None
        stamp = raw.get("timestamp")
        if not isinstance(stamp, str) or not PATTERNS["timestamp"].match(stamp) or when(raw) is None:
            self.excluded[f"{kind} without a time"] += 1
            return None
        skip = IGNORED | set(spec.get("excluded", ()))
        kept = self._fields(raw, schema_of(kind), skip, kind)
        event = {"event_id": event_id(raw), **kept}
        if source:
            event["source"] = raw.get("source") or source
        return event

    def counts(self):
        return {
            "policy_excluded_count": sum(self.excluded.values()),
            "values_replaced": sum(self.replaced.values()),
            "unknown_fields_dropped": sum(self.unknown.values()),
        }

    def _fields(self, value, schema, skip, path):
        # A field already in the form it leaves in, from a sanitized file.
        left = {output_name(name, rule): output_rule(rule) for name, rule in schema.items()}
        kept = {}
        for key, item in value.items():
            if key in skip:
                continue
            if key in schema:
                name, result = output_name(key, schema[key]), self._value(item, schema[key], f"{path}.{key}")
            elif key in left:
                name, result = key, self._value(item, left[key], f"{path}.{key}")
            else:
                self.unknown[f"{path}.{key}"] += 1
                continue
            if result is not _DROP:
                kept[name] = result
        return kept

    def _value(self, item, rule, path):
        if item is None:
            return None
        kind, _, arg = rule.partition(":")
        if kind == "hash":
            return short_hash(str(item))
        if kind == "code" and not (isinstance(item, str) and code_allowed(arg, item)):
            # An empty error or reason is no error: left out, as the app does.
            code = CODERS[arg](item)
            return _DROP if code is None else code
        if kind == "host" and item not in POLICY["enums"]["host"]:
            return "default" if item in POLICY["hosts"]["default"] else "custom"
        if kind == "object" and isinstance(item, dict):
            return self._fields(item, POLICY["objects"][arg], IGNORED, path)
        if kind == "list" and isinstance(item, list):
            values = (self._value(part, arg, path) for part in item)
            return [value for value in values if value is not _DROP]
        if kind not in ("object", "list"):
            problems = []
            check(item, rule, path, problems)
            if not problems:
                return item
        self.replaced[path] += 1
        textual = kind in PATTERNS or kind in ("hardware", "enum", "code", "host")
        return OTHER if textual and kind not in EXACT else _DROP


def validate_event(event, path="event"):
    problems = []
    kind = event.get("event")
    if kind not in POLICY["events"]:
        return [f"{path}: no such event in the policy"]
    path = f"{path}:{kind}"
    if not isinstance(event.get("event_id"), str) or not PATTERNS["hex16"].match(event["event_id"]):
        problems.append(f"{path}.event_id: missing or not `hex16`")
    if not isinstance(event.get("timestamp"), str) or not PATTERNS["timestamp"].match(event["timestamp"]):
        problems.append(f"{path}.timestamp: missing or not `timestamp`")
    rest = {key: value for key, value in event.items() if key != "event_id"}
    check_fields(rest, schema_of(kind), path, problems)
    return problems


# Batches.


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def event_lines(events):
    """A batch's events as they are hashed and sent: one line each."""
    return "".join(json.dumps(event, separators=(",", ":"), ensure_ascii=False) + "\n" for event in events).encode(
        "utf-8"
    )


def batch_id_of(body, lines):
    return hashlib.sha256(canonical(body).encode("utf-8") + b"\n" + lines).hexdigest()[:32]


def batch_bytes(body, lines, send):
    envelope = {"event": "ella_batch", **body, "batch_id": batch_id_of(body, lines), **send}
    return (json.dumps(envelope, separators=(",", ":"), ensure_ascii=False) + "\n").encode("utf-8") + lines


def talks_in(events):
    return len({event["talk_id"] for event in events if event.get("talk_id")})


def span_of(events):
    times = sorted((when(event), event["timestamp"]) for event in events)
    return (times[0][1], times[-1][1]) if times else (None, None)


def manual_batch(events, label, counts, now=None):
    """A laptop's file as one batch: `origin` manual, sequence 0, and
    nothing the app measures, which a file does not know."""
    events = sorted(events, key=when)
    first, last = span_of(events)
    body = {name: None for name in BODY}
    body.update(
        policy_version=POLICY["policy_version"],
        install_id=short_hash(f"manual:{label}"),
        batch_seq=0,
        origin="manual",
        events=len(events),
        talks=talks_in(events),
        first_event_at=first,
        last_event_at=last,
        **counts,
    )
    send = {name: None for name in SEND}
    send["prepared_at"] = (now or datetime.now(timezone.utc)).isoformat(timespec="seconds")
    return batch_bytes(body, event_lines(events), send)


def validate_batch(data, sent=True):
    """Problems with one batch file's bytes, or none. `sent`: an app's batch
    must say how its sending went."""
    head, _, lines = data.partition(b"\n")
    try:
        envelope = json.loads(head)
    except ValueError:
        return ["envelope: not JSON"]
    if not isinstance(envelope, dict) or envelope.get("event") != "ella_batch":
        return ["envelope: not an `ella_batch`"]
    problems = []
    for key in envelope:
        if key not in BODY and key not in SEND and key not in ("event", "batch_id"):
            problems.append(f"envelope.{key}: not in the policy")
    for name, rule in {**BODY, **SEND, "batch_id": "hex32"}.items():
        if name not in envelope:
            problems.append(f"envelope.{name}: missing")
        else:
            check(envelope[name], rule, f"envelope.{name}", problems)
    if problems:
        return problems

    body = {name: envelope[name] for name in BODY}
    if body["policy_version"] != POLICY["policy_version"]:
        problems.append(f"envelope.policy_version: not {POLICY['policy_version']}")
    manual = body["origin"] == "manual"
    if manual != (body["batch_seq"] == 0):
        problems.append("envelope.batch_seq: 0 exactly when the batch is manual")
    for name in POLICY["batch"]["measured_by_app"]:
        if manual and body[name] is not None:
            problems.append(f"envelope.{name}: a manual batch cannot know it")
        if not manual and body[name] is None:
            problems.append(f"envelope.{name}: missing from an app's batch")
    if not manual and sent:
        for name in POLICY["batch"]["filled_when_sent"]:
            if envelope[name] is None:
                problems.append(f"envelope.{name}: missing from a sent batch")
    if body["dropped_by_reason"] is not None and body["dropped_event_count"] is not None:
        if sum(body["dropped_by_reason"].get(name) or 0 for name in POLICY["objects"]["dropped_by_reason"]) != body[
            "dropped_event_count"
        ]:
            problems.append("envelope.dropped_event_count: not the sum of dropped_by_reason")
    for count, total in (("dropped_event_count", "dropped_total"), ("log_resets", "log_resets_total")):
        if body[count] is not None and body[total] is not None and body[total] < body[count]:
            problems.append(f"envelope.{total}: less than {count}")
    if envelope["batch_id"] != batch_id_of(body, lines):
        problems.append("envelope.batch_id: not the hash of the body and events")

    events = []
    if lines and not lines.endswith(b"\n"):
        problems.append("events: the last line does not end")
    for number, line in enumerate(lines.split(b"\n")[:-1] if lines else [], start=2):
        try:
            event = json.loads(line)
        except ValueError:
            problems.append(f"line {number}: not JSON")
            continue
        if not isinstance(event, dict):
            problems.append(f"line {number}: not an event")
            continue
        events.append(event)
        problems.extend(validate_event(event, f"line {number}"))
    ids = [event.get("event_id") for event in events]
    if len(set(ids)) != len(ids):
        problems.append("events: an event_id twice")
    if len(events) != body["events"]:
        problems.append("envelope.events: not the number of events")
    if talks_in(events) != body["talks"]:
        problems.append("envelope.talks: not the number of talks")
    if not problems and span_of(events) != (body["first_event_at"], body["last_event_at"]):
        problems.append("envelope.first_event_at, last_event_at: not the events' first and last")
    return problems


# The command.


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


def report_left_behind(sanitizer):
    for title, counts in (
        ("events left out", sanitizer.excluded),
        ("fields with no rule", sanitizer.unknown),
        ("values that broke their rule", sanitizer.replaced),
    ):
        if counts:
            print(f"{title}:", file=sys.stderr)
            for name, count in counts.most_common():
                print(f"  {count:>6} x {name}", file=sys.stderr)


def validate_files(paths, sent):
    failed = 0
    for path in paths:
        problems = validate_batch(path.read_bytes(), sent=sent)
        if problems:
            failed += 1
            print(f"{path}: {len(problems)} problem(s)")
            for problem in problems[:50]:
                print(f"  {problem}")
        else:
            print(f"{path}: ok")
    print(f"{len(paths) - failed} of {len(paths)} batches valid", file=sys.stderr)
    return 1 if failed else 0


def write_batches(paths, given, out):
    out.mkdir(parents=True, exist_ok=True)
    written = 0
    for path in paths:
        label = label_of(path, given)
        sanitizer = Sanitizer()
        events = {}
        for raw in read_events(path):
            event = sanitizer.event(raw)
            if event is not None:
                events.setdefault(event["event_id"], event)
        report_left_behind(sanitizer)
        if not events:
            print(f"{path}: no events to send", file=sys.stderr)
            continue
        data = manual_batch(list(events.values()), label, sanitizer.counts())
        problems = validate_batch(data)
        if problems:
            raise SystemExit(f"{path}: the batch made breaks the policy: {problems[:5]}")
        batch_id = json.loads(data.partition(b"\n")[0])["batch_id"]
        (out / f"{batch_id}.jsonl").write_bytes(data)
        written += 1
        print(f"{path}: {len(events)} events from {label} in {out / batch_id}.jsonl", file=sys.stderr)
    print(f"wrote {written} batch(es) to {out}", file=sys.stderr)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("files", nargs="+", type=Path)
    parser.add_argument("--source", help="the laptop's label, for a single file not named after it")
    parser.add_argument("--batch-out", type=Path, help="write each file as one batch here")
    parser.add_argument("--validate", action="store_true", help="check batch files against the policy")
    parser.add_argument("--unsent", action="store_true", help="with --validate: an app's batch not yet sent")
    parser.add_argument("--data", type=Path, help="Stage 1: ingest into this folder instead of printing")
    parser.add_argument("--ingest-id", help="Stage 1: the new files' name (default: the time, UTC)")
    args = parser.parse_args(argv)
    if args.source and len(args.files) > 1:
        parser.error("--source names one file's laptop; name several files after theirs")
    if args.validate:
        return validate_files(args.files, sent=not args.unsent)
    if args.batch_out:
        return write_batches(args.files, args.source, args.batch_out)

    sanitizer = Sanitizer()
    clean = {}
    read = Counter()
    for path in args.files:
        source = label_of(path, args.source)
        for raw in read_events(path):
            read[source] += 1
            event = sanitizer.event(raw, source)
            if event is not None:
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
                for event in sorted(events, key=when):
                    out.write(json.dumps(event, separators=(",", ":")) + "\n")
            print(f"wrote {len(events)} events to {target}", file=sys.stderr)
    else:
        for events in fresh.values():
            for event in sorted(events, key=when):
                print(json.dumps(event, separators=(",", ":")))

    for source, count in sorted(read.items()):
        print(f"{source}: {count} lines read", file=sys.stderr)
    report_left_behind(sanitizer)
    print(
        f"{new} new events from {', '.join(sorted(read)) or 'nothing'} "
        f"({len(clean) - new} already had)",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
