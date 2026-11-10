#!/usr/bin/env python3
"""Hidden-test driver for the learning-plane benchmark (issue #144).

Usage: verify.py <task_dir> [--with-reference]

Copies the current directory (a Meshloop attempt worktree, or the disposable
repository after integration) to a temporary directory, replaces its `tests/`
with the task's pristine `hidden/tests/`, and runs the task's `test_command`
there. The exit code is the test command's exit code, so success is measured by
tests, not by what a worker says. A worker cannot pass by editing tests, and the
checked tree is never modified.

`--with-reference` also overlays `reference/` (the reference solution). The
runner's offline `--validate` mode uses it to prove each task is solvable and
that its seed fails.

Standard library only. Exit 2 means the driver itself failed.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

SKIP = {".git", ".meshloop", "target", "__pycache__", ".pytest_cache"}
TIMEOUT_S = 180


def copy_tree(src, dst):
    for name in os.listdir(src):
        if name in SKIP:
            continue
        s = os.path.join(src, name)
        d = os.path.join(dst, name)
        if os.path.islink(s):
            continue
        if os.path.isdir(s):
            os.makedirs(d, exist_ok=True)
            copy_tree(s, d)
        else:
            shutil.copy2(s, d)


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
    command = list(task["test_command"])
    if command and command[0] == "python3":
        command[0] = sys.executable
    with tempfile.TemporaryDirectory(prefix="ml-lp-verify-") as tmp:
        copy_tree(os.getcwd(), tmp)
        shutil.rmtree(os.path.join(tmp, "tests"), ignore_errors=True)
        copy_tree(os.path.join(task_dir, "hidden"), tmp)
        if with_reference:
            copy_tree(os.path.join(task_dir, "reference"), tmp)
        env = dict(os.environ)
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        env["CARGO_TERM_COLOR"] = "never"
        env.pop("CARGO_TARGET_DIR", None)
        try:
            proc = subprocess.run(
                command,
                cwd=tmp,
                env=env,
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
