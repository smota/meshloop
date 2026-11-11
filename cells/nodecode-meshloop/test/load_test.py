#!/usr/bin/env python3
"""Load the nodecode-meshloop cell on a throwaway nodecode home and run it.

Starts `nodecode serve` with HOME set to a scratch directory and an emptied
environment, links this cell into that home, opens sessions in fixture
repositories, and runs /meshloop commands through the gateway's
POST /api/gateway/slash, which runs a cell command with no model turn.

Read-only: doctor, status, inspect, report and review-plan's plan view, the
failure lines (missing binary, older binary, no JSON, timeout, noisy stderr),
that these changed nothing in the repository or the meshloop store, and that
switching the cell off leaves the meshloop CLI's answers as they were.

Writes (#136): with meshloop.write_commands off every write command refuses in
one line and runs nothing; accept and review-plan refuse without an identity
and pass the typed one as --as (read back from meshloop's event log); the :tool
veto refuses calls that run accept, review-plan or integrate (exercised through
/meshloop hook-check, which runs the installed hook function on a synthetic
call, since a real tool call needs a model turn); a fixture graph goes plan,
review-plan, run (detached), accept, resume, integrate; and cancel leaves no
process and no worktree meshloop does not own.

Routing priors (#139): /meshloop learn, priors show and priors check only
read meshloop; learn refuses a --out outside the repository's .agent-runs/
before it reads anything; /meshloop priors accept and revoke refuse and run
nothing; the :tool veto refuses priors accept and revoke and lets show and check
through. Against a binary without the #139 attempt fields learn fails closed in
one line and writes no proposal. With a binary that has `meshloop priors', a
round trip with no model turn: a fixture run of five independent nodes whose
template passes {model_ref} is folded by /meshloop learn into a proposal
(checked against the v1 rules and its SHA-256), read back by /meshloop priors
check, checked and accepted by the operator in a shell (`meshloop priors
accept --as test-operator', never the cell), and a second run is routed with
the priors in effect: priors show reports in_effect and routed attempts.
Without that verb those checks are skipped with a note.

Board (#137): with meshloop.board off nothing is posted; on, exactly one pointer
post per event of a fixture run and none again after a second sync, a lost
cursor write, an interrupted run and resume, or a cell restart; posts saying
accept change nothing; no post mentions anyone with wake_on_gates off, and with
it on only current gate posts mention the supervising session, which the Board
wakes (its turn fails at once: the throwaway home has no model).

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
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

CELL = Path(__file__).resolve().parent.parent
SESSION = "s-MESHLOOPCELL"
GRAPH = "g135"
DRIVE_GRAPH = "g136"
SECOND_GRAPH = "g136b"
CANCEL_GRAPH = "g136c"
CHECKS = []
WRITE_REFUSED = "refused: write commands are off"


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


def make_fixture(repo, fixture_harness, invoke=None, graph=GRAPH, nodes=None, extra=None):
    """A git repository whose meshloop.toml runs the fixture harness with
    INVOKE, and a committed plan.json for GRAPH."""
    repo.mkdir(parents=True)
    git(repo, "init", "--quiet")
    git(repo, "config", "user.email", "fixture@example.invalid")
    git(repo, "config", "user.name", "Fixture")
    (repo / "README.md").write_text("seed\n")
    (repo / ".gitignore").write_text("/.meshloop/\n/worktrees/\n")
    invoke = invoke or ["--prompt-file", "{prompt_file}"]
    (repo / "meshloop.toml").write_text(
        f"""selected_harnesses = ["fixture"]
[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 120
[verify]
verify_command = []
[harnesses.fixture]
executable = {json.dumps(str(fixture_harness))}
version_args = ["--version"]
invoke_args_template = {json.dumps(invoke)}
model_ref = "fixture-model"
model_tier = "top"
"""
    )
    nodes = nodes or [
        {"id": 1, "description": "first", "depends_on": [], "tier": None},
        {"id": 2, "description": "second", "depends_on": [1], "tier": None},
    ]
    (repo / "plan.json").write_text(json.dumps({"graph_id": graph, "nodes": nodes}))
    for name, text in (extra or {}).items():
        (repo / name).write_text(text)
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

    def slash(self, line, session=SESSION):
        answer = self.call("POST", "/api/gateway/slash", {"line": line, "session_id": session})
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


def processes_in(path):
    """(pid, command line) of every process whose working directory is PATH
    or under it: what meshloop or a harness left running there."""
    found = []
    root = str(Path(path).resolve())
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            cwd = os.readlink(entry / "cwd")
            cmdline = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").strip()
        except OSError:
            continue
        if cwd == root or cwd.startswith(root + "/"):
            found.append((int(entry.name), cmdline))
    return found


def cli_json(meshloop, repo, *args):
    out = run([str(meshloop), *args, "--json"], repo)
    try:
        return json.loads(out.stdout)
    except ValueError:
        return {"ok": False, "error": f"no JSON (exit {out.returncode}): {out.stderr[-300:]}"}


def wait_until(what, probe, seconds=90):
    """PROBE() once it is truthy, polled; None after SECONDS."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(0.5)
    note(f"timed out waiting for {what}")
    return None


def node_states(meshloop, repo, graph):
    status = cli_json(meshloop, repo, "status", "--graph", graph)
    data = status.get("data") or {}
    return {n["id"]: n["state"] for n in data.get("nodes", [])}, data


def fake_binary(path, body):
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)
    return path


MARKER = ".nodecode-meshloop-load-test"


def prepare_work(path):
    """WORK made empty, refusing anything this script may not own: a path
    outside the repository's .agent-runs/, the temp directory or $RUNNER_TEMP,
    or an existing directory without the marker this script writes."""
    work = path.resolve()
    roots = [CELL.parent.parent / ".agent-runs", Path(tempfile.gettempdir())]
    if os.environ.get("RUNNER_TEMP"):
        roots.append(Path(os.environ["RUNNER_TEMP"]))
    roots = [root.resolve() for root in roots]
    if not any(work != root and root in work.parents for root in roots):
        raise SystemExit(f"--work {work} must be inside one of: {', '.join(map(str, roots))}")
    if work.exists():
        if not (work / MARKER).is_file():
            raise SystemExit(f"--work {work} exists and was not made by this script; remove it yourself")
        shutil.rmtree(work)
    work.mkdir(parents=True)
    (work / MARKER).write_text("made by cells/nodecode-meshloop/test/load_test.py\n")
    return work


def has_priors(meshloop, repo):
    """Whether MESHLOOP has the #139 kernel side: `priors show --json` answers
    an envelope that is ok."""
    out = run([str(meshloop), "priors", "show", "--json"], repo)
    try:
        return json.loads(out.stdout).get("ok") is True
    except ValueError:
        return False


def proposal_holds_v1_rules(path):
    """The v1 file rules a schema validator would check and the loader's own:
    (ok, why). Python -I has no jsonschema here; the pure test's sample is
    validated against the schema separately."""
    doc = json.loads(Path(path).read_text())
    if doc.get("schema") != "meshloop/advisory-priors" or doc.get("version") != 1:
        return False, "schema or version"
    if set(doc) - {"schema", "version", "generated_at", "generator", "source", "priors"}:
        return False, f"top-level fields {sorted(doc)}"
    keys = set()
    for entry in doc["priors"]:
        if set(entry) != {"harness", "model_ref", "tier", "dispatch_mode", "attempts", "successes", "provenance"}:
            return False, f"entry fields {sorted(entry)}"
        key = (entry["harness"], entry["model_ref"], entry["tier"], entry["dispatch_mode"])
        refs = [(r["graph_id"], r["task_id"], r["attempt_id"]) for r in entry["provenance"]]
        if key in keys or entry["dispatch_mode"] != "subprocess" or entry["model_ref"] == "unverified":
            return False, f"key {key}"
        keys.add(key)
        if len(refs) != len(set(refs)) or len(refs) != entry["attempts"]:
            return False, f"provenance of {key}"
        if entry["successes"] != sum(1 for r in entry["provenance"] if r["passed"] is True):
            return False, f"successes of {key}"
    return True, ""


def priors_section(answer, home, repo, meshloop):
    """#139 on the read-only fixture: learn, priors show|check, the refusals and the veto."""
    proposals = home / ".nodecode" / "meshloop" / "priors"
    kernel = has_priors(meshloop, repo)
    note(f"[note] meshloop priors {'present' if kernel else 'absent'} in {meshloop}")

    text = answer("/meshloop learn --graph g135 --out ../escape.json")
    check(
        "learn refuses a --out with a .. step before reading meshloop",
        text.startswith("meshloop learn: ") and "has a .. step" in text and "\n" not in text,
        text,
    )
    text = answer("/meshloop learn --graph g135 --out /tmp/escape.json")
    check("learn refuses a --out outside .agent-runs/", text.startswith("meshloop learn: ") and "is outside" in text, text)
    text = answer("/meshloop learn --force")
    check("learn refuses an unknown flag", text.startswith("meshloop learn: takes no --force"), text)
    for verb, line in (("accept", "/meshloop priors accept --file .agent-runs/p.json --as person-one"),
                       ("revoke", "/meshloop priors revoke --as person-one")):
        text = answer(line)
        check(
            f"/meshloop priors {verb} refuses and runs nothing",
            text.startswith("meshloop priors: refused: this cell never") and f"meshloop priors {verb}" in text
            and "\n" not in text,
            text,
        )
    for name, form in {
        "priors accept": '(sh "meshloop priors accept --file .agent-runs/p.json --as me")',
        "priors revoke": '(uiop:run-program \'("meshloop" "priors" "revoke" "--as" "me"))',
        "priors revoke, slash line": '(post "/api/gateway/slash" "{\\"line\\":\\"/meshloop priors revoke --as me\\"}")',
    }.items():
        text = answer(f"/meshloop hook-check {form}")
        check(
            f"tool veto refuses: {name}",
            text.startswith("meshloop hook-check: refused: ERROR: refused by nodecode-meshloop")
            and "hook on this organism's :tool chain" in text,
            text,
        )
    for name, form in {
        "priors show": '(sh "meshloop priors show --json")',
        "priors check": '(sh "meshloop priors check --file .agent-runs/p.json --json")',
    }.items():
        text = answer(f"/meshloop hook-check {form}")
        check(f"tool veto lets through: {name}", text.startswith("meshloop hook-check: passes"), text)

    text = answer("/meshloop learn --graph g135")
    if not kernel:
        check(
            "learn fails closed on a binary without the #139 attempt fields, in one line, writing nothing",
            text.startswith("meshloop learn: the answer lacks data.nodes[]") and "may be older than this cell" in text
            and "\n" not in text and not proposals.exists(),
            text,
        )
        text = answer("/meshloop priors show")
        check("priors show says unknown on a binary without priors", text.startswith("meshloop priors: promoted priors in effect: unknown"), text)
        note("[skip] learn's proposal, priors show and priors check against #139 fields: this meshloop has no `priors' verb")
        return
    # The main fixture's template passes no {model_ref}: model_passed is false, so
    # nothing counts and nothing is written.
    check(
        "learn: an attempt without {model_ref} is no evidence; no proposal",
        text.startswith("meshloop learn: no attempt counts as evidence") and "model_not_passed 1" in text
        and not proposals.exists(),
        text,
    )
    text = answer("/meshloop priors show")
    check(
        "priors show: nothing promoted, and why",
        text == "meshloop priors: promoted priors in effect: none (not_promoted)",
        text,
    )


def priors_proposal_section(answer, organism, work, meshloop, fixture_harness):
    """#139 round trip, no model turn: a fixture run whose template passes
    {model_ref}, folded by /meshloop learn into a proposal under .agent-runs/;
    the operator checks and accepts it in a shell (never the cell); a second
    run is then routed with the priors in effect, and priors show says so."""
    repo = work / "priors"
    # No plan tier: meshloop assigns one at plan time (an independent node is Tier1),
    # reports it in inspect, and routes with it; the fold keys evidence by it.
    independent = [{"id": i, "description": f"node {i}", "depends_on": [], "tier": None} for i in range(1, 6)]
    make_fixture(
        repo,
        fixture_harness,
        invoke=["--model", "{model_ref}", "--prompt-file", "{prompt_file}"],
        graph="g139",
        nodes=independent,
        extra={
            ".gitignore": "/.meshloop/\n/worktrees/\n/.agent-runs/\n",
            "plan2.json": json.dumps({"graph_id": "g139b", "nodes": [
                {"id": 1, "description": "routed with priors", "depends_on": [], "tier": None}]}),
        },
    )
    created = organism.call("POST", "/api/gateway/sessions", {"session_id": "s-PRIORS", "cwd": str(repo)})
    check("priors session opened", "http_error" not in created, json.dumps(created))
    if not has_priors(meshloop, repo):
        note("[skip] the priors round trip: this meshloop has no `priors' verb (the #139 kernel side)")
        return
    out = run([str(meshloop), "run", "--plan", "plan.json", "--accept-plan", "--worktree-base", "worktrees"], repo)
    check("priors fixture run (5 independent nodes) by the meshloop CLI", out.returncode == 0, out.stderr[-400:])
    store_before = snapshot(repo)["store"]

    text = answer("/meshloop learn --graph g139 --out .agent-runs/priors/p.json", "s-PRIORS")
    path = repo / ".agent-runs" / "priors" / "p.json"
    check(
        "learn writes the proposal and lists each prior key with its counts",
        text.startswith("meshloop learn: wrote a routing-prior proposal to ") and path.is_file()
        and "prior fixture / fixture-model / Tier1: 5 attempts, 5 passed, 0 failed; clears the kernel's floor (n >= 5)" in text
        and "excluded by reason: not_subprocess 0, model_not_passed 0, model_unverified 0, cancelled 0, no_check_result 0" in text
        and "promoted priors in effect: none" in text
        and f"meshloop priors accept --file {path} --as <your-name>" in text,
        text,
    )
    if not path.is_file():
        return
    ok, why = proposal_holds_v1_rules(path)
    check("the proposal holds the v1 rules", ok, why)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    check("the digest names the file's SHA-256", f"sha256:{digest}" in text, digest)
    check("learn wrote nothing to the meshloop store", snapshot(repo)["store"] == store_before)
    text = answer("/meshloop learn --graph g139 --out .agent-runs/priors/p.json", "s-PRIORS")
    check("learn refuses to overwrite a proposal", "exists; name a new file" in text, text)
    text = answer("/meshloop priors check .agent-runs/priors/p.json", "s-PRIORS")
    check(
        "/meshloop priors check reads the proposal: one usable entry, nothing promoted",
        text.startswith("meshloop priors check: .agent-runs/priors/p.json: 1 usable, 0 ignored")
        and "usable fixture / fixture-model / Tier1: 5 attempts, 5 passed" in text and "nothing promoted" in text,
        text,
    )
    check("priors check wrote nothing to the meshloop store", snapshot(repo)["store"] == store_before)

    # The operator, in a shell: check, then accept, naming themselves. Never the cell.
    shell_check = cli_json(meshloop, repo, "priors", "check", "--file", str(path))
    data = shell_check.get("data") or {}
    check(
        "operator's meshloop priors check: no file fault, the entry usable, its references resolved",
        shell_check.get("ok") is True and data.get("file_fault") is None and len(data.get("usable", [])) == 1
        and data.get("resolved_refs") == 5 and data.get("unresolved_refs") == 0
        and data.get("priors_sha256") == digest,
        json.dumps(shell_check)[:600],
    )
    accepted = run([str(meshloop), "priors", "accept", "--file", str(path), "--as", "test-operator"], repo)
    check("operator's meshloop priors accept --as test-operator", accepted.returncode == 0, accepted.stderr[-400:])
    text = answer("/meshloop priors show", "s-PRIORS")
    check(
        "/meshloop priors show: in effect, accepted by test-operator, no routed attempt yet",
        text.startswith(f"meshloop priors: promoted priors in effect: {digest} (ledger row ")
        and ", accepted by test-operator); 1 usable entry; changed_top on 0 of 0 routed attempts" in text,
        text,
    )

    # A second run is routed with the priors in effect.
    out = run([str(meshloop), "run", "--plan", "plan2.json", "--accept-plan", "--worktree-base", "worktrees"], repo)
    check("second run by the meshloop CLI", out.returncode == 0, out.stderr[-400:])
    shown = cli_json(meshloop, repo, "priors", "show")
    data = shown.get("data") or {}
    routing = data.get("routing") or {}
    check(
        "priors show after the second run: in effect, routing.attempts > 0",
        data.get("in_effect") is True and data.get("priors_sha256") == digest
        and (routing.get("attempts") or 0) > 0 and (data.get("row") or {}).get("identity") == "test-operator",
        json.dumps(shown)[:600],
    )
    text = answer("/meshloop priors show", "s-PRIORS")
    check(
        "/meshloop priors show counts the routed attempts",
        f"changed_top on {routing.get('changed_top')} of {routing.get('attempts')} routed attempt" in text,
        text,
    )
    text = answer("/meshloop learn --graph g139 --graph g139b --out .agent-runs/priors/p2.json", "s-PRIORS")
    check(
        "learn after promotion: both runs folded, the promoted digest named",
        text.startswith("meshloop learn: wrote a routing-prior proposal to ")
        and "folded 2 graphs (g139, g139b)" in text and "prior fixture / fixture-model / Tier1: 6 attempts, 6 passed" in text
        and f"promoted priors in effect: {digest}" in text,
        text,
    )


def drive_section(answer, organism, home, work, meshloop, fixture_harness):
    """#136: the commands that write, on fixture repositories of their own."""
    write_config(home, {"binary": str(meshloop), "timeout_seconds": 30, "write_commands": True})
    organism.restart_cell()

    planner_repo = work / "planner"
    make_fixture(planner_repo, fixture_harness, invoke=["--emit-graph"])
    drive_repo = work / "drive"
    second = json.dumps(
        {"graph_id": SECOND_GRAPH, "nodes": [{"id": 1, "description": "another", "depends_on": [], "tier": None}]}
    )
    make_fixture(drive_repo, fixture_harness, graph=DRIVE_GRAPH, extra={"plan-second.json": second})
    cancel_repo = work / "cancel"
    make_fixture(
        cancel_repo,
        fixture_harness,
        invoke=["--prompt-file", "{prompt_file}", "60000"],
        graph=CANCEL_GRAPH,
        nodes=[{"id": 1, "description": "slow, to cancel", "depends_on": [], "tier": None}],
    )
    sessions = {"s-MESHLOOPPLAN": planner_repo, "s-MESHLOOPDRIVE": drive_repo, "s-MESHLOOPCANCEL": cancel_repo}
    for session, repo in sessions.items():
        created = organism.call("POST", "/api/gateway/sessions", {"session_id": session, "cwd": str(repo)})
        check(f"session {session} opened", "http_error" not in created, json.dumps(created))

    # plan and the plan view, on a repository whose harness plans.
    text = answer("/meshloop plan --out planned.json add a widget", "s-MESHLOOPPLAN")
    check(
        "plan writes a plan and starts nothing",
        text.startswith("meshloop plan: graph fixture-") and "2 nodes, written to planned.json" in text
        and "Nothing runs yet" in text,
        text,
    )
    text = answer("/meshloop review-plan --plan planned.json", "s-MESHLOOPPLAN")
    check("the new plan is shown, not reviewed", "not reviewed in this repository yet" in text, text)
    check("plan staged nothing", cli_json(meshloop, planner_repo, "status").get("data", {}).get("runs") == [])

    d = "s-MESHLOOPDRIVE"
    # (b) Identity: refused without one, passed as typed with one.
    text = answer("/meshloop review-plan --plan plan.json --accept", d)
    check("review-plan --accept without --as refuses", text.startswith("meshloop review-plan: names no identity"), text)
    text = answer("/meshloop review-plan --plan plan.json --accept --as --graph", d)
    check("a flag is no identity", text.startswith("meshloop review-plan: ") and "no identity" in text, text)
    text = answer("/meshloop run --plan plan.json", d)
    check("run refuses a plan nobody accepted", text.startswith("meshloop run: exited 2") and "acceptance" in text, text)
    text = answer("/meshloop review-plan --plan plan.json --accept --as person-one", d)
    check(
        "review-plan --accept --as records the decision",
        text.startswith(f"meshloop review-plan: graph {DRIVE_GRAPH}, accept as person-one: PlanAccepted"),
        text,
    )
    text = answer("/meshloop run --plan plan.json", d)
    check(
        "run starts a detached driver and points to inspect",
        text.startswith(f"meshloop run: graph {DRIVE_GRAPH}, driver started detached (running)")
        and f"/meshloop inspect {DRIVE_GRAPH}" in text,
        text,
    )
    states = wait_until("node 1 to await review", lambda: (node_states(meshloop, drive_repo, DRIVE_GRAPH)[0].get(1) == "AwaitingReview") or None)
    check("the detached run reached the gate", states is not None)
    text = answer(f"/meshloop inspect {DRIVE_GRAPH}", d)
    check("inspect shows the gate", "#1 AwaitingReview, waiting for acceptance" in text, text)
    text = answer(f"/meshloop accept --task 1 --as you --graph {DRIVE_GRAPH}", d)
    check("a placeholder identity refuses", text.startswith('meshloop accept: "you" is a placeholder'), text)
    text = answer(f"/meshloop accept --task 1 --as <your-name> --graph {DRIVE_GRAPH}", d)
    check("the hint's own placeholder refuses", "is a placeholder" in text, text)
    # A second run waiting for acceptance: accept without --graph must ask which.
    answer("/meshloop review-plan --plan plan-second.json --accept --as person-one", d)
    answer("/meshloop run --plan plan-second.json", d)
    waiting = wait_until(
        "the second run to await review",
        lambda: (node_states(meshloop, drive_repo, SECOND_GRAPH)[0].get(1) == "AwaitingReview") or None,
    )
    check("the second run reached the gate", waiting is not None)
    text = answer("/meshloop accept --task 1 --as person-one", d)
    check(
        "accept without --graph refuses when two runs wait",
        text.startswith("meshloop accept: 2 runs wait for acceptance") and "--graph" in text,
        text,
    )
    text = answer(f"/meshloop accept --task 1 --graph {DRIVE_GRAPH}", d)
    check("accept without --as refuses", text.startswith("meshloop accept: names no identity"), text)
    check("nothing was accepted", node_states(meshloop, drive_repo, DRIVE_GRAPH)[0].get(1) == "AwaitingReview")
    text = answer(f"/meshloop accept --task 1 --as person-one --graph {DRIVE_GRAPH}", d)
    check(
        "accept passes the typed identity as --as",
        text.startswith(f"meshloop accept: task 1 of graph {DRIVE_GRAPH} accepted as person-one; meshloop recorded \"accepted by person-one at "),
        text,
    )
    attempts = cli_json(meshloop, drive_repo, "inspect", "--attempts", "--graph", DRIVE_GRAPH)
    evidence = [
        e.get("summary_redacted", "")
        for n in attempts.get("data", {}).get("nodes", [])
        if n.get("id") == 1
        for a in n.get("attempts", [])
        for e in a.get("evidence", [])
        if e.get("kind") == "human_acceptance"
    ]
    check("the event log holds the acceptance by person-one", any("accepted by person-one" in e for e in evidence), json.dumps(evidence))
    text = answer(f"/meshloop resume {DRIVE_GRAPH}", d)
    check("resume starts a detached driver", text.startswith(f"meshloop resume: graph {DRIVE_GRAPH}, driver started detached"), text)
    states = wait_until("node 2 to await review", lambda: (node_states(meshloop, drive_repo, DRIVE_GRAPH)[0].get(2) == "AwaitingReview") or None)
    check("node 2 reached the gate after resume", states is not None)
    text = answer(f"/meshloop accept --task 2 --as person-one --graph {DRIVE_GRAPH}", d)
    check("second accept", "accepted as person-one" in text, text)
    answer(f"/meshloop resume {DRIVE_GRAPH}", d)

    def complete():
        states, data = node_states(meshloop, drive_repo, DRIVE_GRAPH)
        return states and all(v == "Integrated" for v in states.values()) and data.get("driver") != "alive"

    check("every node integrated into the integrate worktree", wait_until("the graph to complete", complete) is not None)
    branch = git(drive_repo, "symbolic-ref", "--short", "HEAD").strip()
    head_before = git(drive_repo, "rev-parse", "HEAD").strip()
    text = answer(f"/meshloop integrate --graph {DRIVE_GRAPH} --into {branch}", d)
    check(
        "integrate without --accept-integrate points at the worktree and lands nothing",
        text.startswith(f"meshloop integrate: graph {DRIVE_GRAPH} not landed. Review the integrate worktree")
        and git(drive_repo, "rev-parse", "HEAD").strip() == head_before,
        text,
    )
    text = answer(f"/meshloop integrate --graph {DRIVE_GRAPH} --into {branch} --accept-integrate", d)
    check("integrate lands on the branch", text.startswith(f"meshloop integrate: graph {DRIVE_GRAPH} landed on {branch}; 2 Integrated"), text)
    check("the branch moved", git(drive_repo, "rev-parse", "HEAD").strip() != head_before)
    worktrees = [
        line for line in git(drive_repo, "worktree", "list", "--porcelain").splitlines()
        if line.startswith("worktree ") and f"/{DRIVE_GRAPH}/" in line
    ]
    branches = git(drive_repo, "for-each-ref", "--format=%(refname)", f"refs/heads/meshloop/{DRIVE_GRAPH}")
    check("the landed run left no worktree or branch (auto-reclaim)", not worktrees and not branches.strip(), f"{worktrees} {branches}")
    text = answer(f"/meshloop report {DRIVE_GRAPH}", d)
    check(
        "report after landing",
        "dispatched: fixture (model_ref fixture-model, 2 attempts)" in text
        and "accepted by person-one at" in text,
        text,
    )

    # (d) cancel: a slow worker, cancelled from the cell.
    c = "s-MESHLOOPCANCEL"
    answer("/meshloop review-plan --plan plan.json --accept --as person-one", c)
    answer("/meshloop run --plan plan.json", c)
    live = wait_until(
        "the slow attempt to start",
        lambda: any(n.get("live") == "Live" for n in node_states(meshloop, cancel_repo, CANCEL_GRAPH)[1].get("nodes", [])) or None,
    )
    check("the slow attempt is running", live is not None)
    check("its harness process is there before cancel", any("fixture_harness" in c for _, c in processes_in(cancel_repo)), json.dumps(processes_in(cancel_repo)))
    text = answer(f"/meshloop resume {CANCEL_GRAPH}", c)
    check(
        "resume refuses while the run's driver is alive",
        text.startswith(f"meshloop resume: refused: a driver is running for {CANCEL_GRAPH}") and "\n" not in text,
        text,
    )
    text = answer(f"/meshloop cancel {CANCEL_GRAPH}", c)
    check(
        "cancel answers what is left",
        text.startswith(f"meshloop cancel: graph {CANCEL_GRAPH} cancelled; 1 Cancelled")
        and "attempt processes alive: none" in text
        and "every one ledger-owned" in text,
        text,
    )
    left = wait_until("processes in the cancelled repository to exit", lambda: (not processes_in(cancel_repo)) or None, 30)
    check("no process left in the cancelled repository", left is not None, json.dumps(processes_in(cancel_repo)))
    states, data = node_states(meshloop, cancel_repo, CANCEL_GRAPH)
    check(
        "meshloop status: cancelled, no live attempt, driver not alive",
        states == {1: "Cancelled"}
        and all(n.get("live") != "Live" for n in data.get("nodes", []))
        and data.get("driver") != "alive",
        json.dumps(data)[:600],
    )
    doctor = cli_json(meshloop, cancel_repo, "doctor")
    check("meshloop doctor is ready after cancel", doctor.get("ok") is True and doctor["data"]["harnesses_ready"], json.dumps(doctor)[:400])
    inventory = cli_json(meshloop, cancel_repo, "workspace", "inventory")
    entries = inventory.get("data", {}).get("entries", [])
    listed = git(cancel_repo, "worktree", "list", "--porcelain")
    paths = [line.split(" ", 1)[1] for line in listed.splitlines() if line.startswith("worktree ")][1:]
    owned = {str((cancel_repo / ".meshloop" / "worktrees" / e["location"]["relative_path"]).resolve()) for e in entries if e.get("ownership") == "ledger-owned"}
    check(
        "every worktree left is one meshloop owns (no orphan)",
        all(e.get("ownership") == "ledger-owned" for e in entries) and all(str(Path(p).resolve()) in owned for p in paths),
        f"{paths} {json.dumps(entries)[:600]}",
    )
    note(f"cancelled run keeps {len(paths)} worktree(s), all ledger-owned: {paths}")


BOARD_GRAPH = "g137"
OPERATOR = "s-MESHLOOPBOARD"
SUPERVISOR = "s-MESHLOOPSUPER"


def board_name(repo):
    """The board the cell names for REPO: meshloop- and 12 hex digits of the
    SHA-256 of its real path, computed here independently of the cell."""
    return "meshloop-" + hashlib.sha256(os.path.realpath(repo).encode()).hexdigest()[:12]


def board_lines(home, name):
    """Every line of the board's file, the Board's own rendering of its log."""
    files = sorted((home / ".nodecode" / "boards").glob(f"*/{name}.txt"))
    return [line for f in files for line in f.read_text().splitlines() if line.strip()]


def projected(lines):
    """The cell's posts among LINES: those by the operator's session."""
    return [line for line in lines if f"] {OPERATOR}: meshloop {BOARD_GRAPH} " in line]


def event_seqs(lines):
    seqs = []
    for line in projected(lines):
        marker = f"meshloop {BOARD_GRAPH} event "
        if marker in line:
            seqs.append(int(line.split(marker, 1)[1].split(":", 1)[0]))
    return seqs


def log_seqs(meshloop, repo):
    out = cli_json(meshloop, repo, "events", "--graph", BOARD_GRAPH)
    return [e["seq"] for e in out.get("data", {}).get("events", [])], out


def turns(organism, session):
    timeline = organism.call("GET", f"/api/gateway/sessions/{session}/timeline")
    return timeline.get("turns", []) if isinstance(timeline, dict) else []


def board_section(answer, organism, home, work, meshloop, fixture_harness):
    """#137: run events projected onto a nodecode Board, one-way."""
    repo = work / "board"
    # Each attempt takes 4 s, so the run can be interrupted mid-attempt.
    make_fixture(repo, fixture_harness, invoke=["--prompt-file", "{prompt_file}", "4000"], graph=BOARD_GRAPH)
    name = board_name(repo)
    base = {"binary": str(meshloop), "timeout_seconds": 30, "write_commands": True}
    write_config(home, base)
    organism.restart_cell()
    on = organism.call("POST", "/api/gateway/cells", {"name": "nodecode-board", "on": True})
    board_row = next((c for c in on.get("cells", []) if c.get("name") == "nodecode-board"), {}) if isinstance(on, dict) else {}
    check("the nodecode-board cell is on", board_row.get("started") is True, json.dumps(board_row))
    for session in (OPERATOR, SUPERVISOR):
        created = organism.call("POST", "/api/gateway/sessions", {"session_id": session, "cwd": str(repo)})
        check(f"session {session} opened", "http_error" not in created, json.dumps(created))
    text = answer(f"/board {name}", SUPERVISOR)
    check("the supervising session joins the repository's board by its derived name", text.startswith(f"board {name}: "), text)
    cursor_dir = home / ".nodecode" / "meshloop"

    # (a) Disabled, the default: nothing posts, nothing more runs.
    text = answer("/meshloop board-sync", OPERATOR)
    check("board off: board-sync refuses in one line", text.startswith("meshloop board: off") and "\n" not in text, text)
    answer("/meshloop review-plan --plan plan.json --accept --as person-one", OPERATOR)
    answer("/meshloop run --plan plan.json", OPERATOR)

    def gate(task):
        states, data = node_states(meshloop, repo, BOARD_GRAPH)
        return states.get(task) == "AwaitingReview" and data.get("driver") != "alive"

    check("node 1 reached the gate", wait_until("node 1 to await review", lambda: gate(1)) is not None)
    text = answer("/meshloop status", OPERATOR)
    check("board off: status answers as before, no board line", "meshloop board" not in text, text)
    check("board off: no post and no cursor", not projected(board_lines(home, name)) and not cursor_dir.exists())

    # (b) On, wake_on_gates off: exactly one post per transition, no mention.
    write_config(home, {**base, "board": True})
    organism.restart_cell()
    join_line = f"meshloop board: run /meshloop board-sync to put this session on {name} (it leaves its current board)"
    # Only an explicit board-sync joins a session: a sync after a command, from a
    # session board-sync never put on the board, posts nothing and says how to join.
    for session in (OPERATOR, SUPERVISOR):
        text = answer("/meshloop status", session)
        check(f"board on, {session} not joined by board-sync: status posts nothing and says how", text.endswith("\n" + join_line), text)
    check("nothing posted before board-sync", not projected(board_lines(home, name)))
    text = answer("/meshloop board-sync", OPERATOR)
    seqs, _ = log_seqs(meshloop, repo)
    lines = board_lines(home, name)
    check(
        "board-sync joins this session, says so, and names the run",
        text.startswith(f"meshloop board: {name}; this session is now on {name}; it left any board it was on\n  {BOARD_GRAPH}: "),
        text,
    )
    check("one post per event of the run, in order", event_seqs(lines) == seqs and len(seqs) >= 4, f"{event_seqs(lines)} vs {seqs}")
    check("one post for the plan state", sum(" plan 1: PlanAccepted" in l for l in projected(lines)) == 1, "\n".join(lines))
    check("wake_on_gates off: no post mentions anyone", not any("@" in l for l in projected(lines)), "\n".join(lines))
    check("every post fits post_chars", all(len(l.split(": ", 1)[1]) <= 1000 for l in projected(lines)))
    check(
        "posts carry the run id, node, state and evidence path, nothing of the run's text",
        all(f"meshloop {BOARD_GRAPH} " in l for l in projected(lines))
        and any("node #1 AwaitingReview (from Verifying, DeterministicChecksPassed); evidence .meshloop/worktrees/" in l for l in lines)
        and not any("first" in l.split(": ", 1)[1] or "FIXTURE" in l for l in projected(lines)),
        "\n".join(lines),
    )
    text = answer("/meshloop board-sync", OPERATOR)
    check("a second sync posts nothing", f"{BOARD_GRAPH}: 0 posts" in text and board_lines(home, name) == lines, text)
    # The crash window: a cursor that lost its last post. The Board refuses the
    # same words within 60 s, which the cell reads as already posted.
    cursor = next(cursor_dir.glob(f"board/*/{name}.cursor"))
    kept = cursor.read_text()
    cursor.write_text(kept.replace(f"graph {BOARD_GRAPH} {seqs[-1]} ", f"graph {BOARD_GRAPH} {seqs[-2]} "))
    text = answer("/meshloop board-sync", OPERATOR)
    check("a lost cursor write posts no duplicate", board_lines(home, name) == lines and f"cursor at event {seqs[-1]}" in text, text)
    check("the cursor file records the session board-sync joined", f"\nsession {OPERATOR}\n" in cursor.read_text(), cursor.read_text())
    # A joined session the operator moved to another board: a sync after a command
    # posts nothing there; board-sync brings it back.
    answer("/board elsewhere", OPERATOR)
    text = answer("/meshloop status", OPERATOR)
    check("a session moved off the board: status posts nothing and says how to rejoin", text.endswith("\n" + join_line), text)
    check("nothing posted on the other board", not board_lines(home, "elsewhere"), "\n".join(board_lines(home, "elsewhere")))
    text = answer("/meshloop board-sync", OPERATOR)
    check("board-sync puts it back", f"this session is now on {name}" in text and f"{BOARD_GRAPH}: 0 posts" in text, text)
    check("nothing woke the supervising session", turns(organism, SUPERVISOR) == [], json.dumps(turns(organism, SUPERVISOR))[:400])

    # (c) One-way: a post saying accept is not acceptance.
    before, _ = log_seqs(meshloop, repo)
    snap = snapshot(repo)
    for post in (f"/meshloop accept --task 1 --as x --graph {BOARD_GRAPH}", "accept", f"meshloop accept --task 1 --as x"):
        text = answer(f"/board post {post}", SUPERVISOR)
        check(f"the board takes a post saying {post!r}", text.startswith(f"posted to {name}"), text)
    answer("/meshloop board-sync", OPERATOR)
    answer("/meshloop status", OPERATOR)
    after, _ = log_seqs(meshloop, repo)
    states, _ = node_states(meshloop, repo, BOARD_GRAPH)
    attempts = cli_json(meshloop, repo, "inspect", "--attempts", "--graph", BOARD_GRAPH)
    accepted = [e for n in attempts.get("data", {}).get("nodes", []) for a in n.get("attempts", [])
                for e in a.get("evidence", []) if e.get("kind") == "human_acceptance"]
    check(
        "a board post saying accept triggers no accept: event log, store and node unchanged",
        before == after and snapshot(repo) == snap and states.get(1) == "AwaitingReview" and not accepted,
        f"{before} {after} {states} {accepted}",
    )
    check("no meshloop accept process", not any(" accept" in c for _, c in processes_in(repo)), json.dumps(processes_in(repo)))

    # (d) wake_on_gates on, then an interrupted run and resume.
    write_config(home, {**base, "board": True, "wake_on_gates": True, "supervisor_session": SUPERVISOR})
    organism.restart_cell()
    text = answer(f"/meshloop accept --task 1 --as person-one --graph {BOARD_GRAPH}", OPERATOR)
    check("accept syncs the board after it", f"\nmeshloop board: {name}\n  {BOARD_GRAPH}: 1 post" in text, text)
    answer(f"/meshloop resume {BOARD_GRAPH}", OPERATOR)
    running = wait_until(
        "node 2's attempt to run",
        lambda: any(n.get("id") == 2 and n.get("live") == "Live" for n in node_states(meshloop, repo, BOARD_GRAPH)[1].get("nodes", [])) or None,
    )
    check("node 2's attempt is running", running is not None)
    killed = [pid for pid, _ in processes_in(repo)]
    for pid in killed:
        try:
            os.kill(pid, 9)
        except OSError:
            pass
    check("the run was interrupted: its driver and worker killed", killed and wait_until("the killed processes to go", lambda: (not processes_in(repo)) or None, 30) is not None, json.dumps(killed))
    text = answer("/meshloop board-sync", OPERATOR)
    check("a sync of the interrupted run", f"meshloop board: {name}" in text, text)
    answer(f"/meshloop resume {BOARD_GRAPH}", OPERATOR)

    def settled():
        states, data = node_states(meshloop, repo, BOARD_GRAPH)
        if data.get("driver") == "alive":
            return None
        if states.get(2) in ("Failed", "Blocked"):
            answer(f"/meshloop resume {BOARD_GRAPH} --retry", OPERATOR)
            return None
        return states.get(2) == "AwaitingReview" or None

    check("node 2 reached the gate after resume", wait_until("node 2 to await review", settled, 120) is not None,
          json.dumps(node_states(meshloop, repo, BOARD_GRAPH)[0]))
    text = answer("/meshloop board-sync", OPERATOR)
    seqs, _ = log_seqs(meshloop, repo)
    lines = board_lines(home, name)
    posted = event_seqs(lines)
    check("after the interrupted run and resume: every event posted exactly once", posted == seqs, f"{posted} vs {seqs}")
    gate_lines = [l for l in projected(lines) if "@" in l]
    gates = ("AwaitingPlanReview", "AwaitingReview", "Blocked", "Failed")
    # The killed attempt makes node 2 Failed (a gate, then current), and the
    # retry brings it to AwaitingReview: each mentions the supervisor once.
    check(
        "wake_on_gates on: only gate posts mention, only the supervising session, last word",
        gate_lines
        and all(l.endswith(f" @{SUPERVISOR}") and l.count("@") == 1 for l in gate_lines)
        and all(any(f"node #2 {g} (from" in l for g in gates) for l in gate_lines)
        and "node #2 AwaitingReview (from Verifying, DeterministicChecksPassed)" in gate_lines[-1]
        and not any("@" in l for l in projected(lines) if not any(f" {g} (from" in l for g in gates)),
        "\n".join(projected(lines)),
    )
    check("the Board reports the wake", f"woke {SUPERVISOR}" in text, text)
    woken = wait_until("the supervising session's turns", lambda: (len(turns(organism, SUPERVISOR)) >= len(gate_lines)) or None, 30)
    woken = turns(organism, SUPERVISOR)
    check(
        "the idle supervising session was woken once per gate post, by the Board",
        len(woken) == len(gate_lines)
        and all(t.get("prompt", "").startswith(f"[board {name}: {OPERATOR} mentioned you") for t in woken),
        json.dumps(woken)[:600],
    )
    # This throwaway home has no model: the woken turn ends at once, spending nothing.
    for turn in woken:
        note(f"woken turn {turn.get('turn_id')}: status {turn.get('status')}, {turn.get('ms')} ms, detail {turn.get('detail')}")
    check(
        "the woken turns made no model call",
        woken and all(t.get("calls") == [] and t.get("status") != "running" for t in woken),
        json.dumps(woken)[:600],
    )

    # A crash after a gate post that woke the supervisor, before its cursor write:
    # the post in flight is posted again at the next sync, without the mention,
    # so the supervisor is not woken twice (at least once, never a second wake).
    cursor = next(cursor_dir.glob(f"board/*/{name}.cursor"))
    kept = cursor.read_text()
    entry = next(l for l in kept.splitlines() if l.startswith(f"graph {BOARD_GRAPH} "))
    words = entry.split()
    words[2], words[6] = str(seqs[-2]), f"e{seqs[-1]}"
    cursor.write_text(kept.replace(entry, " ".join(words)))
    woken_before = len(turns(organism, SUPERVISOR))
    text = answer("/meshloop board-sync", OPERATOR)
    again = board_lines(home, name)
    check(
        "a post left in flight is posted again without its mention, waking no one",
        len(again) == len(lines) + 1 and again[-1].endswith(gate_lines[-1].split(": ", 1)[1].rsplit(" @", 1)[0])
        and "@" not in again[-1] and f"cursor at event {seqs[-1]}" in text and "woke" not in text,
        "\n".join(again[-2:]) + "\n" + text,
    )
    time.sleep(2)
    check("no second wake", len(turns(organism, SUPERVISOR)) == woken_before, json.dumps(turns(organism, SUPERVISOR))[-400:])
    lines = again

    # (e) The cursor is the cell's file, not its memory: a restarted cell posts nothing again.
    organism.restart_cell()
    text = answer("/meshloop board-sync", OPERATOR)
    check("after the cell restarts a sync posts nothing", f"{BOARD_GRAPH}: 0 posts" in text and board_lines(home, name) == lines, text)

    # (f) A cursor kept on another ledger (as after .meshloop/ was replaced) starts
    # the graph over and says so, instead of skipping the new ledger's events.
    kept = cursor.read_text()
    entry = next(l for l in kept.splitlines() if l.startswith(f"graph {BOARD_GRAPH} "))
    words = entry.split()
    words[5] = "ffffffffffffffff"
    cursor.write_text(kept.replace(entry, " ".join(words)))
    text = answer("/meshloop board-sync", OPERATOR)
    check("another ledger: the cursor resets and the answer says so", "cursor reset: it was kept on another ledger" in text, text)
    real = next(l for l in cursor.read_text().splitlines() if l.startswith(f"graph {BOARD_GRAPH} ")).split()
    check("the reset cursor is kept on this ledger and at its last event", real[5] != "ffffffffffffffff" and real[2] == str(seqs[-1]), " ".join(real))
    write_config(home, base)
    organism.restart_cell()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--meshloop", required=True, help="absolute path of a meshloop built from the checkout")
    parser.add_argument("--fixture-harness", required=True, help="absolute path of target/debug/fixture_harness")
    parser.add_argument("--nodecode", default="nodecode")
    parser.add_argument("--work", required=True, help="scratch directory; removed and recreated")
    args = parser.parse_args()

    meshloop = Path(args.meshloop).resolve()
    work = prepare_work(Path(args.work))
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

        def answer(line, session=SESSION):
            text = organism.slash(line, session)
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
        text = answer("/meshloop frobnicate 1")
        check("no other verb", text.startswith("meshloop: ") and "no verb" in text, text)
        text = answer("/meshloop")
        check("bare /meshloop answers its usage", text.startswith("meshloop: /meshloop doctor"), text)

        # Read-only additions (#136): the run report and the plan view.
        text = answer("/meshloop report")
        check(
            "report names what was dispatched and what was not",
            text.startswith(f"meshloop report: graph {GRAPH} ")
            and "dispatched: fixture (model_ref fixture-model, 1 attempt)" in text
            and "not exercised: none of the 1 selected" in text
            and "otherwise it is the entry's label" in text,
            text,
        )
        text = answer("/meshloop review-plan --plan plan.json")
        check(
            "review-plan without a decision only shows the plan",
            text.startswith(f"meshloop review-plan: plan plan.json, graph {GRAPH}, 2 nodes, plan state PlanAccepted")
            and "#2, after #1: second" in text
            and "--accept --as <your-name>" in text,
            text,
        )

        # (a) Every write command refuses while meshloop.write_commands is off.
        writes = {
            "plan": "/meshloop plan add a widget",
            "review-plan": "/meshloop review-plan --plan plan.json --accept --as person-one",
            "run": "/meshloop run --plan plan.json",
            "resume": f"/meshloop resume {GRAPH}",
            "cancel": f"/meshloop cancel {GRAPH}",
            "accept": f"/meshloop accept --task 1 --as person-one --graph {GRAPH}",
            "integrate": f"/meshloop integrate --graph {GRAPH} --into master --accept-integrate",
        }
        for verb, line in writes.items():
            text = answer(line)
            check(
                f"write commands off: {verb} refuses in one line",
                text.startswith(f"meshloop {verb}: {WRITE_REFUSED}") and "\n" not in text,
                text,
            )

        # (c) The :tool veto. A real tool call needs a model turn, so hook-check runs
        # the installed hook function on a synthetic eval call; the form is never run.
        vetoed = {
            "shell string": '(sh "meshloop accept --task 1 --as me")',
            "path and flags": '(sh "cd /r && /opt/bin/meshloop --json integrate --graph g --into main")',
            "argv list": '(sb-ext:run-program "/x/target/debug/meshloop" (list "review-plan" "--accept"))',
            "quoted argv": "(uiop:run-program '(\"meshloop\" \"meshloop:accept\" \"--task\" \"1\"))",
            "MCP tool name": '(mcp-call "meshloop_integrate" (list))',
            "the slash line": '(post "/api/gateway/slash" "{\\"line\\":\\"/meshloop accept --task 1 --as me\\"}")',
            "the cell's own code": '(meshloop::accept-of "s" "--task 1 --as me")',
        }
        for name, form in vetoed.items():
            text = answer(f"/meshloop hook-check {form}")
            check(
                f"tool veto refuses: {name}",
                text.startswith("meshloop hook-check: refused: ERROR: refused by nodecode-meshloop")
                and "hook on this organism's :tool chain" in text,
                text,
            )
        passed = {
            "a read verb": '(sh "meshloop status --json")',
            "the eval verbs": "(list (meshloop:status) (meshloop:report))",
            "the word in a message": "(sh \"git commit -m 'accept and integrate the meshloop docs'\")",
        }
        for name, form in passed.items():
            text = answer(f"/meshloop hook-check {form}")
            check(
                f"tool veto lets through: {name}",
                text.startswith("meshloop hook-check: passes") and "hook on this organism's :tool chain" in text,
                text,
            )
        priors_section(answer, home, repo, meshloop)
        check("no write to the repository or meshloop state", snapshot(repo) == before)

        drive_section(answer, organism, home, work, meshloop, Path(args.fixture_harness).resolve())
        board_section(answer, organism, home, work, meshloop, Path(args.fixture_harness).resolve())
        write_config(home, {"binary": str(meshloop), "timeout_seconds": 30})
        organism.restart_cell()
        priors_proposal_section(answer, organism, work, meshloop, Path(args.fixture_harness).resolve())

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

        # A binary whose child outlives it: the timeout must kill the whole process
        # group, or the grandchild holds the pipe and the read blocks.
        grandchild = "613.135"
        write_config(
            home,
            {
                "binary": str(fake_binary(work / "hang-meshloop", f"sleep {grandchild} &\nsleep 614.135\n")),
                "timeout_seconds": 2,
            },
        )
        organism.restart_cell()
        started = time.time()
        text = answer("/meshloop status")
        elapsed = time.time() - started
        check(
            "timeout stops the process group",
            text.startswith("meshloop status: gave no answer in 2 s") and elapsed < 6,
            f"{elapsed:.1f} s: {text}",
        )
        left = run(["pgrep", "-f", f"sleep {grandchild}"], work)
        check("no grandchild left after the timeout", left.returncode == 1, left.stdout)

        # More stderr than the cell keeps: it must drain the rest, not stall the binary.
        write_config(
            home,
            {
                "binary": str(
                    fake_binary(
                        work / "noisy-meshloop",
                        f"head -c 300000 /dev/zero | tr '\\000' e >&2\nexec {json.dumps(str(meshloop))} \"$@\"\n",
                    )
                ),
                "timeout_seconds": 30,
            },
        )
        organism.restart_cell()
        text = answer("/meshloop status")
        check("300 KB of stderr is drained", text.startswith(f"meshloop status: graph {GRAPH}, "), text)

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
    if failed:
        note(f"kept {work} for inspection")
        return 1
    shutil.rmtree(work)
    return 0


if __name__ == "__main__":
    sys.exit(main())
