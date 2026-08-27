#!/usr/bin/env python3
"""Measure what the KV cache and the decode rate actually do on this machine.

This settles the two numbers the whole architecture rests on
(docs/desktop-architecture.md §4 and §5):

  1. Cold prefill cost for the ~3,300-token system prompt. The plan says "tens
     of seconds" on four cores. If it is not, the cache-warming work in §4 rule
     2 matters less than we think.
  2. Warm time-to-first-token, and steady-state decode rate. The design assumes
     8-12 tok/s, and that Piper consumes audio at roughly 4 tok/s equivalent —
     so if decode comes in under ~5 tok/s, playback will starve mid-sentence
     and a 3B is too big for this chip.

It also verifies the thing most likely to be silently broken: that turn 2
really is fast, i.e. that llama-server is reusing the prefix rather than
re-prefilling it every time.

    python desktop/bench/bench_llm.py --base-url http://127.0.0.1:8080/v1
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
import urllib.error
import urllib.request

# Roughly the real prompt size: base-prompt.md + conv-unit-framing.md +
# profile-{level}.md is ~2,500 words ≈ 3,300 tokens.
FILLER_WORDS = 2500


def build_system_prompt(words: int) -> str:
    """A stable, realistic-length prefix.

    Deliberately not lorem ipsum: repeated tokens compress in ways real prose
    does not, which would make prefill look faster than it is.
    """
    sentence = (
        "Zoe speaks warmly and briefly to a young learner in India, responds to "
        "meaning before form, asks at most one question per turn, and never "
        "mentions the hidden lesson plan or the assessment rubric. "
    )
    out = []
    count = 0
    while count < words:
        out.append(sentence)
        count += len(sentence.split())
    return "You are Zoe.\n\n" + "".join(out)


def post_stream(base_url: str, payload: dict, timeout: float):
    """Yield (elapsed_seconds, token_text) for each streamed delta."""
    url = f"{base_url.rstrip('/')}/chat/completions"
    body = json.dumps({**payload, "stream": True}).encode()
    request = urllib.request.Request(
        url, data=body, headers={"Content-Type": "application/json"}
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        for raw in response:
            line = raw.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                return
            try:
                chunk = json.loads(data)
            except json.JSONDecodeError:
                continue
            for choice in chunk.get("choices") or []:
                delta = (choice.get("delta") or {}).get("content")
                if delta:
                    yield time.perf_counter() - started, delta


def run_turn(base_url: str, system: str, user: str, slot: int, max_tokens: int,
             timeout: float) -> tuple[float, float, int]:
    """One turn. Returns (time-to-first-token, total seconds, token count)."""
    payload = {
        "model": "local",
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": max_tokens,
        "temperature": 0.7,
        "cache_prompt": True,
    }
    if slot >= 0:
        payload["id_slot"] = slot

    ttft = None
    tokens = 0
    last = 0.0
    for elapsed, _text in post_stream(base_url, payload, timeout):
        if ttft is None:
            ttft = elapsed
        tokens += 1
        last = elapsed
    return (ttft or 0.0), last, tokens


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:8080/v1")
    parser.add_argument("--turns", type=int, default=5)
    parser.add_argument("--slot", type=int, default=0)
    parser.add_argument("--max-tokens", type=int, default=120)
    parser.add_argument("--timeout", type=float, default=600.0)
    parser.add_argument(
        "--words", type=int, default=FILLER_WORDS,
        help="system prompt length in words (default matches the real prompt)",
    )
    args = parser.parse_args()

    system = build_system_prompt(args.words)
    approx_tokens = int(len(system.split()) * 1.33)
    print(f"System prompt: {len(system.split())} words ≈ {approx_tokens} tokens")
    print(f"Endpoint: {args.base_url}  slot={args.slot}\n")

    prompts = [
        "I woke up early today and helped my mother.",
        "We ate rice and dal for lunch.",
        "My friend Ravi plays cricket after school.",
        "Yesterday it rained a lot in our village.",
        "I want to become a teacher when I grow up.",
        "My little sister is learning to read.",
    ]

    try:
        print("Turn 1 (cold prefill — this is the number that decides §4)...")
        ttft, total, tokens = run_turn(
            args.base_url, system, prompts[0], args.slot,
            args.max_tokens, args.timeout,
        )
    except (urllib.error.URLError, OSError) as exc:
        print(f"could not reach llama-server: {exc}", file=sys.stderr)
        return 1

    cold_ttft = ttft
    print(f"  time to first token: {ttft:6.2f} s")
    print(f"  decode:              {tokens} tokens in {total-ttft:.2f} s "
          f"({tokens/max(total-ttft, 1e-6):.1f} tok/s)\n")

    warm_ttfts, rates = [], []
    for i in range(1, args.turns):
        prompt = prompts[i % len(prompts)]
        ttft, total, tokens = run_turn(
            args.base_url, system, prompt, args.slot,
            args.max_tokens, args.timeout,
        )
        decode = max(total - ttft, 1e-6)
        rate = tokens / decode
        warm_ttfts.append(ttft)
        rates.append(rate)
        print(f"Turn {i+1}: ttft {ttft:5.2f} s   decode {rate:5.1f} tok/s "
              f"({tokens} tokens)")

    if not warm_ttfts:
        return 0

    median_ttft = statistics.median(warm_ttfts)
    median_rate = statistics.median(rates)

    print("\n--- verdict ---")
    print(f"cold prefill ttft : {cold_ttft:6.2f} s")
    print(f"warm ttft (median): {median_ttft:6.2f} s")
    print(f"decode  (median)  : {median_rate:6.1f} tok/s")

    # The three things this benchmark exists to decide.
    if cold_ttft > 0 and median_ttft > cold_ttft * 0.5:
        print("\n✗ KV CACHE IS NOT BEING REUSED.")
        print("  Warm turns cost about as much as the cold one, which means "
              "every turn re-prefills the whole prompt.")
        print("  Check: cache_prompt, a stable id_slot, and that the system "
              "prompt is byte-identical between turns (§4 rule 1).")
    else:
        print("\n✓ prefix cache is being reused")

    if median_ttft > 1.0:
        print(f"✗ warm ttft {median_ttft:.2f} s exceeds the 0.3-0.6 s budget in §5")
    else:
        print("✓ warm ttft is within the §5 budget")

    # Piper consumes audio at roughly 4 tok/s equivalent. Below that, playback
    # starves mid-sentence and the model is too big for this machine.
    if median_rate < 5.0:
        print(f"✗ decode {median_rate:.1f} tok/s is below the ~4 tok/s speech "
              f"rate with no margin — Zoe will stutter mid-sentence.")
        print("  This is the signal to drop to a smaller model (§3).")
    elif median_rate < 8.0:
        print(f"~ decode {median_rate:.1f} tok/s is below the 8-12 tok/s the "
              f"plan assumes, but still ahead of speech. Watch for stutter.")
    else:
        print(f"✓ decode {median_rate:.1f} tok/s comfortably outruns speech")
    return 0


if __name__ == "__main__":
    sys.exit(main())
