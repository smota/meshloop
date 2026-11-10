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

Result channel (lp-4). The editable code runs inside the test process, so a bare exit
code is not trusted: `import os; os._exit(0)` in a module would otherwise pass. The
parent sends a random nonce on the runner's stdin. The runner reads it into a function
local and closes fd 0 before any tested code is imported. Only after a successful run
with tests does it print `RESULT ok tests=<n> nonce=<nonce>`. The parent requires exit 0
and that exact line, with `<n>` equal to `expected_tests` in `task.json`. An early exit,
a forged line without the nonce, or a different test count fails.

Rust tasks run `cargo test --offline` on the same constructed tree (pristine
`Cargo.toml`, `Cargo.lock` and tests, the candidate's `src/lib.rs`), in an environment
without `CARGO_*`/`RUST*` variables apart from an explicit `RUSTUP_HOME`, and with a
fresh `CARGO_HOME`, so user cargo configuration does not apply. The parent requires
exit 0, no failing `test result:` line, and passed counts that add up to
`expected_tests`.

Threat model: this is a benchmark of cooperative workers, not a security boundary. The
editable code runs in the test process. Code that deliberately patches the test
framework from inside (for example replacing `unittest.TestResult.wasSuccessful`),
reads the nonce out of the runner's memory, or prints a matching Rust summary and then
exits is not prevented. These checks stop accidental and simple passes.

`--with-reference` overlays the reference solution, which may contain only editable
files; `xtask bench-learning --validate` uses it. The checked tree is never modified.
Standard library only. Exit 2 means the driver itself failed.
"""

import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile

TIMEOUT_S = 180
SHADOW_NAMES = {"conftest.py", "sitecustomize.py", "usercustomize.py"}

# Runs inside `python3 -I -c`. The nonce is read from stdin into a local and fd 0 is
# closed before any tested code is imported. unittest comes from the standard library
# before the tested tree is added to sys.path, so a module in the tree cannot replace it.
PY_RUNNER = """
import os
import sys


def _main():
    nonce = sys.stdin.readline().strip()
    try:
        sys.stdin.close()
    finally:
        try:
            os.close(0)
        except OSError:
            pass
    import unittest

    sys.path.insert(0, sys.argv[1])
    prog = unittest.main(module=None, argv=["verify", "discover", "-s", "tests"], exit=False)
    result = prog.result
    if result.wasSuccessful() and result.testsRun > 0:
        sys.stderr.flush()
        sys.stdout.write("RESULT ok tests=%d nonce=%s\\n" % (result.testsRun, nonce))
        sys.stdout.flush()
        return 0
    return 1


sys.exit(_main())
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


def rust_env():
    """Environment for cargo without user cargo configuration or flags."""
    env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith(("CARGO", "RUST")) and k not in ("RUSTFLAGS", "RUSTC_WRAPPER")
    }
    env["RUSTUP_HOME"] = os.environ.get("RUSTUP_HOME") or os.path.join(
        os.path.expanduser("~"), ".rustup"
    )
    if os.environ.get("RUSTUP_TOOLCHAIN"):
        env["RUSTUP_TOOLCHAIN"] = os.environ["RUSTUP_TOOLCHAIN"]
    env["CARGO_TERM_COLOR"] = "never"
    return env


def python_passed(output, nonce, expected):
    """True only for the runner's exact result line with this nonce and test count."""
    want = f"RESULT ok tests={expected} nonce={nonce}"
    return any(line.strip() == want for line in output.splitlines())


def rust_passed(output, expected):
    """All `test result:` lines ok and their passed counts add up to `expected`."""
    total = 0
    seen = False
    for line in output.splitlines():
        line = line.strip()
        if not line.startswith("test result:"):
            continue
        seen = True
        m = re.match(r"test result: ok\. (\d+) passed; 0 failed;", line)
        if not m:
            return False
        total += int(m.group(1))
    return seen and total == expected


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
    expected = task.get("expected_tests")
    if not isinstance(expected, int) or expected <= 0:
        print("verify: task.json needs a positive expected_tests", file=sys.stderr)
        return 2
    nonce = secrets.token_hex(16)
    with tempfile.TemporaryDirectory(prefix="ml-lp-verify-") as tmp:
        try:
            build_tree(task_dir, task, os.getcwd(), tmp, with_reference)
        except (OSError, ValueError, KeyError) as e:
            print(f"verify: cannot build the tested tree: {e}", file=sys.stderr)
            return 2
        stdin = None
        if task["language"] == "python":
            command = [sys.executable, "-I", "-c", PY_RUNNER, tmp]
            env = dict(os.environ)
            env["PYTHONDONTWRITEBYTECODE"] = "1"
            stdin = (nonce + "\n").encode()
        elif task["language"] == "rust":
            command = ["cargo", "test", "--offline", "--quiet"]
            env = rust_env()
            env["CARGO_HOME"] = os.path.join(tmp, ".cargo-home")
        else:
            print(f"verify: unsupported language {task['language']}", file=sys.stderr)
            return 2
        try:
            proc = subprocess.run(
                command,
                cwd=tmp,
                env=env,
                input=stdin if stdin is not None else b"",
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
        output = proc.stdout.decode("utf-8", "replace")
        if task["language"] == "python":
            passed = proc.returncode == 0 and python_passed(output, nonce, expected)
        else:
            passed = proc.returncode == 0 and rust_passed(output, expected)
        tail = output.replace(nonce, "<nonce>").splitlines()[-25:]
        print("\n".join(tail))
        print(
            f"verify: {task['id']}: exit {proc.returncode}, "
            f"{'passed' if passed else 'not passed'} (expected_tests={expected})"
        )
        return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
