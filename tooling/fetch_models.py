#!/usr/bin/env python3
"""Download the offline models into the layout the desktop build expects.

Roughly 3.7 GB with Canary and the Whisper fallback. On a campus connection
that is a real download, so every file
is resumable, verified by size, and skipped when already present — running this
twice must not re-download anything.

    python desktop/fetch_models.py --dest ella_app/build/engines
    python desktop/fetch_models.py --llm llama-3.2-3b        # bake-off variant
    python desktop/fetch_models.py --only llm,stt --dry-run

The Piper voice is not downloadable: it is a custom NavGurukul voice that lives
on origin/feature/navgurukul-piper. Point --piper-voice at a checkout of that
branch and both the .onnx and its .onnx.json are copied. See §7.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "models.json"

HF_BASE = "https://huggingface.co"


def hf_url(repo: str, filename: str) -> str:
    return f"{HF_BASE}/{repo}/resolve/main/{filename}"


def human(mb: float) -> str:
    return f"{mb/1024:.1f} GB" if mb >= 1024 else f"{mb:.0f} MB"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def validate_file(
    dest: Path, expected_mb: int | None, expected_sha256: str | None
) -> tuple[bool, str]:
    if not dest.is_file():
        return False, "missing"
    size_mb = dest.stat().st_size / 1024 / 1024
    if expected_mb is not None and size_mb < expected_mb * 0.95:
        return False, f"truncated ({human(size_mb)}; expected about {human(expected_mb)})"
    if expected_sha256:
        actual = sha256_file(dest)
        if actual.lower() != expected_sha256.lower():
            return False, f"SHA-256 mismatch ({actual})"
    return True, human(size_mb)


def download(
    url: str,
    dest: Path,
    expected_mb: int | None,
    expected_sha256: str | None = None,
) -> bool:
    """Resumable download with a progress line. Returns True on success."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    partial = dest.with_suffix(dest.suffix + ".part")

    # Already there and plausibly complete.
    if dest.exists():
        valid, detail = validate_file(dest, expected_mb, expected_sha256)
        if valid:
            print(f"  ✓ {dest.name} already present ({detail})")
            return True
        print(f"  ! {dest.name} is invalid: {detail}; refetching")
        dest.unlink()

    resume_from = partial.stat().st_size if partial.exists() else 0
    request = urllib.request.Request(url)
    if resume_from:
        # A half-finished 2 GB download must not start over.
        request.add_header("Range", f"bytes={resume_from}-")
        print(f"  ↻ resuming {dest.name} from {human(resume_from/1024/1024)}")

    try:
        with urllib.request.urlopen(request) as response:
            total = int(response.headers.get("Content-Length") or 0) + resume_from
            mode = "ab" if resume_from else "wb"
            written = resume_from
            with open(partial, mode) as handle:
                while chunk := response.read(1024 * 512):
                    handle.write(chunk)
                    written += len(chunk)
                    if total:
                        pct = written / total * 100
                        print(
                            f"\r  ↓ {dest.name} {pct:5.1f}% "
                            f"({human(written/1024/1024)})",
                            end="",
                            flush=True,
                        )
            print()
    except urllib.error.HTTPError as exc:
        if exc.code == 416 and partial.exists():
            # Range not satisfiable — the partial is already the whole file.
            partial.rename(dest)
            print(f"  ✓ {dest.name} was already complete")
            return True
        print(f"  ✗ {dest.name}: HTTP {exc.code} {exc.reason}", file=sys.stderr)
        return False
    except (urllib.error.URLError, OSError) as exc:
        print(f"  ✗ {dest.name}: {exc}", file=sys.stderr)
        return False

    partial.rename(dest)
    valid, detail = validate_file(dest, expected_mb, expected_sha256)
    if not valid:
        print(f"  ✗ {dest.name} failed validation: {detail}", file=sys.stderr)
        return False
    print(f"  ✓ verified {dest.name} ({detail})")
    return True


def copy_local(source: Path, dest: Path, sidecar: str | None) -> bool:
    if not source.exists():
        print(f"  ✗ {source} not found", file=sys.stderr)
        return False
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, dest)
    print(f"  ✓ copied {source.name}")

    if sidecar:
        # Piper reads sample rate and the phoneme id map from the sidecar.
        # Without it, synthesis fails later with a much worse error.
        side_src = Path(str(source) + sidecar)
        if not side_src.exists():
            print(f"  ✗ required sidecar {side_src} not found", file=sys.stderr)
            return False
        shutil.copy2(side_src, Path(str(dest) + sidecar))
        print(f"  ✓ copied {side_src.name}")
    return True


def main() -> int:
    manifest = json.loads(MANIFEST.read_text())
    groups = manifest["models"]

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--dest",
        default="ella_app/build/engines",
        help="engine root; models land under <dest>/models/",
    )
    parser.add_argument("--llm", help="LLM variant (see models.json)")
    parser.add_argument("--stt", help="primary STT variant (see models.json)")
    parser.add_argument(
        "--stt-fallback", help="fallback STT variant (see models.json)"
    )
    parser.add_argument(
        "--only", help="comma-separated subset of: " + ",".join(groups)
    )
    parser.add_argument(
        "--piper-voice",
        help="path to en_IN-navgurukul-medium.onnx from the navgurukul-piper branch",
    )
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--verify-only",
        action="store_true",
        help="validate selected files without downloading or changing them",
    )
    args = parser.parse_args()

    selected = (
        [g.strip() for g in args.only.split(",")] if args.only else list(groups)
    )
    overrides = {
        "llm": args.llm,
        "stt": args.stt,
        "stt_fallback": args.stt_fallback,
    }

    root = Path(args.dest).resolve()
    total_mb = 0
    plan: list[tuple[str, dict, Path]] = []

    for name in selected:
        group = groups.get(name)
        if group is None:
            print(f"unknown model group: {name}", file=sys.stderr)
            return 2
        variant_name = overrides.get(name) or group["default"]
        variant = group["variants"].get(variant_name)
        if variant is None:
            print(
                f"unknown {name} variant {variant_name!r}; "
                f"choose from {', '.join(group['variants'])}",
                file=sys.stderr,
            )
            return 2
        dest = root / group["target"]
        plan.append((name, variant, dest))
        total_mb += variant.get("size_mb", 0)

    print(f"Engine root: {root}")
    print(f"Planned download: {human(total_mb)}\n")

    if args.dry_run:
        for name, variant, dest in plan:
            src = variant.get("repo") or variant.get("source", "local")
            print(f"  {name:8} {src:45} -> {dest}")
        return 0

    if args.verify_only:
        failures = []
        for name, variant, dest in plan:
            valid, detail = validate_file(
                dest, variant.get("size_mb"), variant.get("sha256")
            )
            mark = "✓" if valid else "✗"
            print(f"  {mark} {name:12} {dest}: {detail}")
            if not valid:
                failures.append(name)
        if failures:
            print(
                "\nInstall or repair with the same command without --verify-only.",
                file=sys.stderr,
            )
            return 1
        print("\nAll selected models passed validation.")
        return 0

    failures = []
    for name, variant, dest in plan:
        print(f"[{name}]")
        if variant.get("source") == "local":
            if name == "tts":
                if not args.piper_voice:
                    print(
                        "  ✗ --piper-voice is required: the NavGurukul voice is "
                        "not on a public host. See docs/desktop-architecture.md §7.",
                        file=sys.stderr,
                    )
                    failures.append(name)
                    continue
                ok = copy_local(
                    Path(args.piper_voice), dest, variant.get("requires_sidecar")
                )
            else:
                print(f"  ✗ {name} has no download source", file=sys.stderr)
                ok = False
        else:
            ok = download(
                hf_url(variant["repo"], variant["file"]),
                dest,
                variant.get("size_mb"),
                variant.get("sha256"),
            )
            sidecar = variant.get("requires_sidecar")
            if ok and sidecar:
                side_name = Path(variant["file"]).with_suffix("").name
                ok = download(
                    hf_url(variant["repo"], sidecar.lstrip(".")),
                    Path(str(dest) + sidecar),
                    None,
                ) or ok
                if not ok:
                    print(
                        f"  ! {name} sidecar {sidecar} missing; "
                        f"the engine may refuse to load",
                        file=sys.stderr,
                    )
        if not ok:
            failures.append(name)
        print()

    if failures:
        print(f"Failed: {', '.join(failures)}", file=sys.stderr)
        return 1
    print("All models present.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
