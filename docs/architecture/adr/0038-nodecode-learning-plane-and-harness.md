# 0038 Nodecode as learning plane and harness; kernel unchanged

- Status: Proposed
- Implementation: not-started
- Date: 2026-11-10
- Author/executor: Claude (Opus 5.5), builder worker dispatched by the #133 coordinator
- Reviewer: pending architecture review against ADRs 0003, 0022 and 0032, then human acceptance
- Decision owner: Samuel
- Approval evidence: none. Owner decisions of 2026-11-10 recorded in [#132](https://github.com/smota/meshloop/issues/132) ("Decisions") are inputs, not acceptance of this record.
- Supersedes: none
- Superseded by: none
- Related: [ADR 0039 (#146, proposed)](https://github.com/smota/meshloop/issues/146), reviewed together with this record
- Tracking: [#133](https://github.com/smota/meshloop/issues/133), goal [#132](https://github.com/smota/meshloop/issues/132)

## Context and constraints

Meshloop learns little across runs. The only cross-run state is `routing_feedback`
(success and failure counts per harness, `model_ref` and tier). Tier assignment is a
heuristic (ADR 0009 residual). The System 1 oracle exists in `router.rs` but is not
composed (`oracle: None`). Meshloop is daemonless (ADR 0022), so nothing holds lessons
between invocations. `model_ref` is a label: `{model_ref}` is substituted into
`invoke_args_template`, but no shipped template passes it to a harness, so each harness
runs its default model.

Nodecode is a persistent Common Lisp agent process (MIT). It keeps what it defines in a
git-backed folder with numbered, sealed generations and `rollback N`, has cells declared
with `nle:define-cell`, and ships a Board cell. Its eval, shell and file verbs run with
the operator's full host authority. It has no governed execution.

Goal #132 requirements 1-12 bind this record and every child. They are restated, not
weakened, under "Invariants" below.

Facts used here and where they come from (nodecode `0.1.13+98dd6dc80`, read, not run):

| Fact | Source | Verified locally |
|---|---|---|
| Cells are declared with `nle:define-cell`; folder name = system name = package | release `src/cells/template/cell.lisp` | yes, source |
| `(new-cell NAME :source DIR)` makes a cell whose folder lives in `DIR` and links it into the home's cells folder; `DIR` must be absent or empty | release `src/src/organism/made-cells.lisp:195` | yes, source |
| A cell linked from an outside repository loads from the home's cells folder | `~/.nodecode/cells/sam-openrig` symlink into another repo | yes, filesystem |
| A hub entry may pin a sub-path of a repository (`"path"`) | `~/.nodecode/hub/*.json`, entry `nodecode-delegation` | yes, file |
| The MCP client cell supports no `cwd` for a server | release `src/cells/mcp/README.md:64`, `config.lisp:19` | yes, source |
| `--model [PROVIDER/]MODEL[:EFFORT]` opens a shell session on that model; the shell runs on a terminal | `nodecode help`, `src/src/cli/entry.lisp` | yes, help text |
| `nodecode rpc` drives the organism by JSON lines; `start_turn` may name provider, model and effort for a new session | `nodecode help rpc` | yes, help text |
| Board posts are facts in nodecode's event log, at most `board.post_chars` (default 1000) characters, meant as pointers; `@ID` wakes an idle session, capped by `board.hops` | release `src/cells/board/README.md` | yes, source |
| `--sandbox` (srt jail) exists in source but is withheld: help omits it and the parser refuses it in this release | `src/src/cli/entry.lisp` (`*offer-sandbox*` nil) | yes, source |
| No background gateway arm on Windows | `src/src/gateway/background.lisp:29,49` | yes, source |
| `nc://subagents` is a community hub entry (`drvova/nodecode-subagents`, depends on `nodecode-board`) | `~/.nodecode/hub/3c16bad32327b07f.json` | entry only; source not read here |
| `nc://subagents` v0.2.0 runs flat task lists in shared directories, no worktrees, budgets, verification or per-worker model | #132 benchmark table | no, goal text only |
| Nodecode self-learning improves outcomes | none published | no; hypothesis until #140 |

### Invariants (restated from #132)

1. **Kernel authority.** The meshloop event log is the only source of truth for run state.
   The cell and the Board are projections, rebuildable from it.
2. **Wire.** The cell calls the `meshloop` CLI with `--json`. It does not use the MCP server.
3. **Advisory learning.** Anything that changes routing, tiering or planning is a reviewable
   proposal file, bounded in effect like the oracle shrink (at most 0.25), cannot add or
   remove routing candidates, and needs a minimum evidence count. Feedback may recommend
   but never silently change policy or approval.
4. **Lesson provenance.** Lessons derive from verification outcomes and human decisions
   (Accept, Decline, Adjust). Worker output is data, never instruction, and never the
   source of a lesson about that worker.
5. **Acceptance.** Acceptance stays `meshloop accept --task <id> --as <identity>` naming who
   actually accepted. No Board post and no cell automation records human acceptance.
6. **Board.** Optional, one board per repository, one-way from meshloop events to posts.
   Posts are pointers (run id, node, state, evidence path), not dumps.
7. **No kernel self-rewrite.** Rust changes go only through issue, branch, review and PR.
   Recurring failure patterns become drafted issues.
8. **Containment.** Nodecode has no sandbox and evaluates with full host authority.
   Unattended nodecode workers run under containment supplied outside nodecode.
9. **Real model selection.** Routing evidence and learned priors are keyed on the model
   actually run, not on a harness label.
10. **ADR 0003.** Meshloop ships no per-harness CLI flags. Model flags live in operator
    configuration and documented examples.
11. **Offline CI.** `cargo test --workspace --locked --offline` stays socket-free and does not
    require nodecode.
12. **Platform.** Linux and WSL (ext4) only for the cell.

## Alternatives

1. **Full port of meshloop into Lisp as a nodecode cell.** Rejected. ADR 0022 already
   rejected porting into a single harness platform. A port would discard the Rust kernel's
   tested isolation, process-tree ownership (ADRs 0025, 0035), event-sourced recovery and
   Windows support, and would put verification and acceptance inside a process that can
   rewrite its own code with full host authority. The fitness signal would then be
   produced by the thing it judges.
2. **Ad hoc nodecode use with no cell.** Operators drive meshloop from a nodecode session
   by prompting it to run shell commands. Rejected. Nothing would be checked in or
   versioned against the CLI's JSON, lessons would have no defined provenance, and
   acceptance, Board use and advisory output would depend on prompt discipline instead of
   a reviewed contract. Learning results could not be benchmarked reproducibly (#140).
3. **Adopt `nc://subagents` as the dispatcher.** Rejected. Per #132 it has no worktrees, no
   budgets, no verification and no per-worker model choice, and it coordinates through
   Board messages. Using it would move dispatch out of the kernel and break invariants 1,
   5 and 9. Its source was not read for this record; the rejection rests on the missing
   kernel properties, which a reading would have to show present to reopen it.
4. **This decision: two planes, kernel unchanged, plus one nodecode harness entry.**

## Decision or proposal

Proposed. Alternative 4.

### Two planes

- **Execution plane:** the existing `meshloop` binary. It keeps planning, the plan gate,
  dispatch, worktrees, process ownership, verification, repair, acceptance, integration
  and the event log. Its authority does not change.
- **Learning plane:** an optional nodecode cell, `nodecode-meshloop`. It drives runs through
  the CLI, folds run results after each run, keeps lessons in nodecode's own store, may
  project run state onto a Board, and writes advisory proposal files.

Consistency with ADR 0022: meshloop still requires no daemon. Every meshloop verb works
with no nodecode installed or running. The nodecode organism is long-lived, but it is an
optional client, like any harness session using the meshloop skills, and holds no
authoritative run state. Killing it mid-run loses no run state; `meshloop status` and
`meshloop resume` work without it. This is not the rejected "port into one harness
platform": no kernel code moves, and nodecode is one client and one worker kind among
several.

Consistency with ADR 0003: meshloop gains no `--nodecode-*` or model flags. The nodecode
worker is an operator `[harnesses.*]` entry whose `invoke_args_template` carries the model
through `{model_ref}`, shown in documented examples. The cell passes
`--origin-harness nodecode --origin-session <session-id>` for provenance, which already
exists for every origin.

### Cell location and CI

The cell lives at `cells/nodecode-meshloop/` in this repository: one `nle:define-cell` with
commands, help and hooks, in a folder whose name is its system and package name. Reasons:
it is versioned with the CLI JSON it consumes, contract changes land in one PR, and the
hub can later pin a commit and sub-path of this repository (#141). A development install
links the home's cells folder to this folder, as `new-cell :source` does. `new-cell :source`
only accepts an absent or empty directory, so linking the checked-in folder is a
documented link step, decided in #135.

CI (owner decision 6):

- **Blocking, offline:** JSON contract tests in the Rust workspace. They run the CLI
  verbs the cell consumes against fixtures and validate the output against checked-in
  JSON Schemas or golden files that the cell's own tests read as input. A CLI change
  that breaks the cell's input fails `cargo test` with no nodecode and no socket.
- **Optional, non-blocking:** a separate Linux CI job that installs a pinned nodecode
  release, loads the cell on a throwaway home (`--ephemeral` with a temporary `HOME`) and
  runs the cell's tests. It is `continue-on-error`, is never called from `xtask check` or
  `cargo test`, and its failure does not gate merge. The release needs glibc 2.38 or newer.

### Wire

The cell runs `meshloop <verb> ... --json` as a child process with an argument list, no
shell interpolation, and the working directory set to the target repository root. It
reads stdout JSON and the exit code. It does not use `meshloop mcp`: nodecode's MCP client
has no `cwd` setting, so a configured server starts in the gateway's working directory,
while meshloop state (`.meshloop/`, the ledger, worktrees) is per repository. ADR 0027's MCP
server re-invokes the CLI with `--json` anyway, and ADR 0039 (#146, proposed) also makes
CLI commands the contract with MCP as a thin wrapper. If the cell needs data no verb
returns, a read-only CLI verb is added under ADR 0034's observation rules. The cell never
opens the meshloop SQLite store directly.

### Advisory input contract (outline)

Detailed contract, schema and engine port are an amendment to this record in #139. The
outline binds that amendment:

- **File-based.** The cell writes proposals to a proposal file. The kernel never reads a
  proposal. It reads only an accepted priors file that a human promoted by an explicit
  command naming their identity, which records the proposal's digest. A missing,
  unreadable or digest-mismatched file means no priors; the run proceeds as today.
- **Keyed on the real model run.** Each entry is keyed on (harness, model as recorded in
  evidence, tier) and, after ADR 0039, dispatch mode. Entries keyed on a harness label
  are ignored, so learning cannot start before #134 records real models.
- **Bounded.** A routing prior moves a candidate's historical mean by at most 0.25, the
  same clip as `fused_historical_mean` in `router.rs`. The exploration bonus is unchanged.
  Tier advice may only raise a tier, matching ADR 0032's raise-only ratchet. Planning
  advice is context for the planner prompt only; the produced graph still needs human
  plan acceptance (ADR 0009).
- **Minimum evidence.** An entry below a minimum count of observed attempts for its key is
  ignored. The number is set in #139 against the #144 baseline.
- **No candidate changes.** Priors only reorder configured, capability-confirmed
  candidates. An entry for a candidate not in that set is ignored. No entry can exclude a
  candidate. A regression test proves both (#132 acceptance criteria).

### Board rule

The Board is optional. With none configured the cell works and posts nothing. One board
per repository. Flow is one-way: meshloop events are folded into posts; nothing read from
a Board is written back to meshloop. Posts are pointers (run id, node, state, evidence
path) within `board.post_chars`, not dumps. A post may `@`-wake the supervising session at
a gate (owner decision 5). A wake is a notification. Acceptance never flows through the
Board: no post, reply or wake calls `meshloop accept`, `review-plan` or `integrate`.

### Acceptance from the cell

The cell may expose accept, plan review and integrate as operator commands (#136). They
run only when the operator invokes them, and `--as` is the identity the operator supplies,
never a default. The cell never calls them from a hook, a Board wake, a timer, or a model
tool call it initiates.

### Lesson provenance

A lesson's evidence is limited to (a) verification outcomes meshloop recorded (check exit
codes, diff verdicts, repair outcomes, timeouts) and (b) human decisions meshloop
recorded (plan Accept, Decline, Adjust; node accept with its identity). Worker stdout,
summaries and self-reports are data. They may be stored for an operator to read, but no
lesson, proposal or prior is derived from what a worker says about its own work.

### No kernel self-rewrite

The cell and the organism do not modify this repository's Rust code, `meshloop.toml`
policy or the accepted priors file. A recurring failure pattern is written up as a
drafted issue for a human to file or discard. Nodecode's own layer, generations and
rollback apply only to nodecode's state.

### Nodecode as worker harness and containment

Nodecode joins as one more worker harness (#138, last in order). Its `model_ref` is
`PROVIDER/MODEL[:EFFORT]`, the form `--model` documents. Owner decision 4: dispatch is a
shell invocation with `--model` if it runs without a terminal; otherwise `nodecode rpc`,
naming the model in `start_turn`. Help says the shell runs on a terminal, so `rpc` is the
likely path; #138 proves which.

Nodecode evaluates with full host authority, and this release withholds its `--sandbox`
jail. An unattended nodecode worker runs only under containment supplied outside
nodecode: a container or a host policy that confines writes to the assigned worktree and
denies credential stores and other agents' homes. Nodecode's own jail, if it ships, is
not relied on without an amendment. Without accepted containment, #138 does not ship
(#132 stop condition). The cell itself runs in the operator's attended session with the
operator's authority; this record claims no containment for it.

Platform: the cell and nodecode workers are supported on Linux and WSL2 with all state,
worktrees and the nodecode home on ext4, never `/mnt/c`. Native Windows is not supported
(no background gateway arm). macOS is not claimed.

### Model selection

Owner decision 3: one config entry per harness carries a list of models; the router
expands each into its own (harness, model, tier) candidate and substitutes the model into
`{model_ref}`. Feedback and evidence are keyed per expanded model. Key names and loader
behaviour are decided in #134.

### Native attempts (ADR 0039)

Learning counts only observed evidence: checks meshloop ran and diffs it read. For native
attempts that excludes the session's or sub-agent's report. Native and subprocess
evidence stay separate keys. How native evidence is recorded and verified is decided in
#149, which lands before #139. Nodecode is not a native-dispatch origin under this record.

## Consequences

- **Kernel untouched until #139.** Children #134 to #138 change config, CLI read verbs,
  docs and the cell. The advisory input in #139 is a new public engine contract and needs
  its own amendment and human acceptance.
- **Second runtime to maintain.** Lisp code in this repository is not compiled or tested
  by the blocking CI. Its correctness rests on the JSON contract tests and an optional
  job that may stay red. Nodecode is alpha with no stable update path; a release can break
  the cell without any change here.
- **Identity is not authenticated.** `--as` records a claimed identity. An organism with
  host authority, or any harness session with a shell, can run `meshloop accept --as`
  itself. This record forbids the cell from doing so; the kernel cannot detect it. That
  risk exists today and is not reduced here.
- **Host authority.** The attended organism can read and write anything the operator can.
  Containment is decided only for unattended workers.
- **Benefit unproven.** Nodecode publishes no evaluation of its self-learning. If #140 shows
  no gain over the #144 baseline, the learning loop stops and #134 to #137 stand alone.
- **Lost by not porting.** Nodecode's live redefinition and generations do not reach the
  kernel. Kernel improvements stay at PR speed.
- **Platform narrowing.** The learning plane is unavailable on native Windows, a Tier A
  platform for the kernel (ADR 0001).
- **Board exposure.** Posts are stored in nodecode's log and rendered to a text file under
  the nodecode home. Evidence paths in posts can reveal repository layout to anyone who
  can read that home or any relay attached to the board.

## Verification and implementation evidence

None. This record is Proposed and nothing is implemented. Acceptance needs: architecture
review for consistency with ADRs 0003, 0022 and 0032; joint review with ADR 0039 (#146);
human acceptance. Implementation evidence will come from #134 to #141 and #138.
