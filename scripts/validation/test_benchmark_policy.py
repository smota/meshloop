"""Exercise admission with real disposable Git repositories and no Rust workload."""

import contextlib
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import benchmark_policy as policy
import dagger_check


class BenchmarkPolicyTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(
            os.environ,
            {name: value for name, value in os.environ.items() if not name.upper().startswith("GIT_")},
            clear=True,
        )
        environment.start()
        self.addCleanup(environment.stop)
        self.scratch = tempfile.TemporaryDirectory(prefix="meshloop-policy-test-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name) / "repo"
        self.root.mkdir()
        self.git("init", "--quiet", "--initial-branch=work/local-validation")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write("mise.toml", "base\n")
        self.write("crates/product.rs", "base\n")
        self.write(".gitignore", "ignored/\n")
        self.write("artifacts/bench/smoke-phases.json", "{}\n")
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        approved_base = patch.object(policy, "APPROVED_BASE", base)
        approved_base.start()
        self.addCleanup(approved_base.stop)
        self.record = Path(self.scratch.name) / "benchmark-ran"
        self.command = (
            sys.executable,
            "-c",
            "from pathlib import Path; import sys; Path(sys.argv[1]).write_text('ran'); "
            "raise SystemExit(int(sys.argv[2]))",
            str(self.record),
            "0",
        )

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "core.hooksPath=", "-c", "commit.gpgSign=false", *args],
            cwd=self.root,
            stderr=subprocess.PIPE,
            text=True,
        )

    def write(self, name, content="changed\n"):
        target = self.root / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")

    def commit(self):
        self.git("add", "-A")
        self.git("commit", "--quiet", "-m", "fixture")

    def require_benchmark(self, exit_code=0):
        self.record.unlink(missing_ok=True)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertFalse(policy.assess(self.root).deferred)
            self.assertEqual(policy.container_task(self.root), "verify")
            self.assertEqual(
                policy.run_benchmark(self.root, (*self.command[:-1], str(exit_code))),
                exit_code,
            )
        self.assertEqual(self.record.read_text(), "ran")

    def test_approved_layers_defer_and_select_core(self):
        for name in policy.APPROVED_PATHS:
            self.write(name)
        self.write(".gitignore", "ignored/\n")
        self.write("docs/engineering/testing.md")
        self.commit()
        self.write("mise.toml", "staged\n")
        self.git("add", "mise.toml")
        self.write("mise.toml", "unstaged\n")
        self.write("scripts/validation/benchmark_policy.py")
        self.write("ignored/product.rs")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(policy.container_task(self.root), "verify:core")
            self.assertEqual(policy.run_benchmark(self.root, self.command), 0)
        self.assertIn(policy.DEFERRED_MESSAGE, output.getvalue())
        self.assertFalse(self.record.exists())

    def test_other_branch_requires_benchmark(self):
        self.git("checkout", "--quiet", "-b", "work/other")
        self.require_benchmark()

    def test_snapshot_branch_cannot_authorize(self):
        self.git("checkout", "--quiet", "-b", "validation-snapshot")
        self.require_benchmark()

    def test_detached_head_requires_benchmark(self):
        self.git("checkout", "--quiet", "--detach")
        self.require_benchmark()

    def test_product_in_each_layer_requires_benchmark(self):
        for layer in ("committed", "staged", "unstaged", "untracked"):
            with self.subTest(layer=layer):
                self.git("reset", "--hard", policy.APPROVED_BASE)
                name = "new-product.rs" if layer == "untracked" else "crates/product.rs"
                self.write(name)
                if layer == "staged":
                    self.git("add", name)
                elif layer == "committed":
                    self.commit()
                self.require_benchmark()

    def test_worktree_reversal_does_not_hide_committed_product_change(self):
        self.write("crates/product.rs")
        self.commit()
        self.write("crates/product.rs", "base\n")
        self.require_benchmark()

    def test_unstaged_reversal_does_not_hide_staged_product_change(self):
        self.write("crates/product.rs")
        self.git("add", "crates/product.rs")
        self.write("crates/product.rs", "base\n")
        self.require_benchmark()

    def test_missing_base_requires_benchmark(self):
        with patch.object(policy, "APPROVED_BASE", "0" * 40):
            self.require_benchmark()

    def test_unrelated_base_requires_benchmark(self):
        self.git("checkout", "--quiet", "--orphan", "unrelated")
        self.write("mise.toml", "unrelated\n")
        self.commit()
        unrelated = self.git("rev-parse", "HEAD").strip()
        self.git("checkout", "--quiet", "work/local-validation")
        with patch.object(policy, "APPROVED_BASE", unrelated):
            self.require_benchmark()

    def test_rename_checks_both_ends(self):
        for source, destination in (
            ("crates/product.rs", "scripts/validation/benchmark_policy.py"),
            ("mise.toml", "unknown.toml"),
        ):
            for layer in ("staged", "committed"):
                with self.subTest(source=source, layer=layer):
                    self.git("reset", "--hard", policy.APPROVED_BASE)
                    (self.root / destination).parent.mkdir(parents=True, exist_ok=True)
                    self.git("mv", source, destination)
                    if layer == "committed":
                        self.commit()
                    self.require_benchmark()

    def test_missing_git_requires_benchmark(self):
        real_check_output = subprocess.check_output

        def missing_git(command, **kwargs):
            if command[0] == "git":
                raise FileNotFoundError("Git unavailable")
            return real_check_output(command, **kwargs)

        with patch.object(subprocess, "check_output", side_effect=missing_git):
            self.require_benchmark()

    def test_missing_repository_requires_benchmark(self):
        self.root = Path(self.scratch.name) / "not-a-repository"
        self.root.mkdir()
        self.require_benchmark()

    def test_required_benchmark_failure_propagates(self):
        self.write("product.rs")
        self.require_benchmark(exit_code=7)

    def test_required_benchmark_unavailable_fails(self):
        self.write("product.rs")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(policy.run_benchmark(self.root, ("missing-benchmark-command",)), 1)

    def core_command(self, exit_code=0):
        return (
            sys.executable,
            "-c",
            "from pathlib import Path; import sys; "
            "Path('artifacts/bench/smoke-phases.json').write_text('generated'); "
            "raise SystemExit(int(sys.argv[1]))",
            str(exit_code),
        )

    def test_core_generated_tracked_report_preserves_initial_deferral(self):
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(policy.run_verify(self.root, self.core_command(), self.command), 0)
        self.assertEqual((self.root / "artifacts/bench/smoke-phases.json").read_text(), "generated")
        self.assertFalse(policy.assess(self.root).deferred)
        self.assertFalse(self.record.exists())

    def test_core_failure_stops_verification(self):
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(policy.run_verify(self.root, self.core_command(9), self.command), 9)
        self.assertFalse(self.record.exists())

    def test_preexisting_dirty_report_or_product_requires_benchmark(self):
        for name in ("artifacts/bench/smoke-phases.json", "crates/product.rs"):
            with self.subTest(name=name):
                self.record.unlink(missing_ok=True)
                self.git("reset", "--hard", policy.APPROVED_BASE)
                self.write(name)
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(policy.run_verify(self.root, self.core_command(), self.command), 0)
                self.assertEqual(self.record.read_text(), "ran")

    def test_required_benchmark_failure_after_core_propagates(self):
        self.write("crates/product.rs")
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(
                policy.run_verify(self.root, self.core_command(), (*self.command[:-1], "7")), 7
            )

    def test_unavailable_core_fails_without_benchmark(self):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(policy.run_verify(self.root, ("missing-core-command",), self.command), 1)
        self.assertFalse(self.record.exists())

    def test_dagger_uses_shared_policy_without_sdk_import(self):
        self.assertIs(dagger_check.container_task, policy.container_task)


if __name__ == "__main__":
    unittest.main()
