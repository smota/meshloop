# /// script
# requires-python = ">=3.12,<3.15"
# dependencies = ["dagger-io==0.21.12"]
# ///
"""Run the native mise task in Linux without importing station state."""
from __future__ import annotations

# SDK reference: https://github.com/dagger/dagger/tree/v0.21.12/sdk/python

import asyncio
import os
import shutil
import subprocess
import sys
import tempfile
from contextlib import AsyncExitStack
from pathlib import Path, PurePosixPath
from typing import TYPE_CHECKING

from benchmark_policy import container_task

if TYPE_CHECKING:
    import dagger

ROOT = Path(__file__).resolve().parents[2]
INPUT_ROOTS = {
    "adapters",
    "benches",
    "cells",
    "config",
    "crates",
    "docs",
    "lib",
    "schemas",
    "scripts",
    "skills",
    "tests",
    "xtask",
}
INPUT_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "mise.toml",
    "README.md",
    "LICENSE",
    "NOTICE",
    "meshloop.toml",
}


def source_files() -> list[str]:
    """Import candidate build inputs, including nonignored new source files."""
    tracked = (
        subprocess.check_output(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            cwd=ROOT,
        )
        .decode()
        .split("\0")
    )
    files = []
    for name in tracked:
        path = PurePosixPath(name)
        if not name:
            continue
        if name not in INPUT_FILES and path.parts[0] not in INPUT_ROOTS:
            continue
        if any(
            part
            in {
                ".git",
                ".meshloop",
                ".agent-runs",
                "node_modules",
                "__pycache__",
                "private",
                "secrets",
            }
            or part.startswith(".env")
            for part in path.parts
        ):
            continue
        if path.suffix.lower() in {".pem", ".key", ".pfx", ".p12"}:
            continue
        candidate = ROOT
        for part in path.parts:
            candidate /= part
            if candidate.is_symlink() or candidate.is_junction():
                raise ValueError(f"Source link requires explicit review: {name}")
        if not candidate.is_file():
            continue
        files.append(name)
    return sorted(files)


async def containment_probe(container: dagger.Container) -> bool:
    import dagger

    probe = container.with_exec(
        [
            "bwrap",
            "--unshare-user",
            "--disable-userns",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--unshare-net",
            "--die-with-parent",
            "--new-session",
            "--ro-bind",
            "/",
            "/",
            "true",
        ],
        expect=dagger.ReturnType.ANY,
    )
    containment_available = await probe.exit_code() == 0
    if containment_available:
        print(
            "Linux containment probe passed; containment tests are required",
            flush=True,
        )
    else:
        print(
            f"WARNING: containment not exercised; Linux containment acceptance unavailable: {await probe.stderr()}",
            file=sys.stderr,
            flush=True,
        )
    return containment_available


async def main(
    staged_source: str, revision: str, dirty: bool, file_count: int, task: str
) -> int:
    import dagger

    os.environ["DO_NOT_TRACK"] = "1"
    print(
        f"Linux source snapshot: HEAD={revision} dirty={dirty} files={file_count}",
        flush=True,
    )
    async with AsyncExitStack() as stack:
        connection = dagger.Connection(dagger.Config(log_output=sys.stderr))
        client = await asyncio.wait_for(
            stack.enter_async_context(connection), timeout=90
        )
        source = client.host().directory(staged_source)
        print(f"Linux source digest: {await source.digest()}", flush=True)
        mise = (
            client.container()
            .from_("ghcr.io/jdx/mise:2026.11.6")
            .file("/usr/local/bin/mise")
        )
        checks = (
            client.container()
            .from_("rust:1.98.0-bookworm")
            .with_exec(
                [
                    "sh",
                    "-ec",
                    "apt-get update && apt-get install -y --no-install-recommends git python3 bubblewrap && rm -rf /var/lib/apt/lists/*",
                ]
            )
            .with_file("/usr/local/bin/mise", mise)
            .with_env_variable("MISE_YES", "1")
            .with_env_variable("MISE_TRUSTED_CONFIG_PATHS", "/src")
            .with_env_variable("DO_NOT_TRACK", "1")
            .with_env_variable("CARGO_BUILD_JOBS", "2")
            .with_exec(
                [
                    "rustup",
                    "component",
                    "add",
                    "--toolchain",
                    "1.98.0",
                    "rustfmt",
                    "clippy",
                ]
            )
            .with_mounted_cache(
                "/usr/local/cargo/registry",
                client.cache_volume("meshloop-cargo-registry"),
            )
            .with_mounted_cache(
                "/src/target", client.cache_volume("meshloop-linux-rust-1.98-target")
            )
            .with_directory("/src", source)
            .with_workdir("/src")
            .with_exec(
                ["git", "init", "--quiet", "--initial-branch=validation-snapshot"]
            )
            .with_exec(
                [
                    "chmod",
                    "+x",
                    "adapters/nodecode/containment-test.sh",
                    "adapters/nodecode/meshloop-nodecode-sandbox",
                    "adapters/sandbox/meshloop-verify-sandbox",
                    "adapters/sandbox/verify-containment-test.sh",
                ]
            )
        )
        containment_available = await containment_probe(checks)
        if containment_available:
            checks = checks.with_env_variable("MESHLOOP_REQUIRE_CONTAINMENT", "1")
        await checks.with_exec(["mise", "run", task]).sync()
        return 0 if containment_available else 2


if __name__ == "__main__":
    task = container_task(ROOT)
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT))
    files = source_files()
    with tempfile.TemporaryDirectory(prefix="meshloop-validation-") as staged_source:
        for name in files:
            destination = Path(staged_source) / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        raise SystemExit(
            asyncio.run(
                asyncio.wait_for(
                    main(staged_source, revision, dirty, len(files), task), timeout=3600
                )
            )
        )
