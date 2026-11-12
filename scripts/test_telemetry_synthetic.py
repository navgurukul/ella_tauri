"""Tests of the weekly report over SYNTHETIC telemetry
(scripts/fixtures/telemetry-synthetic/, made by telemetry-synthetic.py):
the whole pipeline, ingest to report_version 4, on data made up to show
every section, and never to be taken for a laptop's.

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


synthetic = load("telemetry-synthetic.py", "telemetry_synthetic")
sanitize = synthetic.sanitize
weekly = synthetic.weekly
FIXTURES = synthetic.FIXTURES
INBOX, EXPECTED = FIXTURES / "inbox", FIXTURES / "expected"
REMAKE = "the fixtures are out of date: run `npm run telemetry:synthetic -- --write` and read the diff"


def committed_batches():
    return {
        path.relative_to(INBOX).as_posix(): path.read_bytes() for path in sorted(INBOX.rglob("*.jsonl"))
    }


def every_event():
    """(file, envelope, events) of each committed batch, unchecked."""
    for name, data in committed_batches().items():
        head, _, lines = data.partition(b"\n")
        yield name, json.loads(head), [json.loads(line) for line in lines.splitlines()]


def values(section):
    return {name: shown["value"] for name, shown in section.items()}


class TheFixtures(unittest.TestCase):
    def test_they_are_what_the_generator_makes(self):
        made = {
            f"{ingest}/{name}": data for ingest, files in synthetic.batches().items() for name, data in files.items()
        }
        self.assertEqual(sorted(made), sorted(committed_batches()), REMAKE)
        for name, data in committed_batches().items():
            self.assertEqual(made[name], data, f"{name}: {REMAKE}")

    def test_the_generator_makes_the_same_bytes_every_time(self):
        self.assertEqual(synthetic.batches(), synthetic.batches())

    def test_every_batch_passes_ingest_but_the_one_made_to_be_refused(self):
        for name, data in committed_batches().items():
            problems = sanitize.validate_batch(data)
            if name.endswith("synthetic-refused.jsonl"):
                self.assertEqual(problems, ["envelope.batch_id: not the hash of the body and events"])
            else:
                self.assertEqual(problems, [], name)


class MarkedSynthetic(unittest.TestCase):
    """Nothing here can be taken for a laptop's telemetry."""

    def test_every_file_and_laptop_is_named_synthetic(self):
        for name in committed_batches():
            self.assertTrue(Path(name).name.startswith("synthetic-"), name)
        for label in synthetic.LAPTOPS:
            self.assertTrue(label.startswith("synthetic-"), label)

    def test_every_event_and_app_batch_runs_a_synthetic_version(self):
        for name, envelope, events in every_event():
            if envelope["origin"] == "app":
                self.assertTrue(envelope["app_version"].endswith("-synthetic"), name)
            for event in events:
                self.assertTrue(event["app_version"].endswith("-synthetic"), f"{name}: {event['event']}")

    def test_every_event_is_from_before_ella_sent_telemetry(self):
        for name, _, events in every_event():
            for event in events:
                self.assertTrue("2026-01-01" <= event["timestamp"] < "2026-01-11", f"{name}: {event['timestamp']}")

    def test_the_report_says_so(self):
        report = json.loads((EXPECTED / f"{synthetic.WEEK}.json").read_text(encoding="utf-8"))
        self.assertEqual(report["tools_ref"], "synthetic-fixture")
        self.assertEqual(set(report["this"]["volume"]["app_versions"]), set(synthetic.VERSIONS))
        text = (EXPECTED / f"{synthetic.WEEK}.md").read_text(encoding="utf-8")
        self.assertIn("by tools at `synthetic-fixture`", text)
        self.assertIn("9.9.1-synthetic", text)

    def test_the_folder_says_so_first(self):
        readme = (FIXTURES / "README.md").read_text(encoding="utf-8")
        self.assertTrue(readme.startswith("# SYNTHETIC TELEMETRY: TEST FIXTURES ONLY"))
        self.assertIn("* -text", (FIXTURES / ".gitattributes").read_text(encoding="utf-8"))


class ThePipeline(unittest.TestCase):
    """The committed batches through ingest and the weekly report, as
    telemetry-repo.py runs them."""

    @classmethod
    def setUpClass(cls):
        cls.as_json, cls.text = synthetic.run(INBOX)
        cls.report = json.loads(cls.as_json)
        cls.analytics = cls.report["analytics"]

    def test_the_report_is_the_expected_one(self):
        self.assertEqual(self.as_json, (EXPECTED / f"{synthetic.WEEK}.json").read_text(encoding="utf-8"), REMAKE)
        self.assertEqual(self.text, (EXPECTED / f"{synthetic.WEEK}.md").read_text(encoding="utf-8"), REMAKE)

    def test_it_is_report_version_4_with_every_key_of_3(self):
        self.assertEqual(self.report["report_version"], 4)
        self.assertEqual(
            list(self.report),
            ["report_version", "week", "week_start", "week_end", "timezone", "previous_week", "generated_at",
             "tools_ref", "data_through", "thresholds", "this", "previous", "late_events_previous_week", "flags",
             "notes", "collection", "analytics"],
        )

    def test_the_command_writes_the_same_report(self):
        """latency-weekly.py's own command, as telemetry-repo.py runs it, in
        a checkout the fixtures were taken into: the same report, but for
        the time it was made."""
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            synthetic.ingest(INBOX, repo)
            for week in (synthetic.PREVIOUS, synthetic.WEEK):
                with contextlib.redirect_stdout(io.StringIO()):
                    weekly.main([
                        str(repo / "data"), "--ledger", str(repo / "ledger"), "--out", str(repo / "reports"),
                        "--tools-ref", synthetic.TOOLS_REF, "--week", week,
                    ])
            written = json.loads((repo / "reports" / f"{synthetic.WEEK}.json").read_text(encoding="utf-8"))
        self.assertEqual({**written, "generated_at": None}, {**self.report, "generated_at": None})

    def test_every_section_has_something_to_show(self):
        usage = values(self.analytics["usage"])
        self.assertEqual((usage["installs"], usage["new_installs"]), (6, 1))
        self.assertEqual(set(usage["talks_by_type"]), {"free", "lesson", "placement", "word"})

        lifecycle = values(self.analytics["talk_lifecycle"])
        self.assertEqual(
            {k: lifecycle[k] for k in ("completed", "left", "resumed", "unresolved")},
            {"completed": 14, "left": 1, "resumed": 1, "unresolved": 1},
        )
        for rate in ("completion_rate", "left_rate"):
            self.assertEqual(self.analytics["talk_lifecycle"][rate]["status"], "unavailable")

        self.assertTrue(self.report["flags"], "the week is slower than the one before")

        app = values(self.analytics["reliability"]["app"])
        self.assertEqual(
            {k: app[k] for k in ("no_speech", "system_errors", "recap_errors", "recap_only_talks", "cloud_down")},
            {"no_speech": 1, "system_errors": 1, "recap_errors": 1, "recap_only_talks": 1, "cloud_down": 3},
        )
        self.assertEqual(app["update_errors_by_step"], {"check_failed": 1})

        collection = self.report["collection"]
        self.assertEqual(collection["batches"], {"new": 8, "duplicate": 1, "seq_conflict": 0, "manual": 1, "refused": 1})
        self.assertEqual([gap["batch_seq"] for gap in collection["sequence"]["pending_gaps"]], [2])
        self.assertEqual(collection["loss"]["dropped_by_reason"]["queue_full"], 3)
        self.assertEqual(collection["delivery"]["attempts"], {"1": 7, "3+": 1})
        self.assertEqual(
            values(self.analytics["reliability"]["collection"]), {"reported_in": "collection"}
        )

        platform = values(self.analytics["platform"])
        self.assertEqual(platform["installs_by_os_family"], {"macOS": 3, "small groups": 3})
        self.assertEqual(platform["installs_by_ram_band"], {"small groups": 6}, "one group alone is folded whole")
        self.assertTrue(platform["wait_ms_laptop_by_os_family"]["macOS"]["sufficient"])

    def test_nothing_made_up_as_a_laptops_own_reaches_the_report(self):
        """Even made up, the report holds only what real data could show:
        no laptop's label, talk, launch or hardware as written."""
        whole = self.as_json + self.text
        laptops = synthetic.logs()
        for label, laptop in laptops.items():
            self.assertNotIn(label, whole)
            self.assertNotIn(laptop.cpu, whole)
            self.assertNotIn(laptop.os, whole)
            for event in laptop.events:
                for raw in (event.get("session_id"), event.get("launch_id")):
                    if raw:
                        self.assertNotIn(raw, whole)
                        self.assertNotIn(sanitize.short_hash(raw), whole)


if __name__ == "__main__":
    unittest.main()
