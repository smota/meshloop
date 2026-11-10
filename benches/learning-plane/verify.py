#!/usr/bin/env python3
"""Hidden-test driver for the learning-plane benchmark (issue #144).

Usage: verify.py <task_dir> [--with-reference]

The current directory is the candidate tree: a Meshloop attempt worktree, or the
disposable repository after integration. The exit code is the hidden tests' result,
so success is measured by tests, not by what a worker says.

Tamper resistance (lp-3). The tree that is tested is built from:

1. the task's pristine `seed/`;
2. the candidate's version of each file listed in `task.json` `editable` (the files
   SPEC.md allows to change), taken only if it is a regular file;
3. the pristine `hidden/tests/`, replacing `tests/`.

Nothing else from the candidate is copied: an extra `unittest.py`, `conftest.py`,
`sitecustomize.py`, `usercustomize.py`, `*.pth` or other module cannot reach the
tests. As a second guard, any top-level file with one of those names or a `.py`
extension that is not part of the seed is removed before the tests run.

Python tasks run as `python3 -I` (isolated: no user site, no PYTHON* environment
variables, current directory not on `sys.path`). `unittest` is imported from the
standard library first; only then is the tested tree put on `sys.path` explicitly.
Rust tasks run `cargo test --offline` on the same constructed tree (pristine
`Cargo.toml`, `Cargo.lock` and tests, the candidate's `src/lib.rs`).

`--with-reference` overlays the reference solution, which may contain only editable
files; `xtask bench-learning --validate` uses it. The checked tree is never modified.
Standard library only. Exit 2 means the driver itself failed.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

TIMEOUT_S = 180
SHADOW_NAMES = {"conftest.py", "sitecustomize.py", "usercustomize.py"}

# Runs inside `python3 -I -c`. unittest comes from the standard library before the
# tested tree is added to sys.path, so a module in the tree cannot replace it.
PY_RUNNER = """
import sys
import unittest
root = sys.argv[1]
sys.path.insert(0, root)
prog = unittest.main(module=None, argv=["verify", "discover", "-s", "tests"], exit=False)
ok = prog.result.wasSuccessful() and prog.result.testsRun > 0
sys.exit(0 if ok else 1)
"""


def copy_tree(src, dst):
    for name in os.listdir(src):
        s = os.path.join(src, name)
        d = os.path.join(dst, name)
        if os.path.islink(s):
            continue
        if os.path.isdir(s):
            os.makedirs(d, exist_ok=True)
            copy_tree(s, d)
        elif os.path.isfile(s):
            shutil.copy2(s, d)


def seed_files(seed):
    out = set()
    for base, _dirs, files in os.walk(seed):
        for f in files:
            out.add(os.path.relpath(os.path.join(base, f), seed).replace(os.sep, "/"))
    return out


def safe_rel(rel):
    parts = rel.split("/")
    return rel and not os.path.isabs(rel) and ".." not in parts


def build_tree(task_dir, task, candidate, tmp, with_reference):
    seed = os.path.join(task_dir, "seed")
    copy_tree(seed, tmp)
    editable = task["editable"]
    for rel in editable:
        if not safe_rel(rel):
            raise ValueError(f"bad editable path {rel!r}")
        src = os.path.join(candidate, rel)
        dst = os.path.join(tmp, rel)
        if os.path.isfile(src) and not os.path.islink(src):
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copy2(src, dst)
        elif os.path.exists(dst):
            os.remove(dst)  # the worker deleted an editable file
    if with_reference:
        ref = os.path.join(task_dir, "reference")
        for rel in seed_files(ref):
            if rel not in editable:
                raise ValueError(f"reference file {rel} is not editable")
        copy_tree(ref, tmp)
    shutil.rmtree(os.path.join(tmp, "tests"), ignore_errors=True)
    copy_tree(os.path.join(task_dir, "hidden"), tmp)
    allowed = seed_files(seed)
    for name in os.listdir(tmp):
        path = os.path.join(tmp, name)
        if not os.path.isfile(path) or name in allowed:
            continue
        if name in SHADOW_NAMES or name.endswith((".py", ".pth")):
            os.remove(path)


def main(argv):
    if len(argv) < 2:
        print("usage: verify.py <task_dir> [--with-reference]", file=sys.stderr)
        return 2
    task_dir = os.path.abspath(argv[1])
    with_reference = "--with-reference" in argv[2:]
    try:
        with open(os.path.join(task_dir, "task.json"), encoding="utf-8") as f:
            task = json.load(f)
    except (OSError, ValueError) as e:
        print(f"verify: cannot read task.json: {e}", file=sys.stderr)
        return 2
    env = dict(os.environ)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env["CARGO_TERM_COLOR"] = "never"
    env.pop("CARGO_TARGET_DIR", None)
    with tempfile.TemporaryDirectory(prefix="ml-lp-verify-") as tmp:
        try:
            build_tree(task_dir, task, os.getcwd(), tmp, with_reference)
        except (OSError, ValueError, KeyError) as e:
            print(f"verify: cannot build the tested tree: {e}", file=sys.stderr)
            return 2
        if task["language"] == "python":
            command = [sys.executable, "-I", "-c", PY_RUNNER, tmp]
        elif task["language"] == "rust":
            command = ["cargo", "test", "--offline", "--quiet"]
        else:
            print(f"verify: unsupported language {task['language']}", file=sys.stderr)
            return 2
        try:
            proc = subprocess.run(
                command,
                cwd=tmp,
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=TIMEOUT_S,
                check=False,
            )
        except subprocess.TimeoutExpired:
            print(f"verify: {task['id']}: timed out after {TIMEOUT_S}s")
            return 1
        except OSError as e:
            print(f"verify: cannot run {command[0]}: {e}", file=sys.stderr)
            return 2
        tail = proc.stdout.decode("utf-8", "replace").splitlines()[-25:]
        print("\n".join(tail))
        print(f"verify: {task['id']}: exit {proc.returncode}")
        return 0 if proc.returncode == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
