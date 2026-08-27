#!/usr/bin/env python3
"""How long is a session report really? The §6 / §11 open question.

`analyze_and_grade_session` asks for up to 3,500 tokens. That ceiling is what
makes grading a 5-7 minute job at local decode rates. But the plan is explicit
that 3,500 is a *ceiling, not observed usage* — "if real outputs run ~800
tokens, this is ~90 seconds and the problem mostly dissolves."

Two modes:

  --from-db   Reconstruct the JSON from session_analyses rows already in the
              database and measure it. Needs no LLM and no network, and uses
              real student sessions rather than a guess.

  --live      Re-run the actual grading call against an endpoint and measure
              the completion. Slower, but it is the only way to see what a
              *local 3B* produces, which may differ a lot from what the hosted
              model produced.

    python desktop/bench/measure_grading_tokens.py --from-db backend/zoe.db
    python desktop/bench/measure_grading_tokens.py --live \\
        --base-url http://127.0.0.1:8080/v1 --db backend/zoe.db --limit 10
"""
from __future__ import annotations

import argparse
import json
import sqlite3
import statistics
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

# The value in llm_service.analyze_and_grade_session.
REQUESTED_MAX_TOKENS = 3500
# Decode rate for a 3B Q4 on the reference machine (§3).
ASSUMED_TOK_PER_S = 10.0


def approx_tokens(text: str) -> int:
    """Rough token count without a tokenizer.

    ~4 characters per token is the usual English heuristic and is close enough
    for JSON, which is punctuation-dense and tokenizes slightly worse. Good to
    within ~15%, which is plenty to tell 800 from 3,500.
    """
    return max(1, round(len(text) / 4))


def reconstruct_report(row: sqlite3.Row) -> dict:
    """Rebuild the grading JSON from the persisted columns."""
    notes = []
    if row["transcript_notes_json"]:
        try:
            notes = json.loads(row["transcript_notes_json"])
        except json.JSONDecodeError:
            notes = []

    def load(value):
        if not value:
            return []
        if isinstance(value, (list, dict)):
            return value
        try:
            return json.loads(value)
        except (json.JSONDecodeError, TypeError):
            return []

    return {
        "overall": row["overall"],
        "zoe_recap": row["zoe_recap"],
        "best_moment": {
            "quote": row["best_moment_quote"],
            "note": row["best_moment_note"],
        },
        "courage_or_motivation": {
            "type": row["courage_type"],
            "content": row["courage_content"],
        },
        "try_next_time": {
            "skill": row["try_next_skill"],
            "example": row["try_next_example"],
        },
        "scores": load(row["skills_evidenced"]),
        "transcript_notes": notes,
    }


def from_db(db_path: Path, limit: int | None) -> list[int]:
    if not db_path.exists():
        print(f"no database at {db_path}", file=sys.stderr)
        return []

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    try:
        query = "SELECT * FROM session_analyses ORDER BY created_at DESC"
        if limit:
            query += f" LIMIT {int(limit)}"
        rows = conn.execute(query).fetchall()
    except sqlite3.Error as exc:
        print(f"could not read session_analyses: {exc}", file=sys.stderr)
        return []
    finally:
        conn.close()

    counts = []
    for row in rows:
        # separators=(",",":") would understate it; the model emits readable
        # JSON, so measure it the way the model would produce it.
        report = json.dumps(reconstruct_report(row), ensure_ascii=False)
        counts.append(approx_tokens(report))
    return counts


def live(base_url: str, db_path: Path, limit: int, timeout: float) -> list[int]:
    """Re-grade real transcripts and measure the completions."""
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    try:
        rows = conn.execute(
            """
            SELECT c.id, c.topic, m.role, m.content
            FROM conversations c
            JOIN messages m ON m.conversation_id = c.id
            WHERE c.completed = 1
            ORDER BY c.created_at DESC, m.turn_number
            """
        ).fetchall()
    finally:
        conn.close()

    sessions: dict[str, dict] = {}
    for row in rows:
        session = sessions.setdefault(
            row["id"], {"topic": row["topic"], "messages": []}
        )
        session["messages"].append(
            {"role": "user" if row["role"] == "user" else "assistant",
             "content": row["content"]}
        )
    chosen = list(sessions.items())[:limit]
    if not chosen:
        print("no completed conversations with messages found", file=sys.stderr)
        return []

    prompt_path = (
        Path(__file__).resolve().parents[2]
        / "backend" / "app" / "prompts" / "end-result-prompt.md"
    )
    template = prompt_path.read_text()

    counts = []
    for session_id, session in chosen:
        transcript = "\n".join(
            f"{'STUDENT' if m['role'] == 'user' else 'ZOE'}: {m['content']}"
            for m in session["messages"]
        )
        system = (
            template
            .replace("{{LEVEL}}", "A1")
            .replace("{{UNIT_LABEL}}", "Finding My Voice")
            .replace("{{TOPIC}}", session["topic"] or "Free Talk")
            .replace("{{TARGET_SKILLS}}", "- U1-VOC-01: names familiar objects")
            .replace("{{FULL_TRANSCRIPT}}", transcript)
        )
        payload = {
            "model": "local",
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content":
                 "Analyze the transcript, extract facts, grade targeted "
                 "skills, and return the combined JSON report."},
            ],
            "max_tokens": REQUESTED_MAX_TOKENS,
            "temperature": 0.4,
        }
        request = urllib.request.Request(
            f"{base_url.rstrip('/')}/chat/completions",
            data=json.dumps(payload).encode(),
            headers={"Content-Type": "application/json"},
        )
        started = time.perf_counter()
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                body = json.loads(response.read())
        except (urllib.error.URLError, OSError) as exc:
            print(f"  ✗ {session_id[:8]}: {exc}", file=sys.stderr)
            continue
        elapsed = time.perf_counter() - started

        text = body["choices"][0]["message"]["content"]
        usage = body.get("usage") or {}
        # Prefer the server's own count over the heuristic when offered.
        tokens = usage.get("completion_tokens") or approx_tokens(text)
        counts.append(tokens)
        rate = tokens / elapsed if elapsed else 0
        print(f"  {session_id[:8]}  {tokens:5} tokens  {elapsed:6.1f} s "
              f"({rate:4.1f} tok/s)  {len(session['messages'])} turns")
    return counts


def report(counts: list[int]) -> None:
    if not counts:
        print("\nno samples measured.")
        return

    counts = sorted(counts)
    median = statistics.median(counts)
    p90 = counts[int(len(counts) * 0.9) - 1] if len(counts) >= 10 else counts[-1]

    print(f"\n--- {len(counts)} session reports ---")
    print(f"min      {counts[0]:5}")
    print(f"median   {median:5.0f}")
    print(f"p90      {p90:5}")
    print(f"max      {counts[-1]:5}")
    print(f"ceiling  {REQUESTED_MAX_TOKENS:5}  (current max_tokens)")

    print(f"\nAt {ASSUMED_TOK_PER_S:.0f} tok/s on the reference machine:")
    for label, value in (("median", median), ("p90", p90), ("max", counts[-1]),
                         ("ceiling", REQUESTED_MAX_TOKENS)):
        seconds = value / ASSUMED_TOK_PER_S
        print(f"  {label:8} {value:5.0f} tokens -> {seconds:5.0f} s "
              f"({seconds/60:.1f} min)")

    print("\n--- verdict ---")
    headroom = REQUESTED_MAX_TOKENS / max(p90, 1)
    if p90 < 1000:
        print(f"p90 is {p90} tokens — {headroom:.1f}x below the 3,500 ceiling.")
        print(f"Real grading is ~{p90/ASSUMED_TOK_PER_S:.0f} s, not 5-7 minutes.")
        print("→ Lower max_tokens to about "
              f"{int(p90 * 1.3 // 100 * 100)} and the §6 problem mostly "
              "dissolves. Async grading is still right (nobody should watch a "
              "90-second spinner) but it stops being the scary number.")
    elif p90 < 2000:
        print(f"p90 is {p90} tokens — roughly "
              f"{p90/ASSUMED_TOK_PER_S/60:.1f} min locally.")
        print("→ Async grading is required. Lowering max_tokens to "
              f"~{int(p90 * 1.2 // 100 * 100)} trims the worst case.")
    else:
        print(f"p90 is {p90} tokens — the ceiling is realistic and grading "
              "genuinely takes minutes.")
        print("→ Async grading is required, and splitting analysis from "
              "skill grading (§6) is worth doing so partial results land early.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from-db", dest="from_db", metavar="PATH",
                        help="measure stored analyses; no LLM needed")
    parser.add_argument("--live", action="store_true",
                        help="re-run grading against --base-url")
    parser.add_argument("--base-url", default="http://127.0.0.1:8080/v1")
    parser.add_argument("--db", default="backend/zoe.db")
    parser.add_argument("--limit", type=int, default=None)
    parser.add_argument("--timeout", type=float, default=900.0)
    args = parser.parse_args()

    if args.live:
        print(f"Re-grading real transcripts against {args.base_url}\n")
        report(live(args.base_url, Path(args.db),
                    args.limit or 10, args.timeout))
        return 0

    db = Path(args.from_db or args.db)
    print(f"Measuring stored session reports in {db}\n")
    report(from_db(db, args.limit))
    return 0


if __name__ == "__main__":
    sys.exit(main())
