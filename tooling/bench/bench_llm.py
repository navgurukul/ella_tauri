#!/usr/bin/env python3
"""Replay fixed conversation prompts against one local llama.cpp endpoint.

Run each candidate separately so two loaded models do not compete for CPU or
memory. Compare their JSON reports with --compare after both runs complete.
Responses are retained for a human quality review; this tool does not pretend
to score conversational quality automatically.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import statistics
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit, urlunsplit

DEFAULT_DATASET = Path(__file__).with_name("llm_conversations.json")
DEFAULT_BASE_URL = "http://127.0.0.1:39091/v1"
DEFAULT_MAX_TOKENS = 50
DEFAULT_TEMPERATURE = 0.65


def health_url(base_url: str) -> str:
    parsed = urlsplit(base_url)
    path = parsed.path.rstrip("/")
    if path.endswith("/v1"):
        path = path[:-3]
    path = f"{path}/health" if path else "/health"
    return urlunsplit((parsed.scheme, parsed.netloc, path, "", ""))


def load_dataset(path: Path) -> tuple[dict[str, Any], str]:
    raw = path.read_bytes()
    dataset = json.loads(raw)
    if not isinstance(dataset, dict):
        raise ValueError("dataset must be a JSON object")
    if dataset.get("schema_version") != 1:
        raise ValueError("dataset schema_version must be 1")
    if not isinstance(dataset.get("system_prompt"), str) or not dataset["system_prompt"].strip():
        raise ValueError("dataset system_prompt must be a non-empty string")
    scenarios = dataset.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        raise ValueError("dataset must contain at least one scenario")
    for scenario in scenarios:
        if not isinstance(scenario, dict):
            raise ValueError("each scenario must be a JSON object")
        if not isinstance(scenario.get("name"), str) or not scenario["name"].strip():
            raise ValueError("each scenario must have a non-empty name")
        if "system_prompt" in scenario and (
            not isinstance(scenario["system_prompt"], str) or not scenario["system_prompt"].strip()
        ):
            raise ValueError(f"scenario {scenario['name']!r} system_prompt must be a non-empty string")
        turns = scenario.get("turns")
        if not isinstance(turns, list) or not turns:
            raise ValueError(f"scenario {scenario['name']!r} must contain turns")
        for turn in turns:
            if not isinstance(turn, dict):
                raise ValueError(f"each turn in {scenario['name']!r} must be a JSON object")
            if not isinstance(turn.get("user"), str) or not turn["user"].strip():
                raise ValueError(f"each turn in {scenario['name']!r} needs user text")
            history = turn.get("history", [])
            if not isinstance(history, list):
                raise ValueError("turn history must be a list")
            for message in history:
                if (
                    not isinstance(message, dict)
                    or message.get("role") not in ("user", "assistant")
                    or not isinstance(message.get("content"), str)
                ):
                    raise ValueError("history messages need role user/assistant and string content")
    return dataset, hashlib.sha256(raw).hexdigest()


def stream_turn(
    base_url: str,
    messages: list[dict[str, str]],
    slot: int,
    max_tokens: int,
    temperature: float,
    timeout: float,
) -> dict[str, Any]:
    """Send a release-shaped request and collect response and llama.cpp timings."""
    body = {
        "model": "local",
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "stream": True,
        "stream_options": {"include_usage": True},
        "cache_prompt": True,
        "id_slot": slot,
    }
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/chat/completions",
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    started = time.perf_counter()
    first_token_ms = None
    text: list[str] = []
    timings: dict[str, int] = {}
    usage: dict[str, int] = {}
    with urllib.request.urlopen(request, timeout=timeout) as response:
        for raw_line in response:
            line = raw_line.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                break
            try:
                chunk = json.loads(data)
            except json.JSONDecodeError:
                continue
            chunk_timings = chunk.get("timings")
            if isinstance(chunk_timings, dict):
                for key in ("prompt_n", "prompt_ms", "predicted_n", "predicted_ms"):
                    if isinstance(chunk_timings.get(key), (int, float)):
                        timings[key] = round(chunk_timings[key])
            chunk_usage = chunk.get("usage")
            if isinstance(chunk_usage, dict):
                for key in ("prompt_tokens", "completion_tokens"):
                    if isinstance(chunk_usage.get(key), (int, float)):
                        usage[key] = round(chunk_usage[key])
            for choice in chunk.get("choices") or []:
                delta = (choice.get("delta") or {}).get("content")
                if isinstance(delta, str) and delta:
                    if first_token_ms is None:
                        first_token_ms = (time.perf_counter() - started) * 1000
                    text.append(delta)
    completion_ms = (time.perf_counter() - started) * 1000
    return {
        "ttft_ms": round(first_token_ms if first_token_ms is not None else completion_ms),
        "completion_ms": round(completion_ms),
        "response": "".join(text).strip(),
        "timings": timings,
        "usage": usage,
    }


def percentile50(values: list[float]) -> float | None:
    return round(statistics.median(values), 1) if values else None


def summarize(turns: list[dict[str, Any]]) -> dict[str, Any]:
    def values(field: str, selection: list[dict[str, Any]]) -> list[float]:
        return [float(item[field]) for item in selection if item.get(field) is not None]

    warm = [turn for turn in turns if not turn["first_request"]]
    generated_rates = []
    for turn in warm:
        timing = turn["timings"]
        count, elapsed = timing.get("predicted_n"), timing.get("predicted_ms")
        if count and elapsed:
            generated_rates.append(count * 1000 / elapsed)
    return {
        "requests": len(turns),
        "first_request_ttft_ms": turns[0]["ttft_ms"] if turns else None,
        "first_request_completion_ms": turns[0]["completion_ms"] if turns else None,
        "warm_ttft_p50_ms": percentile50(values("ttft_ms", warm)),
        "warm_completion_p50_ms": percentile50(values("completion_ms", warm)),
        "warm_prompt_eval_p50_ms": percentile50(
            [float(turn["timings"]["prompt_ms"]) for turn in warm if turn["timings"].get("prompt_ms") is not None]
        ),
        "warm_decode_tokens_per_second_p50": percentile50(generated_rates),
    }


def run(args: argparse.Namespace) -> int:
    dataset, digest = load_dataset(Path(args.dataset))
    results = []
    try:
        for iteration in range(1, args.iterations + 1):
            for scenario in dataset["scenarios"]:
                for turn_index, turn in enumerate(scenario["turns"], start=1):
                    messages = [
                        {
                            "role": "system",
                            "content": scenario.get("system_prompt", dataset["system_prompt"]),
                        },
                        *turn.get("history", []),
                        {"role": "user", "content": turn["user"]},
                    ]
                    result = stream_turn(
                        args.base_url,
                        messages,
                        args.slot,
                        args.max_tokens,
                        args.temperature,
                        args.timeout,
                    )
                    if not result["response"]:
                        raise RuntimeError(
                            f"empty model response for {scenario['name']} turn {turn_index}, iteration {iteration}"
                        )
                    results.append(
                        {
                            "iteration": iteration,
                            "scenario": scenario["name"],
                            "turn": turn_index,
                            "user": turn["user"],
                            "first_request": len(results) == 0,
                            **result,
                        }
                    )
                    print(
                        f"{scenario['name']} #{turn_index} iteration {iteration}: "
                        f"TTFT {result['ttft_ms']} ms, total {result['completion_ms']} ms"
                    )
    except urllib.error.HTTPError as exc:
        response = exc.read().decode("utf-8", "replace").strip()
        exc.close()
        detail = f"HTTP {exc.code} {exc.reason}"
        if response:
            detail = f"{detail}: {response}"
        raise RuntimeError(f"llama-server rejected the request at {args.base_url}: {detail}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(
            f"could not reach llama-server at {args.base_url}. Start it first and confirm "
            f"{health_url(args.base_url)} responds; see the Windows benchmark instructions."
        ) from exc

    report = {
        "schema_version": 1,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "candidate": args.candidate,
        "model": args.model,
        "runtime": args.runtime,
        "threads": args.threads,
        "power_note": args.power_note,
        "machine": {
            "system": platform.platform(),
            "processor": platform.processor(),
            "logical_cpus": __import__("os").cpu_count(),
        },
        "endpoint": args.base_url,
        "dataset_sha256": digest,
        "dataset_description": dataset.get("description", ""),
        "settings": {
            "max_tokens": args.max_tokens,
            "temperature": args.temperature,
            "slot": args.slot,
            "iterations": args.iterations,
        },
        "summary": summarize(results),
        "turns": results,
    }
    serialized = json.dumps(report, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        Path(args.output).write_text(serialized, encoding="utf-8")
        print(f"Report saved to {args.output}")
    else:
        print(serialized)
    return 0


def compare(paths: list[str]) -> int:
    missing = [path for path in paths if not Path(path).is_file()]
    if missing:
        raise FileNotFoundError(
            f"report not found: {', '.join(missing)}. Run the benchmark successfully for each "
            "candidate before comparing; a failed run does not create a report."
        )
    reports = [json.loads(Path(path).read_text(encoding="utf-8")) for path in paths]
    baseline = reports[0]
    for path, report in zip(paths[1:], reports[1:]):
        if report.get("dataset_sha256") != baseline.get("dataset_sha256"):
            raise ValueError(f"{path}: dataset differs; candidate comparisons must use identical prompts")
        if report.get("settings") != baseline.get("settings"):
            raise ValueError(f"{path}: benchmark settings differ")
        baseline_turns = [(t["iteration"], t["scenario"], t["turn"]) for t in baseline["turns"]]
        report_turns = [(t["iteration"], t["scenario"], t["turn"]) for t in report["turns"]]
        if report_turns != baseline_turns:
            raise ValueError(f"{path}: measured turn sets differ")

    print(f"Dataset SHA-256: {baseline['dataset_sha256']}")
    print(f"{'Candidate':24} {'First TTFT':>12} {'Warm TTFT p50':>15} {'Warm total p50':>16} {'Decode tok/s':>13}")
    for report in reports:
        summary = report["summary"]
        print(
            f"{report['candidate'][:24]:24} "
            f"{summary['first_request_ttft_ms']!s:>12} "
            f"{summary['warm_ttft_p50_ms']!s:>15} "
            f"{summary['warm_completion_p50_ms']!s:>16} "
            f"{summary['warm_decode_tokens_per_second_p50']!s:>13}"
        )
    print("\nBlind quality review: compare the responses below for relevance, warmth,")
    print("level-appropriate language, one non-repeated question, and safety.")
    for turn_index, key in enumerate(
        ((t["iteration"], t["scenario"], t["turn"]) for t in baseline["turns"]),
        start=1,
    ):
        baseline_turn = next(t for t in baseline["turns"] if (t["iteration"], t["scenario"], t["turn"]) == key)
        print(f"\n[{turn_index}] {key[1]} turn {key[2]} - learner: {baseline_turn['user']}")
        for candidate_index, report in enumerate(reports, start=1):
            candidate_turn = next(t for t in report["turns"] if (t["iteration"], t["scenario"], t["turn"]) == key)
            print(f"  Candidate {candidate_index}: {candidate_turn['response']}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compare", nargs="+", metavar="REPORT", help="compare two or more saved reports")
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--candidate", default="candidate")
    parser.add_argument("--model", default="unspecified", help="model/checkpoint identifier for the report")
    parser.add_argument("--runtime", default="unspecified", help="llama.cpp build/backend identifier")
    parser.add_argument("--threads", type=int, help="llama-server thread count used for this run")
    parser.add_argument("--power-note", help="record AC/battery and OS power mode")
    parser.add_argument("--dataset", default=str(DEFAULT_DATASET))
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--slot", type=int, default=0)
    parser.add_argument("--max-tokens", type=int, default=DEFAULT_MAX_TOKENS)
    parser.add_argument("--temperature", type=float, default=DEFAULT_TEMPERATURE)
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--output", help="write the complete JSON report to this path")
    args = parser.parse_args()
    try:
        if args.compare:
            if len(args.compare) < 2:
                parser.error("--compare needs at least two report paths")
            return compare(args.compare)
        if args.iterations < 1 or args.max_tokens < 1 or args.timeout <= 0:
            parser.error("--iterations, --max-tokens, and --timeout must be positive")
        return run(args)
    except (OSError, ValueError, KeyError, TypeError, TimeoutError, RuntimeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
