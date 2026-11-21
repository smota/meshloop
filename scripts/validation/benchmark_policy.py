"""Bound the explicit issue 212 offline-learning deferral to its tooling candidate."""

import argparse
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

APPROVED_BRANCH = "work/local-validation"
APPROVED_BASE = "3e8bf1e17eb01a0e99db386dc318cd1460a5f5d6"
APPROVED_PATHS = frozenset(
    {
        ".github/workflows/ci.yml",
        ".gitignore",
        "agent-workflow.config.json",
        "docs/engineering/testing.md",
        "lefthook.yml",
        "mise.toml",
        "scripts/validation/dagger_check.py",
        "scripts/validation/benchmark_policy.py",
        "scripts/validation/test_benchmark_policy.py",
    }
)
BENCHMARK = ("cargo", "run", "--locked", "-p", "xtask", "--", "bench-learning", "--offline")
CORE = ("mise", "run", "verify:core")
DEFERRED_MESSAGE = (
    "offline-learning benchmark deferred by explicit tooling-only authorization; "
    "failed fold comparison retained in #214"
)


@dataclass(frozen=True)
class Decision:
    deferred: bool
    reason: str


def assess(root: Path) -> Decision:
    """Require known ancestry and the union of changes in every Git layer."""
    def git(*args: str) -> bytes:
        return subprocess.check_output(
            ["git", *args], cwd=root, stderr=subprocess.PIPE
        )

    try:
        branch = git("symbolic-ref", "--quiet", "--short", "HEAD").decode().strip()
        if branch != APPROVED_BRANCH:
            return Decision(False, "branch is outside the explicit authorization")
        git("merge-base", "--is-ancestor", APPROVED_BASE, "HEAD")
        paths = set()
        for args in (
            ("diff", "--name-only", "--no-renames", "-z", APPROVED_BASE, "HEAD", "--"),
            ("diff", "--cached", "--name-only", "--no-renames", "-z", "--"),
            ("diff", "--name-only", "--no-renames", "-z", "--"),
            ("ls-files", "--others", "--exclude-standard", "-z"),
        ):
            paths.update(name.decode("utf-8") for name in git(*args).split(b"\0") if name)
        outside = paths - APPROVED_PATHS
        if outside:
            return Decision(False, f"paths outside authorization: {', '.join(sorted(outside))}")
        return Decision(True, DEFERRED_MESSAGE)
    except (OSError, subprocess.CalledProcessError, UnicodeError) as error:
        return Decision(False, f"Git admission unavailable ({type(error).__name__})")


def container_task(root: Path) -> str:
    """Select on the real host, before source staging discards its Git history."""
    decision = assess(root)
    print(decision.reason, flush=True)
    return "verify:core" if decision.deferred else "verify"


def _benchmark(root: Path, decision: Decision, command: tuple[str, ...]) -> int:
    print(decision.reason, flush=True)
    if decision.deferred:
        return 0
    print("Offline-learning benchmark required", flush=True)
    try:
        return subprocess.run(command, cwd=root, check=False).returncode
    except OSError as error:
        print(f"Cannot run required benchmark: {error}", file=sys.stderr)
        return 1


def run_benchmark(root: Path, command: tuple[str, ...] = BENCHMARK) -> int:
    return _benchmark(root, assess(root), command)


def run_verify(
    root: Path,
    core: tuple[str, ...] = CORE,
    benchmark: tuple[str, ...] = BENCHMARK,
) -> int:
    """Freeze admission before core checks generate tracked benchmark artifacts."""
    decision = assess(root)
    print(f"Pre-core benchmark admission: {decision.reason}", flush=True)
    try:
        result = subprocess.run(core, cwd=root, check=False).returncode
    except OSError as error:
        print(f"Cannot run required core checks: {error}", file=sys.stderr)
        return 1
    if result:
        return result
    return _benchmark(root, decision, benchmark)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true", help="run core checks before benchmark dispatch")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    raise SystemExit(run_verify(root) if args.verify else run_benchmark(root))
