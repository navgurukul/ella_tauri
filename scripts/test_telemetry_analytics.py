"""Tests for the weekly report's analytics (telemetry-analytics.py).

  npm run test:telemetry
"""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


analytics = load("telemetry-analytics.py", "telemetry_analytics")
report = load("latency-report.py", "latency_report")
sanitize = load("latency-sanitize.py", "latency_sanitize")
weekly = load("latency-weekly.py", "latency_weekly")

LAUNCH = "d0ec3333-119c-4ae0-ac10-b69071ab640a"
NO_COUNTS = {"policy_excluded_count": 0, "values_replaced": 0, "unknown_fields_dropped": 0}


def turn(at, **extra):
    """A turn as a laptop's own latency.jsonl holds it, before sanitizing."""
    event = {
        "event": "ella_turn_latency",
        "schema_version": 2,
        "timestamp": at,
        "app_version": "0.1.27",
        "launch_id": LAUNCH,
        "interaction_id": f"i-{at}",
        "status": "ok",
        "kind": "voice",
        "session_id": "talk-1",
        "stt_ms": 100,
        "total_ms": 1000,
    }
    event.update(extra)
    return event


def write(path, lines):
    path.write_text("".join(line + "\n" for line in lines), encoding="utf-8")
    return path


def read_files(*paths):
    return analytics.read(weekly.read_all(list(paths)))


class Model(unittest.TestCase):
    def test_a_metric_says_how_it_was_come_by(self):
        self.assertEqual(analytics.metric(3, analytics.DIRECT), {"value": 3, "status": "direct"})
        self.assertEqual(
            analytics.metric(2, analytics.HISTORICALLY_LIMITED, "a floor"),
            {"value": 2, "status": "historically_limited", "note": "a floor"},
        )
        with self.assertRaises(ValueError):
            analytics.metric(1, "guessed")

    def test_an_unavailable_metric_has_a_reason_and_no_value(self):
        self.assertEqual(
            analytics.unavailable("not tracked"),
            {"value": None, "status": "unavailable", "note": "not tracked"},
        )
        with self.assertRaises(ValueError):
            analytics.metric(0, analytics.UNAVAILABLE)

    def test_the_sections_are_fixed_and_in_order(self):
        self.assertEqual(
            list(analytics.analyse([])),
            ["usage", "talk_lifecycle", "latency", "reliability", "platform"],
        )


class Distribution(unittest.TestCase):
    def test_nothing_measured_has_a_count_of_zero_and_no_percentiles(self):
        self.assertEqual(
            analytics.distribution([]),
            {"count": 0, "p50": None, "p75": None, "p95": None, "excluded": 0},
        )

    def test_one_value_is_every_percentile(self):
        self.assertEqual(
            analytics.distribution([420]),
            {"count": 1, "p50": 420, "p75": 420, "p95": 420, "excluded": 0},
        )

    def test_percentiles_are_the_other_reports_own(self):
        values = [100 * (i + 1) for i in range(20)]
        shown = analytics.distribution(reversed(values))
        self.assertEqual((shown["count"], shown["p50"], shown["p75"], shown["p95"]), (20, 1100, 1500, 1900))
        for p in (50, 75, 95):
            self.assertEqual(shown[f"p{p}"], report.pct(values, p))

    def test_missing_values_are_skipped_and_bad_ones_counted_apart(self):
        shown = analytics.distribution([300, None, 100, "slow", -5, float("nan"), True, 200])
        self.assertEqual(shown["count"], 3)
        self.assertEqual(shown["p50"], 200)
        self.assertEqual(shown["excluded"], 4, "text, a negative, NaN and a bool")


class Reader(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_laptops_own_log_is_read_as_it_would_leave_the_laptop(self):
        log = write(self.dir / "latency.jsonl", [json.dumps(turn("2026-11-02T06:00:00Z"))])
        events, counts = read_files(log)
        self.assertEqual(len(events), 1)
        event = events[0]
        self.assertEqual(event, sanitize.Sanitizer().event(turn("2026-11-02T06:00:00Z")))
        self.assertNotIn("launch_id", event)
        self.assertNotIn("session_id", event)
        self.assertNotIn(LAUNCH, json.dumps(event))
        self.assertEqual(event["talk_id"], sanitize.short_hash("talk-1"))
        self.assertEqual(counts, {"events_read": 1, "unreadable": 0, **NO_COUNTS})

    def test_events_already_sanitized_come_through_as_they_are(self):
        clean = [sanitize.Sanitizer().event(turn(f"2026-11-02T06:00:0{i}Z")) for i in range(3)]
        events, counts = analytics.read(clean)
        self.assertEqual(events, clean)
        self.assertEqual(counts, {"events_read": 3, "unreadable": 0, **NO_COUNTS})

    def test_events_come_oldest_first(self):
        late, early = turn("2026-11-02T07:00:00Z"), turn("2026-11-02T06:00:00Z")
        events, _ = analytics.read([late, early])
        self.assertEqual([e["timestamp"] for e in events], [early["timestamp"], late["timestamp"]])

    def test_lines_that_are_no_event_are_skipped_not_a_crash(self):
        log = write(
            self.dir / "latency.jsonl",
            ["{cut off mid-wr", "3", '"text"', "[1, 2]", "null", "", json.dumps(turn("2026-11-02T06:00:00Z"))],
        )
        events, counts = read_files(log)
        self.assertEqual(len(events), 1)
        self.assertEqual(counts["unreadable"], 0, "read_all skips them before the analytics sees them")

    def test_items_that_are_no_event_are_counted_unreadable(self):
        events, counts = analytics.read([3, "text", [1], None, turn("2026-11-02T06:00:00Z")])
        self.assertEqual(len(events), 1)
        self.assertEqual(counts["unreadable"], 4)

    def test_an_event_that_breaks_the_sanitizer_is_left_out_alone(self):
        class Odd(dict):
            def items(self):
                raise RuntimeError("odd")

        odd = Odd(turn("2026-11-02T06:00:00Z"))
        events, counts = analytics.read([odd, turn("2026-11-02T06:00:01Z")])
        self.assertEqual(len(events), 1)
        self.assertEqual(counts["unreadable"], 1)

    def test_what_the_policy_does_not_allow_is_left_behind_and_counted(self):
        raw = [
            {"event": "ella_debug", "timestamp": "2026-11-02T06:00:00Z"},
            {"event": "ella_cloud", "up": True},
            {"event": "ella_cloud", "timestamp": "yesterday", "up": True},
            turn("2026-11-02T06:00:00Z", transcript="my name is Asha", prompt="You are Ella", stt_ms="slow"),
        ]
        events, counts = analytics.read(raw)
        self.assertEqual(len(events), 1)
        shown = json.dumps(events)
        for private in ("Asha", "You are Ella", "transcript", "prompt"):
            self.assertNotIn(private, shown)
        self.assertNotIn("stt_ms", events[0], "a number that is none leaves not at all")
        self.assertEqual(
            counts,
            {"events_read": 1, "unreadable": 0, "policy_excluded_count": 3, "values_replaced": 1,
             "unknown_fields_dropped": 2},
        )

    def test_an_event_is_counted_once_whichever_files_it_came_in(self):
        raw = [turn("2026-11-02T06:00:00Z"), turn("2026-11-02T06:00:01Z")]
        clean = [sanitize.Sanitizer().event(e) for e in raw]
        log = write(self.dir / "latency.jsonl", [json.dumps(e) for e in raw])
        copy = write(self.dir / "copy.jsonl", [json.dumps(e) for e in raw])
        sent = write(self.dir / "sent.jsonl", [json.dumps(e) for e in clean])
        events, counts = read_files(log, copy, sent)
        self.assertEqual(len(events), 2)
        self.assertEqual(counts["events_read"], 2)

    def test_a_batchs_events_keep_its_install(self):
        clean = [sanitize.Sanitizer().event(turn("2026-11-02T06:00:00Z"))]
        path = self.dir / "batch.jsonl"
        path.write_bytes(sanitize.manual_batch(clean, "lab-01", NO_COUNTS))
        events, counts = read_files(path)
        self.assertEqual([e["install_id"] for e in events], [sanitize.short_hash("manual:lab-01")])
        self.assertEqual(counts["unknown_fields_dropped"], 0)

    def test_an_install_is_a_hash_never_a_label(self):
        stage_one = sanitize.Sanitizer().event(turn("2026-11-02T06:00:00Z"), "lab-win-hp-02")
        bad = {**turn("2026-11-02T06:00:01Z"), "install_id": "Asha's laptop"}
        none = turn("2026-11-02T06:00:02Z")
        events, _ = analytics.read([stage_one, bad, none])
        self.assertEqual(events[0]["install_id"], sanitize.short_hash("manual:lab-win-hp-02"))
        self.assertNotIn("install_id", events[1])
        self.assertNotIn("install_id", events[2])
        shown = json.dumps(events)
        self.assertNotIn("lab-win-hp-02", shown)
        self.assertNotIn("Asha", shown)

    def test_reading_changes_no_file(self):
        log = write(self.dir / "latency.jsonl", [json.dumps(turn("2026-11-02T06:00:00Z")), "{cut"])
        before = (log.read_bytes(), log.stat().st_mtime_ns)
        read_files(self.dir)
        self.assertEqual((log.read_bytes(), log.stat().st_mtime_ns), before)
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["latency.jsonl"])

    def test_nothing_to_read_is_no_events(self):
        empty = write(self.dir / "latency.jsonl", [])
        self.assertEqual(read_files(empty), ([], {"events_read": 0, "unreadable": 0, **NO_COUNTS}))
        self.assertEqual(analytics.read([]), ([], {"events_read": 0, "unreadable": 0, **NO_COUNTS}))


INSTALL_A, INSTALL_B = "a" * 16, "b" * 16


def clean(at, install=INSTALL_A, talk="talk-1", talk_type="free", event="ella_turn_latency", **extra):
    """A sanitized event of one install's, as the reader gives it."""
    raw = {**turn(at, session_id=talk, talk=talk_type), **extra}
    raw["event"] = event
    if talk_type is None:
        raw.pop("talk")
    if talk is None:
        raw.pop("session_id")
    events, _ = analytics.read([{**raw, "install_id": install} if install else raw])
    return events[0]


def values(section):
    return {name: shown["value"] for name, shown in section.items()}


class Usage(unittest.TestCase):
    WEEK = "2026-W45"  # Mon 2 Nov to Sun 8 Nov 2026, IST

    def usage(self, week_events, all_events=None):
        return analytics.analyse(week_events, week_events if all_events is None else all_events, week=self.WEEK)["usage"]

    def test_nothing_used_is_zero_not_missing(self):
        shown = values(self.usage([]))
        self.assertEqual(
            {k: v for k, v in shown.items() if k != "turns_per_talk"},
            {
                "installs": 0, "new_installs": 0, "launches": 0, "launch_lines": 0, "turns": 0,
                "turns_by_talk_type": {}, "talks": 0, "talks_by_type": {}, "recaps": 0,
                "active_install_days": 0, "installs_by_active_days": {},
            },
        )
        self.assertEqual(shown["turns_per_talk"]["count"], 0)

    def test_a_week_of_use_is_counted(self):
        events = [
            clean("2026-11-02T06:00:00Z", talk="t1"),
            clean("2026-11-02T06:01:00Z", talk="t1"),
            clean("2026-11-02T06:02:00Z", talk="t1"),
            clean("2026-11-03T06:00:00Z", talk="t2", talk_type="lesson"),
            clean("2026-11-03T06:00:00Z", install=INSTALL_B, talk="t3", talk_type="word"),
            clean("2026-11-03T06:05:00Z", event="ella_assessment", talk="t3", talk_type=None),
            clean("2026-11-02T05:59:00Z", event="ella_launch", talk=None, talk_type=None),
        ]
        shown = values(self.usage(events))
        self.assertEqual((shown["installs"], shown["launches"], shown["launch_lines"]), (2, 1, 1))
        self.assertEqual((shown["turns"], shown["talks"], shown["recaps"]), (5, 3, 1))
        self.assertEqual(shown["turns_by_talk_type"], {"free": 3, "lesson": 1, "word": 1})
        self.assertEqual(shown["talks_by_type"], {"free": 1, "lesson": 1, "word": 1})
        self.assertEqual(
            {k: shown["turns_per_talk"][k] for k in ("count", "p50", "p95")}, {"count": 3, "p50": 1, "p95": 3}
        )
        self.assertEqual(shown["active_install_days"], 3)
        self.assertEqual(shown["installs_by_active_days"], {"1": 1, "2": 1})

    def test_an_active_day_is_an_ist_day_with_a_turn(self):
        events = [
            clean("2026-11-02T18:00:00Z"),  # 23:30 IST, Monday
            clean("2026-11-02T18:40:00Z"),  # 00:10 IST, Tuesday
            clean("2026-11-05T06:00:00Z", event="ella_launch", talk=None, talk_type=None),
        ]
        shown = values(self.usage(events))
        self.assertEqual(shown["active_install_days"], 2, "a launch alone is no active day")
        self.assertEqual(shown["installs_by_active_days"], {"2": 1})

    def test_a_talk_of_an_old_build_has_an_unknown_type_and_says_so(self):
        shown = self.usage([clean("2026-11-02T06:00:00Z", talk_type=None), clean("2026-11-02T06:01:00Z", talk="t2")])
        self.assertEqual(shown["talks_by_type"]["value"], {"free": 1, "unknown": 1})
        self.assertEqual(shown["talks_by_type"]["status"], analytics.HISTORICALLY_LIMITED)
        self.assertIn("unknown", shown["turns_by_talk_type"]["note"])
        typed = self.usage([clean("2026-11-02T06:00:00Z")])
        self.assertEqual(typed["talks_by_type"]["status"], analytics.DERIVED)

    def test_an_install_is_new_only_in_the_week_it_was_first_seen(self):
        before = clean("2026-10-28T06:00:00Z", install=INSTALL_A)
        week = [clean("2026-11-02T06:00:00Z", install=INSTALL_A), clean("2026-11-02T06:00:00Z", install=INSTALL_B)]
        shown = self.usage(week, [before] + week)
        self.assertEqual(shown["new_installs"]["value"], 1)
        self.assertEqual(shown["new_installs"]["status"], analytics.HISTORICALLY_LIMITED)

    def test_events_with_no_install_are_not_counted_as_one(self):
        shown = self.usage([clean("2026-11-02T06:00:00Z", install=None), clean("2026-11-02T06:01:00Z")])
        self.assertEqual(shown["installs"]["value"], 1)
        self.assertIn("1 events name no install", shown["installs"]["note"])
        self.assertEqual(shown["turns"]["value"], 2)

    def test_turns_with_no_talk_are_turns_but_no_talk(self):
        shown = values(self.usage([clean("2026-11-02T06:00:00Z", talk=None), clean("2026-11-02T06:01:00Z")]))
        self.assertEqual((shown["turns"], shown["talks"]), (2, 1))

    def test_no_install_or_talk_is_named_in_the_usage(self):
        events = [clean("2026-11-02T06:00:00Z", talk="Asha's talk"), clean("2026-11-02T06:01:00Z", install=INSTALL_B)]
        shown = json.dumps(self.usage(events))
        for named in (INSTALL_A, INSTALL_B, "Asha", sanitize.short_hash("Asha's talk")):
            self.assertNotIn(named, shown)

    def test_the_same_events_give_the_same_usage(self):
        events = [clean(f"2026-11-0{d}T06:00:00Z", talk=f"t{d}", talk_type=t) for d, t in ((2, "free"), (3, "lesson"), (4, "free"))]
        self.assertEqual(json.dumps(self.usage(events)), json.dumps(self.usage(list(reversed(events)))))

    def test_without_a_week_new_installs_are_unavailable(self):
        shown = analytics.analyse([clean("2026-11-02T06:00:00Z")])["usage"]
        self.assertEqual(shown["new_installs"]["status"], analytics.UNAVAILABLE)


if __name__ == "__main__":
    unittest.main()
