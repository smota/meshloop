#!/usr/bin/env python3
"""Load the nodecode-meshloop cell on a throwaway nodecode home and run it.

Starts `nodecode serve` with HOME set to a scratch directory and an emptied
environment, links this cell into that home, opens one session in a fixture
repository, and runs /meshloop doctor, status and inspect through the
gateway's POST /api/gateway/slash, which runs a cell command with no model
turn. It checks the summaries, the failure lines (missing binary, older
binary, no JSON), that the cell changed nothing in the repository or the
meshloop store, and that switching the cell off leaves the meshloop CLI's
answers as they were.

It never touches the operator's nodecode home: every path it writes is under
--work. Run it with `python3 -I`. Exit status 0 means every check held.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

CELL = Path(__file__).resolve().parent.parent
SESSION = "s-MESHLOOPCELL"
GRAPH = "g135"
CHECKS = []


def note(line=""):
    print(line, flush=True)


def check(name, ok, detail=""):
    CHECKS.append((name, bool(ok)))
    note(f"[{'pass' if ok else 'FAIL'}] {name}" + (f": {detail}" if detail and not ok else ""))


def run(argv, cwd, env=None):
    out = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True, timeout=120)
    return out


def git(repo, *args):
    out = run(["git", *args], repo)
    if out.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed: {out.stderr}")
    return out.stdout


def make_fixture(repo, fixture_harness):
    repo.mkdir(parents=True)
    git(repo, "init", "--quiet")
    git(repo, "config", "user.email", "fixture@example.invalid")
    git(repo, "config", "user.name", "Fixture")
    (repo / "README.md").write_text("seed\n")
    (repo / ".gitignore").write_text("/.meshloop/\n/worktrees/\n")
    (repo / "meshloop.toml").write_text(
        f"""selected_harnesses = ["fixture"]
[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 30
[verify]
verify_command = []
[harnesses.fixture]
executable = {json.dumps(str(fixture_harness))}
version_args = ["--version"]
invoke_args_template = ["--prompt-file", "{{prompt_file}}"]
model_ref = "fixture-model"
model_tier = "top"
"""
    )
    (repo / "plan.json").write_text(
        json.dumps(
            {
                "graph_id": GRAPH,
                "nodes": [
                    {"id": 1, "description": "first", "depends_on": [], "tier": None},
                    {"id": 2, "description": "second", "depends_on": [1], "tier": None},
                ],
            }
        )
    )
    git(repo, "add", ".")
    git(repo, "commit", "--quiet", "-m", "seed")


def snapshot(repo):
    """Everything the cell could change: the work tree's status, refs,
    worktrees, and every file under .meshloop/ with its bytes' digest and
    mtime. SQLite's own -wal/-shm sidecars are left out, as the Rust
    read-only tests leave them out."""
    files = {}
    store = repo / ".meshloop"
    if store.exists():
        for path in sorted(store.rglob("*")):
            if path.name.endswith(("-wal", "-shm")) or not path.is_file():
                continue
            data = path.read_bytes()
            files[str(path.relative_to(repo))] = (hashlib.sha256(data).hexdigest(), path.stat().st_mtime_ns)
    return {
        "status": git(repo, "status", "--porcelain", "--untracked-files=all", "--ignored"),
        "refs": git(repo, "for-each-ref", "--format=%(refname) %(objectname)"),
        "worktrees": git(repo, "worktree", "list", "--porcelain"),
        "store": files,
    }


class Organism:
    def __init__(self, nodecode, home, work):
        self.nodecode, self.home, self.work = nodecode, home, work
        self.proc = None
        self.port = None
        self.token = None

    def env(self):
        # An emptied environment: no NODECODE_* port or channel token leaks in
        # from the caller, so the throwaway gateway binds its own port and no
        # channel starts.
        return {"HOME": str(self.home), "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}

    def start(self):
        owner = self.home / ".nodecode" / "history.db.owner"
        if owner.exists():
            owner.unlink()
        log = open(self.work / "serve.log", "a")
        self.proc = subprocess.Popen(
            [self.nodecode, "serve"], cwd=self.work, env=self.env(), stdout=log, stderr=subprocess.STDOUT
        )
        deadline = time.time() + 180
        while time.time() < deadline:
            if self.proc.poll() is not None:
                raise SystemExit(f"nodecode serve exited {self.proc.returncode}; see {self.work / 'serve.log'}")
            try:
                self.port = json.loads(owner.read_text())["port"]
                self.token = (self.home / ".nodecode" / "operator-token").read_text().strip()
                self.call("GET", "/api/gateway/cells")
                return
            except (OSError, ValueError, KeyError, urllib.error.URLError):
                time.sleep(1)
        raise SystemExit("nodecode serve did not answer within 180 s")

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=60)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}",
            data=data,
            method=method,
            headers={"Authorization": f"Bearer {self.token}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                return json.loads(response.read() or b"null")
        except urllib.error.HTTPError as error:
            return {"http_error": error.code, "body": error.read().decode(errors="replace")}

    def slash(self, line):
        answer = self.call("POST", "/api/gateway/slash", {"line": line, "session_id": SESSION})
        text = answer.get("text") if isinstance(answer, dict) else None
        return text if isinstance(text, str) else json.dumps(answer)

    def cell(self):
        listing = self.call("GET", "/api/gateway/cells")
        rows = listing.get("cells", listing) if isinstance(listing, dict) else listing
        for row in rows if isinstance(rows, list) else []:
            if isinstance(row, dict) and row.get("name") == "nodecode-meshloop":
                return row
        return None

    def restart_cell(self):
        return self.call("POST", "/api/gateway/cells", {"name": "nodecode-meshloop"})


def write_config(home, section):
    (home / ".nodecode" / "config.jsonc").write_text(json.dumps({"meshloop": section}, indent=2) + "\n")


def fake_binary(path, body):
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--meshloop", required=True, help="absolute path of a meshloop built from the checkout")
    parser.add_argument("--fixture-harness", required=True, help="absolute path of target/debug/fixture_harness")
    parser.add_argument("--nodecode", default="nodecode")
    parser.add_argument("--work", required=True, help="scratch directory; removed and recreated")
    args = parser.parse_args()

    meshloop = Path(args.meshloop).resolve()
    work = Path(args.work).resolve()
    if work.exists():
        shutil.rmtree(work)
    home, repo = work / "home", work / "fixture"
    (home / ".nodecode" / "cells").mkdir(parents=True)
    (home / ".nodecode" / "cells" / "nodecode-meshloop").symlink_to(CELL, target_is_directory=True)
    write_config(home, {"binary": str(meshloop), "timeout_seconds": 30})
    make_fixture(repo, Path(args.fixture_harness).resolve())
    cell_files_before = git(CELL, "status", "--porcelain", "--ignored", "--untracked-files=all", ".")

    organism = Organism(args.nodecode, home, work)
    organism.start()
    try:
        # The gateway starts cells after recovery, a moment after it answers.
        deadline = time.time() + 120
        row = organism.cell()
        while time.time() < deadline and not (row and row.get("started")):
            time.sleep(1)
            row = organism.cell()
        note(f"cell record: {json.dumps(row)}")
        check(
            "cell loaded and started",
            row is not None and row.get("started") is True and not row.get("refusal"),
            json.dumps(row),
        )
        created = organism.call("POST", "/api/gateway/sessions", {"session_id": SESSION, "cwd": str(repo)})
        check("session opened in the fixture repository", "http_error" not in created, json.dumps(created))

        def answer(line):
            text = organism.slash(line)
            note(f"$ {line}\n{text}\n")
            return text

        before_empty = snapshot(repo)
        text = answer("/meshloop status")
        check("status with no runs", text.startswith("meshloop status: no managed runs in "), text)
        check("no write on an empty repository", snapshot(repo) == before_empty)

        # The operator runs meshloop; the cell only reads what it left.
        out = run([str(meshloop), "run", "--plan", "plan.json", "--accept-plan", "--worktree-base", "worktrees"], repo)
        check("fixture run by the meshloop CLI", out.returncode == 0, out.stderr[-400:])
        cli_status = run([str(meshloop), "status", "--json"], repo).stdout
        before = snapshot(repo)

        text = answer("/meshloop doctor")
        check("doctor summary", text.startswith("meshloop doctor: ready") and "fixture" in text, text)
        text = answer("/meshloop status")
        check(
            "status summary",
            text.startswith(f"meshloop status: graph {GRAPH}, plan PlanAccepted, 2 nodes")
            and "#1 AwaitingReview, waiting for acceptance: first" in text,
            text,
        )
        text = answer("/meshloop inspect")
        check("inspect summary", text.startswith(f"meshloop inspect: graph {GRAPH} awaiting_acceptance"), text)
        text = answer(f"/meshloop inspect {GRAPH}")
        check("inspect GRAPH summary", text.startswith(f"meshloop inspect: graph {GRAPH} "), text)
        text = answer("/meshloop inspect no-such-graph")
        check("unknown graph is one line", text.startswith("meshloop inspect: ") and "\n" not in text, text)
        text = answer("/meshloop status --graph")
        check("flag-shaped graph id refused", text.startswith("meshloop status: ") and "no graph id" in text, text)
        text = answer("/meshloop accept 1")
        check("no other verb", text.startswith("meshloop: ") and "no verb" in text, text)
        text = answer("/meshloop")
        check("bare /meshloop answers its usage", text.startswith("meshloop: /meshloop doctor"), text)
        check("no write to the repository or meshloop state", snapshot(repo) == before)

        # Failure lines: each is one line naming the verb, never a condition.
        failures = {
            "missing binary": (str(work / "no-such-meshloop"), "cannot run"),
            "older binary": (
                str(
                    fake_binary(
                        work / "old-meshloop",
                        'echo \'{"ok": true, "command": "meshloop:status", "data": {"graph_id": "x"}}\'\n',
                    )
                ),
                "may be older than this cell",
            ),
            "no JSON": (
                str(fake_binary(work / "nojson-meshloop", "echo 'error: unknown flag --json' >&2\nexit 2\n")),
                "answered no JSON envelope (exit 2): error: unknown flag --json",
            ),
        }
        for name, (binary, expected) in failures.items():
            write_config(home, {"binary": binary, "timeout_seconds": 30})
            organism.restart_cell()
            text = answer("/meshloop status")
            check(
                f"{name} is one clear line",
                text.startswith("meshloop status: ") and expected in text and "\n" not in text,
                text,
            )

        # Off: the section vetoed, the command gone, meshloop as it was.
        write_config(home, {"enabled": False, "binary": str(meshloop)})
        organism.restart_cell()
        text = answer("/meshloop status")
        check("switched off, /meshloop is gone", "unknown command /meshloop" in text, text)
        after_off = run([str(meshloop), "status", "--json"], repo)
        check("meshloop CLI answers as before with the cell off", after_off.stdout == cli_status)
        check("still no write after switching off", snapshot(repo) == before)
    finally:
        organism.stop()

    cell_files_after = git(CELL, "status", "--porcelain", "--ignored", "--untracked-files=all", ".")
    check("loading wrote nothing into the cell folder", cell_files_before == cell_files_after, cell_files_after)
    failed = [name for name, ok in CHECKS if not ok]
    note(f"{len(CHECKS) - len(failed)} of {len(CHECKS)} checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
