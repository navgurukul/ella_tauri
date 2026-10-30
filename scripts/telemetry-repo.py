#!/usr/bin/env python3
"""Run ingest and the weekly report against a checkout of the private
telemetry repository, navgurukul/ella-telemetry, with the scripts beside
this one (latency-sanitize.py, latency-weekly.py, telemetry-health.py).

  npm run telemetry:repo -- ingest --repo ../ella-telemetry            # inbox/ -> data/ + ledger/
  npm run telemetry:repo -- pull --repo ../ella-telemetry              # the relay -> data/ + ledger/
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

pull takes in, the same way, the batches waiting at the telemetry relay
(the `ella-telemetry` Supabase function, its bucket's incoming/), with the
pull token in ELLA_TELEMETRY_PULL_TOKEN; ELLA_TELEMETRY_URL names another
relay. They are downloaded to a temporary folder outside the checkout, so
nothing raw is ever in Git, and deleted at the relay only once the commit
that records them is pushed. A pull cut short before that leaves them
there: the next one takes them again, and the ledger records them
`duplicate`.

POSIX only (macOS, Linux): the lock is flock(2).
"""
import argparse
import fcntl
import hashlib
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
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

# The telemetry relay, and what it is asked with.
RELAY = "https://xoeydvvslyvslvpaajmi.supabase.co/functions/v1/ella-telemetry"
BATCH_ID = re.compile(r"[0-9a-f]{32}")
PULL_TOKEN = re.compile(r"[0-9a-f]{64}")
# Batches listed per request; the relay gives at most this many.
PAGE = 1000


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


def take_in(repo, files, refused_total, from_inbox):
    """Ingests `files` into data/ and ledger/ and stages them: (ingest id,
    ledger records). A refused file is set aside in .refused/, never
    committed. Files from inbox/ leave the tree; any others (a pull's) are
    copied aside and left where they are, for a try after a push the branch
    refused."""
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
        if from_inbox:
            git(repo, "rm", "-q", "--cached", "--ignore-unmatch", "--", str(path.relative_to(repo)))
        if hashes[path] in refused:
            aside.mkdir(exist_ok=True)
            target = aside / f"{ingest_id}-{path.name}"
            if from_inbox:
                shutil.move(str(path), target)
            else:
                shutil.copyfile(path, target)
            refused_total.append(target)
        elif from_inbox:
            path.unlink()
    git(repo, "add", "--", "data", "ledger")
    return ingest_id, lines


def summary(lines):
    """`1 duplicate, 2 new, 1 refused`: what an ingest recorded."""
    outcomes = {}
    for r in lines:
        key = r.get("outcome", r["record"])
        outcomes[key] = outcomes.get(key, 0) + 1
    return ", ".join(f"{count} {name.replace('_', ' ')}" for name, count in sorted(outcomes.items()))


# The relay.


class NoRedirects(urllib.request.HTTPRedirectHandler):
    """A redirect is an answer, never followed: the token goes nowhere else."""

    def redirect_request(self, *args, **kwargs):
        return None


class Relay:
    """The relay's pull routes. The token is sent only in the
    Authorization header, and never written in a message."""

    def __init__(self, url, token, opener=None):
        self.url = url.rstrip("/")
        self._token = token
        self._open = (opener or urllib.request.build_opener(NoRedirects)).open

    def _call(self, method, path):
        request = urllib.request.Request(
            self.url + path, method=method, headers={"Authorization": f"Bearer {self._token}"}
        )
        try:
            with self._open(request, timeout=30) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()
        except (urllib.error.URLError, OSError) as error:
            reason = getattr(error, "reason", error)
            raise Failed(f"the relay could not be reached: {reason}") from None

    @staticmethod
    def _refused(what, status, data):
        try:
            code = json.loads(data).get("error", "")
        except (ValueError, AttributeError):
            code = ""
        return Failed(f"the relay did not {what}: HTTP {status}" + (f" ({code})" if code else ""))

    def waiting(self):
        """The ids of every batch waiting, in name order."""
        ids, offset = [], 0
        while True:
            status, data = self._call("GET", f"/v1/pull/incoming?limit={PAGE}&offset={offset}")
            if status != 200:
                raise self._refused("list the batches waiting", status, data)
            try:
                page = json.loads(data)
                objects, listed = page["objects"], int(page["listed"])
            except (ValueError, KeyError, TypeError):
                raise Failed("the relay's list of batches could not be read") from None
            ids += [o["batch_id"] for o in objects if BATCH_ID.fullmatch(str(o.get("batch_id", "")))]
            # `listed` counts what Storage gave for the page, batches or not.
            if listed < PAGE:
                return ids
            offset += listed

    def get(self, batch_id):
        """A batch's bytes, or None when it is gone (another pull took it)."""
        status, data = self._call("GET", f"/v1/pull/incoming/{batch_id}")
        if status == 404:
            return None
        if status != 200:
            raise self._refused(f"give batch {batch_id}", status, data)
        return data

    def delete(self, batch_id):
        status, data = self._call("DELETE", f"/v1/pull/incoming/{batch_id}")
        if status not in (200, 204):
            raise self._refused(f"delete batch {batch_id}", status, data)


def relay_from_env():
    token = os.environ.get("ELLA_TELEMETRY_PULL_TOKEN", "").strip()
    if not PULL_TOKEN.fullmatch(token):
        raise Failed("ELLA_TELEMETRY_PULL_TOKEN must hold the pull token (64 hex); see OPERATIONS.md")
    return Relay(os.environ.get("ELLA_TELEMETRY_URL", "").strip() or RELAY, token)


# The commands.


def ingest(repo):
    refused_total = []

    def attempt(ref):
        files = sorted(p for p in (repo / "inbox").iterdir() if p.is_file() and p.name != KEEP)
        if not files:
            print("inbox/ is empty: nothing to take in", file=sys.stderr)
            return None
        ingest_id, lines = take_in(repo, files, refused_total, from_inbox=True)
        return f"Ingest {ingest_id}: {len(files)} file(s), {summary(lines)}\n\nTools: ella_tauri {ref}"

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


def pull_relay(repo, relay):
    refused_total = []
    with locked(repo):
        require_clean(repo)
        pull(repo)
        require_pinned(repo)
        waiting = relay.waiting()
        if not waiting:
            print("the relay has nothing waiting: nothing to take in", file=sys.stderr)
            return 0
        # Outside the checkout: nothing raw can be staged, and nothing is
        # left behind however the run ends.
        with tempfile.TemporaryDirectory(prefix="ella-telemetry-pull-") as folder:
            files, taken = [], []
            for batch_id in waiting:
                data = relay.get(batch_id)
                if data is None:
                    continue
                path = Path(folder) / f"{batch_id}.jsonl"
                path.write_bytes(data)
                files.append(path)
                taken.append(batch_id)
            if not files:
                print("the batches listed were gone: nothing to take in", file=sys.stderr)
                return 0

            def attempt(ref):
                ingest_id, lines = take_in(repo, files, refused_total, from_inbox=False)
                return (
                    f"Ingest {ingest_id}: {len(files)} batch(es) pulled from the relay, {summary(lines)}"
                    f"\n\nTools: ella_tauri {ref}"
                )

            try:
                committed(repo, head(repo), attempt)
            except BaseException:
                for path in refused_total:
                    path.unlink(missing_ok=True)
                raise
        print(f"pushed {head(repo)[:12]}", file=sys.stderr)
        # Only now: every batch taken is in the ledger, on the remote.
        for done, batch_id in enumerate(taken):
            try:
                relay.delete(batch_id)
            except Failed as error:
                raise Failed(
                    f"{error}; pushed, but {len(taken) - done} batch(es) are still at the relay: "
                    "the next pull takes them again, and the ledger records them duplicate"
                ) from None
    if refused_total:
        print(f"{len(refused_total)} batch(es) refused, kept out of Git in {REFUSED}/:", file=sys.stderr)
        for path in refused_total:
            print(f"  {path.name}", file=sys.stderr)
    print(f"{len(taken)} batch(es) taken in and deleted at the relay", file=sys.stderr)
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
        ("pull", "take in the batches waiting at the relay, commit and push, then delete them there"),
        ("report", "write the weekly report, commit and push"),
        ("health", "print collection health; changes nothing"),
    ):
        command = commands.add_parser(name, help=text)
        command.add_argument("--repo", type=Path, required=True, help="a checkout of navgurukul/ella-telemetry")
        if name in ("report", "health"):
            command.add_argument("--week", help="such as 2026-W44 (default: the last whole week, IST)")
    commands.choices["health"].add_argument("--json", action="store_true", help="print the JSON")
    args = parser.parse_args(argv)
    repo = args.repo.resolve()
    try:
        require_repo(repo)
        if args.command == "ingest":
            return ingest(repo)
        if args.command == "pull":
            return pull_relay(repo, relay_from_env())
        if args.command == "report":
            return report(repo, args.week)
        return health(repo, args.week, args.json)
    except Failed as error:
        print(f"telemetry {args.command}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
