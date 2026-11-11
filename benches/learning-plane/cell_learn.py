#!/usr/bin/env python3
"""Fold warm-up graphs with the real nodecode-meshloop cell (#140).

Runs `/meshloop learn --graph G ... --out FILE` through the nodecode gateway's
POST /api/gateway/slash, which runs a cell command with no model turn, in a session
opened at REPO (the warm-up's shared repository and store). Then `/meshloop priors
check FILE`, read-only. It never promotes: a person does that in a shell.

It reuses the load test's organism (cells/nodecode-meshloop/test/load_test.py): a
throwaway HOME under --work, an emptied environment, nodecode's own updates off, and
a private copy of the pinned release, checked against the load test's sha256 list
before it runs. The operator's nodecode install and home are only read (the copy's
source). --work is removed afterwards unless --keep. Run it with `python3 -I`.
"""

import argparse
import hashlib
import json
import shutil
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CELL = ROOT / "cells" / "nodecode-meshloop"
sys.dont_write_bytecode = True  # leave nothing in the cell's folder
sys.path.insert(0, str(CELL / "test"))
import load_test  # noqa: E402

SESSION = "s-LPCELLFOLD"


def release_copy(source, dest, version):
    """A private copy of the pinned release, its executable checked against the
    load test's list."""
    shutil.copytree(source, dest, symlinks=True)
    sums = (CELL / "test" / f"nodecode-{version}.sha256").read_text().splitlines()
    want = next(line.split()[0] for line in sums if line.endswith("  nodecode"))
    got = hashlib.sha256((dest / "nodecode").read_bytes()).hexdigest()
    if got != want:
        raise SystemExit(f"release copy: nodecode sha256 {got} is not the pinned {want}")
    return dest / "nodecode"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--meshloop", required=True, help="absolute path of the meshloop binary")
    parser.add_argument("--repo", required=True, help="the warm-up's shared repository")
    parser.add_argument("--graphs-from", help="a bench-learning results.json whose runs to fold")
    parser.add_argument("--graph", action="append", default=[], help="a graph to fold (repeatable)")
    parser.add_argument("--out", required=True, help="proposal path inside REPO/.agent-runs/")
    parser.add_argument("--work", required=True, help="scratch directory; removed and recreated")
    parser.add_argument("--version", default="98dd6dc80")
    parser.add_argument("--release", default=str(Path.home() / ".local/share/nodecode/98dd6dc80"))
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    graphs = list(args.graph)
    if args.graphs_from:
        graphs += [run["graph_id"] for run in json.loads(Path(args.graphs_from).read_text())["runs"]]
    if not graphs:
        raise SystemExit("name graphs with --graph or --graphs-from")
    repo = Path(args.repo).resolve()
    work = load_test.prepare_work(Path(args.work))
    nodecode = release_copy(Path(args.release), work / "release", args.version)
    home = work / "home"
    (home / ".nodecode" / "cells").mkdir(parents=True)
    (home / ".nodecode" / "cells" / "nodecode-meshloop").symlink_to(CELL, target_is_directory=True)
    load_test.write_config(home, {"binary": str(Path(args.meshloop).resolve()), "timeout_seconds": 120})

    organism = load_test.Organism(str(nodecode), home, work)
    status = 1
    organism.start()
    try:
        deadline = time.time() + 120
        row = organism.cell()
        while time.time() < deadline and not (row and row.get("started")):
            time.sleep(1)
            row = organism.cell()
        if not (row and row.get("started")) or row.get("refusal"):
            raise SystemExit(f"cell did not start: {json.dumps(row)}")
        created = organism.call("POST", "/api/gateway/sessions", {"session_id": SESSION, "cwd": str(repo)})
        if "http_error" in created:
            raise SystemExit(f"session: {json.dumps(created)}")
        line = "/meshloop learn " + " ".join(f"--graph {g}" for g in graphs) + f" --out {args.out}"
        answer = organism.slash(line, SESSION)
        print(answer)
        if answer.startswith("meshloop learn: wrote "):
            print(organism.slash(f"/meshloop priors check {args.out}", SESSION))
            status = 0
    finally:
        organism.stop()
    left = load_test.processes_in(work)
    if left:
        print(f"processes left under {work}: {left}")
        status = 1
    if args.keep or status:
        print(f"kept {work}")
    else:
        shutil.rmtree(work)
    return status


if __name__ == "__main__":
    sys.exit(main())
