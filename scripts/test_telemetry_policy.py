"""Tests for the telemetry policy (telemetry-fields.json), its shared
vectors (telemetry-vectors.json) and the batch format, as
latency-sanitize.py applies and checks them.

  npm run test:telemetry
"""
import contextlib
import importlib.util
import io
import json
import re
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


sanitize = load("latency-sanitize.py", "latency_sanitize")
POLICY = sanitize.POLICY
VECTORS = json.loads((HERE / "telemetry-vectors.json").read_text(encoding="utf-8"))
NOW = datetime(2026, 10, 26, 6, 0, tzinfo=timezone.utc)
COUNTS = {"policy_excluded_count": 0, "values_replaced": 0, "unknown_fields_dropped": 0}


def clean_events():
    """What left, once each: some vectors are one event in two forms."""
    events = {}
    for vector in VECTORS["events"]:
        if vector["sanitized"]:
            events.setdefault(vector["sanitized"]["event_id"], vector["sanitized"])
    return list(events.values())


def clean(kind):
    return next(event for event in clean_events() if event["event"] == kind)


def split(data):
    head, _, lines = data.partition(b"\n")
    return json.loads(head), lines


def join(envelope, lines):
    return (json.dumps(envelope, separators=(",", ":")) + "\n").encode("utf-8") + lines


def app_batch(events, body=None, send=None):
    """An app's batch, as Phase 2 will send one: sequenced and measured."""
    events = sorted(events, key=sanitize.when)
    first, last = sanitize.span_of(events)
    full = {
        "policy_version": 2, "install_id": "0123456789abcdef", "batch_seq": 1, "origin": "app",
        "app_version": "0.1.26", "events": len(events), "talks": sanitize.talks_in(events),
        "first_event_at": first, "last_event_at": last, "dropped_event_count": 2,
        "dropped_by_reason": {"queue_full": 0, "unreadable_line": 2, "write_failed": 0},
        "log_resets": 0, "dropped_total": 5, "log_resets_total": 0, **COUNTS,
    }
    full.update(body or {})
    sent = {
        "prepared_at": "2026-10-26T06:00:00+00:00", "sent_at": "2026-10-26T06:05:00+00:00",
        "attempts": 2, "first_attempt_at": "2026-10-26T06:01:00+00:00",
        "oldest_queued_event_age_ms": 518400000, "clock_suspect": False,
        "backlog_after_events": 0, "backlog_after_bytes": 0,
    }
    sent.update(send or {})
    return sanitize.batch_bytes(full, sanitize.event_lines(events), sent)


class TheVectors(unittest.TestCase):
    def test_every_event_leaves_as_recorded(self):
        for vector in VECTORS["events"]:
            with self.subTest(vector["name"]):
                sanitizer = sanitize.Sanitizer()
                self.assertEqual(sanitizer.event(vector["raw"]), vector["sanitized"])
                self.assertEqual(sorted(sanitizer.unknown), vector["unknown"])
                self.assertEqual(sorted(sanitizer.replaced), vector["replaced"])
                self.assertEqual(sorted(sanitizer.excluded), vector["excluded"])

    def test_what_leaves_keeps_the_policy_strictly(self):
        for vector in VECTORS["events"]:
            if vector["sanitized"]:
                with self.subTest(vector["name"]):
                    self.assertEqual(sanitize.validate_event(vector["sanitized"]), [])

    def test_sanitizing_what_left_changes_nothing(self):
        for vector in VECTORS["events"]:
            event = vector["sanitized"]
            if not event:
                continue
            with self.subTest(vector["name"]):
                sanitizer = sanitize.Sanitizer()
                self.assertEqual(sanitizer.event(event), event)
                self.assertEqual(sanitizer.counts(), COUNTS)

    def test_errors_and_reasons_become_their_codes(self):
        for text, code in VECTORS["error_codes"]:
            with self.subTest(text):
                self.assertEqual(sanitize.error_code(text), code)
        for text, code in VECTORS["reason_codes"]:
            with self.subTest(text):
                self.assertEqual(sanitize.reason_code(text), code)

    def test_the_batch_id_is_the_hash_of_its_body_and_lines(self):
        vector = VECTORS["batch_id"]
        self.assertEqual(
            sanitize.batch_id_of(vector["body"], vector["lines"].encode("utf-8")), vector["batch_id"]
        )

    def test_no_learner_words_in_any_vector_that_left(self):
        left = json.dumps([vector["sanitized"] for vector in VECTORS["events"]])
        for words in ("Asha", "hello", "market", "A2", "You are Ella", "secret", "C:\\\\Users", "ASHA-PC"):
            self.assertNotIn(words, left)


class ThePolicy(unittest.TestCase):
    def rules(self):
        for schema in (POLICY["every_event"], POLICY["batch"]["body"], POLICY["batch"]["send"]):
            yield from schema.values()
        for spec in POLICY["events"].values():
            yield from spec["fields"].values()
        for schema in POLICY["objects"].values():
            yield from schema.values()

    def test_every_rule_is_one_the_sanitizer_knows(self):
        for rule in self.rules():
            with self.subTest(rule):
                kind, _, arg = rule.partition(":")
                while kind == "list":
                    kind, _, arg = arg.partition(":")
                if kind == "object":
                    self.assertIn(arg, POLICY["objects"])
                elif kind == "enum":
                    self.assertIn(arg, POLICY["enums"])
                elif kind == "code":
                    self.assertIn(arg, POLICY["coders"])
                elif kind == "hash":
                    self.assertRegex(arg, r"^[a-z_]+$")
                else:
                    self.assertIn(kind, set(POLICY["patterns"]) | {"num", "int", "bool", "hardware", "host"})

    def test_no_two_fields_leave_under_one_name(self):
        for kind in POLICY["events"]:
            with self.subTest(kind):
                names = [sanitize.output_name(name, rule) for name, rule in sanitize.schema_of(kind).items()]
                self.assertEqual(len(names), len(set(names)))
                self.assertNotIn("event_id", names)
                excluded = set(POLICY["events"][kind].get("excluded", ()))
                self.assertFalse(excluded & set(sanitize.schema_of(kind)))

    def test_the_learners_own_fields_never_have_a_rule(self):
        for kind in POLICY["events"]:
            for field in ("topic", "level", "interaction_id", "transcript", "name", "learner_name"):
                self.assertNotIn(field, sanitize.schema_of(kind))

    def test_the_batch_lists_name_its_own_fields(self):
        batch = POLICY["batch"]
        self.assertFalse(set(batch["body"]) & set(batch["send"]))
        self.assertLessEqual(set(batch["measured_by_app"]), set(batch["body"]))
        self.assertLessEqual(set(batch["filled_when_sent"]), set(batch["send"]))
        # A float would hash differently in Rust and Python: the body has none.
        for rule in batch["body"].values():
            self.assertNotEqual(rule, "num")

    def test_times_are_taken_in_the_forms_the_app_writes_them(self):
        # chrono's to_rfc3339 writes 0, 3, 6 or 9 digits of a second.
        for stamp in ("2026-10-20T06:00:00Z", "2026-10-20T06:00:00.123+05:30",
                      "2026-10-20T06:00:00.123456+00:00", "2026-10-20T06:00:00.123456789+00:00"):
            with self.subTest(stamp):
                self.assertIsNotNone(sanitize.Sanitizer().event({"event": "ella_cloud", "timestamp": stamp, "up": True}))
        for stamp in ("2026-10-20T06:00:00.5+00:00", "2026-10-20 06:00:00", "yesterday", 1760940000):
            with self.subTest(stamp):
                sanitizer = sanitize.Sanitizer()
                self.assertIsNone(sanitizer.event({"event": "ella_cloud", "timestamp": stamp, "up": True}))
                self.assertEqual(sanitizer.counts()["policy_excluded_count"], 1)

    def test_every_pattern_is_anchored(self):
        for name, pattern in POLICY["patterns"].items():
            with self.subTest(name):
                self.assertTrue(pattern.startswith("^") and pattern.endswith("$"))
                re.compile(pattern)


class StrictValidation(unittest.TestCase):
    def test_an_event_with_anything_the_policy_does_not_allow_is_refused(self):
        good = clean_events()[0]
        cases = {
            "a field with no rule": {**good, "transcript": "x"},
            "a Stage-1 source": {**good, "source": "lab-01"},
            "an error code not in the set": {**good, "error_code": "VALIDATION_FAILED"},
            "text in a token": {**good, "status": "Something went wrong"},
            "a number as text": {**good, "total_ms": "406"},
            "an unknown machine field": {**good, "machine": {"hostname": "x"}},
            "no event_id": {key: value for key, value in good.items() if key != "event_id"},
            "a host": {**clean("ella_engine"), "cloud": "proxy.example.org"},
        }
        for name, event in cases.items():
            with self.subTest(name):
                self.assertTrue(sanitize.validate_event(event))

    def test_problems_name_the_field_never_its_value(self):
        secret = "my name is Asha and I live at 12 Park Road"
        event = {**clean_events()[0], "status": secret, "transcript": secret}
        problems = sanitize.validate_event(event)
        self.assertTrue(problems)
        self.assertNotIn("Asha", " ".join(problems))


class Batches(unittest.TestCase):
    def test_a_manual_batch_is_valid_and_named_for_its_laptop_only_by_hash(self):
        data = sanitize.manual_batch(clean_events(), "lab-01", COUNTS, now=NOW)
        self.assertEqual(sanitize.validate_batch(data), [])
        envelope, _ = split(data)
        self.assertEqual(envelope["install_id"], sanitize.short_hash("manual:lab-01"))
        self.assertNotIn(b"lab-01", data)
        self.assertEqual((envelope["origin"], envelope["batch_seq"]), ("manual", 0))
        self.assertIsNone(envelope["dropped_event_count"])

    def test_an_app_batch_is_valid(self):
        self.assertEqual(sanitize.validate_batch(app_batch(clean_events())), [])

    def test_a_retry_is_the_same_batch(self):
        first = split(app_batch(clean_events()))[0]["batch_id"]
        again = split(app_batch(clean_events(), send={"attempts": 7, "sent_at": "2026-10-27T00:00:00+00:00"}))[0]
        self.assertEqual(again["batch_id"], first)

    def test_the_id_covers_every_byte_of_the_events_and_the_body(self):
        first = split(app_batch(clean_events()))[0]["batch_id"]
        changed = [dict(event) for event in clean_events()]
        changed[0]["total_ms"] += 1
        self.assertNotEqual(split(app_batch(changed))[0]["batch_id"], first)
        self.assertNotEqual(split(app_batch(clean_events(), body={"batch_seq": 2}))[0]["batch_id"], first)

    def test_a_batch_changed_after_it_was_made_is_refused(self):
        envelope, lines = split(app_batch(clean_events()))
        tampered = lines.replace(b'"total_ms":406', b'"total_ms":407')
        self.assertIn("envelope.batch_id: not the hash of the body and events", sanitize.validate_batch(join(envelope, tampered)))

    def test_a_batch_that_breaks_the_rules_is_refused(self):
        events = clean_events()
        cases = {
            "an envelope field with no rule": (lambda e: {**e, "hostname": "x"}, None),
            "a missing field": (lambda e: {k: v for k, v in e.items() if k != "talks"}, None),
            "a negative count": (None, {"values_replaced": -1}),
            "drops that do not add up": (None, {"dropped_event_count": 3}),
            "a total below its batch": (None, {"dropped_total": 1}),
            "a manual batch in sequence": (None, {"origin": "manual", "batch_seq": 4, "app_version": None}),
            "an app batch out of sequence": (None, {"batch_seq": 0}),
            "a manual batch that claims to measure": (None, {"origin": "manual", "batch_seq": 0}),
            "another policy": (None, {"policy_version": 1}),
            "a wrong count of events": (None, {"events": 99}),
            "a wrong count of talks": (None, {"talks": 9}),
            "a wrong first event": (None, {"first_event_at": "2026-01-01T00:00:00Z"}),
            "an install that is not hex": (None, {"install_id": "ASHA-PC-0000000"}),
        }
        for name, (edit, body) in cases.items():
            with self.subTest(name):
                data = app_batch(events, body=body)
                if edit:
                    envelope, lines = split(data)
                    data = join(edit(envelope), lines)
                self.assertTrue(sanitize.validate_batch(data))

    def test_an_app_batch_must_say_how_its_sending_went_once_sent(self):
        data = app_batch(clean_events(), send={"sent_at": None, "attempts": None})
        self.assertTrue(sanitize.validate_batch(data))
        self.assertEqual(sanitize.validate_batch(data, sent=False), [])

    def test_broken_lines_are_refused(self):
        envelope, lines = split(app_batch(clean_events()))
        self.assertTrue(sanitize.validate_batch(join(envelope, lines[:-1])))
        self.assertTrue(sanitize.validate_batch(join(envelope, lines + lines)))
        self.assertEqual(sanitize.validate_batch(b"not json\n"), ["envelope: not JSON"])
        self.assertEqual(sanitize.validate_batch(b'{"event":"ella_turn_latency"}\n'), ["envelope: not an `ella_batch`"])

    def test_the_command_writes_batches_that_it_then_accepts(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            raw = tmp / "lab-01.jsonl"
            raw.write_text("".join(json.dumps(v["raw"]) + "\n" for v in VECTORS["events"]), encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                sanitize.main([str(raw), "--batch-out", str(tmp / "out")])
                batches = sorted((tmp / "out").glob("*.jsonl"))
                self.assertEqual(len(batches), 1)
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(sanitize.main(["--validate", str(batches[0])]), 0)
            envelope, _ = split(batches[0].read_bytes())
            self.assertEqual(envelope["policy_excluded_count"], 2)
            self.assertEqual(envelope["unknown_fields_dropped"], 4)
            self.assertEqual(envelope["values_replaced"], 7)
            self.assertNotIn(b"Asha", batches[0].read_bytes())


if __name__ == "__main__":
    unittest.main()
