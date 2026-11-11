# Nodecode worker containment

- Issue: [#138](https://github.com/smota/meshloop/issues/138), goal [#132](https://github.com/smota/meshloop/issues/132)
- Decisions it implements: [ADR 0038](../architecture/adr/0038-nodecode-learning-plane-and-harness.md)
  "Nodecode worker and containment", ADR 0003 (operator config, no shipped flags),
  ADRs 0025 and 0035 (process-tree ownership)
- Status: proposed for coordinator acceptance (the owner delegated human approvals for #132)
- Built and measured: 2026-11-11, nodecode `0.1.14+cef1be5a3`, bubblewrap 0.11.1,
  Ubuntu (glibc 2.43) on WSL2, kernel 6.18, ext4

This page stands alone for review. It says what the recipe contains, what it does not,
how an operator sets it up, and what was run to show it.

## What it is

Nodecode has no sandbox of its own. Its `--sandbox` jail is withheld in this release,
and it evaluates with the full authority of the user who runs it. ADR 0038 allows an
unattended nodecode worker only under containment supplied outside nodecode.

The recipe is two files under `adapters/nodecode/`:

- `meshloop-nodecode-sandbox` is a bash wrapper. The operator points a
  `[harnesses.nodecode]` entry's `executable` at it. It builds a
  [bubblewrap](https://github.com/containers/bubblewrap) sandbox and runs one nodecode
  turn inside it.
- `nodecode-rpc-turn.py` runs inside the sandbox. It drives that turn through
  `nodecode rpc` and exits when the turn ends (see
  [harnesses.md](harnesses.md#nodecode-worker-138) for why the shell cannot be used).

It is operator configuration, not a shipped default. Meshloop's code has no nodecode
path: the probe, dispatch, timeout and cancel are the ones every subprocess harness
uses.

## Threat model

The worker is a nodecode organism driving a model. Treat the model's tool calls as
untrusted: prompt injection from the repository, a confused model, or a hostile
dependency can make it run any command its process may run. The assets are the
operator's files and credentials, the operator's own nodecode organism and its home
(`~/.nodecode`, serving a live gateway), other repositories, and the host's processes.

### Contained

| Property | Mechanism | Shown by |
|---|---|---|
| Writes land only in the attempt worktree | The worktree is the only host path bound read-write. The sandbox root is a tmpfs remounted read-only after setup. `/usr`, a short `/etc` list, the nodecode release folder and the worktree's git metadata are bound read-only. | `containment-test.sh` §1: writes to the repository root above the worktree, the git common dir, the operator's home, `~/.nodecode`, `/usr`, `/etc` and `/` fail and nothing appears on the host |
| The worker cannot redirect meshloop's host-side git | The worktree's `.git` file is bound read-only over the worktree, so the worker cannot point it at a gitdir whose config would run commands when meshloop runs `git diff` on the host after the attempt. The wrapper never runs git on the worktree: it parses `.git` itself and binds the admin and common dirs only when the admin dir's `gitdir` file points back at this worktree | §1: rewriting, removing or moving `.git` fails; `git rev-parse` still works inside |
| The operator's home, `~/.nodecode`, `~/.ssh`, `~/.config` and other repositories are not even readable | Not mounted. The sandbox `HOME` is `/home/worker`, a fresh tmpfs | §2 (visibility) and §6 (`/proc/self/mountinfo` names no operator home) |
| The worker runs on a nodecode home of its own | `NODECODE_HOME=/home/worker/.nodecode` on that tmpfs, plus `rpc --ephemeral`. The home and store vanish with the sandbox; a cancelled or killed attempt leaves no temp home behind | live run; operator home manifest unchanged (test e below) |
| Never attaches to the operator's organism | A client finds a gateway through the store's sidecar in its home, which the worker cannot see. `NODECODE_GATEWAY_PORT` is cleared, so the worker binds its own ephemeral port | source `gateway/gateway.lisp` (`open-gateway-listener`); environment check §3 |
| Exactly one provider credential | The operator's key file is bound read-only at `<home>/secrets/providers.<id>.api_key`, nodecode's secret-file tier. The wrapper never reads or copies it, and it is not in any argv or environment | §3: present, read-only, nowhere in the worktree |
| No other credential | `--clearenv`, then a fixed allow-list (`HOME`, `NODECODE_HOME`, `PATH`, `TERM`, `NO_COLOR`, `LANG`, `TMPDIR`, `USER`). The operator's Slack tokens, `NODECODE_GATEWAY_PORT` and any `*_API_KEY` do not reach the worker | §3: a canary variable set by the caller is absent; nothing outside the allow-list |
| No self-update of the operator's install | The worker's `config.jsonc` sets `update.mode` to `off`. The release folder is bound read-only and its parent, where the `current` link sits, is not mounted, so nodecode does not see an installed layout either | §3 config check; install manifest unchanged (test e) |
| Process-tree ownership (ADR 0025) | The wrapper `exec`s bwrap, so the PID and process group meshloop owns are bwrap's. `--unshare-pid` puts every worker process in a new PID namespace whose init is bwrap's child; `--die-with-parent` kills that init when bwrap or its parent dies, and the kernel then kills the whole namespace, including processes that called `setsid` (nodecode's `serve` and its shell tool both do) | §4 and §5; live cancel, timeout and owner-kill below |
| No new namespaces, capabilities or terminal injection | `--unshare-user --disable-userns`, `--cap-drop ALL`, `--new-session` (no controlling terminal, so no `TIOCSTI`), `--unshare-ipc --unshare-uts --unshare-cgroup-try` | `containment-test.sh` builds with these flags |
| Linux and WSL on a native filesystem only (ADR 0038 invariant 12) | The wrapper refuses a worktree that is not on ext4, xfs or btrfs (so never `/mnt/c`), refuses the operator's home or anything containing it, and refuses a turn outside a linked git worktree | wrapper refusals, exit 125 |

### Not contained

- **Network.** The worker shares the host network namespace, because the model API needs
  it and bubblewrap has no egress filter (no `pasta` or `slirp4netns` is installed
  here). The worker can reach any host the operator can, **including services on the
  host's loopback** (the operator's nodecode gateway on 127.0.0.1, which still needs its
  operator token, and any unauthenticated local service) and abstract Unix sockets. It
  can exfiltrate what it can read: the worktree, the git history bound read-only, the
  one credential, and world-readable system files under `/usr` and the bound `/etc`
  entries. Narrowing this needs a network namespace with a forwarding proxy; that is a
  follow-up, not part of this recipe.
- **The one credential.** The worker can read its own key and send it anywhere on the
  network. Give each worker a key scoped and budget-capped at the provider.
- **CPU, memory and processes.** Not limited, except that the sandbox home and `/tmp`
  are tmpfs mounts capped by `--tmpfs-size` (default 1 GiB each; tmpfs counts against
  RAM). meshloop's `task_timeout_seconds` bounds wall time. An operator who wants hard
  limits can wrap the executable in `systemd-run --user --scope -p MemoryMax=... -p
  CPUQuota=...` where a user systemd instance runs; this was not tested.
- **The model API and its cost.** Which provider and model run is the operator's
  `model_ref`; the recipe does not cap spend. The driver prints nodecode's `turn.usage`
  (tokens and cost when known) into the attempt's stdout evidence.
- **Reads of the worktree's git metadata.** With `--git-metadata ro` (default) the
  worktree's admin dir and the repository's common `.git` are bound read-only, so the
  worker can read every branch and the repository's `.git/config`. Use
  `--git-metadata none` if `.git/config` holds secrets; `git` then does not work inside
  the worker.
- **Kernel attack surface.** No seccomp filter is installed. The sandbox relies on
  unprivileged user namespaces and the kernel's enforcement of mounts and namespaces.
- **Output content.** Whatever the worker prints is stored as evidence (meshloop
  redacts known secret shapes). A worker that prints its own key would put it in the
  ledger; the live runs below did not.

## Operator steps

1. Linux or WSL2 with the worktrees on ext4. Install bubblewrap 0.8 or newer (for
   `--disable-userns`), git and python3. Check that unprivileged user namespaces work:
   `bwrap --unshare-user --disable-userns --unshare-pid --ro-bind / / true`.
2. Make a private, read-only copy of one nodecode release folder (the binary and its
   `src/` stay together), so no worker ever runs the operator's installed `current`
   release in place:
   `cp -a ~/.local/share/nodecode/<release> ~/.local/share/meshloop/nodecode/ && chmod -R a-w ~/.local/share/meshloop/nodecode/<release>`
3. Put one provider key in a file of its own, mode 0600, outside every repository:
   `umask 077; printf '%s\n' "$KEY" > ~/.config/meshloop/nodecode-<provider>.key`.
   The provider id is the models.dev id nodecode uses (for example `openrouter`).
4. Optionally keep a copy of the models.dev catalog (`https://models.dev/api.json`,
   public) and pass it with `--models-catalog`. Without it each attempt fetches the
   catalog first and the driver waits up to 60 s for it.
5. Add the entry below (also in `config/meshloop.example.toml`), then run
   `adapters/nodecode/containment-test.sh` and `meshloop doctor --json`. The probe's
   version line names the containment, for example
   `nodecode 0.1.14+cef1be5a3 (contained: meshloop-nodecode-sandbox, bubblewrap 0.11.1)`.

```toml
[harnesses.nodecode]
kind = "nodecode"
executable = "/path/to/meshloop/adapters/nodecode/meshloop-nodecode-sandbox"
version_args = ["--nodecode", "/home/me/.local/share/meshloop/nodecode/<release>/nodecode", "--version"]
invoke_args_template = [
  "--nodecode", "/home/me/.local/share/meshloop/nodecode/<release>/nodecode",
  "--credential-provider", "<provider>",
  "--credential-file", "/home/me/.config/meshloop/nodecode-<provider>.key",
  "--model", "{model_ref}", "--prompt-file", "{prompt_file}",
]
models = [{ model_ref = "<provider>/<model-id>", model_tier = "mid" }]
```

`model_ref` is `PROVIDER/MODEL[:EFFORT]` and the provider is required. One entry holds
one credential, so every model in its `models` list must be on that provider; use a
second entry (another harness name) for another provider. Nodecode runs only as a
subprocess worker: native dispatch for nodecode is refused (ADR 0039).

Never point the entry at `~/.nodecode`, a profile, or the learning-plane home, and never
add `--run-command` or `--` to `invoke_args_template`: those modes exist for tests and
spikes.

## Evidence (2026-11-11)

Commands ran in this worktree; disposable repositories lived under `.agent-runs/` and
were removed afterwards. The live model was `munito/deepseek-ai/deepseek-v4-flash`
through the operator's Munito key, the cheapest reachable model with a credential on
this host. OpenCode Zen's keyless free tier, nodecode's default, answered HTTP 429
"Free usage limit reached" for clients other than OpenCode.

| Check | Result |
|---|---|
| a. Probe | `doctor --json`: harness `nodecode` dispatchable, version `nodecode 0.1.14+cef1be5a3 (contained: meshloop-nodecode-sandbox, bubblewrap 0.11.1)`, model `munito/deepseek-ai/deepseek-v4-flash` with tier `TopTier` |
| b. One task | `review-plan --accept`, `run`: task 1 `AwaitingReview`, outcome `DeterministicChecksPassed`, `model_ref` recorded as `munito/deepseek-ai/deepseek-v4-flash`, `model_passed: true`; evidence: git-diff `HELLO.txt \| 1 +`, verify `grep -qx "hello from nodecode" HELLO.txt` exit 0, harness output ending `nodecode-turn: turn.completed`. 3 s per run; three runs, the last with the final wrapper and a private copy of the release |
| c. Cancel, timeout, owner kill | Each task asked the model to run `sleep 600`. While it ran, the worker's PID namespace held 7 or 8 processes (bwrap init, the driver, `nodecode rpc`, `setsid -w nodecode serve`, `nodecode serve`, the shell tool's `setsid -w bash`, `sleep`). After `meshloop cancel --graph` (task `Cancelled`), after the 25 s `task_timeout_seconds` (attempt `HarnessCrashedOrTimeout`, task `Failed`), and after `kill -KILL` of the meshloop driver itself: no process left (the cancel case was repeated with the final wrapper) in that namespace and none whose argv named the run |
| d. Writes outside the worktree | `containment-test.sh`: 29 checks pass (writes, the `.git` file, visibility, environment, credential, update-off config, group kill, bwrap kill, parent kill, mounts). Each one-line mutation of the final wrapper fails it: dropping `--die-with-parent` (3 failures), dropping `--clearenv` (2), binding the repository root instead of the worktree (1), dropping the read-only `.git` bind (2). It also runs as `cargo test -p meshloop-cli --test nodecode_containment` |
| e. Operator's nodecode home | Metadata manifest (path, type, size, mtime, inode; no file read) of `~/.nodecode` before and after each task run. Runs 1 and 2: no change. Run 3: two entries changed, `history.db-wal` and one `snapshots/` folder's mtime. Over the longer kill-test window, 345 entries changed, all under `history.db*`, `otel/`, `snapshots/`, `hub/` and `bad/`. These are the kinds the live organism changed during a 45 s control window with no worker running, and no changed path names the run. The separation does not rest on timing: the sandbox never mounts `~/.nodecode` (§2, §6), so no worker can be the writer. `~/.local/bin/nodecode`, `~/.local/share/nodecode/current` (still `cef1be5a3`, last moved at 09:26 that day, before the first sandboxed run) and the release binaries' SHA-256 were unchanged from before run 2 to after the last cancel test |
| f. Credentials | The key's value was searched for in every file under the run directories (298 files: ledgers, worktrees, harness output, raw rpc frame logs, spike output) and in all tracked and new files of the repository: no match. The key file was deleted afterwards |

Model spend: each completed turn cost under 0.001 USD by nodecode's own `turn.usage`
figures (four completed turns: one spike, three task runs). The four killed tasks were
cut off during the model's first tool call and reported no usage; their cost is not
measured but is of the same order.

## Limits of this evidence

- One host (WSL2), one nodecode release, one provider. Another distribution may lack
  unprivileged user namespaces (Ubuntu 24.04's AppArmor restriction, for example); the
  test skips there and the wrapper fails closed.
- The deterministic test runs probe commands under `--run-command`, not nodecode. The
  sandbox is the same except that nodecode mode adds the read-only release folder and
  the driver file.
- No test tried a deliberate kernel or network escape; see "Not contained".
