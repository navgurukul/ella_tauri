#!/usr/bin/env python3
"""SYNTHETIC telemetry, made up for testing the weekly report: never a
laptop's, never a learner's, never to be taken in with real data.

Six made-up laptops (`synthetic-mac-01` and so on) over two weeks of
January 2026, before Ella sent any telemetry, every event of them on an
app version ending `-synthetic`. Each laptop's events are written as a
laptop writes them, put through the telemetry policy and made into the
batches the app sends (or, for one laptop, a manual export), with a seq
missing, a retry, dropped events, a batch sent twice and one refused, so
every section of report_version 4 has something to show.

The pipeline then runs as telemetry-repo.py runs it, with the same tools:
latency-sanitize.py --validate takes the batches into data/ and ledger/,
three ingests at fixed times, and latency-weekly.py writes the reports of
2026-W01 and 2026-W02 at a fixed time, so the same scripts always give the
same report.

  npm run telemetry:synthetic                 # prints the 2026-W02 report
  npm run telemetry:synthetic -- --out DIR    # writes its .md and .json
  npm run telemetry:synthetic -- --write      # remakes the fixtures

--write remakes scripts/fixtures/telemetry-synthetic/: the batches, and the
expected 2026-W02 report the tests compare against. Run it after a change
that is meant to change the report, and read the diff.
"""
import argparse
import contextlib
import importlib.util
import io
import json
import shutil
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures" / "telemetry-synthetic"


def _sibling(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


sanitize = _sibling("latency-sanitize.py", "latency_sanitize")
weekly = _sibling("latency-weekly.py", "latency_weekly")

# Every made-up thing says so: the version every event runs, each laptop's
# label (and so each file's name), the tools a report names.
MARK = "synthetic"
VERSIONS = ("9.9.0-synthetic", "9.9.1-synthetic")
TOOLS_REF = "synthetic-fixture"
WEEK, PREVIOUS = "2026-W02", "2026-W01"
# When the report is made: the Monday after the week, 11:30 IST.
NOW = datetime(2026, 1, 12, 6, 0, tzinfo=timezone.utc)
# The three ingests: (name, received at, the batches it takes in).
INGESTS = (
    ("ingest-1", datetime(2026, 1, 4, 12, 0, tzinfo=timezone.utc)),
    ("ingest-2", datetime(2026, 1, 7, 10, 0, tzinfo=timezone.utc)),
    ("ingest-3", datetime(2026, 1, 11, 8, 0, tzinfo=timezone.utc)),
)
LAPTOPS = {
    # label: os, arch, cpu, ram_mb, install place, version
    "synthetic-mac-01": ("macOS 26.0 (synthetic)", "aarch64", "Apple M2 (synthetic)", 16384, "applications", 1),
    "synthetic-mac-02": ("macOS 26.0 (synthetic)", "aarch64", "Apple M2 (synthetic)", 16384, "applications", 1),
    "synthetic-mac-03": ("macOS 26.0 (synthetic)", "aarch64", "Apple M3 (synthetic)", 16384, "applications", 1),
    "synthetic-win-01": ("Windows 11 (synthetic)", "x86_64", "Intel(R) Core(TM) i5 (synthetic)", 8192, "applications", 0),
    "synthetic-win-02": ("Windows 11 (synthetic)", "x86_64", "Intel(R) Core(TM) i5 (synthetic)", 8192, "applications", 0),
    "synthetic-linux-01": ("Linux 6.8 (synthetic)", "x86_64", "AMD Ryzen 5 (synthetic)", 16384, "source", 0),
}


def stamp(day, hour, minute=0, second=0):
    """`day` of January 2026 at an IST time, as the app writes UTC."""
    at = datetime(2026, 1, day, hour, minute, second, tzinfo=sanitize.IST).astimezone(timezone.utc)
    return at.strftime("%Y-%m-%dT%H:%M:%S.%fZ")


class Laptop:
    """One made-up laptop's log, written as the app writes latency.jsonl."""

    def __init__(self, label):
        self.label = label
        self.os, self.arch, self.cpu, self.ram_mb, self.place, version = LAPTOPS[label]
        self.version = VERSIONS[version]
        self.events = []
        self.launches = 0
        self.launch = None
        self.talks = 0

    def base(self, kind, at):
        return {"event": kind, "schema_version": 2, "timestamp": at, "app_version": self.version, "launch_id": self.launch}

    def start(self, day, hour, server="started", ready_ms=9000):
        """A launch: its line, the engines coming up, the cloud, the updater."""
        self.launches += 1
        self.launch = f"{self.label}-launch-{self.launches}"
        self.events.append({
            **self.base("ella_launch", stamp(day, hour)), "os": self.os, "arch": self.arch, "cpu": self.cpu,
            "ram_mb": self.ram_mb, "cores_physical": 8, "cores_logical": 8,
            "install": {"bundle": "app", "place": self.place, "also_at": []},
        })
        self.events.append({
            **self.base("ella_engine", stamp(day, hour, 0, 9)), "since_launch_ms": ready_ms, "llm_server": server,
            "stt": "canary", "piper": "ok",
        })
        self.events.append({**self.base("ella_cloud", stamp(day, hour, 0, 10)), "up": True})
        self.events.append({**self.base("ella_update", stamp(day, hour, 0, 20)), "step": "up_to_date", "ms": 900})

    def talk(self, day, hour, turns, talk="free", cloud_every=2, wait=1500, end="over", recap_kind="talk",
             left=False, recap_at=None, errors=()):
        """A talk of `turns` turns from `day` `hour`:00 IST, a minute apart.
        `end`: "over" (the last turn says so, and a recap follows), or None.
        `left`: its turns noted `talk_left`, as the app sends a talk left.
        `errors`: (turn number, error text, stage) of turns that failed."""
        self.talks += 1
        session = f"{self.label}-talk-{self.talks}"
        failed = {number: (text, stage) for number, text, stage in errors}
        for number in range(1, turns + 1):
            at = stamp(day, hour, number)
            cloud = cloud_every and number % cloud_every == 0
            # The laptop is slower than the cloud, and slower on a later turn.
            speech = wait + 120 * number if cloud else 3 * wait + 400 * number
            event = {
                **self.base("ella_turn_latency", at), "interaction_id": f"{session}-{number}", "kind": "voice",
                "status": "ok", "session_id": session, "talk": talk, "turn": number,
                "stt_ms": 150 + 10 * number, "llm_ttft_ms": speech // 2, "llm_completion_ms": speech - 200,
                "speech_ms": speech, "total_ms": speech + 300, "tts_first_audio_ms": 250 + 5 * number,
                "llm_runs": [{"backend": "cloud", "ttft_ms": speech // 2, "ms": speech - 200}] if cloud
                else [{"ttft_ms": speech // 2, "ms": speech - 200, "gen_tokens": 40, "gen_ms": 1600}],
            }
            if number == 1:
                event["warm_up"] = {"began_ms_before": 20000, "ms": 4000 + 100 * self.talks, "evaluated_tokens": 900}
            if number in failed:
                text, stage = failed[number]
                event.update(status="error", error=text, failed_at=stage)
            if left:
                event["notes"] = ["talk_left"]
            if end == "over" and number == turns:
                event["talk_over"] = True
            self.events.append(event)
        if end == "over":
            self.recap(session, recap_at or stamp(day, hour, turns, 30), recap_kind)
        return session

    def recap(self, session, at, kind="talk", status="ok"):
        judges = [{"judge": "notes", "backend": "cloud", "status": "ok", "ms": 2500}]
        if status != "ok":
            judges = [{"judge": "notes", "backend": "cloud", "status": "error", "error": "Local engine error: x"}]
        event = {**self.base("ella_assessment", at), "session_id": session, "kind": kind, "status": status,
                 "total_ms": 5200, "scored_ms": 4800, "judges": judges}
        if status != "ok":
            event["error"] = "Local engine error: the recap failed"
        self.events.append(event)

    def resume(self, session, day, hour, turns):
        """More turns of a talk the app sent as left, then its end: no note."""
        for number in range(1, turns + 1):
            speech = 4200 + 300 * number
            event = {
                **self.base("ella_turn_latency", stamp(day, hour, number)), "interaction_id": f"{session}-r{number}",
                "kind": "voice", "status": "ok", "session_id": session, "talk": "free", "turn": 10 + number,
                "stt_ms": 160, "llm_ttft_ms": speech // 2, "llm_completion_ms": speech - 200, "speech_ms": speech,
                "total_ms": speech + 300, "llm_runs": [{"ttft_ms": speech // 2, "ms": speech - 200}],
            }
            if number == turns:
                event["talk_over"] = True
            self.events.append(event)
        self.recap(session, stamp(day, hour, turns, 30))

    def cloud_down(self, day, hour, reason):
        self.events.append({**self.base("ella_cloud", stamp(day, hour, 30)), "up": False, "reason": reason})
        self.events.append({**self.base("ella_cloud", stamp(day, hour, 45)), "up": True})

    def update_failed(self, day, hour):
        self.events.append({
            **self.base("ella_update", stamp(day, hour, 50)), "step": "check_failed", "ms": 80,
            "error": "File or process error: could not reach the update server",
        })

    def between(self, first, last):
        """This laptop's events from `first` to `last` (IST days of January),
        sanitized, oldest first: what one batch of the app holds."""
        lo, hi = stamp(first, 0), stamp(last + 1, 0)
        chosen = [e for e in self.events if lo <= e["timestamp"] < hi]
        return sanitize.oldest_first([sanitize.Sanitizer().event(e) for e in chosen])


def logs():
    """Every laptop's two weeks: 2026-W01 (29 Dec to 4 Jan) is mostly
    quiet, for the week before; 2026-W02 (5 to 11 Jan) is the report's."""
    laptops = {label: Laptop(label) for label in LAPTOPS}
    # The week before: three days, each Mac and Windows laptop a talk or two,
    # the laptop a little faster; one talk's recap comes on the Monday after.
    for label, laptop in laptops.items():
        if label.startswith("synthetic-linux"):
            continue
        # Dates are of January; 2026-W01 begins on 29 December, so its days
        # here are the 2nd to the 4th.
        laptop.start(2, 9)
        laptop.talk(2, 10, 6, wait=1000)
        laptop.talk(3, 11, 5, talk="lesson", wait=1000)
    laptops["synthetic-mac-03"].talk(4, 23, 4, wait=1000, recap_at=stamp(5, 0, 20))

    mac1, mac2, mac3 = (laptops[f"synthetic-mac-0{n}"] for n in (1, 2, 3))
    win1, win2, linux = laptops["synthetic-win-01"], laptops["synthetic-win-02"], laptops["synthetic-linux-01"]
    for mac in (mac1, mac2, mac3):
        mac.start(5, 9)
        mac.talk(5, 10, 6)
        mac.talk(6, 11, 5, talk="lesson")
        mac.start(8, 9)
        mac.talk(8, 10, 4, cloud_every=0)
    mac1.talk(8, 18, 2, talk="word", recap_kind="word")
    # Left: no end and no turn for 12 hours, so the app sent it noted.
    mac1.talk(9, 19, 3, end=None, left=True)
    # Resumed: sent as left, then taken up again and ended.
    session = mac2.talk(6, 19, 2, end=None, left=True)
    mac2.resume(session, 7, 15, 2)
    mac3.cloud_down(5, 9, "could not connect")
    mac3.cloud_down(8, 9, "HTTP 503")

    # Win-01's 5th and 6th go in the batch that never arrives (seq 2): this
    # talk's turns with them, its recap on the 7th in the next batch, so it
    # is a recap whose turns are not here.
    win1.start(6, 9, server="external", ready_ms=2500)
    lost = win1.talk(6, 10, 4, end=None)
    win1.recap(lost, stamp(7, 8))
    win1.start(9, 9, server="external", ready_ms=2600)
    win1.talk(9, 10, 6, errors=[
        (2, "Local engine request failed: connection refused", "llm:start"),
        (4, "NO_SPEECH_DETECTED: nothing heard", "stt"),
    ])
    win1.update_failed(9, 9)

    win2.start(6, 9)
    win2.talk(6, 10, 5, talk="placement", recap_kind="placement")
    win2.talk(7, 10, 4)
    win2.recap(win2.talk(7, 16, 3, end=None), stamp(7, 16, 4), status="error")
    win2.cloud_down(6, 9, "timed out")

    # A laptop running from source, whose tester exported its log by hand:
    # a talk with no end and no note, as an export sends one.
    linux.start(10, 14)
    linux.talk(10, 15, 4, end=None)
    return laptops


def app_batch(laptop, seq, events, sent_at, attempts=1, dropped=0, dropped_total=0):
    """The batch the app makes of `events`: sequenced, measured, sent."""
    first, last = sanitize.span_of(events)
    body = {name: None for name in sanitize.BODY}
    body.update(
        policy_version=sanitize.POLICY["policy_version"], install_id=sanitize.short_hash(f"app:{laptop.label}"),
        batch_seq=seq, origin="app", app_version=laptop.version, events=len(events), talks=sanitize.talks_in(events),
        first_event_at=first, last_event_at=last, dropped_event_count=dropped,
        dropped_by_reason={"queue_full": dropped, "unreadable_line": 0, "write_failed": 0}, log_resets=0,
        dropped_total=dropped_total, log_resets_total=0, policy_excluded_count=0, values_replaced=0,
        unknown_fields_dropped=0,
    )
    sent_at = sent_at.strftime("%Y-%m-%dT%H:%M:%SZ")
    send = {
        "prepared_at": sent_at, "sent_at": sent_at, "attempts": attempts, "first_attempt_at": sent_at,
        "oldest_queued_event_age_ms": 3_600_000, "clock_suspect": False, "backlog_after_events": 0,
        "backlog_after_bytes": 0,
    }
    return sanitize.batch_bytes(body, sanitize.event_lines(events), send)


def batches():
    """{ingest: {file name: bytes}}: what each ingest takes in."""
    laptops = logs()
    received = dict(INGESTS)
    taken = {name: {} for name, _ in INGESTS}

    def send(ingest, laptop, seq, first, last, **extra):
        events = laptop.between(first, last)
        at = received[ingest] - timedelta(minutes=30)
        data = app_batch(laptop, seq, events, at, **extra)
        taken[ingest][f"{laptop.label}-{seq}.jsonl"] = data
        return data

    for label, laptop in laptops.items():
        if label.startswith("synthetic-linux"):
            continue
        send("ingest-1", laptop, 1, 2, 4)
    for label in ("synthetic-mac-01", "synthetic-mac-02", "synthetic-mac-03"):
        send("ingest-2", laptops[label], 2, 5, 6)
        send("ingest-3", laptops[label], 3, 7, 11)
    # Seq 2, the 5th and 6th, never arrives; seq 3 took three tries.
    laptops["synthetic-win-01"].between(5, 6)
    send("ingest-3", laptops["synthetic-win-01"], 3, 7, 11, attempts=3)
    # Win-02 dropped 3 events, its queue full; the same batch comes twice.
    twice = send("ingest-2", laptops["synthetic-win-02"], 2, 5, 11, dropped=3, dropped_total=3)
    taken["ingest-3"]["synthetic-win-02-2-again.jsonl"] = twice
    # A manual export, the whole log at once.
    linux = laptops["synthetic-linux-01"]
    counts = {"policy_excluded_count": 0, "values_replaced": 0, "unknown_fields_dropped": 0}
    taken["ingest-3"]["synthetic-linux-01-manual.jsonl"] = sanitize.manual_batch(
        linux.between(1, 11), linux.label, counts, now=received["ingest-3"]
    )
    # A batch whose events were changed after it was made: refused whole.
    head, _, lines = taken["ingest-2"]["synthetic-mac-01-2.jsonl"].partition(b"\n")
    taken["ingest-2"]["synthetic-refused.jsonl"] = head + b"\n" + lines.replace(b'"stt_ms":160', b'"stt_ms":161', 1)
    return taken


def quietly(function, *args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        return function(*args, **kwargs)


def ingest(inbox, repo):
    """Takes `inbox`/<ingest>/ in, ingest by ingest, into `repo`'s data/
    and ledger/, as telemetry-repo.py does, at the fixed times."""
    for name, received in INGESTS:
        files = sorted((inbox / name).glob("*.jsonl"))
        quietly(
            sanitize.validate_files, files, True, repo / "data", f"{name}-{MARK}", repo / "ledger", received,
        )


def report(repo, week=WEEK, previous_report=None):
    """The weekly report of `repo`, as latency-weekly.py writes it, made at
    NOW: (JSON, Markdown)."""
    events = weekly.read_all([repo / "data"])
    ledger = weekly.health.read([repo / "ledger"])
    built = weekly.build(events, week, previous_report, TOOLS_REF, now=NOW, ledger=ledger)
    return json.dumps(built, indent=2) + "\n", weekly.markdown(built)


def run(inbox):
    """The whole pipeline over the batches in `inbox`: the 2026-W02
    report, against 2026-W01's, as (JSON, Markdown)."""
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp)
        ingest(inbox, repo)
        before, _ = report(repo, PREVIOUS)
        return report(repo, WEEK, json.loads(before))


def write_batches(target):
    for name, files in batches().items():
        folder = target / name
        folder.mkdir(parents=True, exist_ok=True)
        for file, data in files.items():
            (folder / file).write_bytes(data)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", type=Path, help=f"write {WEEK}.md and {WEEK}.json here")
    parser.add_argument("--write", action="store_true", help="remake the fixtures and the expected report")
    args = parser.parse_args(argv)
    if args.write:
        inbox = FIXTURES / "inbox"
        shutil.rmtree(inbox, ignore_errors=True)
        write_batches(inbox)
        as_json, text = run(inbox)
        expected = FIXTURES / "expected"
        expected.mkdir(parents=True, exist_ok=True)
        (expected / f"{WEEK}.json").write_text(as_json, encoding="utf-8")
        (expected / f"{WEEK}.md").write_text(text, encoding="utf-8")
        print(f"wrote {FIXTURES}", file=sys.stderr)
        return 0
    with tempfile.TemporaryDirectory() as tmp:
        inbox = Path(tmp)
        write_batches(inbox)
        as_json, text = run(inbox)
    if args.out:
        args.out.mkdir(parents=True, exist_ok=True)
        (args.out / f"{WEEK}.json").write_text(as_json, encoding="utf-8")
        (args.out / f"{WEEK}.md").write_text(text, encoding="utf-8")
    print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
