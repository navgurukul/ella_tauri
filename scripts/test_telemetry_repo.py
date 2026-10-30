"""Tests for telemetry-repo.py, against throwaway Git remotes and clones
laid out as navgurukul/ella-telemetry is, with batches made by the scripts
beside it.

  npm run test:telemetry
"""
import contextlib
import fcntl
import http.server
import importlib.util
import io
import json
import os
import subprocess
import tempfile
import threading
import unittest
import unittest.mock
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name, module):
    spec = importlib.util.spec_from_file_location(module, HERE / name)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


telemetry = load("telemetry-repo.py", "telemetry_repo")
policy_tests = load("test_telemetry_policy.py", "test_telemetry_policy")
EVENTS = policy_tests.clean_events()
# The commit the test repository pins, and these scripts are taken to be.
PINNED = "4818e817a545474a39f3b1cbf4d96b1f7f2b588a"
ELSEWHERE = "0123456789abcdef0123456789abcdef01234567"
# The telemetry repository's own rules, as it has them.
GITATTRIBUTES = "*.jsonl -text\n"
GITIGNORE = ".ingest.lock\n.refused/\n"


def sh(cwd, *args):
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True, text=True).stdout


def quietly(*argv):
    """telemetry-repo.py's main, quiet: (exit status, stderr)."""
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as err:
        code = telemetry.main(list(argv))
    return code, err.getvalue()


class Repos(unittest.TestCase):
    """A remote like ella-telemetry, and clones of it: `uploader` puts
    batches in inbox/; `a` and `b` are two maintainers' checkouts."""

    attributes = True

    def setUp(self):
        self.real_tools_commit = telemetry.tools_commit
        telemetry.tools_commit = lambda path=None: (PINNED, True)
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.remote = self.root / "remote.git"
        sh(self.root, "git", "init", "-q", "--bare", "-b", "main", str(self.remote))
        seed = self.clone("seed")
        (seed / ".gitignore").write_text(GITIGNORE)
        if self.attributes:
            (seed / ".gitattributes").write_text(GITATTRIBUTES)
        (seed / "TOOLS_REF").write_text(PINNED + "\n")
        for folder in ("inbox", "data", "ledger", "reports"):
            (seed / folder).mkdir()
            (seed / folder / ".gitkeep").touch()
        sh(seed, "git", "add", ".")
        sh(seed, "git", "commit", "-q", "-m", "seed")
        sh(seed, "git", "push", "-q", "-u", "origin", "main")
        self.uploader, self.a, self.b = self.clone("uploader"), self.clone("a"), self.clone("b")

    def tearDown(self):
        telemetry.tools_commit = self.real_tools_commit
        self.tmp.cleanup()

    def clone(self, name, *config):
        path = self.root / name
        options = [arg for pair in config for arg in ("-c", pair)]
        sh(self.root, "git", "clone", "-q", *options, str(self.remote), str(path))
        sh(path, "git", "config", "user.name", name)
        sh(path, "git", "config", "user.email", f"{name}@example.org")
        return path

    def upload(self, files):
        """Commits files to inbox/ as an upload through GitHub would."""
        sh(self.uploader, "git", "pull", "-q", "--ff-only")
        for name, data in files.items():
            (self.uploader / "inbox" / name).write_bytes(data)
        sh(self.uploader, "git", "add", "inbox")
        sh(self.uploader, "git", "commit", "-q", "-m", "upload")
        sh(self.uploader, "git", "push", "-q")

    def on_remote(self, folder):
        """Files under `folder` at the remote's main, with their contents."""
        listing = sh(self.root, "git", "--git-dir", str(self.remote), "ls-tree", "-r", "--name-only", "main", folder)
        return {
            path: sh(self.root, "git", "--git-dir", str(self.remote), "show", f"main:{path}")
            for path in listing.split() if not path.endswith(".gitkeep")
        }

    def ledger(self):
        return [json.loads(line) for text in self.on_remote("ledger").values() for line in text.splitlines()]

    def stored(self):
        return [json.loads(line) for text in self.on_remote("data").values() for line in text.splitlines()]

    def batch(self, events=EVENTS, **body):
        return policy_tests.app_batch(events, body=body)


class Ingest(Repos):
    def test_batches_are_taken_in_and_leave_inbox_in_one_commit(self):
        batch = self.batch()
        self.upload({"one.jsonl": batch, "again.jsonl": batch})
        code, _ = quietly("ingest", "--repo", str(self.a))
        self.assertEqual(code, 0)
        self.assertEqual(self.on_remote("inbox"), {})
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["duplicate", "new"])
        self.assertEqual(len(self.stored()), len(EVENTS))
        changed = sh(self.a, "git", "show", "--name-status", "--format=", "HEAD").split("\n")
        self.assertIn("D\tinbox/one.jsonl", changed)
        self.assertTrue(any(line.startswith("A\tledger/") for line in changed))
        self.assertTrue(any(line.startswith("A\tdata/") for line in changed))
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")

    def test_a_refused_file_is_set_aside_locally_and_never_committed(self):
        raw = b'{"event":"ella_turn_latency","topic":"Asha said hello"}\n'
        envelope, lines = policy_tests.split(self.batch())
        broken = policy_tests.join(envelope, lines.replace(b'"total_ms":406', b'"total_ms":407'))
        self.upload({"latency-raw.jsonl": raw, "broken.jsonl": broken, "good.jsonl": self.batch(batch_seq=2)})
        code, err = quietly("ingest", "--repo", str(self.a))
        self.assertEqual(code, 1)
        self.assertIn("2 file(s) refused", err)
        aside = sorted(p.name.split("-", 2)[-1] for p in (self.a / ".refused").iterdir())
        self.assertEqual(aside, ["broken.jsonl", "latency-raw.jsonl"])
        self.assertEqual(self.on_remote("inbox"), {})
        self.assertEqual(sorted(r.get("outcome", r["record"]) for r in self.ledger()), ["new", "refused", "refused"])
        tree = sh(self.root, "git", "--git-dir", str(self.remote), "ls-tree", "-r", "--name-only", "main")
        self.assertNotIn(".refused", tree)
        for text in self.on_remote("ledger").values():
            self.assertNotIn("Asha", text)
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")

    def test_an_empty_inbox_commits_nothing(self):
        before = sh(self.a, "git", "rev-parse", "HEAD")
        code, err = quietly("ingest", "--repo", str(self.a))
        self.assertEqual(code, 0)
        self.assertIn("nothing to take in", err)
        self.assertEqual(sh(self.a, "git", "rev-parse", "HEAD"), before)

    def test_two_checkouts_never_record_one_batch_new_twice(self):
        self.upload({"one.jsonl": self.batch()})
        sh(self.b, "git", "pull", "-q", "--ff-only")
        self.assertEqual(quietly("ingest", "--repo", str(self.a))[0], 0)
        # b had pulled before a pushed: its first try works from the old tip.
        real_pull, pulls = telemetry.pull, []

        def stale_first(repo):
            pulls.append(repo)
            if len(pulls) > 1:
                real_pull(repo)

        telemetry.pull = stale_first
        try:
            code, err = quietly("ingest", "--repo", str(self.b))
        finally:
            telemetry.pull = real_pull
        self.assertEqual(code, 0)
        self.assertIn("the branch moved on", err)
        self.assertEqual([r["outcome"] for r in self.ledger()], ["new"])
        self.assertEqual(len(self.stored()), len(EVENTS))
        self.assertEqual(sh(self.b, "git", "status", "--porcelain"), "")
        self.assertEqual(sh(self.b, "git", "rev-parse", "HEAD"), sh(self.a, "git", "rev-parse", "HEAD"))

    def test_the_second_ingest_of_a_reupload_records_a_duplicate(self):
        batch = self.batch()
        self.upload({"one.jsonl": batch})
        quietly("ingest", "--repo", str(self.a))
        self.upload({"one-again.jsonl": batch})
        self.assertEqual(quietly("ingest", "--repo", str(self.b))[0], 0)
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["duplicate", "new"])
        self.assertEqual(len(self.stored()), len(EVENTS))

    def test_a_run_that_fails_before_its_commit_leaves_the_checkout_as_it_was(self):
        self.upload({"one.jsonl": self.batch()})
        real = telemetry.check_staged

        def cut_short(repo):
            raise telemetry.Failed("cut short")

        telemetry.check_staged = cut_short
        try:
            code, err = quietly("ingest", "--repo", str(self.a))
        finally:
            telemetry.check_staged = real
        self.assertEqual(code, 2)
        self.assertIn("cut short", err)
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")
        self.assertTrue((self.a / "inbox" / "one.jsonl").exists())
        self.assertEqual(self.on_remote("ledger"), {})
        # Taken in whole the next time, its events once.
        self.assertEqual(quietly("ingest", "--repo", str(self.a))[0], 0)
        self.assertEqual([r["outcome"] for r in self.ledger()], ["new"])
        self.assertEqual(len(self.stored()), len(EVENTS))

    def test_one_run_at_a_time_in_a_checkout(self):
        with open(self.a / ".ingest.lock", "w") as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            code, err = quietly("ingest", "--repo", str(self.a))
        self.assertEqual(code, 2)
        self.assertIn("another ingest or report is running", err)

    def test_a_checkout_with_changes_of_its_own_is_not_used(self):
        (self.a / "notes.txt").write_text("mine")
        code, err = quietly("ingest", "--repo", str(self.a))
        self.assertEqual(code, 2)
        self.assertIn("changes of its own", err)

    def test_a_commit_may_hold_only_data_ledger_reports_and_inbox_removals(self):
        (self.a / "notes.txt").write_text("stray")
        (self.a / "ledger" / "x.jsonl").write_text("{}\n")
        sh(self.a, "git", "add", "notes.txt", "ledger/x.jsonl")
        sh(self.a, "git", "rm", "-q", "data/.gitkeep")
        with self.assertRaises(telemetry.Failed) as stopped:
            telemetry.check_staged(self.a)
        self.assertIn("A notes.txt", str(stopped.exception))
        self.assertIn("D data/.gitkeep", str(stopped.exception))
        self.assertNotIn("ledger/x.jsonl", str(stopped.exception))


class LineEndings(Repos):
    def test_a_windows_checkout_keeps_a_batchs_bytes(self):
        self.upload({"one.jsonl": self.batch()})
        windows = self.clone("windows", "core.autocrlf=true")
        self.assertNotIn(b"\r\n", (windows / "inbox" / "one.jsonl").read_bytes())
        self.assertEqual(quietly("ingest", "--repo", str(windows))[0], 0)
        self.assertEqual([r["outcome"] for r in self.ledger()], ["new"])


class WithoutAttributes(Repos):
    attributes = False

    def test_without_gitattributes_a_windows_checkout_breaks_the_hash(self):
        # Why .gitattributes is there: the same upload, refused.
        self.upload({"one.jsonl": self.batch()})
        windows = self.clone("windows", "core.autocrlf=true")
        self.assertIn(b"\r\n", (windows / "inbox" / "one.jsonl").read_bytes())
        self.assertEqual(quietly("ingest", "--repo", str(windows))[0], 1)


class Report(Repos):
    def test_the_weekly_report_is_committed_with_its_tools_commit(self):
        self.upload({"one.jsonl": self.batch()})
        quietly("ingest", "--repo", str(self.a))
        code, err = quietly("report", "--repo", str(self.b), "--week", "2026-W43")
        self.assertEqual(code, 0, err)
        reports = self.on_remote("reports")
        self.assertEqual(sorted(reports), ["reports/2026-W43.json", "reports/2026-W43.md"])
        saved = json.loads(reports["reports/2026-W43.json"])
        self.assertEqual(saved["tools_ref"], PINNED)
        self.assertIn("collection", saved)
        self.assertEqual(sh(self.b, "git", "status", "--porcelain"), "")

    def test_health_changes_nothing(self):
        self.upload({"one.jsonl": self.batch()})
        quietly("ingest", "--repo", str(self.a))
        before = sh(self.a, "git", "rev-parse", "HEAD")
        with contextlib.redirect_stdout(io.StringIO()) as out:
            code = telemetry.main(["health", "--repo", str(self.a), "--json"])
        self.assertEqual(code, 0)
        self.assertIn("health_version", out.getvalue())
        self.assertEqual(sh(self.a, "git", "rev-parse", "HEAD"), before)
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")


class Provenance(Repos):
    def test_scripts_other_than_the_pinned_ones_take_nothing_in(self):
        self.upload({"one.jsonl": self.batch()})
        for commit, exact, said in ((ELSEWHERE, True, "TOOLS_REF pins 4818e817a545"), (PINNED, False, "scripts/ has changes")):
            with self.subTest(commit=commit, exact=exact):
                telemetry.tools_commit = lambda path=None: (commit, exact)
                for command in ("ingest", "report"):
                    code, err = quietly(command, "--repo", str(self.a))
                    self.assertEqual(code, 2)
                    self.assertIn(said, err)
                self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")
                self.assertTrue((self.a / "inbox" / "one.jsonl").exists())
        self.assertEqual(self.on_remote("ledger"), {})

    def test_a_new_pin_pulled_during_a_run_is_obeyed(self):
        self.upload({"one.jsonl": self.batch()})
        (self.uploader / "TOOLS_REF").write_text(ELSEWHERE + "\n")
        sh(self.uploader, "git", "commit", "-q", "-am", "Tools: ella_tauri elsewhere")
        sh(self.uploader, "git", "push", "-q")
        code, err = quietly("ingest", "--repo", str(self.a))
        self.assertEqual(code, 2)
        self.assertIn("TOOLS_REF pins 0123456789ab", err)
        self.assertEqual(self.on_remote("ledger"), {})

    def test_each_commit_names_the_tools_that_made_it(self):
        self.upload({"one.jsonl": self.batch()})
        quietly("ingest", "--repo", str(self.a))
        self.assertIn(f"Tools: ella_tauri {PINNED}", sh(self.a, "git", "log", "-1", "--format=%B"))

    def test_health_only_warns(self):
        telemetry.tools_commit = lambda path=None: (ELSEWHERE, True)
        with contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
            code = telemetry.main(["health", "--repo", str(self.a)])
        self.assertEqual(code, 0)
        self.assertIn("note:", err.getvalue())
        self.assertIn("collection health", out.getvalue())

    def test_only_a_telemetry_checkout_is_used(self):
        for broken in ("missing", "short"):
            with self.subTest(broken):
                if broken == "missing":
                    (self.a / "TOOLS_REF").unlink()
                else:
                    (self.a / "TOOLS_REF").write_text("4818e81\n")
                code, err = quietly("ingest", "--repo", str(self.a))
                self.assertEqual(code, 2)
                self.assertIn("TOOLS_REF", err)
                sh(self.a, "git", "checkout", "-q", "--", "TOOLS_REF")

    def test_the_tools_commit_is_read_from_the_checkout(self):
        tools = self.root / "tools"
        tools.mkdir()
        sh(tools, "git", "init", "-q")
        (tools / "latency-sanitize.py").write_text("# the script\n")
        sh(tools, "git", "add", ".")
        sh(tools, "git", "-c", "user.name=t", "-c", "user.email=t@example.org", "commit", "-q", "-m", "tools")
        commit = sh(tools, "git", "rev-parse", "HEAD").strip()
        real = self.real_tools_commit
        self.assertEqual(real(tools), (commit, True))
        (tools / "notes.txt").write_text("not tracked, never loaded")
        self.assertEqual(real(tools), (commit, True))
        (tools / "latency-sanitize.py").write_text("# changed\n")
        self.assertEqual(real(tools), (commit, False))
        plain = self.root / "plain"
        plain.mkdir()
        self.assertEqual(real(plain), (None, False))


def batch_id_of(data):
    return policy_tests.split(data)[0]["batch_id"]


class FakeRelay:
    """The relay's incoming/ in memory: `cut` makes that many deletes fail."""

    def __init__(self, *batches):
        self.objects = {batch_id_of(data): data for data in batches}
        self.cut = 0
        self.asked = []

    def waiting(self):
        self.asked.append("waiting")
        return sorted(self.objects)

    def get(self, batch_id):
        return self.objects.get(batch_id)

    def delete(self, batch_id):
        if self.cut:
            self.cut -= 1
            raise telemetry.Failed(f"the relay did not delete batch {batch_id}: HTTP 503 (storage)")
        self.objects.pop(batch_id, None)


class Pull(Repos):
    def setUp(self):
        super().setUp()
        self.real_relay_from_env = telemetry.relay_from_env
        self.relay = FakeRelay()
        telemetry.relay_from_env = lambda: self.relay
        self.real_mkdtemp = tempfile.mkdtemp
        self.pulled_into = []

        def watched(*args, **kwargs):
            folder = self.real_mkdtemp(*args, **kwargs)
            self.pulled_into.append(Path(folder))
            return folder

        tempfile.mkdtemp = watched

    def tearDown(self):
        telemetry.relay_from_env = self.real_relay_from_env
        tempfile.mkdtemp = self.real_mkdtemp
        super().tearDown()

    def test_batches_waiting_are_taken_in_in_one_commit_then_deleted_at_the_relay(self):
        self.relay = FakeRelay(self.batch(), self.batch(batch_seq=2, events=EVENTS[:1]))
        code, err = quietly("pull", "--repo", str(self.a))
        self.assertEqual(code, 0, err)
        self.assertEqual(self.relay.objects, {})
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["new", "new"])
        # Each event once in data/, though the second batch repeats one.
        self.assertEqual(len(self.stored()), len(EVENTS))
        changed = sh(self.a, "git", "show", "--name-status", "--format=", "HEAD").split()
        self.assertTrue(all(p.startswith(("data/", "ledger/")) for p in changed if "/" in p))
        self.assertIn("pulled from the relay", sh(self.a, "git", "log", "-1", "--format=%B"))
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")
        # Nothing raw stayed anywhere: not in the checkout, not in the temporary folder.
        self.assertEqual(self.pulled_into and [p.exists() for p in self.pulled_into], [False])
        self.assertFalse(self.pulled_into[0].is_relative_to(self.a))

    def test_a_batch_already_taken_in_is_a_duplicate_and_still_deleted(self):
        batch = self.batch()
        self.upload({"one.jsonl": batch})
        quietly("ingest", "--repo", str(self.a))
        self.relay = FakeRelay(batch)
        code, _ = quietly("pull", "--repo", str(self.a))
        self.assertEqual(code, 0)
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["duplicate", "new"])
        self.assertEqual(len(self.stored()), len(EVENTS))
        self.assertEqual(self.relay.objects, {})

    def test_a_refused_batch_is_set_aside_locally_never_committed_and_deleted(self):
        envelope, lines = policy_tests.split(self.batch())
        broken = policy_tests.join(envelope, lines.replace(b'"total_ms":406', b'"total_ms":407'))
        self.relay = FakeRelay(broken, self.batch(batch_seq=2))
        code, err = quietly("pull", "--repo", str(self.a))
        self.assertEqual(code, 1)
        self.assertIn("1 batch(es) refused", err)
        aside = list((self.a / ".refused").iterdir())
        self.assertEqual(len(aside), 1)
        self.assertEqual(aside[0].read_bytes(), broken)
        self.assertEqual(sorted(r.get("outcome", r["record"]) for r in self.ledger()), ["new", "refused"])
        tree = sh(self.root, "git", "--git-dir", str(self.remote), "ls-tree", "-r", "--name-only", "main")
        self.assertNotIn(".refused", tree)
        self.assertEqual(self.relay.objects, {})
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")

    def test_nothing_waiting_commits_nothing(self):
        before = sh(self.a, "git", "rev-parse", "HEAD")
        code, err = quietly("pull", "--repo", str(self.a))
        self.assertEqual(code, 0)
        self.assertIn("nothing waiting", err)
        self.assertEqual(sh(self.a, "git", "rev-parse", "HEAD"), before)

    def test_nothing_is_deleted_at_the_relay_before_the_push(self):
        self.relay = FakeRelay(self.batch())
        real = telemetry.push

        def refused(repo):
            raise telemetry.Failed("git push: remote said no")

        telemetry.push = refused
        try:
            code, err = quietly("pull", "--repo", str(self.a))
        finally:
            telemetry.push = real
        self.assertEqual(code, 2)
        self.assertIn("remote said no", err)
        self.assertEqual(len(self.relay.objects), 1)
        self.assertEqual(self.on_remote("ledger"), {})
        self.assertEqual(sh(self.a, "git", "status", "--porcelain"), "")
        self.assertEqual(self.pulled_into and [p.exists() for p in self.pulled_into], [False])
        # The next pull takes it in whole.
        self.assertEqual(quietly("pull", "--repo", str(self.a))[0], 0)
        self.assertEqual([r["outcome"] for r in self.ledger()], ["new"])
        self.assertEqual(self.relay.objects, {})

    def test_a_pull_cut_short_after_its_push_takes_the_rest_again_as_duplicates(self):
        self.relay = FakeRelay(self.batch(), self.batch(batch_seq=2))
        self.relay.cut = 1
        code, err = quietly("pull", "--repo", str(self.a))
        self.assertEqual(code, 2)
        self.assertIn("pushed, but 2 batch(es) are still at the relay", err)
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["new", "new"])
        self.assertEqual(len(self.relay.objects), 2)
        self.assertEqual(quietly("pull", "--repo", str(self.b))[0], 0)
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["duplicate", "duplicate", "new", "new"])
        self.assertEqual(len(self.stored()), len(EVENTS))
        self.assertEqual(self.relay.objects, {})

    def test_a_pull_whose_push_the_branch_refused_takes_its_batches_in_again(self):
        self.upload({"one.jsonl": self.batch()})
        sh(self.b, "git", "pull", "-q", "--ff-only")
        self.assertEqual(quietly("ingest", "--repo", str(self.a))[0], 0)
        self.relay = FakeRelay(self.batch(batch_seq=2))
        real_pull, pulls = telemetry.pull, []

        def stale_first(repo):
            pulls.append(repo)
            if len(pulls) > 1:
                real_pull(repo)

        telemetry.pull = stale_first
        try:
            code, err = quietly("pull", "--repo", str(self.b))
        finally:
            telemetry.pull = real_pull
        self.assertEqual(code, 0, err)
        self.assertIn("the branch moved on", err)
        self.assertEqual(sorted(r["outcome"] for r in self.ledger()), ["new", "new"])
        self.assertEqual(self.relay.objects, {})

    def test_a_batch_another_pull_took_meanwhile_is_skipped(self):
        self.relay = FakeRelay(self.batch())
        gone = FakeRelay.get
        self.relay.get = lambda batch_id: None
        code, err = quietly("pull", "--repo", str(self.a))
        self.relay.get = gone.__get__(self.relay)
        self.assertEqual(code, 0)
        self.assertIn("were gone", err)
        self.assertEqual(self.on_remote("ledger"), {})

    def test_scripts_other_than_the_pinned_ones_pull_nothing(self):
        self.relay = FakeRelay(self.batch())
        telemetry.tools_commit = lambda path=None: (ELSEWHERE, True)
        code, err = quietly("pull", "--repo", str(self.a))
        self.assertEqual(code, 2)
        self.assertIn("TOOLS_REF pins", err)
        self.assertEqual(self.relay.asked, [])
        self.assertEqual(len(self.relay.objects), 1)

    def test_the_pull_token_must_be_set(self):
        telemetry.relay_from_env = self.real_relay_from_env
        for value in (None, "", "short", "AB" * 32):
            with self.subTest(value=value):
                env = {k: v for k, v in os.environ.items() if k != "ELLA_TELEMETRY_PULL_TOKEN"}
                if value is not None:
                    env["ELLA_TELEMETRY_PULL_TOKEN"] = value
                with unittest.mock.patch.dict(os.environ, env, clear=True):
                    code, err = quietly("pull", "--repo", str(self.a))
                self.assertEqual(code, 2)
                self.assertIn("ELLA_TELEMETRY_PULL_TOKEN must hold the pull token", err)


class RelayClient(unittest.TestCase):
    """telemetry-repo.py's relay client against a real HTTP server."""

    TOKEN = "9f" * 32

    def setUp(self):
        self.seen = []
        self.answers = {}
        test = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def answer(self):
                test.seen.append((self.command, self.path, self.headers.get("Authorization")))
                status, body, headers = test.answers.get((self.command, self.path.split("?")[0]), (404, b"{}", {}))
                if callable(body):
                    body = body(self.path)
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            do_GET = do_DELETE = answer

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/functions/v1/ella-telemetry"
        self.relay = telemetry.Relay(self.url + "/", self.TOKEN)
        self.base = "/functions/v1/ella-telemetry"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def test_it_lists_every_page_reads_and_deletes_with_the_token(self):
        ids = [f"{n:032x}" for n in range(5)]
        # What Storage holds: an object the relay does not list among them.
        held = ["notes.txt"] + ids
        real_page = telemetry.PAGE
        telemetry.PAGE = 2

        def page(path):
            offset = int(path.split("offset=")[1])
            shown = held[offset:offset + 2]
            objects = [{"batch_id": i, "size": 1, "created_at": None} for i in shown if i in ids]
            return json.dumps({"objects": objects, "listed": len(shown)}).encode()

        self.answers[("GET", f"{self.base}/v1/pull/incoming")] = (200, page, {})
        self.answers[("GET", f"{self.base}/v1/pull/incoming/{ids[0]}")] = (200, b"{}\n", {})
        self.answers[("DELETE", f"{self.base}/v1/pull/incoming/{ids[0]}")] = (204, b"", {})
        try:
            self.assertEqual(self.relay.waiting(), ids)
        finally:
            telemetry.PAGE = real_page
        self.assertEqual(self.relay.get(ids[0]), b"{}\n")
        self.assertIsNone(self.relay.get(ids[1]))
        self.relay.delete(ids[0])
        self.assertTrue(all(auth == f"Bearer {self.TOKEN}" for _, _, auth in self.seen))
        self.assertIn(("GET", f"{self.base}/v1/pull/incoming?limit=2&offset=6", f"Bearer {self.TOKEN}"), self.seen)

    def test_a_refusal_names_its_status_and_never_the_token(self):
        self.answers[("GET", f"{self.base}/v1/pull/incoming")] = (401, b'{"error":"unauthorized"}', {})
        with self.assertRaises(telemetry.Failed) as stopped:
            self.relay.waiting()
        self.assertIn("HTTP 401 (unauthorized)", str(stopped.exception))
        self.assertNotIn(self.TOKEN, str(stopped.exception))
        self.answers[("DELETE", f"{self.base}/v1/pull/incoming/{'a' * 32}")] = (503, b"<html>", {})
        with self.assertRaises(telemetry.Failed) as stopped:
            self.relay.delete("a" * 32)
        self.assertIn("HTTP 503", str(stopped.exception))

    def test_a_redirect_is_never_followed(self):
        self.answers[("GET", f"{self.base}/v1/pull/incoming")] = (302, b"", {"Location": "/elsewhere"})
        with self.assertRaises(telemetry.Failed) as stopped:
            self.relay.waiting()
        self.assertIn("HTTP 302", str(stopped.exception))
        self.assertEqual([path for _, path, _ in self.seen], [f"{self.base}/v1/pull/incoming?limit=1000&offset=0"])

    def test_a_relay_that_cannot_be_reached_stops_the_pull_without_the_token(self):
        closed = telemetry.Relay("http://127.0.0.1:9", self.TOKEN)
        with self.assertRaises(telemetry.Failed) as stopped:
            closed.waiting()
        self.assertIn("could not be reached", str(stopped.exception))
        self.assertNotIn(self.TOKEN, str(stopped.exception))


if __name__ == "__main__":
    unittest.main()
