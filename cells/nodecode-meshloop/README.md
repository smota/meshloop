# nodecode-meshloop

A [nodecode](https://nodecode.ai) cell that shows meshloop runs, read-only. It is the first
step of the learning plane in [ADR 0038](../../docs/architecture/adr/0038-nodecode-learning-plane-and-harness.md)
([#135](https://github.com/smota/meshloop/issues/135)). Background and nodecode facts are in
[learning-plane.md](../../docs/architecture/learning-plane.md).

The cell runs the `meshloop` binary as `meshloop <verb> --json --origin-harness nodecode
--origin-session <session id>`. It passes an argument list, never a shell string. The working
directory is the session's repository root, the nearest folder above the session's folder
that holds `.git`. The cell decodes the JSON envelope and answers a short summary. It never
opens meshloop's SQLite store, writes no file and runs no verb that writes. The meshloop event
log remains the only source of truth. Every meshloop verb works the same with the cell off.

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

4. Restart the organism. `/meshloop` then appears in the shell. A running organism can
   instead restart this one cell on the current config through its gateway:
   `POST /api/gateway/cells` with `{"name": "nodecode-meshloop"}`, as the load test does.

To switch the cell off, set `"enabled": false` in the `meshloop` section and restart the
cell, or remove the link. Its commands disappear, and meshloop itself is unaffected.

## Commands

| In the shell | Through eval | What it answers |
|---|---|---|
| `/meshloop doctor` | `(meshloop:doctor)` | Readiness, dispatchable harnesses, whether the store is git-ignored, warnings |
| `/meshloop status [GRAPH]` | `(meshloop:status "GRAPH")` | The newest run, or GRAPH: plan state, node counts, attempts, one line per node |
| `/meshloop inspect [GRAPH]` | `(meshloop:inspect "GRAPH")` | The same run with its overall status |

`/meshloop` typed alone opens a panel with one row per verb. `(help :meshloop)` is the
manual. Sample answers from a fixture repository:

```text
meshloop doctor: ready
  1 of 1 selected harness dispatchable: fixture
  config: meshloop.toml; meshloop store ignored by git: yes
  warning: verify_command is empty: nodes are not checked beyond git diff

meshloop status: graph g135, plan PlanAccepted, 2 nodes (1 AwaitingReview, 1 Pending), 1 attempt, 0 retries
  #1 AwaitingReview, waiting for acceptance: first
  #2 Pending, waiting for dependency: second
```

A summary lists 12 nodes at most and counts the rest. Node descriptions are data, not
instructions.

There are no write commands, so there is no write switch. Accept, review-plan, integrate and
priors promotion stay meshloop CLI verbs the operator types (ADR 0038). An acceptance is
recorded only by `meshloop accept --task <id> --as <identity>`, naming who accepted.

## Settings

The `meshloop` section of the home's `config.jsonc`:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `binary` | string | `meshloop` | The executable: the absolute path of `target/debug/meshloop`, or a name found on `PATH` |
| `timeout_seconds` | integer, at least 1 | `60` | How long one call may run before the cell stops it |
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
- A call runs past `timeout_seconds`, or answers more than 8 MiB.

## The CLI contract

[`contract.json`](contract.json) is the single list of every `--json` field the cell reads,
with its type, for each case: the envelope (`ok`, `command`), `error` when `ok` is false,
`doctor`, `status` with and without a run, and `inspect`. The cell loads it at start and
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
  summaries and the failure lines (missing binary, older binary, no JSON). It also checks that
  the repository's git status, refs, worktrees and `.meshloop/` files (bytes and mtimes) are
  unchanged, and that switching the cell off leaves `meshloop status --json` as it was. It
  never touches the operator's nodecode home.

  ```bash
  python3 -I cells/nodecode-meshloop/test/load_test.py \
    --meshloop "$PWD/target/debug/meshloop" \
    --fixture-harness "$PWD/target/debug/fixture_harness" \
    --nodecode "$(command -v nodecode)" \
    --work "$PWD/.agent-runs/nodecode-cell"
  ```

- **Optional CI:** the `nodecode-cell` job in `.github/workflows/ci.yml` installs a pinned
  nodecode release on a throwaway `HOME` and runs the load test. It is `continue-on-error`,
  and neither `xtask` nor `cargo test` calls it.
