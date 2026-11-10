"""Tests for the weekly report's analytics (telemetry-analytics.py).

  npm run test:telemetry
"""
import importlib.util
import json
import tempfile
import unittest
from datetime import timedelta
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


def recap(at, talk, **extra):
    return clean(at, event="ella_assessment", talk=talk, talk_type=None, **{"kind": "talk", **extra})


class TalkLifecycle(unittest.TestCase):
    def lifecycle(self, week_events, all_events=(), since=None):
        return analytics.talk_lifecycle(week_events, all_events, since)

    def counts(self, *args, **kwargs):
        shown = self.lifecycle(*args, **kwargs)
        return {k: shown[k]["value"] for k in ("talks", "completed", "left", "resumed", "unresolved")}

    def test_no_talks_is_all_zero_and_no_rate(self):
        shown = self.lifecycle([])
        self.assertEqual(self.counts([]), {"talks": 0, "completed": 0, "left": 0, "resumed": 0, "unresolved": 0})
        self.assertEqual(shown["completion_rate"]["status"], analytics.UNAVAILABLE)

    def test_each_way_a_talk_ends_is_counted(self):
        events = [
            clean("2026-11-02T06:00:00Z", talk="over", talk_over=True),
            clean("2026-11-02T07:00:00Z", talk="recapped"),
            recap("2026-11-02T07:05:00Z", "recapped"),
            clean("2026-11-02T08:00:00Z", talk="left", notes=["talk_left"]),
            clean("2026-11-02T09:00:00Z", talk="resumed", notes=["no_audio_ahead", "talk_left"]),
            clean("2026-11-03T09:00:00Z", talk="resumed", talk_over=True),
            clean("2026-11-02T10:00:00Z", talk="open"),
        ]
        self.assertEqual(
            self.counts(events), {"talks": 5, "completed": 2, "left": 1, "resumed": 1, "unresolved": 1}
        )

    def test_the_talks_are_the_ones_usage_counts(self):
        events = [clean("2026-11-02T06:00:00Z", talk="t1"), clean("2026-11-02T06:00:00Z", talk="t2", install=INSTALL_B)]
        usage = analytics.usage(events, events, "2026-W45")
        self.assertEqual(self.lifecycle(events)["talks"]["value"], usage["talks"]["value"])

    def test_an_end_from_a_later_week_counts(self):
        turn_ = clean("2026-11-08T18:00:00Z", talk="late")  # Sunday night, IST
        later = recap("2026-11-08T18:40:00Z", "late")  # Monday, the week after
        self.assertEqual(self.counts([turn_], [turn_, later])["completed"], 1)
        self.assertEqual(self.counts([turn_], [turn_])["unresolved"], 1)

    def test_a_recap_alone_is_no_talk_of_the_week(self):
        self.assertEqual(self.counts([recap("2026-11-02T06:00:00Z", "no-turns")])["talks"], 0)

    def test_talk_over_false_or_a_note_of_another_kind_is_no_end(self):
        events = [
            clean("2026-11-02T06:00:00Z", talk="t", talk_over=False, notes=["flagged"]),
        ]
        self.assertEqual(self.counts(events)["unresolved"], 1)

    def test_left_and_resumed_are_floors_and_rates_wait_for_a_release(self):
        shown = self.lifecycle([clean("2026-11-02T06:00:00Z", talk_over=True)])
        self.assertEqual(shown["left"]["status"], analytics.HISTORICALLY_LIMITED)
        self.assertEqual(shown["resumed"]["status"], analytics.HISTORICALLY_LIMITED)
        self.assertEqual(shown["completed"]["status"], analytics.DERIVED)
        for rate in ("completion_rate", "left_rate"):
            self.assertEqual(shown[rate]["status"], analytics.UNAVAILABLE)
            self.assertIn("#45", shown[rate]["note"])
        self.assertIsNone(analytics.TALK_LEFT_SINCE, "no release is yet known to send talk_left")

    def test_once_a_release_is_known_rates_count_only_its_talks(self):
        events = [
            clean("2026-11-02T06:00:00Z", talk="old", app_version="0.1.27"),  # unresolved, old build
            clean("2026-11-02T07:00:00Z", talk="done", app_version="0.1.28", talk_over=True),
            clean("2026-11-02T08:00:00Z", talk="gone", app_version="0.1.28", notes=["talk_left"]),
            clean("2026-11-02T09:00:00Z", talk="back", app_version="0.1.28", notes=["talk_left"]),
            clean("2026-11-03T09:00:00Z", talk="back", app_version="0.1.29", talk_over=True),
            clean("2026-11-02T10:00:00Z", talk="mixed", app_version="0.1.27"),
            clean("2026-11-02T10:01:00Z", talk="mixed", app_version="0.1.28", talk_over=True),
        ]
        shown = self.lifecycle(events, since="0.1.28")
        self.assertEqual(shown["completion_rate"]["value"], round(2 / 3, 4))
        self.assertEqual(shown["left_rate"]["value"], round(2 / 3, 4))
        self.assertIn("3 talks", shown["completion_rate"]["note"])

    def test_versions_compare_as_numbers(self):
        self.assertGreater(analytics.version_key("0.1.10"), analytics.version_key("0.1.9"))
        self.assertIsNone(analytics.version_key("other"))
        self.assertIsNone(analytics.version_key(None))

    def test_no_talk_is_named(self):
        events = [clean("2026-11-02T06:00:00Z", talk="Asha's talk", notes=["talk_left"])]
        shown = json.dumps(self.lifecycle(events))
        self.assertNotIn("Asha", shown)
        self.assertNotIn(sanitize.short_hash("Asha's talk"), shown)


def engine(at, since, server="started", **extra):
    return clean(at, event="ella_engine", talk=None, talk_type=None, since_launch_ms=since, llm_server=server, **extra)


def cloud_runs():
    return [{"backend": "cloud", "ttft_ms": 300, "ms": 600}]


class Latency(unittest.TestCase):
    def latency(self, events):
        return analytics.analyse(events)["latency"]

    def test_nothing_measured_is_a_count_of_zero(self):
        shown = self.latency([])
        self.assertEqual(shown["turns"]["value"], 0)
        self.assertEqual(shown["wait_ms"]["value"], {"count": 0, "p50": None, "p75": None, "p95": None, "excluded": 0, "of": 0, "sufficient": False})
        self.assertEqual(shown["update_ms_by_step"]["value"], {})

    def test_only_voice_turns_that_succeeded_are_timed_word_of_the_day_left_out(self):
        events = [
            clean("2026-11-02T06:00:00Z", speech_ms=900),
            clean("2026-11-02T06:01:00Z", speech_ms=5000, status="error", error="NO_SPEECH_DETECTED: x"),
            clean("2026-11-02T06:02:00Z", speech_ms=5000, kind="text"),
            clean("2026-11-02T06:03:00Z", speech_ms=5000, talk_type="word"),
        ]
        shown = self.latency(events)
        self.assertEqual(shown["turns"]["value"], 1)
        self.assertEqual((shown["wait_ms"]["value"]["count"], shown["wait_ms"]["value"]["p50"]), (1, 900))

    def test_a_missing_timing_is_not_measured_and_never_guessed(self):
        events = [clean("2026-11-02T06:00:00Z", speech_ms=700), clean("2026-11-02T06:01:00Z", total_ms=4000)]
        events[1].pop("speech_ms", None)
        wait = self.latency(events)["wait_ms"]["value"]
        self.assertEqual((wait["count"], wait["of"], wait["p50"]), (1, 2, 700), "the total is not taken for the wait")
        tts = self.latency(events)["tts_first_audio_ms"]["value"]
        self.assertEqual((tts["count"], tts["of"]), (0, 2))

    def test_the_split_by_who_answered_is_the_weekly_reports(self):
        events = [
            clean("2026-11-02T06:00:00Z", speech_ms=500, total_ms=600, llm_runs=cloud_runs()),
            clean("2026-11-02T06:01:00Z", speech_ms=4000, total_ms=4200),
            clean("2026-11-02T06:02:00Z", speech_ms=6000, total_ms=6200),
        ]
        shown = self.latency(events)
        self.assertEqual(shown["wait_ms_cloud"]["value"]["p50"], 500)
        self.assertEqual(shown["wait_ms_laptop"]["value"]["count"], 2)
        self.assertEqual(shown["total_ms_cloud"]["status"], analytics.DERIVED)
        self.assertEqual(shown["total_ms"]["status"], analytics.DIRECT)
        self.assertEqual(
            shown["wait_ms_cloud"]["value"]["count"] + shown["wait_ms_laptop"]["value"]["count"],
            shown["wait_ms"]["value"]["count"],
        )

    def test_each_stage_is_its_own_field(self):
        event = clean(
            "2026-11-02T06:00:00Z", stt_ms=120, llm_ttft_ms=800, llm_completion_ms=1500, tts_first_audio_ms=300,
            warm_up={"began_ms_before": 9000, "ms": 4000, "evaluated_tokens": 1000},
        )
        shown = self.latency([event])
        for name, value in (("stt_ms", 120), ("llm_ttft_ms", 800), ("llm_completion_ms", 1500),
                            ("tts_first_audio_ms", 300), ("warm_up_ms", 4000)):
            self.assertEqual(shown[name]["value"]["p50"], value, name)
            self.assertEqual(shown[name]["status"], analytics.DIRECT, name)

    def test_recaps_that_succeeded_are_timed_word_of_the_day_left_out(self):
        events = [
            recap("2026-11-02T06:00:00Z", "t1", total_ms=5000, status="ok"),
            recap("2026-11-02T06:01:00Z", "t2", total_ms=9000, status="error", error="x"),
            recap("2026-11-02T06:02:00Z", "t3", total_ms=100, status="ok", kind="word"),
        ]
        recap_ms = self.latency(events)["recap_total_ms"]["value"]
        self.assertEqual((recap_ms["count"], recap_ms["p50"]), (1, 5000))

    def test_cold_start_keeps_the_weekly_rule_and_shows_ellas_own_server_apart(self):
        events = [
            engine("2026-11-02T06:00:00Z", 15000),
            engine("2026-11-02T07:00:00Z", 2000, server="external"),
            engine("2026-11-02T08:00:00Z", 99000, error="llama-server did not start"),
        ]
        shown = self.latency(events)
        self.assertEqual(shown["cold_start_ms"]["value"]["count"], 2, "an engine with an error is left out")
        self.assertEqual(shown["cold_start_ms_started"]["value"]["count"], 1)
        self.assertEqual(shown["cold_start_ms_started"]["value"]["p50"], 15000)

    def test_update_steps_are_timed_apart(self):
        events = [
            clean(f"2026-11-02T06:0{i}:00Z", event="ella_update", talk=None, talk_type=None, step=step, ms=ms)
            for i, (step, ms) in enumerate((("up_to_date", 900), ("up_to_date", 1100), ("check_failed", 70)))
        ]
        by_step = self.latency(events)["update_ms_by_step"]["value"]
        self.assertEqual(list(by_step), ["check_failed", "up_to_date"])
        self.assertEqual((by_step["up_to_date"]["count"], by_step["up_to_date"]["p50"]), (2, report.pct([900, 1100], 50)))

    def test_it_agrees_with_the_weekly_report(self):
        events = [
            clean(f"2026-11-02T06:{i:02d}:00Z", speech_ms=100 * i + 50, total_ms=100 * i + 90,
                  **({"llm_runs": cloud_runs()} if i % 3 == 0 else {}))
            for i in range(40)
        ] + [engine("2026-11-02T05:00:00Z", 12000), recap("2026-11-02T07:00:00Z", "t", total_ms=4000, status="ok")]
        mine, theirs = self.latency(events), weekly.summarize(events)["latency"]
        for key, old in (("wait_ms", "wait_ms"), ("wait_ms_laptop", "wait_ms_local"), ("wait_ms_cloud", "wait_ms_cloud"),
                         ("total_ms", "total_ms"), ("recap_total_ms", "recap_total_ms"), ("cold_start_ms", "cold_start_ms")):
            shown = mine[key]["value"]
            self.assertEqual((shown["count"], shown["p50"], shown["p95"]),
                             (theirs[old]["n"], theirs[old]["p50"], theirs[old]["p95"]), key)

    def test_the_same_events_in_any_order_give_the_same_latency(self):
        events = [clean(f"2026-11-02T06:{i:02d}:00Z", speech_ms=(i * 37) % 11 * 100) for i in range(20)]
        self.assertEqual(json.dumps(self.latency(events)), json.dumps(self.latency(list(reversed(events)))))


health_tests = load("test_telemetry_health.py", "test_telemetry_health")


class Reliability(unittest.TestCase):
    def reliability(self, week_events, all_events=(), ledger=None, week=None, reading=None):
        return analytics.analyse(week_events, all_events, ledger, week, reading)["reliability"]

    def app(self, *args):
        return {k: v["value"] for k, v in self.reliability(*args)["app"].items()}

    def test_the_sections_and_nothing_given_is_zero_or_unavailable(self):
        shown = self.reliability([])
        self.assertEqual(list(shown), ["app", "collection", "reading"])
        self.assertEqual(self.app([])["system_error_rate"], None)
        self.assertEqual(shown["collection"]["batch_record"]["status"], analytics.UNAVAILABLE)
        self.assertEqual(shown["reading"]["counts"]["status"], analytics.UNAVAILABLE)

    def test_no_speech_is_counted_apart_from_system_errors(self):
        events = [
            clean("2026-11-02T06:00:00Z"),
            clean("2026-11-02T06:01:00Z", status="error", error="NO_SPEECH_DETECTED: quiet", failed_at="stt"),
            clean("2026-11-02T06:02:00Z", status="error", error="engine request failed", failed_at="llm:start"),
            clean("2026-11-02T06:03:00Z", status="error", error="x", talk_type="word"),
        ]
        shown = self.app(events)
        self.assertEqual((shown["turns"], shown["turn_errors"], shown["no_speech"], shown["system_errors"]), (3, 2, 1, 1))
        self.assertEqual(shown["system_error_rate"], round(1 / 3, 4))
        self.assertEqual(shown["system_errors_by_stage"], {"llm:start": 1})
        self.assertNotIn("NO_SPEECH_DETECTED", shown["system_errors_by_code"])

    def test_it_agrees_with_the_weekly_reports_volume(self):
        events = [
            clean("2026-11-02T06:00:00Z", llm_runs=[{"backend": "cloud", "failed": True}, {"ms": 600}]),
            clean("2026-11-02T06:01:00Z", stt_fallback_from="canary"),
            clean("2026-11-02T06:02:00Z", status="error", error="NO_SPEECH_DETECTED: x"),
            clean("2026-11-02T06:03:00Z", status="error", error="boom", failed_at="tts"),
            recap("2026-11-02T06:05:00Z", "talk-1", status="error", error="x", judges=[{"status": "error"}]),
        ]
        mine, theirs = self.app(events), weekly.summarize(events)["volume"]
        for key, old in (("turns", "turns"), ("turn_errors", "errors"), ("no_speech", "no_speech"),
                         ("system_errors", "system_errors"), ("system_error_rate", "system_error_rate"),
                         ("cloud_failed_runs", "cloud_failed_runs"), ("stt_fallbacks", "stt_fallbacks"),
                         ("recaps", "recaps"), ("recap_errors", "recap_errors"), ("failed_judges", "failed_judges")):
            self.assertEqual(mine[key], theirs[old], key)

    def test_a_fallback_or_the_cloud_down_is_no_error(self):
        events = [
            clean("2026-11-02T06:00:00Z", stt_fallback_from="canary"),
            clean("2026-11-02T06:01:00Z", event="ella_cloud", talk=None, talk_type=None, up=False, reason="HTTP 503"),
            clean("2026-11-02T06:02:00Z", event="ella_cloud", talk=None, talk_type=None, up=True),
        ]
        shown = self.app(events)
        self.assertEqual((shown["stt_fallbacks"], shown["cloud_down"], shown["system_errors"]), (1, 1, 0))
        self.assertEqual(shown["cloud_down_by_reason"], {"http_503": 1})

    def test_engine_and_update_errors_are_counted(self):
        events = [
            engine("2026-11-02T06:00:00Z", 9000, server="failed", error="llama-server did not start"),
            engine("2026-11-02T07:00:00Z", 9000),
            clean("2026-11-02T08:00:00Z", event="ella_update", talk=None, talk_type=None, step="check_failed", error="x"),
            clean("2026-11-02T08:01:00Z", event="ella_update", talk=None, talk_type=None, step="up_to_date"),
        ]
        shown = self.app(events)
        self.assertEqual((shown["engine_errors"], shown["update_errors_by_step"]), (1, {"check_failed": 1}))

    def test_a_recap_without_turns_is_a_collection_sign_not_a_talk(self):
        week = [recap("2026-11-02T06:00:00Z", "lost-turns"), clean("2026-11-02T07:00:00Z", talk="t"),
                recap("2026-11-02T07:05:00Z", "t"), recap("2026-11-02T08:00:00Z", "earlier")]
        before = [clean("2026-10-28T06:00:00Z", talk="earlier")]
        shown = self.reliability(week, before + week)["app"]["recap_only_talks"]
        self.assertEqual((shown["value"], shown["status"]), (1, analytics.DERIVED))
        self.assertIn("not counted in talk_lifecycle", shown["note"])
        self.assertEqual(analytics.analyse(week, before + week)["talk_lifecycle"]["talks"]["value"], 1)

    def test_collection_is_telemetry_healths_own_and_names_no_install(self):
        record = health_tests.record
        monday = health_tests.MONDAY
        ledger = [
            record(1, monday, dropped_event_count=2, dropped_total=2,
                   dropped_by_reason={"queue_full": 2, "unreadable_line": 0, "write_failed": 0}),
            record(3, monday + timedelta(days=1), dropped_total=2, send={"attempts": 4}),
            record(1, monday, install=health_tests.OTHER_INSTALL, outcome="duplicate"),
        ]
        week = health_tests.WEEK
        h = analytics.health.summary(ledger, week)
        shown = self.reliability([], (), ledger, week)["collection"]
        values = {k: v["value"] for k, v in shown.items()}
        self.assertEqual(values["batches"], h["batches"])
        self.assertEqual((values["dropped"], values["drop_rate"]), (h["loss"]["dropped"], h["loss"]["drop_rate"]))
        self.assertEqual(values["sequence_gaps_pending"], len(h["sequence"]["pending_gaps"]))
        self.assertEqual(values["sequence_gaps_pending"], 1)
        self.assertEqual(values["upload_attempts"], h["delivery"]["attempts"])
        self.assertEqual(values["flags"], [f["check"] for f in h["flags"]])
        self.assertEqual(shown["dropped"]["status"], analytics.DIRECT)
        self.assertEqual(shown["drop_rate"]["status"], analytics.DERIVED)
        text = json.dumps(shown)
        for install in (health_tests.INSTALL, health_tests.OTHER_INSTALL):
            self.assertNotIn(install[:8], text)

    def test_a_ledger_without_a_week_is_unavailable(self):
        shown = self.reliability([], (), [], None)["collection"]
        self.assertEqual(shown["batch_record"]["status"], analytics.UNAVAILABLE)

    def test_what_reading_left_out_is_shown_as_read(self):
        _, counts = analytics.read([3, {"event": "ella_debug", "timestamp": "2026-11-02T06:00:00Z"}])
        shown = self.reliability([], reading=counts)["reading"]
        self.assertEqual({k: v["value"] for k, v in shown.items()}, counts)
        self.assertEqual((counts["unreadable"], counts["policy_excluded_count"]), (1, 1))


def launch_line(at, install, os="macOS 26.6.2", cpu="Apple M1 Pro", ram_mb=16384, arch="aarch64", place="applications",
                launch=None):
    raw = {
        "event": "ella_launch", "schema_version": 2, "timestamp": at, "app_version": "0.1.27",
        "launch_id": launch or f"launch-{install}", "os": os, "arch": arch, "cpu": cpu, "ram_mb": ram_mb,
        "cores_physical": 8, "cores_logical": 8, "install": {"bundle": "app", "place": place, "also_at": []},
    }
    return analytics.read([{**raw, "install_id": install}])[0][0]


def laptop_turn(at, install, speech_ms, launch=None, version="0.1.27"):
    return clean(at, install=install, speech_ms=speech_ms, launch_id=launch or f"launch-{install}", app_version=version)


INSTALLS = [c * 16 for c in "abcdef"]


class Platform(unittest.TestCase):
    def platform(self, week_events, all_events=()):
        return {k: v["value"] for k, v in analytics.analyse(week_events, all_events)["platform"].items()}

    def fleet(self, n_mac=3, n_windows=2, turns_each=12):
        events = []
        for i, install in enumerate(INSTALLS[: n_mac + n_windows]):
            windows = i >= n_mac
            events.append(launch_line(
                "2026-11-02T05:00:00Z", install,
                os="Windows 11 build 26200.9711 (25H2)" if windows else "macOS 26.6.2",
                cpu="11th Gen Intel(R) Core(TM) i7-11800H @ 2.30GHz" if windows else "Apple M1 Pro",
                arch="x86_64" if windows else "aarch64",
            ))
            events += [laptop_turn(f"2026-11-02T06:{t:02d}:{i:02d}Z", install, 1000 + 100 * t) for t in range(turns_each)]
        return events

    def test_nothing_is_no_groups(self):
        shown = self.platform([])
        self.assertEqual(shown["installs_by_os_family"], {})
        self.assertEqual(shown["wait_ms_laptop_by_os_family"], {})

    def test_classes_never_the_strings_as_written(self):
        self.assertEqual(analytics.os_family("Windows 11 build 26200.9711 (25H2)"), "Windows")
        self.assertEqual(analytics.os_family("macOS 26.6.2"), "macOS")
        self.assertEqual(analytics.os_family("FreeBSD 14"), "other")
        self.assertEqual(analytics.os_family(None), "unknown")
        self.assertEqual(analytics.cpu_class("11th Gen Intel(R) Core(TM) i7-11800H @ 2.30GHz"), "Intel Core i7")
        self.assertEqual(analytics.cpu_class("Apple M1 Pro"), "Apple M-series")
        self.assertEqual(analytics.cpu_class("AMD Ryzen 5 5500U with Radeon Graphics"), "AMD Ryzen 5")
        self.assertEqual(analytics.cpu_class("other"), "unknown", "a value the policy replaced")
        self.assertEqual(analytics.ram_band(16085), "8–16 GB")
        self.assertEqual(analytics.ram_band(32768), "16–32 GB")
        self.assertEqual(analytics.ram_band(-1), "unknown")
        text = json.dumps(self.platform(self.fleet()))
        for written in ("26200", "11800H", "2.30GHz", "M1 Pro", "26.6.2", "16384"):
            self.assertNotIn(written, text)

    def test_a_group_of_fewer_than_three_installs_is_folded_not_dropped(self):
        shown = self.platform(self.fleet(n_mac=3, n_windows=2))
        self.assertEqual(shown["installs_by_os_family"], {"macOS": 3, "small groups": 2})
        self.assertEqual(sum(shown["installs_by_os_family"].values()), 5, "every install is still counted")
        self.assertEqual(shown["installs_by_cpu_class"], {"Apple M-series": 3, "small groups": 2})
        self.assertEqual(shown["installs_by_install_place"], {"applications": 5})

    def test_a_groups_wait_is_shown_only_with_enough_installs_and_turns(self):
        shown = self.platform(self.fleet(n_mac=3, n_windows=2, turns_each=12))
        by_os = shown["wait_ms_laptop_by_os_family"]
        self.assertEqual(list(by_os), ["macOS", "small groups"])
        self.assertEqual(by_os["macOS"]["count"], 36)
        self.assertTrue(by_os["macOS"]["sufficient"])
        self.assertEqual(by_os["small groups"]["count"], 24)
        self.assertNotIn("p50", by_os["small groups"])
        few = self.platform(self.fleet(n_mac=3, n_windows=0, turns_each=5))["wait_ms_laptop_by_os_family"]
        self.assertEqual(list(few), ["small groups"], "3 installs but 15 turns: too few")

    def test_only_turns_the_laptop_answered_are_timed_by_platform(self):
        events = self.fleet(n_mac=3, n_windows=0)
        events.append(clean("2026-11-02T07:00:00Z", install=INSTALLS[0], speech_ms=200,
                            llm_runs=[{"backend": "cloud", "ms": 300}]))
        self.assertEqual(self.platform(events)["wait_ms_laptop_by_os_family"]["macOS"]["count"], 36)

    def test_a_turn_takes_its_own_launchs_laptop(self):
        events = [launch_line("2026-11-02T05:00:00Z", i, launch=f"l-{i}") for i in INSTALLS[:3]]
        events.append(launch_line("2026-11-02T05:00:00Z", INSTALLS[3], os="Windows 11", launch="l-win"))
        # A turn whose install is a Mac's but whose launch line is the Windows one.
        events += [laptop_turn(f"2026-11-02T06:{t:02d}:00Z", INSTALLS[t % 3], 900, launch="l-win") for t in range(30)]
        by_os = self.platform(events)["wait_ms_laptop_by_os_family"]
        self.assertEqual(list(by_os), ["Windows"])

    def test_an_install_is_what_its_newest_line_before_the_week_ended_says(self):
        old = launch_line("2026-10-20T05:00:00Z", INSTALLS[0], ram_mb=8192, launch="old")
        new = launch_line("2026-11-02T05:00:00Z", INSTALLS[0], ram_mb=16384, launch="new")
        later = launch_line("2026-11-20T05:00:00Z", INSTALLS[0], ram_mb=65536, launch="later")
        week = [new, clean("2026-11-02T06:00:00Z", install=INSTALLS[0])]
        _, by_install = analytics.profiles(week, [old, later] + week)
        self.assertEqual(by_install[INSTALLS[0]]["event_id"], new["event_id"])

    def test_an_install_with_no_launch_line_is_unknown(self):
        events = [clean(f"2026-11-02T06:00:0{i}Z", install=install) for i, install in enumerate(INSTALLS[:3])]
        self.assertEqual(self.platform(events)["installs_by_os_family"], {"unknown": 3})

    def test_versions_are_counted_by_install_and_by_turn(self):
        events = []
        for i, install in enumerate(INSTALLS[:4]):
            events.append(clean(f"2026-11-02T06:00:0{i}Z", install=install, app_version="0.1.27"))
        events.append(clean("2026-11-02T07:00:00Z", install=INSTALLS[0], app_version="0.1.28"))
        shown = self.platform(events)
        self.assertEqual(shown["installs_by_app_version"], {"0.1.27": 4, "small groups": 1})
        self.assertEqual(shown["turns_by_app_version"], {"0.1.27": 4, "small groups": 1})

    def test_no_install_is_named_and_order_is_fixed(self):
        events = self.fleet()
        text = json.dumps(self.platform(events))
        for install in INSTALLS:
            self.assertNotIn(install[:8], text)
        self.assertEqual(text, json.dumps(self.platform(list(reversed(events)))))

    def test_cold_start_from_three_launches_is_marked_too_few(self):
        events = [engine(f"2026-11-02T0{i}:00:00Z", 8000 + i) for i in range(3)]
        shown = analytics.analyse(events)["latency"]["cold_start_ms_started"]
        self.assertFalse(shown["value"]["sufficient"])
        self.assertEqual(shown["value"]["count"], 3, "the observations are kept")
        self.assertIn("too few", shown["note"])


if __name__ == "__main__":
    unittest.main()
