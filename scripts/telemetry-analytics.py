"""What a week's telemetry says of how Ella was used, how its talks ended,
how fast and how reliably it ran, and on what: the analytics of the weekly
report (latency-weekly.py, report_version 4).

It reads sanitized events only, and changes nothing: not the events, not a
laptop's queue, not what it uploads. Every number says how it was come by
(`status`):

- `direct`: a field the telemetry holds, counted or measured as it is;
- `derived`: worked out from fields it holds (talks from turns, days from
  times), by a rule written here;
- `historically_limited`: derived, but builds or data from before some
  change cannot show it all, so it is a floor, not the whole;
- `unavailable`: the telemetry does not hold it, and the report says so
  rather than guess.

An install is a laptop's copy of Ella's data, never a person: nothing here
names a learner, and nothing is added to the telemetry to make one up.
"""
import importlib.util
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _sibling(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


sanitize = _sibling("latency-sanitize.py", "latency_sanitize")
# The percentile every telemetry report uses, so their numbers agree.
pct = sanitize.report.pct

DIRECT = "direct"
DERIVED = "derived"
HISTORICALLY_LIMITED = "historically_limited"
UNAVAILABLE = "unavailable"
STATUSES = (DIRECT, DERIVED, HISTORICALLY_LIMITED, UNAVAILABLE)

# The report's sections, in the order the JSON and the Markdown give them.
SECTIONS = ("usage", "talk_lifecycle", "latency", "reliability", "platform")


def metric(value, status, note=None):
    """One number, or table, and how it was come by."""
    if status not in STATUSES:
        raise ValueError(f"unknown status: {status}")
    if status == UNAVAILABLE and value is not None:
        raise ValueError("an unavailable metric has no value")
    shown = {"value": value, "status": status}
    if note:
        shown["note"] = note
    return shown


def unavailable(note):
    """What the telemetry cannot say, and why."""
    return metric(None, UNAVAILABLE, note)


def usable(value):
    """A measurement: a finite number, not negative. True and False are not
    numbers here, though Python counts them as such."""
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value >= 0
    )


def distribution(values):
    """How `values` spread: how many, and P50, P75 and P95, as the other
    reports count them. A value missing (None) is not counted; one that is
    no measurement (text, a negative, NaN) is counted apart, as `excluded`."""
    values = [value for value in values if value is not None]
    kept = sorted(value for value in values if usable(value))
    return {
        "count": len(kept),
        "p50": pct(kept, 50),
        "p75": pct(kept, 75),
        "p95": pct(kept, 95),
        "excluded": len(values) - len(kept),
    }


def analyse(week_events, all_events=(), ledger=None, week=None):
    """The week's analytics, a section each, in `SECTIONS` order.
    `week_events` are the week's sanitized events; `all_events` every event
    read, for what came before the week (an install's first day); `ledger`
    the batch record, for collection, when given."""
    return {section: {} for section in SECTIONS}
