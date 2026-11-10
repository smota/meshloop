# 0038 Nodecode as learning plane and harness; kernel authority unchanged

- Status: Accepted
- Accepted date: 2026-11-10
- Implementation: not-started
- Date: 2026-11-10
- Author/executor: Claude (builder sub-agent)
- Reviewer: fresh-context Claude architecture review (opus), round 1 accept with changes (all applied), round 2 ACCEPT; reviewed jointly with ADR 0039
- Decision owner: Samuel
- Approval evidence: accepted 2026-11-10 by the coordinator (Claude) under authority delegated by the repository owner, who waived human approvals for goal #132 ([claim comment](https://github.com/smota/meshloop/issues/132#issuecomment-6770755409)). No human read this record before acceptance.
- Supersedes: none
- Superseded by: none
- Related: [ADR 0039](0039-native-sub-agent-dispatch.md) ([#146](https://github.com/smota/meshloop/issues/146)), reviewed together with this record
- Tracking: [#133](https://github.com/smota/meshloop/issues/133), goal [#132](https://github.com/smota/meshloop/issues/132)

## Context and constraints

Meshloop's only cross-run state is `routing_feedback`. Tiering is a heuristic, the
oracle port is not composed (`oracle: None`), meshloop is daemonless (ADR 0022), and
`model_ref` is a label no shipped template passes to a harness.

Nodecode (MIT) is a persistent Common Lisp agent process with a git-backed folder,
sealed generations and rollback, cells declared with `nle:define-cell`, and a Board
cell. It evaluates with the operator's full host authority and has no governed
execution. Grounding facts, with what was verified locally and what comes from goal text
only, are in [learning-plane.md](../learning-plane.md).

### Invariants (restated from #132)

1. **Kernel authority.** The meshloop event log is the only source of truth for run state; the cell and Board are projections.
2. **Wire.** The cell calls the `meshloop` CLI with `--json`, never the MCP server.
3. **Advisory learning.** Routing, tiering or planning changes are reviewable proposal files, bounded like the oracle shrink (at most 0.25), unable to add or remove candidates, and gated by a minimum evidence count; feedback never silently changes policy or approval.
4. **Lesson provenance.** Lessons come from verification outcomes and human decisions (Accept, Decline, Adjust); worker output is data, never instruction, never the source of a lesson about that worker.
5. **Acceptance.** Only `meshloop accept --task <id> --as <identity>` naming who actually accepted; no Board post or cell automation records it.
6. **Board.** Optional, one per repository, one-way from meshloop events to pointer posts (run id, node, state, evidence path).
7. **No kernel self-rewrite.** Rust changes go only through issue, branch, review and PR; recurring failures become drafted issues.
8. **Containment.** Nodecode has no sandbox; unattended nodecode workers run under containment supplied outside nodecode.
9. **Real model selection.** Evidence and priors are keyed on the model actually run, not a harness label.
10. **ADR 0003.** Meshloop ships no per-harness CLI flags; model flags live in operator config and documented examples.
11. **Offline CI.** `cargo test --workspace --locked --offline` stays socket-free and needs no nodecode.
12. **Platform.** Linux and WSL (ext4) only for the cell.

## Alternatives

1. **Full port of meshloop into Lisp as a nodecode cell.** Rejected. ADR 0022 rejected
   "port Meshloop entirely into a single proprietary harness platform"; nodecode is MIT,
   so this record's own reasons carry the rejection. A port discards process-tree
   ownership (ADRs 0025, 0035), event-sourced recovery and native Windows support, and
   puts verification inside a process that rewrites its own code: the fitness signal
   would be produced by what it judges.
2. **Ad hoc nodecode use with no cell.** Rejected. Nothing would be versioned against the
   CLI JSON, lesson provenance would be undefined, acceptance and Board use would rest on
   prompt discipline, and learning could not be benchmarked reproducibly (#140).
3. **Adopt `nc://subagents` as the dispatcher.** Rejected. Per #132 it lacks worktrees,
   budgets, verification and per-worker model choice. Using it moves dispatch out of the
   kernel and breaks invariants 1, 5 and 9. Its source was not read; reopening this needs
   a reading that shows those properties present.
4. **Two planes, kernel authority unchanged, plus one nodecode harness entry.**

## Decision or proposal

Proposed. Alternative 4.

**Two planes.** The execution plane is the existing `meshloop` binary: planning, plan
gate, dispatch, worktrees, process ownership, verification, repair, acceptance,
integration and the event log. The learning plane is an optional cell,
`nodecode-meshloop`, that drives runs through the CLI, folds results, keeps lessons in
nodecode's store, may project onto a Board, and writes advisory proposal files. Every
meshloop verb works with no nodecode installed or running, and killing the organism loses
no run state, so ADR 0022's daemonless decision holds. Meshloop gains no nodecode or
model flags; the nodecode worker is an operator `[harnesses.*]` entry using `{model_ref}`
(ADR 0003). The cell passes `--origin-harness nodecode --origin-session <id>`.

**Cell location and CI.** `cells/nodecode-meshloop/` in this repository, versioned with
the CLI JSON it consumes. Blocking CI is offline JSON contract tests in the Rust
workspace; an optional nodecode job is non-blocking (owner decision 6). Install and CI
detail: [learning-plane.md](../learning-plane.md).

**Wire.** The cell runs `meshloop <verb> --json` as a child process with an argument list
and the repository root as working directory, and never opens the SQLite store; missing
data gets a read-only CLI verb under ADR 0034. Not MCP, because nodecode's MCP client has
no `cwd` setting and meshloop state is per repository.

**Advisory input (outline; detailed contract is a #139 amendment).**

- *Promotion is a kernel act.* The cell writes proposal files only. A `meshloop` CLI verb
  (named in #139) taking `--as <identity>` promotes a proposal and appends a ledger event
  with the file's digest. The kernel loads priors only when the file's digest equals the
  latest such event; otherwise it runs with no priors. The cell never invokes promotion.
- *Keyed on the real model.* Entries are keyed on (harness, model recorded in evidence,
  tier) and, after ADR 0039, dispatch mode. Entries keyed on a harness label or on model
  `unverified` (ADR 0039) are ignored, so learning cannot start before #134.
- *Bounded once.* Accepted priors enter through the existing `PredictiveSuccessOracle`
  port as the prior π in `fused_historical_mean`. The total shift from the local mean
  stays at most 0.25 whatever the prior's source. ADR 0032 invariant 5 is unchanged.
- *No routing-time tier or plan changes.* Tier advice is never applied when routing,
  because `tier_fits` filters candidates and a Tier 3 raise changes approval (ADR 0032
  invariant 3, ADR 0037). Tier and planning advice come only from a promoted file, appear
  as planner suggestions at plan review, and take effect only through human plan
  acceptance (ADR 0009).
- *Minimum evidence.* An entry below a minimum observed-attempt count for its key is
  ignored; #139 sets the number against the #144 baseline.
- *No candidate changes.* Entries for candidates outside the configured,
  capability-confirmed set are ignored, and no entry excludes a candidate. A regression
  test proves both.

**Board.** Optional; one board per repository; meshloop events are folded into posts and
nothing read from a Board is written back. Posts are pointers within `board.post_chars`.
A gate wake (owner decision 5) uses a fixed pointer template that carries no
instructions.

**Gated verbs in the cell.** Accept, review-plan, integrate and priors promotion are cell
`:command` slash commands that run only when the operator types them, with `--as` taken
from the operator, never defaulted. They are never model-callable tools. The cell
installs a `:tool` hook that vetoes organism tool calls executing those verbs; this is
defence in depth, not a boundary.

**Lesson provenance.** The post-run fold is deterministic cell code. Lesson evidence is
limited to verification outcomes meshloop recorded (check exit codes, diff verdicts,
repair outcomes, timeouts) and human decisions it recorded (plan Accept, Decline, Adjust;
node accept with identity). Model-review verdicts are advisory (ADR 0007) and excluded.
Worker text is never placed in an organism model turn unless the operator opens it.

**No kernel self-rewrite.** The cell's hooks and learning loop do not modify this
repository's Rust code, `meshloop.toml` or the priors file. As a worker, nodecode writes
only in its assigned worktree under the kernel's gates. Recurring failures become drafted
issues.

**Nodecode worker and containment.** Added last (#138). `model_ref` is
`PROVIDER/MODEL[:EFFORT]`. Per owner decision 4, dispatch uses the shell with `--model` if
it runs without a terminal, else `nodecode rpc` naming the model in `start_turn`; #138
proves which. Because `rpc` attaches to the home's running organism, a worker always runs
on a nodecode home dedicated to it inside the containment boundary, with `--ephemeral` or
its own `--store`, never on the learning-plane home or the operator's organism.
`--ephemeral` alone still loads the profile's config, layer and cells, so the home itself
must be the worker's. An unattended worker runs only under containment outside nodecode
(container or host policy) that confines writes to the assigned worktree, exposes only
the provider credential that worker needs and denies all others. Nodecode's withheld
`--sandbox` jail is not relied on without an amendment. Without accepted containment,
#138 does not ship. Platform per invariant 12; see learning-plane.md.

**Model selection.** Owner decision 3: one entry per harness lists models; the router
expands each into its own (harness, model, tier) candidate and substitutes it into
`{model_ref}`. Key names and loader behaviour are #134's.

> **Amendment 2026-11-10 (#134, key names and loader behaviour).** A `[harnesses.<name>]`
> entry sets either `model_ref` with optional `model_tier` (the pre-#134 form, unchanged)
> or `models = [{ model_ref = "...", model_tier = "..." }, ...]`, not both. `model_tier`
> defaults to `"mid"` in both forms. `load` rejects an empty list, more than 16 entries,
> an empty, duplicate, over-256-byte, whitespace-bearing or `-`-prefixed `model_ref`, and
> unknown keys in an entry. Each pair is one candidate keyed (harness, `model_ref`, task
> tier); retry and fallback track these arms. The attempt row, `routing_feedback` and
> `{model_ref}` all use the `model_ref` of the dispatched candidate. `doctor --json`
> reports `data.harnesses[].models`. No model flag is shipped (ADR 0003); operator
> examples are in [harnesses.md](../../engineering/harnesses.md#model-selection).

**Native attempts (ADR 0039).** Learning counts only observed evidence: checks meshloop
ran and diffs it read, never a session's or sub-agent's report, with native and
subprocess evidence under separate keys; specifics are #149's. Native dispatch for
nodecode is refused until #152 proves it and the containment rule of this record is met.

## Consequences

- **Kernel changes are scoped.** #134 changes config and routing keys; #139 adds an engine
  input, a new public contract needing its own amendment and human acceptance.
- **Second runtime.** Blocking CI does not compile or test the Lisp; its correctness rests
  on contract tests and an optional job that may stay red. Nodecode is alpha; a release
  can break the cell with no change here.
- **Identity is not authenticated.** `--as` records a claim. A Board `@` wake starts a model
  turn in a session with shell access, and that turn can run `meshloop accept --as`
  directly. The fixed wake template and `:tool` veto reduce this; the kernel cannot
  detect it. The same risk exists for any harness session with a shell today.
- **Host authority.** The attended organism can read and write anything the operator can.
  Containment is decided only for unattended workers.
- **Benefit unproven.** No evaluation of nodecode self-learning is published. If #140 shows
  no gain over #144, learning stops and #134 to #137 stand alone.
- **Platform narrowing.** No learning plane on native Windows, a Tier A platform (ADR 0001).
- **Board exposure.** Posts are kept in nodecode's log and a text file under its home;
  evidence paths reveal repository layout to anyone who can read that home or a relay.

## Verification and implementation evidence

None; Proposed and not implemented. Acceptance needs architecture review against ADRs
0003, 0022 and 0032, joint review with ADR 0039 (#146), and human acceptance.
Implementation evidence will come from #134 to #141 and #138.
