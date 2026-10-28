"""Tests for the batch record ingest keeps (latency-sanitize.py --ledger)
and the collection health read from it (telemetry-health.py), alone and in
the weekly report.

  npm run test:telemetry
"""
import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


sanitize = load("latency-sanitize.py", "latency_sanitize")
health = load("telemetry-health.py", "telemetry_health")
weekly = load("latency-weekly.py", "latency_weekly")
policy_tests = load("test_telemetry_policy.py", "test_telemetry_policy")
clean_events, app_batch, split = policy_tests.clean_events, policy_tests.app_batch, policy_tests.split
COUNTS = policy_tests.COUNTS

# 2026-W44 runs Monday 26 October to Sunday 1 November, IST.
WEEK = "2026-W44"
MONDAY = datetime(2026, 10, 26, 6, 0, tzinfo=timezone.utc)
INSTALL = "0123456789abcdef"
OTHER_INSTALL = "fedcba9876543210"


def quietly(function, *args, **kwargs):
    with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()) as out:
        result = function(*args, **kwargs)
    return result, out.getvalue()


def stamp(at):
    return at.strftime("%Y-%m-%dT%H:%M:%SZ")


def record(seq, received, install=INSTALL, outcome="new", origin="app", send=None, **body):
    """A batch record's line, as ingest writes one: a chain of an install's
    batches, each dropping one event, is consistent by default."""
    full = {
        "policy_version": 2, "install_id": install, "batch_seq": seq, "origin": origin,
        "app_version": "0.1.26", "events": 100, "talks": 3,
        "first_event_at": stamp(received - timedelta(days=1)), "last_event_at": stamp(received - timedelta(hours=1)),
        "dropped_event_count": 0, "dropped_by_reason": {"queue_full": 0, "unreadable_line": 0, "write_failed": 0},
        "log_resets": 0, "dropped_total": 0, "log_resets_total": 0, **COUNTS,
    }
    if origin == "manual":
        full.update({name: None for name in sanitize.POLICY["batch"]["measured_by_app"]})
    full.update(body)
    sent = {
        "prepared_at": stamp(received), "sent_at": stamp(received), "attempts": 1,
        "first_attempt_at": stamp(received), "oldest_queued_event_age_ms": 86400000,
        "clock_suspect": False, "backlog_after_events": 0, "backlog_after_bytes": 0,
    }
    sent.update(send or {})
    return {
        "record": "batch", "received_at": stamp(received), "ingest_id": "t", "outcome": outcome,
        "batch_id": f"{install}{seq:016x}", "install_id": install, "batch_seq": seq, "origin": origin,
        "body": full, "send": sent, "events_new": full["events"], "events_known": 0,
    }


class Ingest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.data, self.ledger = self.root / "data", self.root / "ledger"
        self.files = 0

    def tearDown(self):
        self.tmp.cleanup()

    def file(self, data):
        self.files += 1
        path = self.root / "inbox" / f"{self.files}.jsonl"
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(data)
        return path

    def ingest(self, *paths, ingest="i1", received=MONDAY):
        code, out = quietly(
            sanitize.validate_files, list(paths), True, self.data, ingest, self.ledger, received
        )
        return code, out, [json.loads(line) for line in (self.ledger / f"{ingest}.jsonl").read_text().splitlines()]

    def stored(self):
        return [e for path in self.data.rglob("*.jsonl") for e in sanitize.read_events(path)]

    def test_a_batch_seen_again_is_a_duplicate_whose_events_are_stored_once(self):
        batch = app_batch(clean_events())
        retry = app_batch(clean_events(), send={"attempts": 3, "sent_at": "2026-10-26T07:00:00+00:00"})
        self.assertEqual(split(batch)[0]["batch_id"], split(retry)[0]["batch_id"])

        code, _, records = self.ingest(self.file(batch), self.file(batch))
        self.assertEqual(code, 0)
        self.assertEqual([r["outcome"] for r in records], ["new", "duplicate"])
        self.assertEqual([r["events_new"] for r in records], [len(clean_events()), 0])
        self.assertEqual(records[1]["events_known"], len(clean_events()))

        # A retry, or a batch remade from the same lines after its outbox
        # was lost, in a later upload: the same batch, not a conflict.
        _, _, records = self.ingest(self.file(retry), ingest="i2")
        self.assertEqual([r["outcome"] for r in records], ["duplicate"])
        self.assertEqual(records[0]["send"]["attempts"], 3)
        self.assertEqual(len(self.stored()), len(clean_events()))

    def test_a_seq_given_to_another_batch_is_flagged_and_its_events_taken_in(self):
        events = clean_events()
        first, other = app_batch(events[:3]), app_batch(events[3:])
        code, out, records = self.ingest(self.file(first), self.file(other))
        self.assertEqual(code, 0)
        self.assertIn("ok (seq conflict)", out)
        self.assertEqual([r["outcome"] for r in records], ["new", "seq_conflict"])
        self.assertEqual(len(self.stored()), len(events))
        # Against a seq recorded by an earlier ingest, too.
        _, _, records = self.ingest(self.file(app_batch(events[1:4])), ingest="i2")
        self.assertEqual([r["outcome"] for r in records], ["seq_conflict"])

    def test_manual_batches_are_never_in_sequence(self):
        events = clean_events()
        week1 = sanitize.manual_batch(events[:4], "lab-01", COUNTS)
        week2 = sanitize.manual_batch(events, "lab-01", COUNTS)
        _, _, records = self.ingest(self.file(week1), self.file(week2))
        self.assertEqual([r["outcome"] for r in records], ["manual", "manual"])
        self.assertEqual([r["events_new"] for r in records], [4, len(events) - 4])

    def test_a_refused_file_leaves_only_its_hash_size_and_problems(self):
        envelope, lines = split(sanitize.manual_batch(clean_events(), "lab-01", COUNTS))
        envelope["asha_said_hello"] = "Asha lives at 12 Main Road"
        bad = self.file(policy_tests.join(envelope, lines.replace(b'"total_ms":406', b'"total_ms":"Asha"')))
        code, _, records = self.ingest(bad)
        self.assertEqual(code, 1)
        self.assertEqual(self.stored(), [])
        [kept] = records
        self.assertEqual(set(kept), {"record", "received_at", "ingest_id", "file_sha256", "bytes", "problems"})
        self.assertEqual(kept["record"], "refused")
        self.assertEqual(kept["bytes"], bad.stat().st_size)
        self.assertRegex(kept["file_sha256"], r"^[0-9a-f]{32}$")
        self.assertIn("envelope.(a field): not in the policy", kept["problems"])
        text = (self.ledger / "i1.jsonl").read_text()
        for word in ("Asha", "asha", "Main Road", "said_hello"):
            self.assertNotIn(word, text)

    def test_an_ingest_cut_short_before_its_record_is_taken_in_again_whole(self):
        batch = self.file(app_batch(clean_events()))
        # The events were written, and the record was not.
        quietly(sanitize.validate_files, [batch], True, self.data, "i1")
        self.assertFalse(self.ledger.exists())
        _, _, records = self.ingest(batch, ingest="i2")
        self.assertEqual(records[0]["outcome"], "new")
        self.assertEqual((records[0]["events_new"], records[0]["events_known"]), (0, len(clean_events())))
        self.assertEqual(len(self.stored()), len(clean_events()))

    def test_an_ingest_never_overwrites_a_record(self):
        events = clean_events()
        self.ingest(self.file(app_batch(events[:3])))
        with self.assertRaises(SystemExit):
            self.ingest(self.file(app_batch(events[3:], body={"batch_seq": 2})))
        # Refused before any of its new events was written.
        self.assertEqual(len(self.stored()), 3)

    def test_the_ledger_is_only_for_an_ingest(self):
        with self.assertRaises(SystemExit):
            quietly(sanitize.main, ["--validate", "x.jsonl", "--ledger", str(self.ledger)])

    def test_the_command_records_and_reports(self):
        batch = self.file(app_batch(clean_events()))
        code, _ = quietly(sanitize.main, [
            "--validate", str(batch), "--data", str(self.data), "--ledger", str(self.ledger), "--ingest-id", "i1",
        ])
        self.assertEqual(code, 0)
        received = json.loads((self.ledger / "i1.jsonl").read_text())["received_at"]
        week = weekly.label_of(sanitize.when({"timestamp": received}))
        code, out = quietly(health.main, [str(self.ledger), "--week", week, "--json"])
        self.assertEqual(code, 0)
        result = json.loads(out)
        self.assertEqual(result["batches"]["new"], 1)
        self.assertEqual(result["sequence"]["pending_gaps"], [])


class Health(unittest.TestCase):
    def checks(self, records, week=WEEK):
        return [flag["check"] for flag in health.summary(records, week)["flags"]]

    def test_a_consistent_install_is_not_flagged(self):
        records = [
            record(1, MONDAY - timedelta(days=3), dropped_event_count=1, dropped_total=1,
                   dropped_by_reason={"queue_full": 0, "unreadable_line": 1, "write_failed": 0}),
            record(2, MONDAY + timedelta(days=1), dropped_total=1),
            record(3, MONDAY + timedelta(days=2), dropped_event_count=1, dropped_total=2, log_resets=1, log_resets_total=1,
                   dropped_by_reason={"queue_full": 0, "unreadable_line": 0, "write_failed": 1}),
        ]
        result = health.summary(records, WEEK)
        self.assertEqual(result["flags"], [])
        self.assertEqual(result["consistency"], [])
        self.assertEqual(result["batches"]["new"], 2)
        self.assertEqual(result["events_received"], 200)
        self.assertEqual(result["loss"]["dropped_by_reason"], {"queue_full": 0, "unreadable_line": 0, "write_failed": 1})

    def test_a_batch_counts_in_the_week_it_was_received(self):
        late = record(1, MONDAY + timedelta(days=2), last_event_at="2026-10-10T10:00:00Z")
        self.assertEqual(health.summary([late], WEEK)["batches"]["new"], 1)
        self.assertEqual(health.summary([late], "2026-W41")["batches"]["new"], 0)
        self.assertEqual(health.summary([late], WEEK)["delivery"]["delay_ms"]["n"], 1)

    def test_a_gap_is_pending_until_its_batch_comes(self):
        records = [record(1, MONDAY), record(3, MONDAY + timedelta(days=1))]
        result = health.summary(records, WEEK)
        self.assertEqual([(g["install"], g["batch_seq"]) for g in result["sequence"]["pending_gaps"]], [("01234567", 2)])
        self.assertNotIn("probably_lost", self.checks(records))
        records.append(record(2, MONDAY + timedelta(days=9)))
        later = health.summary(records, "2026-W45")
        self.assertEqual(later["sequence"]["pending_gaps"], [])
        self.assertEqual(later["sequence"]["lost_total"], 0)

    def test_a_gap_open_seven_days_is_taken_as_lost_once(self):
        records = [record(1, MONDAY), record(3, MONDAY + timedelta(days=1))]
        lost_week = health.summary(records, "2026-W45")
        self.assertEqual([g["batch_seq"] for g in lost_week["sequence"]["lost_this_week"]], [2])
        self.assertIn("probably_lost", [f["check"] for f in lost_week["flags"]])
        after = health.summary(records, "2026-W46")
        self.assertEqual(after["sequence"]["lost_this_week"], [])
        self.assertEqual(after["sequence"]["lost_total"], 1)
        self.assertNotIn("probably_lost", [f["check"] for f in after["flags"]])

    def test_a_new_install_starts_its_own_sequence(self):
        records = [record(1, MONDAY), record(2, MONDAY), record(1, MONDAY + timedelta(days=1), install=OTHER_INSTALL)]
        result = health.summary(records, WEEK)
        self.assertEqual(result["sequence"]["pending_gaps"], [])
        self.assertEqual(result["installs"]["reporting"], 2)

    def test_seq_conflicts_are_flagged_and_fill_no_gap_twice(self):
        records = [record(1, MONDAY), record(1, MONDAY, outcome="seq_conflict"), record(2, MONDAY)]
        result = health.summary(records, WEEK)
        self.assertEqual(result["sequence"]["conflicts"], [{"install": "01234567", "batch_seq": 1}])
        self.assertEqual(self.checks(records), ["seq_conflict"])
        self.assertEqual(result["events_received"], 300)

    def test_duplicates_are_counted_but_not_received_twice(self):
        records = [record(1, MONDAY), record(1, MONDAY + timedelta(hours=1), outcome="duplicate")]
        result = health.summary(records, WEEK)
        self.assertEqual(result["batches"]["duplicate"], 1)
        self.assertEqual(result["events_received"], 100)
        self.assertEqual(result["flags"], [])

    def test_totals_that_go_back_or_counts_that_do_not_add_up_are_flagged(self):
        back = [record(1, MONDAY, dropped_event_count=3, dropped_total=3), record(2, MONDAY, dropped_total=1)]
        self.assertEqual(
            [i["check"] for i in health.summary(back, WEEK)["consistency"]], ["dropped_total_went_back"]
        )
        off = [record(1, MONDAY), record(2, MONDAY, log_resets=1, log_resets_total=2)]
        self.assertEqual(
            [i["check"] for i in health.summary(off, WEEK)["consistency"]], ["log_resets_does_not_add_up"]
        )
        self.assertIn("inconsistent_counts", self.checks(off))
        # Across a gap only "never back" can be checked.
        gap = [record(1, MONDAY), record(3, MONDAY, dropped_event_count=1, dropped_total=5,
                                         dropped_by_reason={"queue_full": 1, "unreadable_line": 0, "write_failed": 0})]
        self.assertEqual(health.summary(gap, WEEK)["consistency"], [])
        # Shown in the week the later batch came, not again.
        self.assertEqual(health.summary(back, "2026-W45")["consistency"], [])

    def test_drops_are_flagged_past_one_percent_of_events_received(self):
        def week(dropped):
            reasons = {"queue_full": dropped, "unreadable_line": 0, "write_failed": 0}
            return [record(1, MONDAY, dropped_event_count=dropped, dropped_total=dropped, dropped_by_reason=reasons)]

        self.assertEqual(self.checks(week(1)), [])
        self.assertEqual(self.checks(week(2)), ["dropped"])
        self.assertEqual(health.summary(week(2), WEEK)["loss"]["drop_rate"], 0.02)

    def test_manual_batches_do_not_dilute_the_drop_rate(self):
        reasons = {"queue_full": 15, "unreadable_line": 0, "write_failed": 0}
        records = [
            record(1, MONDAY, events=1000, dropped_event_count=15, dropped_total=15, dropped_by_reason=reasons),
            record(0, MONDAY, origin="manual", outcome="manual", install=OTHER_INSTALL, events=600),
        ]
        result = health.summary(records, WEEK)
        self.assertEqual(result["events_received"], 1600)
        self.assertEqual(result["loss"]["drop_rate"], 0.015)
        self.assertIn("(1.5% of the app's events received)", "\n".join(health.markdown_lines(result)))

    def test_a_field_with_no_rule_is_flagged(self):
        self.assertEqual(self.checks([record(1, MONDAY, unknown_fields_dropped=2)]), ["unknown_fields"])

    def test_one_new_field_in_three_events_is_counted_as_three_values_not_three_fields(self):
        sanitizer = sanitize.Sanitizer()
        for event in clean_events()[:3]:
            sanitizer.event({**event, "mood": "fine"})
        dropped = sanitizer.counts()["unknown_fields_dropped"]
        self.assertEqual(dropped, 3)
        text = "\n".join(health.markdown_lines(health.summary([record(1, MONDAY, unknown_fields_dropped=dropped)], WEEK)))
        self.assertIn("▲ 3 value(s) dropped from fields the policy has no rule for", text)
        self.assertNotIn("3 field(s)", text)

    def test_a_clock_in_doubt_is_flagged(self):
        suspect = record(1, MONDAY, send={"clock_suspect": True})
        ahead = record(1, MONDAY, install=OTHER_INSTALL, last_event_at=stamp(MONDAY + timedelta(days=2)))
        result = health.summary([suspect, ahead], WEEK)
        self.assertEqual(len(result["delivery"]["clock_suspect"]), 2)
        self.assertIn("clock_suspect", [f["check"] for f in result["flags"]])

    def test_manual_batches_say_drops_were_not_measured(self):
        manual = record(0, MONDAY, origin="manual", outcome="manual", install=OTHER_INSTALL)
        result = health.summary([manual, record(1, MONDAY)], WEEK)
        self.assertEqual(result["loss"]["unmeasured_batches"], 1)
        self.assertEqual(result["sequence"]["pending_gaps"], [])
        self.assertEqual(result["events_received"], 200)
        self.assertEqual(result["flags"], [])
        self.assertIn("1 manual batch(es) do not measure it", "\n".join(health.markdown_lines(result)))

    def test_refused_files_are_flagged_and_counted_by_problem(self):
        refused = {"record": "refused", "received_at": stamp(MONDAY), "ingest_id": "t", "file_sha256": "0" * 32,
                   "bytes": 9, "problems": ["line 4: not JSON", "line 9: not JSON"]}
        result = health.summary([refused], WEEK)
        self.assertEqual(result["refused_problems"], {"line N: not JSON": 2})
        self.assertEqual(self.checks([refused]), ["refused"])

    def test_an_install_that_sent_before_and_during_the_week_is_reporting(self):
        records = [
            record(1, MONDAY - timedelta(days=10)),
            record(2, MONDAY - timedelta(days=3)),
            record(3, MONDAY + timedelta(days=1)),
            record(1, MONDAY - timedelta(days=5), install=OTHER_INSTALL),
        ]
        result = health.summary(records, WEEK)
        self.assertEqual(result["installs"], {"reporting": 1, "silent": ["fedcba98"]})
        self.assertIn("- Installs: 1 reporting; silent (sent in the 28 days before, not this week): fedcba98",
                      health.markdown_lines(result))

    def test_an_install_that_stopped_sending_is_named_silent(self):
        records = [record(1, MONDAY - timedelta(days=10)), record(1, MONDAY - timedelta(days=40), install=OTHER_INSTALL)]
        self.assertEqual(health.summary(records, WEEK)["installs"]["silent"], ["01234567"])

    def test_installs_are_named_by_eight_hex_only(self):
        records = [record(1, MONDAY), record(3, MONDAY, send={"clock_suspect": True})]
        text = json.dumps(health.summary(records, WEEK)) + "\n".join(health.markdown_lines(health.summary(records, WEEK)))
        self.assertIn("01234567", text)
        self.assertNotIn(INSTALL, text)

    def test_the_jsons_keys_stay_put(self):
        result = health.summary([], WEEK)
        self.assertEqual(list(result), [
            "health_version", "week", "batch_files", "batches", "refused_problems", "installs", "events_received",
            "events_new", "sequence", "loss", "delivery", "app_versions", "consistency", "flags",
        ])
        self.assertEqual(result["batches"], {"new": 0, "duplicate": 0, "seq_conflict": 0, "manual": 0, "refused": 0})
        self.assertEqual(result["flags"], [])


class TheWeeklyReport(unittest.TestCase):
    def test_collection_is_a_section_of_the_report(self):
        events = [dict(e, install_id=INSTALL) for e in clean_events()]
        without = weekly.build(events, WEEK)
        self.assertEqual(without["report_version"], 3)
        self.assertIsNone(without["collection"])
        self.assertIn("No batch record given", weekly.markdown(without))

        report = weekly.build(events, WEEK, ledger=[record(1, MONDAY), record(1, MONDAY, outcome="seq_conflict")])
        self.assertEqual(report["collection"]["batches"]["seq_conflict"], 1)
        text = weekly.markdown(report)
        self.assertIn("**Collection: 1 flagged**", text)
        self.assertIn("## Collection", text)
        self.assertIn("▲ 1 seq(s) given to two batches", text)
        # The latency flags are their own, as in report_version 2.
        self.assertEqual(report["flags"], [])

    def test_a_record_line_among_the_events_is_not_an_event(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp)
            policy_tests_events = clean_events()
            (data / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in policy_tests_events))
            (data / "ledger.jsonl").write_text(json.dumps(record(1, MONDAY, install=OTHER_INSTALL)) + "\n")
            self.assertEqual(len(weekly.read_all([data])), len(policy_tests_events))

    def test_the_command_reads_the_ledger(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / "data").mkdir()
            (tmp / "ledger").mkdir()
            (tmp / "data" / "e.jsonl").write_text("".join(json.dumps(e) + "\n" for e in clean_events()))
            (tmp / "ledger" / "i1.jsonl").write_text(json.dumps(record(2, MONDAY)) + "\n")
            code, _ = quietly(weekly.main, [
                str(tmp / "data"), "--week", WEEK, "--ledger", str(tmp / "ledger"), "--out", str(tmp / "reports"),
            ])
            self.assertEqual(code, 0)
            saved = json.loads((tmp / "reports" / f"{WEEK}.json").read_text())
            self.assertEqual([g["batch_seq"] for g in saved["collection"]["sequence"]["pending_gaps"]], [1])


if __name__ == "__main__":
    unittest.main()
