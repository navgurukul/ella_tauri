"""Tests for latency-sanitize.py and latency-weekly.py.

  npm run test:telemetry
"""
import contextlib
import importlib.util
import io
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


sanitize = load("latency-sanitize.py", "latency_sanitize")
weekly = load("latency-weekly.py", "latency_weekly")

LAUNCH = "d0ec3333-119c-4ae0-ac10-b69071ab640a"


def turn(at, total=1000, status="ok", error=None, launch=LAUNCH, cloud=False, **extra):
    event = {
        "event": "ella_turn_latency",
        "schema_version": 2,
        "timestamp": at,
        "app_version": "0.1.25",
        "launch_id": launch,
        "interaction_id": f"i-{at}",
        "status": status,
        "kind": "voice",
        "session_id": "talk-1",
        "topic": "market",
        "level": "A2",
        "stt_ms": 100,
        "llm_ttft_ms": 300,
        "speech_ms": total - 100,
        "total_ms": total,
        "llm_runs": [{"backend": "cloud", "ttft_ms": 300, "ms": 600}] if cloud else [{"ttft_ms": 300, "ms": 600}],
    }
    if error:
        event["error"] = error
        event["failed_at"] = "stt"
    event.update(extra)
    return event


def write(path, events, tail=""):
    path.write_text("".join(json.dumps(e) + "\n" for e in events) + tail, encoding="utf-8")
    return path


def quietly(function, *args):
    with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()) as out:
        function(*args)
    return out.getvalue()


def week_of_turns(day, count, total, **extra):
    """`count` turns on `day` (YYYY-MM-DD), every one `total` ms."""
    return [turn(f"{day}T06:{i // 60:02d}:{i % 60:02d}.000001+00:00", total, **extra) for i in range(count)]


class Sanitizing(unittest.TestCase):
    def setUp(self):
        self.sanitizer = sanitize.Sanitizer()

    def test_keeps_only_the_allowlist_and_hashes_ids(self):
        raw = turn("2026-10-20T06:00:00+00:00", unknown_new_field="secret", machine={"on_ac": True, "hostname": "x"})
        clean = self.sanitizer.event(raw, "lab-01")
        for gone in ("topic", "level", "interaction_id", "unknown_new_field", "launch_id", "session_id"):
            self.assertNotIn(gone, clean)
        self.assertEqual(clean["machine"], {"on_ac": True})
        self.assertEqual(clean["launch"], sanitize.short_hash(LAUNCH))
        self.assertEqual(clean["talk_id"], sanitize.short_hash("talk-1"))
        self.assertEqual(clean["source"], "lab-01")
        self.assertEqual(clean["total_ms"], 1000)
        self.assertEqual(self.sanitizer.unknown["ella_turn_latency.unknown_new_field"], 1)
        self.assertEqual(self.sanitizer.unknown["ella_turn_latency.machine.hostname"], 1)

    def test_free_text_errors_become_codes(self):
        self.assertEqual(sanitize.error_code("NO_SPEECH_DETECTED: I couldn't hear any words"), "NO_SPEECH_DETECTED")
        self.assertEqual(sanitize.error_code("could not open C:\\Users\\asha\\model.gguf"), "other")
        self.assertIsNone(sanitize.error_code(None))
        clean = self.sanitizer.event(
            {
                "event": "ella_assessment", "timestamp": "2026-10-20T06:00:00+00:00", "status": "ok",
                "judges": [{"judge": "score", "status": "error", "error": "proxy said: the learner wrote ..."}],
            },
            "lab-01",
        )
        self.assertEqual(clean["judges"], [{"judge": "score", "status": "error", "error_code": "other"}])

    def test_the_clouds_reasons_become_codes_and_never_its_words(self):
        cases = {
            "could not connect": "could_not_connect",
            "timed out": "timed_out",
            "HTTP 503": "http_503",
            "HTTP 502: <html>bad gateway</html>": "http_502",
            "cap": "proxy_cap",
            "cap: today's spend has reached the cap": "proxy_cap",
            "upstream_config: DeepSeek refused the request (402): Insufficient Balance": "proxy_upstream_config",
            "no first word in 5 s": "no_first_word",
            "silent for 8 s partway": "silent_partway",
            "the proxy gave no install token": "no_install_token",
            "Connection reset by peer (os error 54)": "other",
            "evil_code: the learner said 'my name is Asha'": "other",
        }
        for text, expected in cases.items():
            with self.subTest(text):
                self.assertEqual(sanitize.reason_code(text), expected)
        down = {
            "event": "ella_cloud", "timestamp": "2026-10-20T06:00:00Z", "launch_id": LAUNCH, "up": False,
            "reason": "upstream_config: DeepSeek refused the request (400): messages[3] said 'hello'",
        }
        clean = self.sanitizer.event(down, "lab-01")
        self.assertNotIn("reason", clean)
        self.assertEqual(clean["reason_code"], "proxy_upstream_config")
        self.assertNotIn("hello", json.dumps(clean))
        up = self.sanitizer.event({**down, "up": True, "reason": ""}, "lab-01")
        self.assertNotIn("reason_code", up)

    def test_an_event_the_allowlist_does_not_name_stays_behind(self):
        self.assertIsNone(self.sanitizer.event({"event": "ella_new_thing", "timestamp": "2026-10-20T06:00:00Z"}, "x"))

    def test_sanitizing_twice_changes_nothing(self):
        once = self.sanitizer.event(turn("2026-10-20T06:00:00+00:00", error="NO_SPEECH_DETECTED: x"), "lab-01")
        self.assertEqual(self.sanitizer.event(once, "other-label"), once)

    def test_weeks_run_monday_to_sunday_in_ist(self):
        # 19:00 UTC on Sunday 18 October is 00:30 IST on Monday the 19th.
        self.assertEqual(sanitize.week_of({"timestamp": "2026-10-18T18:29:59Z"}), "2026-W42")
        self.assertEqual(sanitize.week_of({"timestamp": "2026-10-18T19:00:00Z"}), "2026-W43")

    def test_a_file_must_be_named_after_its_laptop(self):
        for name in ("latency", "Asha Laptop", "a_b"):
            with self.assertRaises(SystemExit):
                sanitize.label_of(Path(f"{name}.jsonl"), None)
        self.assertEqual(sanitize.label_of(Path("lab-win-hp-02.jsonl"), None), "lab-win-hp-02")


class Ingesting(unittest.TestCase):
    def test_cumulative_uploads_are_counted_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            first = week_of_turns("2026-10-20", 3, 1000)
            later = first + week_of_turns("2026-10-21", 2, 1000)
            # The file was copied while Ella wrote to it: its last line is cut.
            write(tmp / "lab-01.jsonl", first, tail='{"event":"ella_turn')
            quietly(sanitize.main, [str(tmp / "lab-01.jsonl"), "--data", str(tmp / "data"), "--ingest-id", "a"])
            write(tmp / "lab-01.jsonl", later)
            quietly(sanitize.main, [str(tmp / "lab-01.jsonl"), "--data", str(tmp / "data"), "--ingest-id", "b"])
            quietly(sanitize.main, [str(tmp / "lab-01.jsonl"), "--data", str(tmp / "data"), "--ingest-id", "c"])

            files = sorted(p.name for p in (tmp / "data" / "2026-W43").iterdir())
            self.assertEqual(files, ["a.jsonl", "b.jsonl"])
            ids = [e["event_id"] for p in (tmp / "data").rglob("*.jsonl") for e in sanitize.read_events(p)]
            self.assertEqual(len(ids), 5)
            self.assertEqual(len(set(ids)), 5)

    def test_a_powershell_copy_reads_the_same(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "lab-01.jsonl"
            path.write_text(json.dumps(turn("2026-10-20T06:00:00Z")) + "\n", encoding="utf-16")
            self.assertEqual(len(sanitize.read_events(path)), 1)


class Reporting(unittest.TestCase):
    def report(self, events, week="2026-W43", previous_report=None):
        return weekly.build(events, week, previous_report)

    def test_no_speech_is_not_a_system_error(self):
        events = week_of_turns("2026-10-20", 8, 1000) + [
            turn("2026-10-20T07:00:00Z", status="error", error="NO_SPEECH_DETECTED: x"),
            turn("2026-10-20T07:01:00Z", status="error", error="LLM_TIMEOUT: y"),
        ]
        volume = self.report(events)["this"]["volume"]
        self.assertEqual((volume["turns"], volume["no_speech"], volume["system_errors"]), (10, 1, 1))
        self.assertEqual(volume["system_error_rate"], 0.1)
        self.assertEqual(volume["system_errors_by_stage"], {"stt": 1})

    def test_percentiles_are_nearest_rank_over_turns_that_succeeded(self):
        events = [turn(f"2026-10-20T06:00:{i:02d}Z", total=100 * (i + 1)) for i in range(20)]
        events.append(turn("2026-10-20T07:00:00Z", total=99999, status="error", error="X_Y: z"))
        latency = self.report(events)["this"]["latency"]["total_ms"]
        self.assertEqual(latency, {"n": 20, "p50": 1100, "p95": 1900, "p99": 2000})

    def test_a_slower_week_is_flagged_only_past_every_threshold(self):
        last = week_of_turns("2026-10-13", 30, 1000)
        cases = {
            "slower": (week_of_turns("2026-10-20", 30, 1300), True),  # +30%, +300 ms
            "under 200 ms": (week_of_turns("2026-10-20", 30, 1150), False),  # +15%, +150 ms
            "too few": (week_of_turns("2026-10-20", 29, 2000), False),
        }
        for name, (this, flagged) in cases.items():
            with self.subTest(name):
                flags = self.report(last + this)["flags"]
                self.assertEqual(any(f["kpi"] == "total_ms" and f["stat"] == "p50" for f in flags), flagged)
        # A quick turn needs the 200 ms too: 400 → 600 ms is just enough.
        quick = self.report(week_of_turns("2026-10-13", 30, 400) + week_of_turns("2026-10-20", 30, 600))
        self.assertTrue(any(f["kpi"] == "total_ms" for f in quick["flags"]))

    def test_a_higher_system_error_rate_is_flagged(self):
        last = week_of_turns("2026-10-13", 40, 1000)
        this = week_of_turns("2026-10-20", 38, 1000) + [
            turn(f"2026-10-20T08:00:0{i}Z", status="error", error="LLM_TIMEOUT: x") for i in range(2)
        ]
        flags = self.report(last + this)["flags"]
        self.assertEqual([f["kpi"] for f in flags], ["system_error_rate"])
        self.assertEqual(flags[0]["delta_pp"], 5.0)

    def test_local_and_cloud_are_reported_apart_and_a_mix_shift_is_noted(self):
        last = week_of_turns("2026-10-13", 10, 800)
        this = week_of_turns("2026-10-20", 5, 800) + [
            turn(f"2026-10-20T09:00:0{i}Z", 2000, cloud=True) for i in range(5)
        ]
        report = self.report(last + this)
        self.assertEqual(report["this"]["latency"]["wait_ms_cloud"]["n"], 5)
        self.assertEqual(report["this"]["latency"]["wait_ms_local"]["p50"], 700)
        self.assertTrue(any("cloud answered 50%" in note for note in report["notes"]))

    def test_word_of_the_day_is_counted_apart_from_the_kpis(self):
        talks = week_of_turns("2026-10-20", 10, 3000)
        words = [turn(f"2026-10-20T10:00:0{i}Z", 400, talk="word", cloud=True) for i in range(5)]
        recap = {"event": "ella_assessment", "timestamp": "2026-10-20T10:01:00Z", "status": "ok", "kind": "word", "total_ms": 900}
        report = self.report(talks + words + [recap])
        self.assertEqual(report["this"]["latency"]["total_ms"]["n"], 10)
        self.assertEqual(report["this"]["latency"]["total_ms"]["p50"], 3000)
        self.assertEqual(report["this"]["latency"]["wait_ms_cloud"]["n"], 0)
        self.assertEqual(report["this"]["latency"]["recap_total_ms"]["n"], 0)
        volume = report["this"]["volume"]
        self.assertEqual((volume["turns"], volume["word_turns"], volume["recaps"]), (10, 5, 0))
        self.assertIn("| Word of the day turns (not in the KPIs) | 5 |", weekly.markdown(report))

    def test_late_events_for_last_week_are_counted(self):
        last = week_of_turns("2026-10-13", 4, 1000)
        before = self.report(last[:3], week="2026-W42")
        report = self.report(last + week_of_turns("2026-10-20", 1, 1000), previous_report=before)
        self.assertEqual(report["late_events_previous_week"], 1)

    def test_a_raw_log_and_its_sanitized_copy_report_the_same(self):
        events = week_of_turns("2026-10-20", 5, 900) + [
            turn("2026-10-20T07:00:00Z", status="error", error="NO_SPEECH_DETECTED: x")
        ]
        sanitizer = sanitize.Sanitizer()
        clean = [sanitizer.event(e, "lab-01") for e in events]
        raw_report, clean_report = self.report(events)["this"], self.report(clean)["this"]
        raw_report["volume"].pop("laptops")
        clean_report["volume"].pop("laptops")
        self.assertEqual(raw_report, clean_report)

    def test_a_line_that_is_json_but_no_event_is_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp) / "latency.jsonl", week_of_turns("2026-10-20", 2, 1000), tail='3\n"text"\n[1]\nnull\n')
            self.assertEqual(len(weekly.read_all([path])), 2)

    def test_a_batch_is_read_as_its_events_from_its_install(self):
        with tempfile.TemporaryDirectory() as tmp:
            events = [sanitize.Sanitizer().event(e) for e in week_of_turns("2026-10-20", 3, 1000)]
            counts = {"policy_excluded_count": 0, "values_replaced": 0, "unknown_fields_dropped": 0}
            path = Path(tmp) / "batch.jsonl"
            path.write_bytes(sanitize.manual_batch(events, "lab-01", counts))
            read = weekly.read_all([path])
            self.assertEqual(len(read), 3)
            self.assertEqual({e["install_id"] for e in read}, {sanitize.short_hash("manual:lab-01")})
            volume = self.report(read)["this"]["volume"]
            self.assertEqual((volume["turns"], volume["laptops"], volume["events"]), (3, 1, 3))

    def test_a_launchs_installer_is_not_taken_for_a_laptop(self):
        launch = {
            "event": "ella_launch", "timestamp": "2026-10-20T05:00:00Z", "launch_id": LAUNCH,
            "install": {"bundle": "app", "place": "applications", "also_at": []},
        }
        volume = self.report([launch] + week_of_turns("2026-10-20", 2, 1000))["this"]["volume"]
        self.assertEqual((volume["laptops"], volume["launches"]), (0, 1))

    def test_the_markdown_and_json_are_written(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            write(tmp / "events.jsonl", week_of_turns("2026-10-13", 30, 1000) + week_of_turns("2026-10-20", 30, 1500))
            text = quietly(weekly.main, [str(tmp / "events.jsonl"), "--week", "2026-W43", "--out", str(tmp / "out")])
            self.assertIn("Triage: 8 flagged", text)
            self.assertIn("▲", text)
            saved = json.loads((tmp / "out" / "2026-W43.json").read_text(encoding="utf-8"))
            self.assertEqual(saved["previous_week"], "2026-W42")
            self.assertEqual(saved["report_version"], 4)
            self.assertEqual(saved["this"]["latency"]["wait_ms"]["p50"], 1400)


class AnalyticsInTheReport(unittest.TestCase):
    """report_version 4: Analytics V1 under its own key and section, every
    other key and line as in 3."""

    NOW = weekly.datetime(2026, 10, 27, 6, 0, tzinfo=weekly.timezone.utc)
    V3_KEYS = [
        "report_version", "week", "week_start", "week_end", "timezone", "previous_week", "generated_at",
        "tools_ref", "data_through", "thresholds", "this", "previous", "late_events_previous_week", "flags",
        "notes", "collection",
    ]

    def events(self):
        return week_of_turns("2026-10-13", 30, 1000) + week_of_turns("2026-10-20", 30, 1500, talk="free") + [
            turn("2026-10-20T07:00:00Z", status="error", error="NO_SPEECH_DETECTED: x"),
        ]

    def build(self, events=None, ledger=None):
        return weekly.build(self.events() if events is None else events, "2026-W43", now=self.NOW, ledger=ledger)

    def test_the_analytics_are_one_key_after_the_rest(self):
        report = self.build()
        self.assertEqual(report["report_version"], 4)
        self.assertEqual(list(report), self.V3_KEYS + ["analytics"])
        self.assertEqual(
            list(report["analytics"]), ["usage", "talk_lifecycle", "latency", "reliability", "platform"]
        )

    def test_every_other_key_is_as_it_was(self):
        report = self.build()
        before = {key: value for key, value in report.items() if key != "analytics"}
        # What build gave before the analytics: summarize, flags and notes as they are.
        this = weekly.summarize([e for e in self.events() if sanitize.week_of(e) == "2026-W43"])
        self.assertEqual(before["this"], this)
        self.assertEqual(before["flags"], weekly.flags_of(this, before["previous"]))

    def test_the_markdown_above_the_analytics_is_unchanged(self):
        report = self.build()
        text = weekly.markdown(report)
        without = weekly.markdown({**report, "analytics": None})
        self.assertTrue(text.startswith(without.rstrip("\n")))
        self.assertNotIn("## Analytics V1", without)
        self.assertIn("\n## Analytics V1\n", text)
        self.assertGreater(text.index("## Analytics V1"), text.index("## Collection"))

    def test_unavailable_and_limited_metrics_are_named_never_zero(self):
        text = weekly.markdown(self.build())
        self.assertIn("| completion_rate | unavailable | **unavailable** |", text)
        self.assertIn("| left | 0 | **historically limited** |", text)
        self.assertNotIn("| completion_rate | 0", text)

    def test_too_few_measurements_are_said_to_be_too_few(self):
        events = self.events() + [
            {"event": "ella_engine", "schema_version": 2, "timestamp": "2026-10-20T05:00:00Z", "app_version": "0.1.25",
             "launch_id": LAUNCH, "since_launch_ms": 9000, "llm_server": "started"},
        ]
        text = weekly.markdown(self.build(events))
        self.assertIn("| cold_start_ms_started | n=1, too few to stand for the fleet |", text)

    def test_collection_is_shown_once(self):
        report = self.build(ledger=[])
        self.assertEqual(
            {k: v["value"] for k, v in report["analytics"]["reliability"]["collection"].items()},
            {"reported_in": "collection"},
        )
        text = weekly.markdown(report)
        self.assertEqual(text.count("## Collection"), 1)

    def test_a_fixed_time_gives_the_same_report(self):
        events = self.events()
        first, second = self.build(events), self.build(list(reversed(events)))
        self.assertEqual(json.dumps(first), json.dumps(second))
        self.assertEqual(weekly.markdown(first), weekly.markdown(second))

    def test_the_analytics_read_a_raw_log_as_it_would_leave(self):
        report = self.build([turn("2026-10-20T06:00:00Z", transcript="my name is Asha")])
        reading = {k: v["value"] for k, v in report["analytics"]["reliability"]["reading"].items()}
        self.assertEqual(reading["unknown_fields_dropped"], 1)
        text = json.dumps(report["analytics"]) + weekly.markdown(report)
        self.assertNotIn("Asha", text)
        self.assertNotIn(LAUNCH, json.dumps(report["analytics"]))

    def test_nothing_this_week_is_an_empty_report_not_a_crash(self):
        report = self.build([])
        self.assertEqual(report["analytics"]["usage"]["turns"]["value"], 0)
        self.assertIn("## Analytics V1", weekly.markdown(report))


    def test_the_analytics_name_no_one_and_nothing_said(self):
        """The finished contract's privacy: whatever the events hold, the
        analytics, JSON and Markdown, hold no learner's words, no prompt, no
        ID of an install, talk, launch or event, and no hardware as written."""
        cpu, os_ = "11th Gen Intel(R) Core(TM) i7-11800H @ 2.30GHz", "Windows 11 build 26200.9711 (25H2)"
        launch = {"event": "ella_launch", "schema_version": 2, "timestamp": "2026-10-20T05:00:00Z", "app_version": "0.1.27",
                  "launch_id": LAUNCH, "os": os_, "cpu": cpu, "arch": "x86_64", "ram_mb": 16085,
                  "cores_physical": 8, "cores_logical": 16, "install": {"bundle": "app", "place": "applications"},
                  "source": "lab-win-hp-02"}
        turns = [
            turn(f"2026-10-20T06:{i:02d}:00Z", session_id="Asha's first talk", talk="free", source="lab-win-hp-02",
                 transcript="my name is Asha", prompt="You are Ella, a tutor", reply="Hello Asha",
                 notes=["talk_left"] if i == 0 else None)
            for i in range(3)
        ]
        recap_ = {"event": "ella_assessment", "schema_version": 2, "timestamp": "2026-10-20T07:00:00Z",
                  "app_version": "0.1.27", "launch_id": LAUNCH, "session_id": "Asha's first talk", "status": "ok",
                  "kind": "talk", "total_ms": 4000, "source": "lab-win-hp-02", "feedback": "Asha did well"}
        events = [launch] + turns + [recap_]
        report = self.build(events)
        text = json.dumps(report["analytics"], ensure_ascii=False)
        markdown = weekly.markdown(report)
        text += markdown[markdown.index("## Analytics V1"):]
        install = sanitize.short_hash("manual:lab-win-hp-02")
        secret = [
            "Asha", "You are Ella", "my name", "lab-win-hp-02", LAUNCH, sanitize.short_hash(LAUNCH),
            sanitize.short_hash("Asha's first talk"), install, install[:8], cpu, "11800H", "2.30GHz", os_, "26200",
            "16085",
        ] + [sanitize.event_id(e) for e in events]
        for value in secret:
            self.assertNotIn(value, text)

    def test_the_finished_contract_holds(self):
        self.assertEqual(weekly.REPORT_VERSION, 4)
        self.assertIsNone(weekly.analytics.TALK_LEFT_SINCE, "lifecycle rates wait for a release known to send talk_left")
        pointer = self.build(ledger=[])["analytics"]["reliability"]["collection"]["reported_in"]
        self.assertEqual(pointer["value"], "collection")
        self.assertIn("a pointer, not a copy", pointer["note"])


if __name__ == "__main__":
    unittest.main()
