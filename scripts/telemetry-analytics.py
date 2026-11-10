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
from collections import Counter
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


def install_of(raw):
    """The laptop an event came from, as the weekly report knows it: the
    `install_id` its batch's envelope gave it, or, for a Stage-1 file, its
    laptop's label hashed as a manual batch's is. None when it has neither;
    never the label itself."""
    install = raw.get("install_id")
    if isinstance(install, str) and sanitize.PATTERNS["hex16"].fullmatch(install):
        return install
    source = raw.get("source")
    if isinstance(source, str) and sanitize.LABEL.fullmatch(source):
        return sanitize.short_hash(f"manual:{source}")
    return None


def read(events):
    """The events as analytics may see them, oldest first, and what reading
    them left out.

    `events` are as latency-weekly.py's `read_all` gives them: each once, by
    its `event_id`, with its batch's `install_id`; this adds no counting-once
    of its own. Each goes through the policy's Sanitizer again, so a laptop's
    own latency.jsonl is read exactly as it would leave the laptop, and an
    event already sanitized comes through as it is. Nothing is written:
    not the files, not a queue, not what is sent.

    The counts: `events_read`, kept; `unreadable`, an item that is no event
    or broke the Sanitizer; and the Sanitizer's own, `policy_excluded_count`
    (events the policy leaves behind, or with no time), `values_replaced`
    and `unknown_fields_dropped`."""
    sanitizer = sanitize.Sanitizer()
    kept = []
    unreadable = 0
    for raw in events:
        if not isinstance(raw, dict):
            unreadable += 1
            continue
        try:
            # The install is the batch's, not a field of the event's: set
            # aside, so the Sanitizer does not count it as one with no rule.
            fields = {key: value for key, value in raw.items() if key != "install_id"}
            event = sanitizer.event(fields)
        except Exception:  # one odd event is left out, not the report lost
            unreadable += 1
            continue
        if event is None:
            continue
        install = install_of(raw)
        if install:
            event["install_id"] = install
        kept.append(event)
    counts = {"events_read": len(kept), "unreadable": unreadable, **sanitizer.counts()}
    return sanitize.oldest_first(kept), counts


TURN = "ella_turn_latency"
RECAP = "ella_assessment"
LAUNCH = "ella_launch"
# A turn of a build from before turns named their talk's type.
UNKNOWN = "unknown"


def counted(counter):
    """A Counter as the report shows one: most first, then by name, so the
    same events always give the same JSON."""
    return dict(sorted(counter.items(), key=lambda item: (-item[1], str(item[0]))))


def ist_day(event):
    at = sanitize.safe_when(event)
    return at.astimezone(sanitize.IST).date() if at is not None else None


def usage(week_events, all_events=(), week=None):
    """How much Ella was used in the week. A talk is the turns that share a
    `talk_id`, its type the `talk` its turns name; an install is a laptop's
    copy of Ella, never a person, and a day it was active is an IST day it
    had a turn. Word of the day is counted here, as a talk of its own type,
    though the latency KPIs leave it out."""
    turns = [e for e in week_events if e.get("event") == TURN]
    by_talk = {}
    for event in turns:
        if isinstance(event.get("talk_id"), str):
            by_talk.setdefault(event["talk_id"], []).append(event)

    def type_of(rows):
        named = [e["talk"] for e in rows if isinstance(e.get("talk"), str) and e["talk"]]
        return named[0] if named else UNKNOWN

    talks_by_type = counted(Counter(type_of(rows) for rows in by_talk.values()))
    turns_by_type = counted(Counter(type_of([e]) for e in turns))
    typed_note = (
        "turns of builds from before turns named their talk's type are `unknown`"
        if UNKNOWN in turns_by_type
        else None
    )
    typed = HISTORICALLY_LIMITED if typed_note else DERIVED

    installs = {e["install_id"] for e in week_events if e.get("install_id")}
    unnamed = sum(1 for e in week_events if not e.get("install_id"))
    install_note = "a laptop's copy of Ella, never a person"
    if unnamed:
        install_note += f"; {unnamed} events name no install (a laptop's own log) and are not counted in"

    # An install is new when its first event read, of any week, is this
    # week's: a laptop that ran Ella before it sent telemetry shows as new
    # the first week it sent.
    first = {}
    for event in all_events:
        install, at = event.get("install_id"), sanitize.safe_when(event)
        if install and at is not None and (install not in first or at < first[install][0]):
            first[install] = (at, event)
    new = sum(1 for install in installs if install in first and sanitize.week_of(first[install][1]) == week)

    days = {}
    for event in turns:
        day = ist_day(event)
        if event.get("install_id") and day is not None:
            days.setdefault(event["install_id"], set()).add(day)

    return {
        "installs": metric(len(installs), DERIVED, install_note),
        "new_installs": (
            metric(new, HISTORICALLY_LIMITED, "first seen in the telemetry this week; a laptop that ran Ella before it sent shows as new")
            if week
            else unavailable("no week given")
        ),
        "launches": metric(
            len({e["launch"] for e in week_events if e.get("launch")}), DERIVED, "launches with any event this week"
        ),
        "launch_lines": metric(sum(1 for e in week_events if e.get("event") == LAUNCH), DIRECT),
        "turns": metric(len(turns), DIRECT, "every turn, Word of the day and errors included"),
        "turns_by_talk_type": metric(turns_by_type, typed, typed_note),
        "talks": metric(
            len(by_talk), DERIVED, "talks with a turn this week; one that spans two weeks is counted in each"
        ),
        "talks_by_type": metric(talks_by_type, typed, typed_note),
        "turns_per_talk": metric(distribution(len(rows) for rows in by_talk.values()), DERIVED),
        "recaps": metric(sum(1 for e in week_events if e.get("event") == RECAP), DIRECT),
        "active_install_days": metric(sum(len(d) for d in days.values()), DERIVED, "IST days an install had a turn"),
        "installs_by_active_days": metric(
            {str(n): count for n, count in sorted(Counter(len(d) for d in days.values()).items())},
            DERIVED,
            "days active: installs",
        ),
    }


# The note the app puts on a talk's turns when it sends a talk that was
# left: no end, and no turn for 12 hours (#45, telemetry_queue.rs).
TALK_LEFT = "talk_left"
# The first released version known to send `talk_left`. Until one is, every
# build sending is 0.1.27 whether it has #45 or not, so a talk with no note
# may be one a build without #45 left, and the share of talks completed or
# left cannot be told: the rates stay unavailable while this is None.
TALK_LEFT_SINCE = None

COMPLETED, LEFT, RESUMED, UNRESOLVED = "completed", "left", "resumed", "unresolved"


def version_key(text):
    """`0.1.28` as (0, 1, 28), or None for what is no such version."""
    parts = text.split(".") if isinstance(text, str) else []
    return tuple(int(part) for part in parts) if parts and all(part.isdigit() for part in parts) else None


def ended(event):
    """Whether `event` ends its talk, as the queue holds a talk until: a turn
    that says the talk is over, or its recap. The queue's 30-minute check is
    no end, and nothing here counts one."""
    if event.get("event") == RECAP:
        return True
    return event.get("event") == TURN and event.get("talk_over") is True


def noted_left(event):
    notes = event.get("notes")
    return event.get("event") == TURN and isinstance(notes, list) and TALK_LEFT in notes


def state_of(rows):
    end = any(ended(e) for e in rows)
    left = any(noted_left(e) for e in rows)
    if left:
        return RESUMED if end else LEFT
    return COMPLETED if end else UNRESOLVED


def talk_lifecycle(week_events, all_events=(), since=TALK_LEFT_SINCE):
    """How the week's talks ended, from the app's own signals only: the
    talks with a turn this week (as `usage` counts them), each judged on
    every event of it read, of any week, so a recap that came later counts.

    - `completed`: it has an end and was never noted left;
    - `left`: noted `talk_left` (no end and no turn for 12 hours), no end;
    - `resumed`: noted left, then an end came;
    - `unresolved`: no end and no note: a build's own export, a build from
      before #45, or a talk a build without #45 left.

    Builds from before #45 never note a talk left, so `left` and `resumed`
    are floors. Rates are given only over talks wholly of builds at or after
    `since`, and are unavailable while it is None."""
    talks = {e["talk_id"] for e in week_events if e.get("event") == TURN and isinstance(e.get("talk_id"), str)}
    rows = {talk: [] for talk in talks}
    # Without every event read, a talk is judged on the week's alone.
    for event in all_events or week_events:
        if event.get("talk_id") in rows:
            rows[event["talk_id"]].append(event)
    states = {talk: state_of(events) for talk, events in rows.items()}
    counts = Counter(states.values())
    floor = "a floor: builds from before #45 never note a talk left"

    shown = {
        "talks": metric(len(states), DERIVED, "talks with a turn this week, as usage counts them"),
        COMPLETED: metric(counts[COMPLETED], DERIVED, "an end (the talk over, or its recap) and no left note"),
        LEFT: metric(counts[LEFT], HISTORICALLY_LIMITED, f"noted {TALK_LEFT}, no end; {floor}"),
        RESUMED: metric(counts[RESUMED], HISTORICALLY_LIMITED, f"noted {TALK_LEFT}, then an end; {floor}"),
        UNRESOLVED: metric(
            counts[UNRESOLVED],
            DERIVED,
            "no end and no note: an export, a build from before #45, or a talk such a build left",
        ),
    }
    if since is None:
        why = "until a released version is known to send talk_left (#45): builds with and without it share a version"
        shown["completion_rate"] = unavailable(why)
        shown["left_rate"] = unavailable(why)
        return shown

    floor_key = version_key(since)
    covered = [
        talk
        for talk, events in rows.items()
        if all((version_key(e.get("app_version")) or ()) >= floor_key for e in events)
    ]
    of_covered = Counter(states[talk] for talk in covered)
    note = f"over the {len(covered)} talks wholly of builds {since} or later"
    shown["completion_rate"] = metric(
        round((of_covered[COMPLETED] + of_covered[RESUMED]) / len(covered), 4) if covered else None,
        DERIVED,
        note + "; completed or resumed",
    )
    shown["left_rate"] = metric(
        round((of_covered[LEFT] + of_covered[RESUMED]) / len(covered), 4) if covered else None,
        DERIVED,
        note + "; ever noted left",
    )
    return shown


def analyse(week_events, all_events=(), ledger=None, week=None):
    """The week's analytics, a section each, in `SECTIONS` order.
    `week_events` are the week's sanitized events; `all_events` every event
    read, for what came before the week (an install's first day); `ledger`
    the batch record, for collection, when given."""
    shown = {section: {} for section in SECTIONS}
    shown["usage"] = usage(week_events, all_events, week)
    shown["talk_lifecycle"] = talk_lifecycle(week_events, all_events)
    return shown
