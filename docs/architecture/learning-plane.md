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

This section holds the mechanism for the ADR 0038 amendment of 2026-11-10 (#139). That
amendment is **Proposed — pending review**, and nothing here is built yet. It covers
routing priors only. Tier hints, planning notes, repair constraints and plan-decision
lessons are deferred until a GO at #140. Plan-decision lessons first need an append-only
`plan_decisions` table, because today `review-plan` answers only the decision it just
recorded (`main.rs:823`) and `runs` keeps only `review_note` (`schema.rs:32-41`). The file
format is [`schemas/advisory-priors.v1.schema.json`](../../schemas/advisory-priors.v1.schema.json).
Code references are at base `ac87e7a`.

### New read-only attempt fields

The #139 build adds three fields to the attempt JSON (`report.rs:154`, `attempt_json`).
Each field is listed as required for `inspect_attempts` in `contract.json`.

| Field | Values | Source |
|---|---|---|
| `dispatch_mode` | `"subprocess"` | A domain `DispatchMode` enum, read with an exhaustive `match`. Today it has the single variant `Subprocess`, because the binary has one dispatch path. When #147 adds `Native`, the match fails to compile until the value is read from the attempt row's mode column. #147 must not merge without that change. |
| `check_result` | `"passed"`, `"failed"` or `null` | The attempt's final deterministic-result event. `DeterministicChecksPassed` or `VerifyGatePassed` gives `passed`; `DeterministicChecksFailed` gives `failed`. Anything else gives `null`, including a crash, a timeout, a cancellation, or a result whose append was refused (#167). |
| `model_passed` | `true`, `false` or `null` | A new nullable `attempts.model_passed` column, written when the attempt row is created for every attempt. It is true when the harness's `invoke_args_template` substitutes `{model_ref}` (`config.rs:156`, `harness.rs:100`). Rows written before #139 hold `null`. Write-once: attempt upserts keep it with `COALESCE(attempts.model_passed, excluded.model_passed)`. |

A column on `attempts` is the least invasive place that is written for every attempt. The
other new tables are written only while priors are in effect.

An older binary refuses a schema-v9 store outright; against an older store it returns
attempt JSON without these fields. The fold treats either as an
error and fails closed: it writes no proposal and says why.

### What counts as evidence

The fold counts an attempt only when every one of these holds:

- `dispatch_mode` is `"subprocess"`;
- `model_passed` is `true`;
- `model_ref` is non-empty and not `unverified`;
- the task is not `Cancelled`;
- `check_result` is `passed` or `failed`.

A failed check counts as a failure. A crash or timeout is excluded and never counts as a
failure. Every excluded attempt is reported with one of these reasons:

- `not_subprocess`
- `model_not_passed`
- `model_unverified`
- `cancelled`
- `no_check_result`

The fold reads only attempt fields (`harness`, `model_ref`, `dispatch_mode`,
`model_passed`, `check_result`), the node tier and the task state. It never reads worker
output, model-review verdicts (ADR 0007), evidence summaries, node descriptions or Board
posts.

### From entry to routing

1. **Snapshot.** The engine port is `AdvisoryPriorsSource::snapshot() ->
   Result<Option<PriorsSnapshot>, StoreError>`. The router takes one snapshot for each
   `select` call (`router.rs:204`) from a single read of the latest ledger row, and may
   cache it by row sequence number. `None` means `oracle: None`, as today
   (`run_loop.rs:1554`).
   The snapshot drops every entry listed in the ledger row's ignored JSON (decided at
   `accept`), plus every entry the loader ignores; routing never re-resolves provenance.
2. **Derive π.** For each surviving entry, the kernel computes
   π = (successes + 1) / (attempts + 2). The result is never 0 or 1.
3. **Look up the arm.** The priors oracle implements `PredictiveSuccessOracle`
   (`ports.rs:582`). It looks up `OracleQuery.arm`, which has the form
   `harness\nmodel_ref\nTierN` (`router.rs:104`). It returns an error, and the router
   then keeps the local mean (`router.rs:112`), in three cases:
   - no entry matches;
   - the entry is ignored;
   - the candidate's harness dispatch is not `subprocess`.

   The third case cannot occur until #147 lands, so its regression test is added with
   #147.
4. **Fuse.** `fused_historical_mean` computes (s + 4π) / (n + 4) and clips the result to
   μ_local ± 0.25 (`router.rs:76-78`).
   - On a fresh store (n = 0, μ_local = 0.5), the fused mean is π limited to 0.25–0.75.
   - With local history, π's weight falls to 4 / (n + 4).
5. **No charge.** The composition passes `spend: None`. The store's ledger has a budget of
   0 and a cost of 1 (`store.rs:39,68`), so any charge would fail
   (`ports.rs:374-386`, `router.rs:99-102`) and silently turn the priors off.
6. **Planner dispatch is unchanged.** Planner-dispatch routing (`run_loop.rs:661`) keeps
   `oracle: None`.
7. **Candidate set.** `select` filters before it scores (`router.rs:214-226`), so a prior
   only reorders candidates that survived the filters.

### Loader diagnostics

Every case below produces a diagnostic, never a failure, and leaves the exit code
unchanged.

**File level: no priors are applied.**

| Code | Cause |
|---|---|
| `not_promoted` | There is no `accept` row. |
| `revoked` | The latest row is `revoke`. |
| `digest_mismatch` | The stored BLOB does not hash to the row's digest, which means the store is corrupted. |
| `too_large` | The file is over 1 MiB. |
| `not_json` | The file is not valid JSON. |
| `wrong_schema` | `schema` is not `meshloop/advisory-priors`. |
| `wrong_version` | `version` is not 1. |

**Entry level: the entry is ignored.**

| Code | Cause |
|---|---|
| `below_evidence_floor` | `attempts` is below 5. |
| `duplicate_key` | The key appears more than once. Every copy is ignored. |
| `duplicate_provenance` | Two references name the same (`graph_id`, `task_id`, `attempt_id`). |
| `count_mismatch` | The length of `provenance` differs from `attempts`. |
| `successes_mismatch` | `successes` differs from the number of references with `passed`. |
| `not_subprocess` | `dispatch_mode` is not `subprocess`. |
| `model_invalid` | The model fails #134's predicate (`config.rs:219-238`, moved to `meshloop-domain`): empty, over 256 bytes, a leading `-`, whitespace or control characters. The value `unverified` also fails. |
| `unknown_entry_field` | The entry or one of its references has a field v1 does not define. |
| `provenance_contradicts` | Raised by `accept` and `check` only. A reference resolved in the local ledger disagrees with the entry's key, mode, `model_passed` or `passed`. |

**Report only.**

| Code | Cause |
|---|---|
| `not_a_candidate` | No configured candidate matches the entry, so the entry is inert. |
| `unknown_field` | A top-level field is not defined in v1, such as `tier_hints` or `notes`. |
| `provenance_unresolved` | A reference is not in the local ledger. |

`provenance_unresolved` does not block, because #140 promotes into fresh stores.

Schema length limits count code points, while the kernel counts UTF-8 bytes. Before any
string from the file or the ledger is displayed (harness, model, identity, source path),
control characters and Unicode format (Cf) characters are stripped, bidi controls
included.

Diagnostics appear in two places:

- `priors show|check` always lists them.
- `doctor` adds a warning only when a ledger row exists but no priors are in effect.

### Promotion ledger

Store schema version 8 (`schema.rs:566`) is bumped by a forward migration. The version
number is coordinated with #147, which also bumps it. The migration adds three things.

**`advisory_promotions`.** Each row holds:

- sequence number;
- action (`accept` or `revoke`);
- `priors_sha256`;
- `bytes` BLOB (accept only, at most 1 MiB);
- identity;
- time;
- source path;
- counts of resolved and unresolved references;
- the ignored entries as JSON (keys and codes).

`BEFORE UPDATE` and `BEFORE DELETE` triggers `RAISE(ABORT)`. The `events` table is not
used, because its rows are graph and task transitions (`schema.rs:19-31`).

**`advisory_routing`.** Columns: `attempt_id PRIMARY KEY REFERENCES attempts`,
`priors_sha256 NOT NULL` and `changed_top BOOLEAN NOT NULL`. A row is written in the same
transaction as the attempt row, and only while priors are in effect. `changed_top` is true
when the candidate `dispatch_ready` dispatches differs from the one it would dispatch with
the oracle off (already-spent candidates excluded in both cases). This
lets #140 check that an effect is "traceable". It is never an Evidence row.

**`attempts.model_passed`.** The nullable column described above.

**`priors accept --file F --as <identity>`** runs these steps:

1. Read F once, at most 1 MiB + 1 bytes. The digest, the parse and the stored BLOB all
   come from that one buffer.
2. Refuse a file-level fault.
3. Resolve the references against the local ledger and ignore any entry that has
   `provenance_contradicts`.
4. Print every diagnostic.
5. Append the row in one transaction.

**`priors revoke --as <identity>`** appends a revoke row.

**`--as`** is required and never defaulted. It is rejected when it is empty, when it
starts with `-`, when it contains control characters, or when it is over 128 bytes.

### Cell side

- **Commands.** `/meshloop priors accept --file F --as <name>` and
  `/meshloop priors revoke --as <name>` are operator-typed slash commands only. They are
  off until `meshloop.write_commands` is set. `/meshloop priors show|check` are
  read-only and not gated.
- **Veto.** The `:tool` veto (`gates.lisp:45-48`) gains the two-word pairs
  `priors accept` and `priors revoke`. It matches them both as tokens and as shell lines.
  `priors show` and `priors check` stay allowed. Tests cover both cases.
- **Proposals.** The cell writes proposals to any path. The kernel keeps no file copy.
- **Lessons.** Lessons are stored as nodecode memories or skills and are never loaded as
  code.

### Digest

After each run, the cell lists:

- the proposal path and its SHA-256;
- one row per prior key: attempts, successes, failures and whether it clears the floor;
- exclusion counts by reason;
- the promoted digest in effect, if any;
- `changed_top` counts from `advisory_routing`.

The digest holds pointers and counts only.

### Required regression tests

All tests run under `cargo test --workspace --locked --offline`, except the cell fixture
tests, which run in the cell's own suite.

**Routing invariants**

1. A file never adds or removes a candidate. For any content, `select` returns the same
   set.
2. For any π and n, |fused − μ_local| ≤ 0.25.
3. Without a promoted row, routing order is byte-identical, and so are the plan,
   review-plan and run outputs. This holds with no row, with a revoked row, and with a
   proposal file present but not promoted. The routing and planning tests pass unmodified
   (store schema-version asserts change with the v9 bump).
4. A zero-budget spend ledger does not turn the priors off.

**Evidence and exclusions**

5. Each exclusion holds in both the fold and the loader:
   - mode missing, `null` or not `subprocess`;
   - `model_passed` false or missing;
   - model `unverified`;
   - a cancelled task;
   - a crash or timeout (`no_check_result`).
6. A native-routed candidate gets no prior. This test is added with #147.
7. The attempt JSON reports `check_result` correctly for each terminal path.

**File and entry validation**

8. An entry below the floor has no effect and is reported.
9. Duplicate keys cause every copy to be ignored.
10. Duplicate provenance causes the entry to be ignored.
11. A `successes` mismatch, a count mismatch, or an unknown field in an entry causes the
    entry to be ignored.
12. Wrong version, malformed JSON and unknown top-level fields produce a diagnostic and
    exit 0.

**Promotion and ledger**

13. `accept` reads F once. Changing F after `accept` has no effect.
14. Routing uses one snapshot per `select` call.
15. `UPDATE` and `DELETE` on `advisory_promotions` are refused.
16. `provenance_contradicts` ignores the entry and is recorded in the ledger row, and the entry is never applied when routing.
17. `--as` validation rejects each bad form.
18. An `advisory_routing` row is written only while priors are in effect, and
    `changed_top` is correct.

**Cell**

19. The veto refuses `priors accept` and `priors revoke` in both readings, and allows
    `priors show` and `priors check`.
20. The digest after a fixture run lists each prior key with its evidence counts.

### Decision for #140 and its known limit

A prior encodes success only. Decision (c): #140's warm-up batch explores every candidate
by pinning each one in turn. The warm-up is never one of the measured runs.

If every arm reaches the success ceiling, as lp-4 does (24/24), every π is about the
same. Every fused mean then clips to the same value, and the order matches the baseline.
Time and token effects cannot flow through a prior. In that case a NO-GO is the expected
result, and it should be reported as measured.

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

