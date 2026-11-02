# Issue 20 WSL2 gate record

Commit `d31271903fe9eecfa0a4e44a4d2103f397489f83`. The kernel identifies this host as WSL2 (`microsoft-standard-WSL2`). Commands used `--locked --offline` from a clean worktree of that commit.

## Gate commands

| Command | Result |
|---|---|
| `cargo run -p xtask -- check` | exit 0. fmt, clippy `-D warnings`, and the workspace tests passed. One pre-existing adapter test stayed ignored: `retained_schema5_binary_refuses_schema6_without_mutation`. |
| `cargo run -p xtask -- live` | exit 0. Doctor reported `daemonless: true` and `live_transport: direct-cli`. `git worktree list` succeeded. |
| `cargo run -p xtask -- smoke` | exit 0. Product wall-clock 0.59s, under the 5.0s gate. Isolation line: integrity check ok, ancestor propagation 100%, 5 sandbox worktrees, 0 host leaks, locks clean. Workspace lifecycle scenario passed. |

No command failed, so no follow-up bug was filed.

## Process-group cancellation

On this host, a new session started a shell and a `sleep` child in the same process group. Both were running. `kill -s KILL` to that group id left both processes gone.

The same `xtask check` run passed `process::tests::kills_live_child_process_tree`. On Unix that helper sends `SIGKILL` to the process group, then to the pid.

## Git index lock

A disposable repository held a rival `.git/index.lock`. `git add` exited 128 and reported that the lock file already exists. After the rival file was removed, `git add` and a local commit succeeded, and `.git/index.lock` was gone. Smoke also reported the sandbox locks clean.

The adapter retries a Git admin command when stderr names `index.lock`. This probe did not call that retry. It shows the WSL2 Git failure the retry is aimed at.
