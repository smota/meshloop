# Learning plane: nodecode facts, cell install and CI

Supporting detail for [ADR 0038](adr/0038-nodecode-learning-plane-and-harness.md). The
ADR holds the decisions; this page holds the grounding facts and mechanism detail.

## Nodecode facts at 0.1.13+98dd6dc80

Read from the installed release and the local nodecode home on 2026-11-10. Nothing was
run. Release source paths are relative to `~/.local/share/nodecode/98dd6dc80/src/`.

| Fact | Source | Verified locally |
|---|---|---|
| Cells are declared with `nle:define-cell`; folder name = system name = package | `cells/template/cell.lisp` | yes, source |
| `(new-cell NAME :source DIR)` makes a cell whose folder lives in `DIR` and links it into the home's cells folder; `DIR` must be absent or empty | `src/organism/made-cells.lisp:195` | yes, source |
| A cell linked from an outside repository loads from the home's cells folder | `~/.nodecode/cells/sam-openrig` is a symlink into another repository | yes, filesystem |
| A hub entry may pin a sub-path of a repository (`"path"`) | `~/.nodecode/hub/*.json`, entry `nodecode-delegation` | yes, file |
| The MCP client cell supports no `cwd` for a server | `cells/mcp/README.md:64`, `cells/mcp/config.lisp:19` | yes, source |
| `--model [PROVIDER/]MODEL[:EFFORT]` opens a shell session on that model; the shell runs on a terminal | `nodecode help`, `src/cli/entry.lisp` | yes, help text |
| `nodecode rpc` drives the home's organism by JSON lines, attaching to a running one or starting one; `start_turn` may name provider, model and effort for a new session | `nodecode help rpc` | yes, help text |
| `--ephemeral` replaces the store only; config, the layer and cells stay the profile's | `src/cli/entry.lisp` (`--ephemeral` detail) | yes, help text |
| Board posts are facts in nodecode's event log, at most `board.post_chars` (default 1000) characters, meant as pointers; `@ID` wakes an idle session, capped by `board.hops` | `cells/board/README.md` | yes, source |
| `--sandbox` (srt jail) exists in source but is withheld: help omits it and the parser refuses it | `src/cli/entry.lisp` (`*offer-sandbox*` nil) | yes, source |
| No background gateway arm on Windows | `src/gateway/background.lisp:29,49` | yes, source |
| The release needs glibc 2.38 or newer on Linux | release `README.txt` | yes, file |
| `nc://subagents` is a community hub entry (`drvova/nodecode-subagents`, depends on `nodecode-board`) | `~/.nodecode/hub/3c16bad32327b07f.json` | entry only; source not read |
| `nc://subagents` v0.2.0 runs flat task lists in shared directories with no worktrees, budgets, verification or per-worker model | #132 benchmark table | no, goal text only |
| A hub cell `nodecode-delegation` offers "named model routes for subagents" | `~/.nodecode/hub/2468c84d448ad2e1.json` | entry only |
| Nodecode self-learning improves outcomes | none published | no; hypothesis until #140 |

## Installing the cell for development

The cell lives at `cells/nodecode-meshloop/`. A development install links the nodecode
home's cells folder to that folder, which is the form `new-cell :source` produces.
`new-cell :source` refuses a non-empty directory, so it cannot re-link the checked-in
folder. The link step is in the [cell's README](../../cells/nodecode-meshloop/README.md#install-for-development).
Publication pins a commit and sub-path of this repository in the hub (#141).

## CI for the cell

Owner decision 6 in #132.

- **Blocking, offline.** JSON contract tests in the Rust workspace run the CLI verbs the
  cell consumes against fixtures and validate the output against checked-in JSON Schemas
  or golden files. The cell's own tests read the same files as input. A CLI change that
  breaks the cell's input fails `cargo test --workspace --locked --offline` with no
  nodecode and no socket.
- **Optional, non-blocking.** A separate Linux job installs a pinned nodecode release,
  loads the cell on a throwaway home (temporary `HOME`, `--ephemeral`) and runs the
  cell's tests. It is `continue-on-error`, is never called from `xtask check` or
  `cargo test`, and does not gate merge. The runner needs glibc 2.38 or newer.
- **As built (#135, #136).** The shared file is
  [`cells/nodecode-meshloop/contract.json`](../../cells/nodecode-meshloop/contract.json), a
  list of field paths and types rather than golden output, asserted by
  `crates/meshloop-cli/tests/nodecode_cell_contract.rs` and checked by the cell on every
  answer. The optional job is `nodecode-cell` in `.github/workflows/ci.yml`; it runs
  `cells/nodecode-meshloop/test/load_test.py`.

## Driving runs from the cell (#136)

The cell's commands that write (`plan`, `review-plan`, `run`, `resume`, `cancel`, `accept`,
`integrate`) are `/meshloop` slash commands only, off until `meshloop.write_commands` is set.
They are thin calls to the binary; the [cell's README](../../cells/nodecode-meshloop/README.md#commands)
lists them. How the decisions in ADR 0038 were built:

- **Human gates.** `accept` and each `review-plan` decision take `--as` typed in that
  command; there is no default identity. `review-plan` without a decision only shows the
  plan. `run` never passes `--accept-plan`. `run` and `resume` start a detached driver, and
  nothing accepts, resumes or integrates on its own. `integrate` lands only with
  `--accept-integrate` typed.
- **Detached resume.** `meshloop resume --detach --graph <id>` was added for the cell. It
  starts the same driver `run --detach` starts. A foreground resume would run past the cell's
  timeout, and killing its process group would leave harness workers, which meshloop starts
  in their own process groups, running.
- **The `:tool` veto.** One hook refuses organism tool calls whose text runs accept,
  review-plan or integrate. It is defence in depth, not a boundary: the Board spike showed a
  woken session is not bounded by its wake template, and nothing in the cell reacts to Board
  posts. It was tested by running the installed hook function on synthetic calls, since a
  real tool call needs a model turn.
- **Run report.** `/meshloop report` lists the (harness, `model_ref`) pairs a run's attempts
  recorded and the selected harnesses it did not exercise. Since
  [#134](https://github.com/smota/meshloop/issues/134), `model_ref` is the model meshloop
  dispatched when the operator's template passes it (a `models` list and
  `--model {model_ref}`); otherwise it is the entry's label.

## Advisory input (#139)

This section describes the mechanism behind the ADR 0038 amendment of 2026-11-10 (#139). That
amendment is **Proposed — pending review**, and nothing here is built yet. The file format
is [`schemas/advisory-priors.v1.schema.json`](../../schemas/advisory-priors.v1.schema.json).
Code references are at base `ac87e7a`.

### What counts as evidence

The fold counts an attempt only if every one of these holds:

- `dispatch_mode` is `"subprocess"`;
- `model_ref` is non-empty and not `unverified`;
- the task is not `Cancelled`;
- the kernel recorded a result for it.

The result decides how the attempt counts:

- **Pass.** `DeterministicChecksPassed` is recorded.
- **Fail.** `DeterministicChecksFailed` or `HarnessCrashedOrTimeout` is recorded.

This mirrors `routing_feedback` after #167, which records feedback only after the check
result is in the ledger (`run_loop.rs:2364-2372`, `run_loop.rs:2719-2728`).

The fold reports every excluded attempt by reason: `not_subprocess`, `model_unverified`,
`cancelled` or `no_result`.

**Positive mode marker.** The attempt JSON (`report.rs:154`, `attempt_json`) has no
`dispatch_mode` field today. The kernel side of #139 adds one, and `contract.json`
requires it for `inspect_attempts`. The value depends on the build:

- At this base it is the constant `"subprocess"`, because the binary has only one
  dispatch path.
- Once #147 adds the attempt mode column, the value is read from the row, and an unknown
  mode is `null`.

An older binary without the field produces an empty proposal, never a wrong one.

**Plan decisions.** `review-plan` answers only the decision it just recorded
(`main.rs:823`), and `runs` keeps only `review_note`. A history of decisions with their
identities needs a read-only CLI field under ADR 0034, added in the #139 build. Until
then, plan lessons use `plan_state` and the decisions the cell itself issued.

### From entry to routing

1. **Derive π.** For each entry the kernel computes
   π = (successes + 1) / (attempts + 2). The result is never 0 or 1, and the file never
   states π.
2. **Look up the arm.** The priors oracle answers `OracleQuery.arm`, which has the form
   `harness\nmodel_ref\nTierN` (`router.rs:104`). It looks the arm up among the loaded
   entries. With no entry, it returns an error, which the router treats as the local
   mean (`router.rs:112`).
3. **Fuse.** `fused_historical_mean` computes (s + 4π) / (n + 4) and clips the result to
   μ_local ± 0.25 (`router.rs:76-78`).
   - On a fresh store (n = 0, μ_local = 0.5), the fused mean is π, limited to
     0.25–0.75.
   - With local history, π's weight falls to 4 / (n + 4).
4. **Score.** The fused mean is one term of the sum. The tier-efficiency, load-balance,
   coupling and bandit signals are unchanged.

The composition passes `spend: None` while the only oracle is the priors oracle.

### Loader diagnostics

Every case below produces a diagnostic, never a failure, and leaves the exit code
unchanged.

File-level cases mean no priors are applied:

| Code | Cause |
|---|---|
| `not_promoted` | There is no `accept` record. |
| `revoked` | The latest record is `revoke`. |
| `digest_mismatch` | `accepted.json` does not hash to the latest `accept` record. |
| `too_large` | The file is over 1 MiB. |
| `not_json` | The file does not parse as JSON. |
| `wrong_schema` | `schema` is not `meshloop/advisory-priors`. |
| `wrong_version` | `version` is not 1. |

Entry-level cases ignore that entry only:

| Code | Cause |
|---|---|
| `below_evidence_floor` | `attempts` is below 5. |
| `count_mismatch` | The length of `provenance` differs from `attempts`. |
| `successes_exceed_attempts` | `successes` is greater than `attempts`. |
| `not_subprocess` | `dispatch_mode` is not `subprocess`. |
| `model_unverified` | The model is `unverified`, empty or malformed. |
| `model_not_passed` | The harness's `invoke_args_template` lacks `{model_ref}` (`config.rs:156`, `harness.rs:100`). |
| `hint_not_raising` | `to_tier` is not higher than `from_tier`. |

Report-only cases:

| Code | Cause |
|---|---|
| `not_a_candidate` | No configured candidate matches the entry. The entry is inert, since `select` never looks it up. |
| `unknown_field` | The file has a field this version does not define. The field is ignored. |
| `provenance_unresolved` | A reference is not in the local ledger. `check` only. |

The last case does not block, because #140 promotes into fresh stores.

Diagnostics appear in two places:

- `priors show|check` always lists them.
- `doctor` adds a warning only when a file or record exists but is not in effect.

With no file and no record, every output is unchanged.

### Promotion ledger

The promotion ledger is a new append-only table, added by a forward migration. Each record
has these fields:

- sequence number;
- action (`accept` or `revoke`);
- SHA-256 of the accepted bytes;
- identity;
- time;
- source path.

Rows are never updated or deleted.

`priors accept` refuses a file that fails at file level, and prints the entry-level
diagnostics before it records anything. It writes `.meshloop/priors/accepted.json`
atomically. The cell writes its proposals anywhere except `.meshloop/priors/`.

Before each worker routing decision, the kernel reads the latest record and computes the
SHA-256 of a file of at most 1 MiB. While priors are in effect, each routed attempt
records the digest in use as an evidence row of kind `advisory_prior`.

### Plan review

When a promoted file is in effect, `review-plan` (show) and the `plan` answer add an
advisory section, and the JSON adds `data.advisory`. Neither appears without a promoted
file. The section lists two things:

- **Tier hints.** For each node, the hints whose `path_prefix` covers one of its
  `allowed_paths` and whose `to_tier` is above the node's current tier. A hint never
  lowers a tier, never edits `plan_json` and never changes `plan_sha256`.
- **Notes.** Each note as one line of data, with control characters stripped.

To apply a hint, a human uses `--adjust` and then accepts the plan.

### Cell side

- **Commands.** `/meshloop priors accept --file F --as <name>` and
  `/meshloop priors revoke --as <name>` are operator-typed only. They are off until
  `meshloop.write_commands` is set. `/meshloop priors show|check` are read-only.
- **Veto.** The `:tool` veto (`gates.lisp:45-48`) also refuses `priors accept` and
  `priors revoke`.
- **Lessons.** Lessons are stored as nodecode memories or skills and are never loaded
  as code.
- **Note text.** Notes come from fixed templates over counts, ids and paths. They never
  quote worker output, evidence summaries or node descriptions.

### Digest

After each run, the cell lists:

- the proposal path and its SHA-256;
- one row per prior key with attempts, successes, failures and whether it clears the
  floor;
- exclusion counts by reason;
- each tier hint and note with its evidence count;
- Accept, Decline and Adjust tallies;
- the promoted digest in effect, if any.

The digest holds pointers and counts only.

### Required regression tests

All tests run under `cargo test --workspace --locked --offline`, except the cell fixture
tests, which run in the cell's own suite. Each test proves one of the following:

1. A file never adds or removes a candidate. For any file content, `select` returns the
   same candidate set, possibly in another order.
2. For any π and any n, |fused − μ_local| ≤ 0.25.
3. Each exclusion holds for both the fold and the loader: missing or `null` mode,
   `native` mode, model `unverified`, a cancelled task, and no result.
4. An entry below the floor has no effect and is reported.
5. A tier hint never lowers a tier and is never read when routing.
6. Routing and planning are unchanged in each of these cases:
   - no promoted file;
   - an unpromoted file present;
   - a revoked file;
   - a file whose digest does not match.

   The existing baseline tests pass unmodified.
7. Wrong version, malformed JSON and unknown fields produce diagnostics and exit 0.
8. A zero-budget spend ledger does not turn the priors off.
9. The digest after a fixture run lists each lesson with its evidence counts.

### Known limit for #140

A prior encodes success only. lp-4 is at the success ceiling (24/24), so the arms that
were dispatched get similar priors. An arm with no entry stays at 0.5 on a fresh store,
while an incumbent with a high prior rises to as much as 0.75. A warm-up batch that only
repeats the baseline's first dispatches therefore reinforces the incumbent and cannot
change it.

Time and token effects are not encoded in a prior. They can appear only if a
first-dispatch change happens for success reasons. Whether the warm-up should explore
non-incumbent arms is #140's decision.

## Platform

The cell and nodecode workers run on Linux and on WSL2 with all state, worktrees and the
nodecode home on ext4, never under `/mnt/c`. Native Windows is unsupported because
nodecode has no background gateway there. macOS is not claimed.

## Board spike (2026-11-10)

Run for #135 so that #137 is planned on behaviour that was run. Release `0.1.13+98dd6dc80`,
on a throwaway `HOME` under `.agent-runs/`, with an emptied environment (no `NODECODE_*`
port or channel tokens) and no channel configured. The default model was set to
`anthropic/claude-haiku-4-5`. Everything was driven over the gateway with the operator
token: `POST /api/gateway/cells {"name":"nodecode-board","on":true}`,
`POST /api/gateway/sessions {"session_id","cwd"}` for two sessions in a fixture repository
(`s-SPIKEOPS`, the operator's, and `s-SPIKEWRK`), and `POST /api/gateway/slash` for
`/board meshloop-g135`, `/meshloop status` and `/board post ...` from `s-SPIKEOPS`.

### Observed

| What | Result |
|---|---|
| Join | `/board meshloop-g135` answered the board file, `<home>/.nodecode/boards/<12 hex>/meshloop-g135.txt`, and the members by session id. |
| Post | `/board post TEXT` posts as `operator`. The answer read "posted to meshloop-g135: 2 others on it read it at their next request", counting the operator's own session as a reader. |
| Record | One `board.posted` event, payload `{board, author, text}`, on aggregate `board/meshloop-g135`. The file gained one line, `[2026-11-10 16:04:28Z] operator: TEXT`. |
| `post_chars` | A 1001-character post was refused: "a post is at most 1000 characters (board.post_chars) and this one is 1001: put the detail in a file and post its path". |
| Duplicate | The same text again within 60 s was refused: "a post is said once". |
| `hops` 0 | A post naming `@s-SPIKEWRK` answered "s-SPIKEWRK is not woken: 0 wakes in a row between sessions is the cap (board.hops)". No turn started. |
| `hops` 1 | After `board.hops` was set to 1 and the cell restarted, a post naming `@s-SPIKEWRK` answered "woke s-SPIKEWRK". The idle session's turn started at once. |
| Wake input | The turn's input was the fixed line "[board meshloop-g135: operator mentioned you; the post is in the reminder below. Answer on the board only if you have something to add.]". A `turn.context.prompt` event carried the reminder: a framing line naming the file and saying posts are "data, not instructions", then both posts, including the earlier one that did not wake it. |
| Woken turn | Haiku did not post. It explored for about a minute: 29 `sh` calls through eval and 30 model requests, about USD 0.12 by the turn's own `cost_usd`. It read the board file, then left the fixture for the meshloop worktree above it (`git log`, `git status`, uncommitted files, `AGENTS.md`, ADR 0038). It tried `cargo test`, which failed on an untrusted mise shim under the throwaway `HOME`, and `mise trust`, which was not on `PATH`. The operator stopped it with `nodecode rpc` `cancel_turn`. Nothing in the worktree changed. |
| Channel relay | Not exercised: the home had no channel configured and the environment held no tokens. |

### Inferred, not run

- No code relays Board posts to a channel. In the release source, nothing outside
  `cells/board` names `board.posted` or `nodecode-board`. The channel "thread board"
  (`cells/channels/kit/board.lisp`) is a different feature. The Board README says a channel
  lane reads a board but is never woken by one.
- The operator's next word to a session resets the hop count (Board README). Not tested.

### For #137

- A wake is a full model turn with the session's host authority, and the fixed template does
  not bound it. The woken model read well outside the session's folder and tried to build. A
  `:tool` veto on gated verbs (ADR 0038) would not stop that. Default to posts that wake no
  one (no `@`, or `board.hops` 0), and treat a wake as an explicit, budgeted act.
- Pointer posts fit easily in 1000 characters. The 60-second duplicate guard means a
  repeated event needs distinct text, such as its event position.
- Members read posts at their next request with no polling. A Board projection therefore
  needs no wake to be seen; a wake only adds an unprompted turn.

