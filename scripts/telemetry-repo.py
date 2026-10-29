#!/usr/bin/env python3
"""Run ingest and the weekly report against a checkout of the private
telemetry repository, navgurukul/ella-telemetry, with the scripts beside
this one (latency-sanitize.py, latency-weekly.py, telemetry-health.py).

  npm run telemetry:repo -- ingest --repo ../ella-telemetry            # inbox/ -> data/ + ledger/
  npm run telemetry:repo -- report --repo ../ella-telemetry [--week W] # reports/<week>.md and .json
  npm run telemetry:repo -- health --repo ../ella-telemetry [--week W] # changes nothing

The telemetry repository names, in its TOOLS_REF, the ella_tauri commit its
data is processed with. ingest and report run only from a checkout of exactly
that commit, scripts/ unchanged, so the rules that took the data in are on
record; the report names it as its `tools_ref`. health only warns.

ingest and report each start from a clean checkout at the tip of its branch
(`git pull --ff-only`), make one commit, and push it. A push refused because
the branch moved on is not forced: the run drops its own commit, pulls, and
runs again, so every ingest on the branch classified its batches against the
whole ledger before it, and no batch is recorded `new` twice. A lock in the
checkout keeps two runs there apart. A run that fails leaves the checkout as
it found it.

Every file in inbox/ leaves the tree in the ingest's commit: a refused one
is moved to .refused/ in the checkout, which is never committed, for a person
to look at; its hash and problems are in the ledger. A file committed to
inbox/ stays in Git history after it leaves the tree.

POSIX only (macOS, Linux): the lock is flock(2).
"""
import argparse
import fcntl
import hashlib
import json
import re
import secrets
import shutil
import subprocess
import sys
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
LOCK = ".ingest.lock"
REFUSED = ".refused"
PIN = "TOOLS_REF"
FOLDERS = ("inbox", "data", "ledger", "reports")
# A push refused because the branch moved on is tried again from the start,
# at most this many times.
ATTEMPTS = 5

# What a commit of this tool may add or change; it may only remove from inbox/.
ADDED = re.compile(r"(data/\d{4}-W\d{2}/[0-9A-Za-z-]+\.jsonl|ledger/[0-9A-Za-z-]+\.jsonl|reports/\d{4}-W\d{2}\.(md|json))")
REMOVED = re.compile(r"inbox/[^/]+")
KEEP = ".gitkeep"
COMMIT = re.compile(r"[0-9a-f]{40}")


class Failed(Exception):
    """A run that cannot go on; the checkout is left as it was found."""


def git(repo, *args, check=True):
    done = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    if check and done.returncode:
        raise Failed(f"git {' '.join(args)}: {done.stderr.strip() or done.stdout.strip()}")
    return done


# Which scripts process the data.


def tools_commit(path=HERE):
    """The ella_tauri commit these scripts are, and whether they are exactly
    it: (None, False) outside a Git checkout. Files Git does not track are
    not counted, since the scripts load each other by name."""
    head = git(path, "rev-parse", "HEAD", check=False)
    if head.returncode:
        return None, False
    changed = git(path, "status", "--porcelain", "--untracked-files=no", "--", ".", check=False).stdout.strip()
    return head.stdout.strip(), not changed


def pinned(repo):
    """The commit the telemetry repository says its data is processed with."""
    try:
        ref = (repo / PIN).read_text(encoding="utf-8").strip()
    except OSError:
        raise Failed(f"{repo} has no {PIN}: not a checkout of the telemetry repository") from None
    if not COMMIT.fullmatch(ref):
        raise Failed(f"{repo / PIN} must hold one full ella_tauri commit (40 hex)")
    return ref


def require_pinned(repo):
    """Stops a run that would take data in, or report on it, with scripts
    other than the ones TOOLS_REF names."""
    ref = pinned(repo)
    commit, exact = tools_commit()
    if commit != ref:
        raise Failed(
            f"these scripts are ella_tauri {commit[:12] if commit else '(not a Git checkout)'}, "
            f"but {PIN} pins {ref[:12]}: run from a checkout of that commit (git checkout {ref}), "
            f"or change {PIN} in a reviewed commit to the telemetry repository"
        )
    if not exact:
        raise Failed(f"scripts/ has changes not in ella_tauri {ref[:12]}: run from that commit as it is")
    return ref


# The checkout.


@contextmanager
def locked(repo):
    """One ingest or report at a time in this checkout. flock is let go
    when the process ends, however it ends."""
    with open(repo / LOCK, "w") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Failed("another ingest or report is running in this checkout") from None
        yield


def require_repo(repo):
    missing = [name for name in (PIN, *FOLDERS) if not (repo / name).exists()]
    if missing:
        raise Failed(f"{repo} is not a checkout of the telemetry repository: no {', '.join(missing)}")


def require_clean(repo):
    changes = git(repo, "status", "--porcelain").stdout.strip()
    if changes:
        raise Failed(
            "the checkout has changes of its own; commit or discard them first "
            "(its OPERATIONS.md: a run cut short)\n" + changes
        )


def pull(repo):
    if git(repo, "rev-parse", "--abbrev-ref", "@{upstream}", check=False).returncode:
        raise Failed("the branch has no upstream to pull from and push to")
    git(repo, "pull", "--ff-only", "-q")


def head(repo):
    return git(repo, "rev-parse", "HEAD").stdout.strip()


def back_to(repo, base):
    """Drops whatever a run did since `base`: its commit, its files."""
    git(repo, "reset", "-q", "--hard", base)
    git(repo, "clean", "-fdq", "--", *FOLDERS)


def check_staged(repo):
    """Stops a commit that would add anything but data, ledger and report
    files, or remove anything but inbox files."""
    stray = []
    for line in git(repo, "diff", "--cached", "--name-status", "--no-renames").stdout.splitlines():
        status, path = line.split("\t", 1)
        allowed = REMOVED if status == "D" else ADDED
        if not allowed.fullmatch(path) or path.endswith(KEEP):
            stray.append(f"{status} {path}")
    if stray:
        raise Failed("the commit would include what this tool never commits:\n  " + "\n  ".join(stray))


def push(repo):
    """True once pushed; False when the branch moved on since the pull."""
    done = git(repo, "push", "-q", check=False)
    if not done.returncode:
        return True
    if re.search(r"rejected|fetch first|non-fast-forward", done.stderr):
        return False
    raise Failed(f"git push: {done.stderr.strip()}")


def run_tool(name, *args, cwd):
    done = subprocess.run([sys.executable, str(HERE / name), *map(str, args)], cwd=cwd, capture_output=True, text=True)
    sys.stdout.write(done.stdout)
    sys.stderr.write(done.stderr)
    return done.returncode


def short_hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()[:32]


def committed(repo, base, attempt):
    """Commits and pushes what `attempt(ref)` staged, starting again from
    a fresh pull when the branch moved on. `attempt` returns the commit's
    message, or None for nothing to commit; and may raise to give up."""
    for _ in range(ATTEMPTS):
        try:
            # After every pull: the pull may have brought a new TOOLS_REF.
            message = attempt(require_pinned(repo))
            if message is None:
                back_to(repo, base)
                return False
            check_staged(repo)
            git(repo, "commit", "-q", "-m", message)
            if push(repo):
                return True
            print("the branch moved on: taking it again from the start", file=sys.stderr)
        except BaseException:
            back_to(repo, base)
            raise
        back_to(repo, base)
        pull(repo)
        base = head(repo)
    raise Failed(f"the branch kept moving on: tried {ATTEMPTS} times")


# The commands.


def ingest(repo):
    refused_total = []

    def attempt(ref):
        files = sorted(p for p in (repo / "inbox").iterdir() if p.is_file() and p.name != KEEP)
        if not files:
            print("inbox/ is empty: nothing to take in", file=sys.stderr)
            return None
        ingest_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + secrets.token_hex(2)
        hashes = {path: short_hash(path) for path in files}
        code = run_tool(
            "latency-sanitize.py", "--validate", *files, "--data", repo / "data",
            "--ledger", repo / "ledger", "--ingest-id", ingest_id, cwd=repo,
        )
        record = repo / "ledger" / f"{ingest_id}.jsonl"
        if code not in (0, 1) or not record.exists():
            raise Failed(f"ingest stopped (exit {code}) before it recorded the batches")
        lines = [json.loads(line) for line in record.read_text(encoding="utf-8").splitlines()]
        refused = {r["file_sha256"] for r in lines if r["record"] == "refused"}
        aside = repo / REFUSED
        # A try whose push was refused set its files aside too: they are
        # back in inbox/ now, and set aside again under this ingest's id.
        for stale in refused_total:
            stale.unlink(missing_ok=True)
        refused_total.clear()
        for path in files:
            git(repo, "rm", "-q", "--cached", "--ignore-unmatch", "--", str(path.relative_to(repo)))
            if hashes[path] in refused:
                aside.mkdir(exist_ok=True)
                target = aside / f"{ingest_id}-{path.name}"
                shutil.move(str(path), target)
                refused_total.append(target)
            else:
                path.unlink()
        git(repo, "add", "--", "data", "ledger")
        outcomes = {}
        for r in lines:
            key = r.get("outcome", r["record"])
            outcomes[key] = outcomes.get(key, 0) + 1
        summary = ", ".join(f"{count} {name.replace('_', ' ')}" for name, count in sorted(outcomes.items()))
        return f"Ingest {ingest_id}: {len(files)} file(s), {summary}\n\nTools: ella_tauri {ref}"

    with locked(repo):
        require_clean(repo)
        pull(repo)
        try:
            pushed = committed(repo, head(repo), attempt)
        except BaseException:
            for path in refused_total:
                path.unlink(missing_ok=True)
            raise
    if refused_total:
        print(f"{len(refused_total)} file(s) refused, kept out of Git in {REFUSED}/:", file=sys.stderr)
        for path in refused_total:
            print(f"  {path.name}", file=sys.stderr)
    if pushed:
        print(f"pushed {head(repo)[:12]}", file=sys.stderr)
    return 1 if refused_total else 0


def report(repo, week):
    def attempt(ref):
        args = ["data", "--ledger", "ledger", "--out", "reports", "--tools-ref", ref]
        if week:
            args += ["--week", week]
        if run_tool("latency-weekly.py", *args, cwd=repo):
            raise Failed("the weekly report failed")
        git(repo, "add", "--", "reports")
        if not git(repo, "diff", "--cached", "--quiet", check=False).returncode:
            print("the report is as already committed", file=sys.stderr)
            return None
        written = sorted(Path(p).stem for p in git(repo, "diff", "--cached", "--name-only").stdout.split())
        return f"Weekly report {written[0]}\n\nTools: ella_tauri {ref}"

    with locked(repo):
        require_clean(repo)
        pull(repo)
        if committed(repo, head(repo), attempt):
            print(f"pushed {head(repo)[:12]}", file=sys.stderr)
    return 0


def health(repo, week, as_json):
    try:
        require_pinned(repo)
    except Failed as error:
        print(f"note: {error}", file=sys.stderr)
    args = ["ledger"] + (["--week", week] if week else []) + (["--json"] if as_json else [])
    return run_tool("telemetry-health.py", *args, cwd=repo)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for name, text in (
        ("ingest", "take in inbox/, commit and push"),
        ("report", "write the weekly report, commit and push"),
        ("health", "print collection health; changes nothing"),
    ):
        command = commands.add_parser(name, help=text)
        command.add_argument("--repo", type=Path, required=True, help="a checkout of navgurukul/ella-telemetry")
        if name != "ingest":
            command.add_argument("--week", help="such as 2026-W44 (default: the last whole week, IST)")
    commands.choices["health"].add_argument("--json", action="store_true", help="print the JSON")
    args = parser.parse_args(argv)
    repo = args.repo.resolve()
    try:
        require_repo(repo)
        if args.command == "ingest":
            return ingest(repo)
        if args.command == "report":
            return report(repo, args.week)
        return health(repo, args.week, args.json)
    except Failed as error:
        print(f"telemetry {args.command}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
