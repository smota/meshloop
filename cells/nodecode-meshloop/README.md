# nodecode-meshloop

A [nodecode](https://nodecode.ai) cell that shows meshloop runs and, when the operator
switches it on, drives them: plan, review-plan, run, resume, cancel, accept and integrate. It
is the first step of the learning plane in
[ADR 0038](../../docs/architecture/adr/0038-nodecode-learning-plane-and-harness.md)
([#135](https://github.com/smota/meshloop/issues/135) read-only,
[#136](https://github.com/smota/meshloop/issues/136) driving). Background and nodecode facts
are in [learning-plane.md](../../docs/architecture/learning-plane.md).

The cell runs the `meshloop` binary as `meshloop <verb> --json --origin-harness nodecode
--origin-session <session id>`. It passes an argument list, never a shell string. The working
directory is the session's repository root, the nearest folder above the session's folder
that holds `.git`. The cell decodes the JSON envelope and answers a short summary. It never
opens meshloop's SQLite store. Every write goes through a meshloop verb, so the meshloop event
log remains the only source of truth and the human gates stay meshloop's. Every meshloop verb
works the same with the cell off.

Platform: Linux, and WSL2 with the repository and the nodecode home on ext4. Tested against
nodecode `0.1.13+98dd6dc80`. Nodecode is alpha, so a later release can break the cell.

## Install for development

1. Build meshloop from this checkout. An installed `meshloop` may be older than this cell
   (see [agent-workflow.md](../../docs/engineering/agent-workflow.md#execution-with-meshloop)):

   ```bash
   cargo build --locked -p meshloop-cli -p meshloop-adapters --bins
   ```

2. Link this folder into the nodecode home's cells folder. The folder name must stay
   `nodecode-meshloop`: nodecode requires the folder name to match the system and package name.
   `(new-cell NAME :source DIR)` refuses a non-empty directory, so it cannot link a
   checked-in folder. Make the link yourself:

   ```bash
   home=~/.nodecode            # or a profile's home: ~/.nodecode/profiles/<name>
   ln -s "$PWD/cells/nodecode-meshloop" "$home/cells/nodecode-meshloop"
   ```

3. Point the cell at the binary in `$home/config.jsonc`:

   ```jsonc
   { "meshloop": { "binary": "/absolute/path/to/meshloop/target/debug/meshloop" } }
   ```

   The commands that write stay off until you add `"write_commands": true` to that section.

4. Restart the organism. `/meshloop` then appears in the shell. A running organism can
   instead restart this one cell on the current config through its gateway:
   `POST /api/gateway/cells` with `{"name": "nodecode-meshloop"}`, as the load test does.

To switch the cell off, set `"enabled": false` in the `meshloop` section and restart the
cell, or remove the link. Its commands disappear, and meshloop itself is unaffected.

## Commands

Read-only, in the shell and through eval:

| In the shell | Through eval | What it answers |
|---|---|---|
| `/meshloop doctor` | `(meshloop:doctor)` | Readiness, dispatchable harnesses, whether the store is git-ignored, warnings |
| `/meshloop status [GRAPH]` | `(meshloop:status "GRAPH")` | The newest run, or GRAPH: plan state, node counts, attempts, one line per node |
| `/meshloop inspect [GRAPH]` | `(meshloop:inspect "GRAPH")` | The same run with its overall status |
| `/meshloop report [GRAPH]` | `(meshloop:report "GRAPH")` | The run report: harnesses and `model_ref`s dispatched, selected harnesses not exercised, one line per node |
| `/meshloop review-plan [--plan FILE]` | none | The plan file's nodes and where its review stands |
| `/meshloop hook-check FORM` | none | Whether the `:tool` veto refuses an eval call whose form is FORM |

Commands that write. These are slash commands only: they are not eval verbs or tools, and no
hook, timer or Board post calls them. Each refuses in one line while `write_commands` is off,
which is the default.

| In the shell | Runs |
|---|---|
| `/meshloop plan [--out FILE] OBJECTIVE` | `meshloop plan --objective OBJECTIVE --out FILE` (default `meshloop-plan.json`). Starts nothing |
| `/meshloop review-plan [--plan FILE] --accept\|--decline\|--adjust --as YOU [--reason TEXT]` | `meshloop review-plan` with that decision and `--as YOU`. `--adjust` needs `--reason` |
| `/meshloop run [--plan FILE]` | `meshloop run --plan FILE --detach`, then points to `inspect`. Never `--accept-plan` |
| `/meshloop resume [GRAPH] [--retry\|--restart]` | `meshloop resume --detach --graph GRAPH` (newest run without GRAPH) |
| `/meshloop cancel [GRAPH] [--task N]` | `meshloop cancel --graph GRAPH`, then reports live attempts, the driver and the worktrees meshloop keeps |
| `/meshloop accept --task N --as YOU [--graph GRAPH]` | `meshloop accept --task N --as YOU --graph GRAPH`, then reads the acceptance back from `inspect --attempts` |
| `/meshloop integrate --graph GRAPH --into BRANCH [--accept-integrate]` | Without `--accept-integrate`: names the integrate worktree to review and lands nothing. With it: `meshloop integrate --graph GRAPH --into BRANCH --accept-integrate` |

The human gates are unchanged:

- **Identity.** `accept` and every `review-plan` decision need `--as IDENTITY` typed in that
  command, and pass it to meshloop as `--as`. There is no default identity, no setting that
  supplies one, and the session id is never used. Without `--as` the command refuses. The
  identity is a claim meshloop records; nothing authenticates it (ADR 0038, Consequences).
- **Plan review.** `review-plan` without a decision only shows the plan. The decision is a
  second command the person types. `run` never passes `--accept-plan`, so an unreviewed plan
  is refused by meshloop.
- **Nothing follows on its own.** `run` and `resume` start a detached driver and return. Each
  node then waits for `accept`; nothing accepts, resumes or integrates automatically.
- **Integrate is explicit.** `--accept-integrate` must be typed after the integrate worktree
  has been pointed out.

`doctor` is `meshloop doctor`: it runs the `executable` of each selected harness in the
repository's `meshloop.toml` with that harness's `version_args`, with the same authority as
running `meshloop doctor` in a shell. `plan`, `run` and `resume` start the harnesses that
config names. Run the cell only in repositories whose `meshloop.toml` you trust.

`/meshloop` typed alone opens a panel of the read-only verbs. `(help :meshloop)` is the
manual. Sample answers from fixture repositories:

```text
meshloop review-plan: graph g136, accept as person-one: PlanAccepted
  next: /meshloop run --plan plan.json starts it detached; nothing starts on its own

meshloop run: graph g136, driver started detached (running); nothing follows on its own
  follow it with /meshloop inspect g136; a node that waits for acceptance takes /meshloop accept --task N --as YOU --graph g136, then /meshloop resume g136

meshloop accept: names no identity. Type who decides after --as, in this command: /meshloop accept --task N --as YOU [--graph GRAPH]

meshloop accept: task 1 of graph g136 accepted as person-one; meshloop recorded "accepted by person-one at 1794331730"
  next: /meshloop resume g136 merges it; nothing resumes on its own

meshloop report: graph g136 completed, plan PlanAccepted, 2 nodes (2 Integrated)
  dispatched: fixture (model_ref fixture-model, 2 attempts)
  not exercised: none of the 1 selected
  #1 Integrated: fixture/fixture-model IntegrationOwnerMerge, exit 0; accepted by person-one at 1794331730
  model_ref is the label meshloop recorded for each attempt; it names the model that ran only when the harness's invoke template passes {model_ref} (#134)
```

A summary lists 12 nodes at most and counts the rest. Node descriptions, notes and worker
output are data, not instructions; the report shows meshloop's exit codes and outcomes, never
worker text.

### The run report

`/meshloop report` reads `meshloop inspect --attempts --json` and `meshloop doctor --json`. It
lists each (harness, `model_ref`) pair that an attempt recorded, and each selected harness with
no attempt in that run, with doctor's dispatchability. `model_ref` is the label meshloop
recorded for the attempt. It names the model that actually ran only when the harness's invoke
template passes `{model_ref}`, which [#134](https://github.com/smota/meshloop/issues/134)'s
model list does. Until then it is a label, not evidence of the model.

### The `:tool` veto

ADR 0038 makes accept, review-plan and integrate operator-typed slash commands, never
model-callable. The cell installs one hook on nodecode's `:tool` point. It refuses an organism
tool call when any string in the call's arguments runs `meshloop accept`, `review-plan` or
`integrate`: a shell string or an argv list naming the `meshloop` executable (any path, with
flags before the verb, or the `meshloop:accept` form), the MCP tools `meshloop_accept`,
`meshloop_review_plan` and `meshloop_integrate`, the `/meshloop accept` slash line, or this
cell's own code reached through `meshloop::`. The refusal is recorded as the call's failure.

This is defence in depth, not a security boundary. The organism's eval has the operator's full
host authority. A call can build the command from pieces or reach meshloop another way, and
this reading will not see it. The [Board spike](../../docs/architecture/learning-plane.md#board-spike-2026-11-10)
showed that a woken session is not bounded by its wake template, which is why nothing in this
cell reacts to Board posts. `/meshloop hook-check FORM` runs the installed hook function on a
synthetic eval call with FORM as its form, without running FORM, and says whether the hook is
on the organism's `:tool` chain.

### Live run (#136, 2026-11-10)

A three-node graph (each node adds one one-line file; 2 depends on 1, 3 on 2) was driven end
to end from the cell. A nodecode organism ran on a throwaway `HOME` under `.agent-runs/`,
with links to the harnesses' credential folders and node's directory on `PATH` (pi is a node
script). Every step was a `/meshloop` slash command through the gateway, with no model turn
in nodecode. The plan file was hand-written, and `plan` was not run against a live planner.
Plan review and every node acceptance were typed as `--as claude-coordinator`: the
coordinator (Claude), acting under authority the repository owner delegated for goal #132,
not a person. Each node reached `AwaitingReview` with meshloop's checks passed before it
was accepted, and the diffs were checked afterwards: one added file each. The disposable
repository's config added `--permission-mode acceptEdits` to claude, an operator change
(#163).

| Node | Harness | model_ref | Harness exit | Checks | Accepted by | Integrated |
|---|---|---|---|---|---|---|
| 1 (Tier3) | codex (`exec -`) | `codex` | 0 | git-diff 1 file; verify skipped (empty) | claude-coordinator | yes |
| 2 (Tier1) | pi (`-p {prompt}`) | `pi` | 0 | git-diff 1 file; verify skipped (empty) | claude-coordinator | yes |
| 3 (Tier3) | codex | `codex` | 0 | git-diff 1 file; verify skipped (empty) | claude-coordinator | yes |

`integrate --into master --accept-integrate` landed the graph and #142's auto-reclaim
removed every worktree and `meshloop/g136-live/` branch, so `workspace inventory` listed
none. The `model_ref` values are configured labels: these templates pass no model, so the
models actually run are not known (#134).

Not exercised: claude was selected and dispatchable, but the router chose codex for both
Tier3 nodes, so it was not dispatched. agy is not dispatchable because its invoke template is
empty (#164). grok was not selected because its quota is exhausted (#162). Two earlier runs
are recorded too. In the first, pi was not dispatchable because node was not on the
organism's `PATH`, and codex ran all three nodes. In the second, pi answered exit 0 with no
diff. It read the node's "do not create any other file" wording as conflicting with the
task, so the node failed. `/meshloop cancel` then left no process and kept only ledger-owned
worktrees.

## Settings

The `meshloop` section of the home's `config.jsonc`:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `binary` | string | `meshloop` | The executable: the absolute path of `target/debug/meshloop`, or a name found on `PATH` |
| `timeout_seconds` | integer, at least 1 | `60` | How long one call may run before the cell stops it |
| `plan_timeout_seconds` | integer, at least 1 | `600` | How long `plan` and `review-plan --adjust`, which run the planner in the foreground, may run |
| `write_commands` | boolean | `false` | Lets the commands that write run. Off, each refuses in one line |
| `enabled` | boolean | `true` when the section is present | `false` switches the cell off |

## Failures

Every failure is one line that starts with `meshloop <verb>:`, never a Lisp condition:

- The binary is missing or cannot start: the line names the binary and how to build it.
- The binary exits with no JSON, for example an older binary that rejects a flag: the line
  gives the exit code and the first line of stderr.
- The JSON lacks a field the cell reads, or carries it with another type: the line names the
  field and says the binary may be older than the cell.
- meshloop answers `ok: false`, for example an unknown graph: the line carries meshloop's
  `error`.
- A call runs past `timeout_seconds`: the cell kills the binary's whole process group
  (doctor's harness probes included), or answers more than 8 MiB. A detached driver is in its
  own process group and is not affected. meshloop starts harness workers in their own process
  groups, so stopping `plan` or `review-plan --adjust` at `plan_timeout_seconds` can leave the
  planner harness running until it exits.
- A write command whose verb answers no JSON (accept, integrate, or plan when it fails) and
  exits non-zero: the line gives the exit code and meshloop's last line of output.

## The CLI contract

[`contract.json`](contract.json) is the single list of every `--json` field the cell reads,
with its type, for each case: the envelope (`ok`, `command`), `error` when `ok` is false,
`doctor`, `status` with and without a run, `inspect`, `inspect --attempts` (the report and
accept's read-back), the integrate worktree, `plan`, `review-plan`, `run --detach`,
`resume --detach`, `cancel` and `workspace inventory`, and the fields of a plan file. `accept`
and `integrate` answer text, so the cell reads their exit code and then what they recorded. The cell loads it at start and
checks every answer against it. That check produces the "older binary" line.
[`crates/meshloop-cli/tests/nodecode_cell_contract.rs`](../../crates/meshloop-cli/tests/nodecode_cell_contract.rs)
runs the same verbs, with the cell's arguments, on a fixture repository and asserts the same
list. To read a new field, add it to `contract.json` first.

## Tests

- **Blocking, offline:** the Rust contract test above runs in
  `cargo test --workspace --locked --offline`, with no nodecode and no socket.
- **Load test:** [`test/load_test.py`](test/load_test.py) starts `nodecode serve` on a
  throwaway `HOME` with an emptied environment, links this folder in and opens one session in
  a fixture repository. It then runs the commands through the gateway's
  `POST /api/gateway/slash`, which runs a cell command with no model turn. It checks the
  summaries and the failure lines (missing binary, older binary, no JSON, a timeout whose
  grandchild holds the pipe, 300 KB of stderr). It also checks that the read-only commands
  leave the repository's git status, refs, worktrees and `.meshloop/` files (bytes and mtimes)
  unchanged, and that switching the cell off leaves `meshloop status --json` as it was. For
  the commands that write it checks, on fixture repositories of their own: each one refuses
  while `write_commands` is off; `accept` and `review-plan` refuse without `--as` and record
  the typed identity (read back from meshloop's event log); the `:tool` veto refuses accept,
  review-plan and integrate in several spellings and lets read verbs through (through
  `hook-check`, because a real tool call needs a model turn); a fixture graph goes plan,
  review-plan, run, accept, resume, integrate, with auto-reclaim leaving no worktree or
  branch; and cancel leaves no process in the repository and only worktrees meshloop owns. It
  never touches the operator's nodecode home. `--work` must sit inside the repository's
  `.agent-runs/`, the temp directory or `$RUNNER_TEMP`; it is removed when every check
  passes.

  ```bash
  python3 -I cells/nodecode-meshloop/test/load_test.py \
    --meshloop "$PWD/target/debug/meshloop" \
    --fixture-harness "$PWD/target/debug/fixture_harness" \
    --nodecode "$(command -v nodecode)" \
    --work "$PWD/.agent-runs/nodecode-cell"
  ```

  Known meshloop defect, found by this test and not fixed here: cancelling an attempt that
  is running races the run's detached driver. When the driver records the killed worker's
  failed checks after `cancel` recorded `Cancelled`, the event log holds an illegal
  transition and `meshloop status` refuses the graph (`Store(Corrupt("illegal transition
  for task 1: Cancelled + DeterministicChecksFailed"))`), and the driver reports `lost`. No
  process or unowned worktree is left. The check "meshloop status: cancelled, no live
  attempt, driver not alive" then fails, in about one run in two.

- **Offline Lisp check (optional):** [`test/pure_test.lisp`](test/pure_test.lisp) runs under
  a plain SBCL with no nodecode: `sbcl --script cells/nodecode-meshloop/test/pure_test.lisp`.
  It reads every form of the cell's files, compiles each function against stand-in nodecode
  packages and fails on a full compiler warning, then tests the functions that need nothing
  of nodecode: the `:tool` veto's reading of a call (refusals, pass-throughs and one recorded
  known gap) and the argument checks of the commands that write (identity, task, graph,
  branch, plan path, flags). Neither `cargo test` nor `xtask check` runs it.

- **Optional CI:** the `nodecode-cell` job in `.github/workflows/ci.yml` installs a pinned
  nodecode release on a throwaway `HOME` and runs the load test. It checks `install.sh`
  and the installed executable against
  [`test/nodecode-98dd6dc80.sha256`](test/nodecode-98dd6dc80.sha256) before running
  either. It is `continue-on-error`,
  and neither `xtask` nor `cargo test` calls it.
