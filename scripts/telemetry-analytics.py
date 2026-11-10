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


ENGINE = "ella_engine"
UPDATE = "ella_update"


def measured(rows, field):
    """A distribution of `field` over `rows`, and `of`, how many rows there
    were: a row without the field is not measured, and none is guessed."""
    get = field if callable(field) else (lambda event: event.get(field))
    return {**distribution(get(event) for event in rows), "of": len(rows)}


def warm_up_ms(event):
    warm = event.get("warm_up")
    return warm.get("ms") if isinstance(warm, dict) else None


def latency(week_events):
    """How long things took, by the rules the weekly report's own latency
    keeps: turns that are voice, succeeded, and are not Word of the day
    (whose turns are the app's own lines or a short cloud answer); recaps
    that succeeded, Word of the day's left out; engines that came up without
    an error. Each is the field the app measured (`direct`), or the same
    split by who answered or how the engine came up (`derived`). The wait is
    `speech_ms` only: a turn without it is not measured, never taken from
    its total."""
    is_word, by_cloud = sanitize.report.is_word, sanitize.report.by_cloud
    turns = [
        e
        for e in week_events
        if e.get("event") == TURN and not is_word(e) and e.get("kind", "voice") == "voice" and e.get("status") == "ok"
    ]
    cloud = [e for e in turns if by_cloud(e)]
    laptop = [e for e in turns if not by_cloud(e)]
    recaps = [e for e in week_events if e.get("event") == RECAP and not is_word(e) and e.get("status") == "ok"]
    engines = [e for e in week_events if e.get("event") == ENGINE and not e.get("error") and not e.get("error_code")]
    started = [e for e in engines if e.get("llm_server") == "started"]
    updates = {}
    for event in week_events:
        if event.get("event") == UPDATE and isinstance(event.get("step"), str):
            updates.setdefault(event["step"], []).append(event)

    split_cloud = "answered by the cloud: the reply the learner heard was the cloud's"
    split_laptop = "answered on the laptop: its own model, or the app's own line"
    return {
        "turns": metric(len(turns), DIRECT, "voice turns that succeeded, Word of the day left out"),
        "wait_ms": metric(measured(turns, "speech_ms"), DIRECT, "learner stops → Ella speaks"),
        "wait_ms_laptop": metric(measured(laptop, "speech_ms"), DERIVED, split_laptop),
        "wait_ms_cloud": metric(measured(cloud, "speech_ms"), DERIVED, split_cloud),
        "total_ms": metric(measured(turns, "total_ms"), DIRECT, "the whole turn"),
        "total_ms_laptop": metric(measured(laptop, "total_ms"), DERIVED, split_laptop),
        "total_ms_cloud": metric(measured(cloud, "total_ms"), DERIVED, split_cloud),
        "stt_ms": metric(measured(turns, "stt_ms"), DIRECT, "speech to text"),
        "llm_ttft_ms": metric(measured(turns, "llm_ttft_ms"), DIRECT, "the language model's first token"),
        "llm_completion_ms": metric(measured(turns, "llm_completion_ms"), DIRECT, "the language model's whole reply"),
        "tts_first_audio_ms": metric(
            measured(turns, "tts_first_audio_ms"), DIRECT, "text to speech's first audio; builds that keep it"
        ),
        "warm_up_ms": metric(
            measured(turns, warm_up_ms),
            DIRECT,
            "setting up a talk: its instructions into the model, reported by the first reply after",
        ),
        "recap_total_ms": metric(measured(recaps, "total_ms"), DIRECT, "recaps that succeeded"),
        "cold_start_ms": metric(measured(engines, "since_launch_ms"), DIRECT, "launch → engines ready"),
        "cold_start_ms_started": metric(
            measured(started, "since_launch_ms"),
            DERIVED,
            "launches where Ella started its own model server; one it found running (`external`) loaded nothing",
        ),
        "update_ms_by_step": metric(
            {step: measured(rows, "ms") for step, rows in sorted(updates.items())}, DERIVED, "the updater, by step"
        ),
    }


CLOUD = "ella_cloud"
# The learner said nothing Ella could hear: counted apart, as the weekly
# report counts it, never as a system error.
NO_SPEECH = "NO_SPEECH_DETECTED"

health = _sibling("telemetry-health.py", "telemetry_health")


def error_code(event):
    """The event's error, as the weekly report reads it: its code, or one
    made from a raw log's text by the policy's own coder."""
    return event.get("error_code") or sanitize.error_code(event.get("error"))


def has_error(event):
    return bool(event.get("error_code") or event.get("error"))


def rate(part, whole):
    return round(part / whole, 4) if whole else None


def app_signals(week_events, all_events):
    """What the app's own events say went wrong, or fell back, in the week.
    A turn's error is a system error unless the learner said nothing
    (NO_SPEECH_DETECTED), as in the weekly report; a fallback, or the cloud
    being down, is what the app did about something, not an error of it."""
    is_word = sanitize.report.is_word
    turns = [e for e in week_events if e.get("event") == TURN and not is_word(e)]
    errors = [e for e in turns if e.get("status") != "ok"]
    no_speech = [e for e in errors if error_code(e) == NO_SPEECH]
    system = [e for e in errors if error_code(e) != NO_SPEECH]
    ok = [e for e in turns if e.get("status") == "ok"]
    recaps = [e for e in week_events if e.get("event") == RECAP and not is_word(e)]
    engines = [e for e in week_events if e.get("event") == ENGINE]
    clouds = [e for e in week_events if e.get("event") == CLOUD]
    updates = [e for e in week_events if e.get("event") == UPDATE]

    # A recap whose talk has no turn among every event read: its turns held,
    # lost, or from before the queue. Which, the telemetry cannot say, so it
    # is a sign for collection, not a failure, and never a lifecycle talk.
    turned = {e.get("talk_id") for e in (all_events or week_events) if e.get("event") == TURN}
    recap_only = {
        e["talk_id"] for e in recaps if isinstance(e.get("talk_id"), str) and e["talk_id"] not in turned
    }

    return {
        "turns": metric(len(turns), DIRECT, "every turn but Word of the day's, as the weekly report counts"),
        "turn_errors": metric(len(errors), DIRECT),
        "no_speech": metric(len(no_speech), DIRECT, "the learner said nothing Ella could hear: no system error"),
        "system_errors": metric(len(system), DIRECT, "turn errors other than no speech"),
        "system_error_rate": metric(rate(len(system), len(turns)), DERIVED, "system errors over turns"),
        "system_errors_by_stage": metric(
            counted(Counter(e.get("failed_at") or "unknown" for e in system)), DIRECT, "where the turn failed"
        ),
        "system_errors_by_code": metric(counted(Counter(error_code(e) or "unknown" for e in system)), DIRECT),
        "cloud_failed_runs": metric(
            sum(1 for e in turns for run in e.get("llm_runs") or [] if run.get("backend") == "cloud" and run.get("failed")),
            DIRECT,
            "cloud generations that failed; the turn may still have been answered",
        ),
        "stt_fallbacks": metric(
            sum(1 for e in ok if e.get("stt_fallback_from") or e.get("stt_engine") == "whisper-small"),
            DIRECT,
            "turns whose speech to text fell back to another engine: a fallback, not an error",
        ),
        "recaps": metric(len(recaps), DIRECT, "Word of the day's left out"),
        "recap_errors": metric(sum(1 for e in recaps if e.get("status") != "ok"), DIRECT),
        "failed_judges": metric(
            sum(1 for e in recaps for judge in e.get("judges") or [] if judge.get("status") == "error"), DIRECT
        ),
        "recap_only_talks": metric(
            len(recap_only),
            DERIVED,
            "recaps whose talk's turns are not among the events read: held, lost, or from before the queue; "
            "a sign for collection, not counted in talk_lifecycle",
        ),
        "engine_errors": metric(sum(1 for e in engines if has_error(e)), DIRECT, "engines that came up with an error"),
        "cloud_down": metric(
            sum(1 for e in clouds if e.get("up") is False),
            DIRECT,
            "times the app found the cloud down and answered on the laptop instead",
        ),
        "cloud_down_by_reason": metric(
            counted(Counter(e.get("reason_code") or "unknown" for e in clouds if e.get("up") is False)), DIRECT
        ),
        "update_errors_by_step": metric(
            counted(Counter(e.get("step") or "unknown" for e in updates if has_error(e))), DIRECT
        ),
    }


def collection(ledger, week):
    """How collection went, as telemetry-health.py's own summary of the
    batch record says it, in counts: nothing is worked out again here, and
    no install is named."""
    if ledger is None or not week:
        why = "no batch record (--ledger) given" if ledger is None else "no week given"
        return {"batch_record": unavailable(why)}
    h = health.summary(ledger, week)
    seq, loss, delivery = h["sequence"], h["loss"], h["delivery"]
    of_record = "the batch record, as telemetry-health reads it"
    return {
        "batches": metric(h["batches"], DIRECT, f"batch files received this week, by outcome; {of_record}"),
        "installs_reporting": metric(h["installs"]["reporting"], DIRECT),
        "installs_silent": metric(len(h["installs"]["silent"]), DERIVED, "sent in the 28 days before, not this week"),
        "events_received": metric(h["events_received"], DIRECT),
        "dropped": metric(loss["dropped"], DIRECT, "events the app dropped before sending, as it counted them"),
        "dropped_by_reason": metric(loss["dropped_by_reason"], DIRECT),
        "drop_rate": metric(loss["drop_rate"], DERIVED, "dropped over the app's events received"),
        "log_resets": metric(loss["log_resets"], DIRECT),
        "sequence_gaps_pending": metric(
            len(seq["pending_gaps"]), DERIVED, "seqs not yet received, seen missing less than 7 days ago"
        ),
        "sequence_gaps_lost_this_week": metric(
            len(seq["lost_this_week"]), DERIVED, "gaps open 7 days, taken as lost: probably, not surely"
        ),
        "sequence_gaps_lost_total": metric(seq["lost_total"], DERIVED),
        "sequence_conflicts": metric(len(seq["conflicts"]), DIRECT),
        "upload_attempts": metric(delivery["attempts"], DIRECT, "the app's batches, by tries to send"),
        "delivery_delay_ms": metric(delivery["delay_ms"], DERIVED, "a batch's last event → received"),
        "backlog_after_events_max": metric(delivery["backlog_after_events_max"], DIRECT, "events still queued"),
        "clock_suspect": metric(len(delivery["clock_suspect"]), DERIVED),
        "flags": metric([flag["check"] for flag in h["flags"]], DERIVED, "telemetry-health's own flags"),
    }


def reliability(week_events, all_events=(), ledger=None, week=None, reading=None):
    """How reliably Ella ran and its telemetry came in: `app`, what the
    app's events say; `collection`, what the batch record says; `reading`,
    what reading the events left out, when given."""
    shown = {"app": app_signals(week_events, all_events), "collection": collection(ledger, week)}
    shown["reading"] = (
        {name: metric(value, DIRECT) for name, value in reading.items()}
        if reading is not None
        else {"counts": unavailable("not given")}
    )
    return shown


def analyse(week_events, all_events=(), ledger=None, week=None, reading=None):
    """The week's analytics, a section each, in `SECTIONS` order.
    `week_events` are the week's sanitized events; `all_events` every event
    read, for what came before the week (an install's first day); `ledger`
    the batch record, for collection, when given; `reading` the counts
    `read` gave."""
    shown = {section: {} for section in SECTIONS}
    shown["usage"] = usage(week_events, all_events, week)
    shown["talk_lifecycle"] = talk_lifecycle(week_events, all_events)
    shown["latency"] = latency(week_events)
    shown["reliability"] = reliability(week_events, all_events, ledger, week, reading)
    return shown
