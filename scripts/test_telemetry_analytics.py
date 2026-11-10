"""Tests for the weekly report's analytics (telemetry-analytics.py).

  npm run test:telemetry
"""
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
