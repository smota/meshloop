# Runtime design reference

This holds the concrete mechanism ("how") behind the five proposed runtime ADRs. ADRs
record the decision, its alternatives, and its consequences; this file records the detail,
so ADR bodies stay short and this file stays the one place to look for "how exactly."
Update this file when implementation reveals a detail was wrong or incomplete, without a
new ADR — unless the change alters the decision itself, in which case it goes through the
owning ADR as a successor, per docs/architecture/adr/README.md.

The full mechanism below is the v1 *target*. Release 1 implements the subset in
ADR 0016 (native Windows, one sequential writer, live Herdr workers, fixture as
CI double, `meshloop:review-plan` as the plan gate). Residuals (WSL2, concurrency
> 1, packaging, queried quota) are named in implementation-status.md. Do not read
unverified paragraphs as product. Operator path: [Getting started](../start.md).

## 1. Product scope, platforms, and operating surface (ADR 0001)

**Platforms.** R1 stamp: native Windows 10/11 only. v1 target Tier A also includes
Linux via WSL2 on that same host — not a separate bare-metal Linux machine.
**WSL2 is unverified in R1** (implementation-status.md). For a given run, the
Meshloop binary, the target repository's worktrees, the Herdr instance, and the harness
CLIs must all stay on one side of the Windows/WSL boundary: crossing it (a worktree on the
Windows filesystem accessed from inside WSL via `/mnt/c`, or the reverse via `\\wsl$`) hits
the well-known path-translation, permission, and symlink hazards of that boundary and is
not tested or supported in v1. Tier B (best-effort, not blocking v1): macOS. Every
acceptance scenario must run on native Windows and on Linux-under-WSL2 before a Tier A
runtime claim is made; Tier B can lag without blocking the rest.

**Toolchain.** Rust 1.98.0 pinned, edition 2024, mise-managed. Building requires a working
linker per platform (MSVC/Windows SDK on native Windows; a standard build toolchain —
gcc/build-essential or equivalent — inside WSL/Linux). This is a build-time requirement
only and does not contradict the shipped binary having zero runtime dependencies.

**Packaging (ADR 0018).** Source-install: `meshloop-domain`,
`meshloop-engine`, `meshloop-adapters`, and `meshloop-cli` are on crates.io
(0.1.0). `xtask` stays unpublished. Operator path:
`cargo install meshloop-cli --locked` then `meshloop bundle` (see
[install.md](../install.md)). Library crates are implementation crates, not a
stable Rust API. No installer, no service registration, no global
configuration writes. Versioned by the workspace `Cargo.toml` version. Further
crates.io versions and git tags `v<version>` are human-gated.

Prebuilt binaries (`meshloop-windows-x86_64.exe`, `meshloop-linux-x86_64`)
remain deferred (Phase 9). A WSL user would run a Linux binary inside their
WSL distribution, never the Windows `.exe` reaching across the boundary.
Tier B (macOS) packaging is decided when Tier B moves toward support.

**Operating surface (ADR 0001, Accepted).** The compiled `meshloop` binary is the
sole **engine**: every capability must be reachable through it, with no saga in
wrappers. R1 **operator UX** is the `meshloop:` skill pack plus local MCP
(`meshloop mcp`, slash `/meshloop:plan`). Skills add zero engine logic (ML-014).
Unprefixed harness words (`plan`, `reviewer`) are rejected. Live workers are
Herdr panes in a Meshloop-owned workspace (`meshloop-<repo>`, ADR 0019);
origin space topology is not mutated. Fixture is the CI double.

**Scope.** A single local Git repository, already checked out, known working-tree state,
one Meshloop instance at a time. Multi-repository orchestration, remote repositories, and
concurrent instances against the same repository are out of scope until a follow-on
decision. Daemon/service mode is deferred; v1 runs in the foreground for one task graph and exits.

**Naming.** "Meshloop" is the product and binary name. "herdr-loop" must never appear in
any Meshloop artifact — it wrongly ties Meshloop's identity to Herdr, the separate tool it
coordinates. Kebab-case `mesh-loop-*` is the correct derived naming for sub-artifacts
(e.g. the two skills above) — a package-naming convention, not a revival of "mesh-loop" as
an alternate product name.

**Validation scenarios.** Demonstrate one bounded task on native Windows and on
Linux-under-WSL2 before claiming Tier A support; record Tier B separately, never implying
parity. Every product capability needs a meshloop-cli acceptance scenario before any skill
wrapper is proposed for it; a wrapper, if built, is verified only by asserting it produces
the exact CLI invocation a human would have typed.

## 2. Harness capability and agent dispatch contract (ADR 0003)

**HarnessCapabilities port** (meshloop-engine::ports, implemented per harness in
meshloop-adapters): four operations, none assuming undiscovered behavior.
1. `probe` resolves the configured executable path, reads its reported version/identity,
   and returns a `HarnessProfile` of capabilities actually observed: non-interactive/
   scripted invocation, structured output, exit-code semantics, cancellation (signal vs.
   explicit command), and any model-selection mechanism exposed. Configuration supplies
   only the executable path and any model identifiers the user entered — no hardcoded
   model name or CLI flag ships for any harness. `HarnessProfile` also records a
   compatibility flag (compatible / degraded / unsupported) from comparing observed
   version output against a small per-harness allow-list; "unsupported" refuses to
   schedule work on that harness and says why.
2. `invoke` starts one bounded unit of work using only capabilities the profile confirmed;
   requesting an unconfirmed capability is rejected before the process is spawned.
3. `cancel` terminates the owned process tree for a handle; idempotent against an
   already-exited process.
4. `collect` retrieves exit status and captured, redacted (see §4) output once a handle
   reports done.

**Error taxonomy** on any failed `invoke`/`collect`, exactly one of:
`CapacityExhausted{retry_after}` (rate limit or subscription quota hit — the harness is
fine, just full), `Unsupported` (operation never confirmed by `probe` — a caller bug caught
before dispatch), `Timeout`, or `ProcessFault` (the harness ran and produced a result, but
the result is the failure). `CapacityExhausted`/`Timeout` return a task to `ready` for
re-routing (§5); `ProcessFault` counts against the retry budget (§3).

**Agent dispatch.** An `AgentSpec` is the unit meshloop-engine hands to `invoke`:
`{ task_id, attempt_id, harness, model_ref (chosen by routing, §5 — never by this module),
worktree_path, prompt, timeout, cancellation_token }`. The agent module
(meshloop-engine::agent) builds it from a `TaskNode` plus only the context that node's
graph edges declare it depends on — never the whole repository or unrelated task history —
and has no routing/policy authority of its own.

**Prompt construction** is a deterministic, testable function, not a model call: a fixed
four-part envelope — shared situation, the specific change requested, the question the
agent must answer, and the expected output contract (what paths it may touch, that it must
leave the worktree buildable) — rendered from `TaskNode` fields. This is a narrow
prompt-construction convention carried from the source docx's SCQA framing, not adopted as
a governing theory.

**Worktree diff is the primary output.** A harness process's exit code and stdout are
secondary, redacted evidence of what it believed it did; verification (§4) evaluates the
worktree diff regardless of what the process reported — harness exit success is never
verification success. A harness with no file-system output channel is `Unsupported`, not a candidate.

**Validation scenarios.** Contract tests plus live disposable read-only sessions for every
claimed capability and selected harness; a fixture harness that misreports its version or
hangs on cancel proves profile and cancel-idempotency without needing all five real CLIs in
CI. Prompt-construction tests assert no context outside a node's declared dependencies
leaks into its prompt. A fixture harness returning a clean exit code with a broken worktree
(or the reverse) must prove the worktree diff, not exit code, gates `verifying`.

## 3. Execution, transport, isolation, and recovery (ADR 0005)

**Herdr transport.** `HerdrSessionPort` (meshloop-engine) exposes `spawn`, `status`,
`cancel`, `cleanup`. v1 ships exactly one implementation: CLI-subprocess invocation of the
`herdr` binary, using structured arguments — never shell-string interpolation — to create,
list, and terminate panes. A native-socket transport is explicitly deferred, not adopted or
rejected: it needs a platform spike confirming what Herdr actually exposes on Windows and
under WSL before any port method can assume it exists. Until then, `HerdrSessionPort` is
implementable entirely via subprocess calls with signatures that don't leak
subprocess-specific types (PIDs, stdio handles) into the engine, so a socket-backed
implementation later is additive, not a rewrite. Panes are never a security or filesystem
boundary — isolation (below) lives in the worktree, not the pane; a pane crash, kill, or
Herdr restart must never leave a worker's changes unrecoverable or misattributed.

**Isolation and integration.** One Git worktree per writer; one integration owner merges,
with candidate-bound validation (§4's evidence, checked against the exact base revision).
Stale bases and conflicts block integration rather than auto-resolving.

**Execution lifecycle.** The full state set and transition table live in
execution-lifecycle.md (`pending -> ready -> running -> verifying -> awaiting-review ->
accepted -> integrated`, plus `failed`/`cancelled`/`blocked`, plus the `awaiting-plan-review`
gate from §5) — accepting this ADR accepts that table as part of the decision, not a
separately-approved document. Key commitments: harness exit success is necessary but never
sufficient for `accepted`; every transition is logged as an event (executor, task/attempt
id, evidence reference) before it takes effect, so recovery replays rather than guesses;
retries always mint a new attempt id, never reuse one; recovery reconciles the event log
against real process/Git/worktree state before resuming anything — a persisted `running`
state with no matching live process becomes `blocked`, not a silent retry.

**Validation scenarios.** Spawn, monitor, cancel, and clean up owned sessions on each Tier A
platform (native Windows, Linux-under-WSL2) without orphan processes; a
kill-the-herdr-cli-mid-session fault test; a duplicate-cleanup-is-a-no-op test. Concurrent
writes, conflicting changes, stale evidence, and interrupted integration in disposable
repositories. Fault-injection for crash, timeout, stale state, partial completion, and
exhausted retries, including killing the Meshloop process itself mid-`running` and
mid-`accepted`-to-`integrated`, asserting recovery reaches `blocked` or a correctly new
attempt — never a duplicate integration.

## 4. Verification and persistence (ADR 0007)

**Evidence kinds**, each bound to the exact candidate revision hash and attempt id it validates:
- `DeterministicEvidence`: linter/build/test tool, exit code, captured redacted output,
  tool version. Produced by meshloop-adapters running real local tools; never fabricated or
  inferred from a model's claim.
- `ModelReviewEvidence`: which harness/model reviewed, its verdict, its stated rationale.
  Advisory only — never substitutes for `DeterministicEvidence`, never treated as a
  correctness guarantee.
- `HumanAcceptanceEvidence`: which human, when, against which exact revision. Required
  unconditionally for Tier 3 tasks before `accepted`. Tier 1/2 may reach `accepted` on
  `DeterministicEvidence` plus, where routing policy requires it, `ModelReviewEvidence` — a
  configurable threshold that can only raise the bar toward more human involvement, never
  remove `DeterministicEvidence` as a precondition.

A candidate missing any evidence kind required for its tier cannot reach `accepted`,
regardless of what the harness reported. Evidence against a stale base revision is
rejected, never silently rebased.

**Persistence.** Embedded SQLite, versioned schema (`schema_version` from row one,
forward-only migrations — a build refuses to run against a newer on-disk schema than it
knows), local only. Proposed table shape (an actual schema file belongs in `schemas/` only
once this ADR is accepted, per `schemas/README.md`):
- `tasks(task_id, graph_id, tier, created_at, ...)` — node metadata, no free-text prompt content.
- `attempts(attempt_id, task_id, harness, model_ref, started_at, ended_at, outcome)`.
- `events(event_id, task_id, attempt_id, from_state, to_state, event_type, reason, executor, evidence_ref, occurred_at)` —
  the durable transition log §3's recovery depends on.
- `evidence(evidence_ref, kind, candidate_revision, summary_redacted, tool_version, created_at)` —
  summaries only; large raw output goes to redacted on-disk log files referenced by path, never inlined.
- `routing_feedback(harness, model_ref, tier, success_count, failure_count, updated_at)` —
  aggregate counters only, feeding §5's routing scores.

**Redaction.** No raw prompt text, no full model completions, no file contents, no
credentials or tokens, no paths outside the target repository, ever persisted.
`summary_redacted` fields pass a redaction step (secret-pattern scan plus a length cap)
before being written. Retention: `events`/`evidence` retained indefinitely by default
(small, audit-relevant); time-based pruning is a future decision if the store grows
unexpectedly, but v1 never silently expires recovery-relevant history.

**Validation scenarios.** Reject tampered tests, missing evidence, stale candidates, false
success, unauthorized acceptance — a candidate with `DeterministicEvidence` and
`ModelReviewEvidence` but no `HumanAcceptanceEvidence` at Tier 3 must be rejected at
`awaiting-review -> accepted`, not merely flagged. Interrupted writes, schema upgrades,
corruption, redaction, retention, recovery consistency; a redaction-bypass test asserting a
synthetic secret pattern never reaches `evidence.summary_redacted` or any log file.

## 5. Planning and routing (ADR 0009)

**Planning.** Decomposition is not a separate code path: it is one `AgentSpec` (§2)
dispatched before any other task exists, routed like any other node (below) at a fixed high
tier — a bad decomposition is higher-risk than almost any single generated change, since
every downstream task inherits its errors. Its prompt is built from the user's authorized
objective plus explicit scope/exclusions; its declared output contract is a task graph
conforming to meshloop-domain::task_graph's schema, not a worktree diff. The planner module
(meshloop-engine::planner) dispatches the decomposition agent, validates its output
structurally (schema conformance, cycle detection, no dangling dependencies — rejecting and
re-dispatching on failure), and assigns each node a risk tier — it does not judge whether
the decomposition is a *good* one; MECE-ness is a semantic property this project cannot
mechanically verify. Because of that, a produced task graph enters
`awaiting-plan-review -> plan-accepted` (parallel to, not replacing, per-node
`awaiting-review` in execution-lifecycle.md) and requires human acceptance before its first
node schedules, for any objective at or above the lowest risk tier. Re-planning (a node's
repeated failure implying the decomposition was wrong, or scope changing mid-run) is out of
scope for v1; a rejected or irrecoverably failed plan restarts as a new planning dispatch
from scratch, not a live graph edit.

**Routing: Quota-Aware Capability Routing (QACR).** "Budget" means subscription
rate-limit/quota headroom, not dollar cost — all harnesses are flat-rate subscriptions, so
the resource allocated is turns-per-reset-window per harness (and per model tier where one
reports separate limits), not spend. Runs per ready task node:
1. Filter to harness/model combinations the user explicitly configured and that `probe`
   (§2) confirmed compatible. No candidate outside this set is ever considered.
2. Filter out any candidate currently in `CapacityExhausted` cooldown (§2), using its most
   recent observed `retry_after`. Quota is tracked reactively from observed exhaustion
   signals, not a proactively queried number most CLIs don't expose.
3. Score remaining candidates on: tier-to-model fit, `historical_success_rate` for this
   exact (harness, model, tier) triple (from `routing_feedback`, §4, decayed toward neutral
   for sparse data), and a load-balancing term favoring a harness with more headroom
   relative to its own reset window, so configured subscriptions get used roughly
   proportional to available capacity.
4. Break ties with `coupling_penalty`: prefer keeping tightly-coupled nodes on the same
   harness/session context, independent nodes on different harnesses.
5. Dispatch to the top-scored candidate. On `CapacityExhausted`/`Timeout`/`ProcessFault`
   (§2), record `routing_feedback` and re-run QACR excluding the failed candidate —
   visibly, as a logged fallback event, never silently.

`historical_success_rate` can only reorder among already-configured, already-capable
candidates; it can never add a candidate the user didn't configure or bypass a capability
rejection. Concurrency: low-coupling nodes with headroom on distinct harnesses run in
bounded parallel; high-coupling nodes run sequentially even when capacity would allow
parallel dispatch, since merge-conflict risk (§3) outweighs throughput gain. Concrete
numeric thresholds (what counts as "hot," concurrency caps, retry budgets) are session
configuration with documented defaults, not constants here — no measurement yet justifies a
fixed default confidently.

**Extensibility.** Step 3's scoring is a weighted sum over independently pluggable
`RoutingSignal` implementations (`fn score(&self, candidate, context) -> SignalScore`), one
per term (tier-fit, historical success, load-balance, coupling, or a future signal such as
latency or a user-supplied preference weight). Adding a signal, harness, or model tier
never requires changing `router`'s control flow — only registering a new `RoutingSignal` or
`HarnessCapabilities` implementation and configuration entry. The harnesses named elsewhere
in these documents are the initial configured set, not an architectural ceiling.

**Validation scenarios.** A decomposition agent returning a cyclic or schema-invalid graph
must be rejected and retried, never partially scheduled; an objective at or above the
lowest risk tier must not reach its first `ready` node without recorded plan-level
acceptance; a `CapacityExhausted` decomposition dispatch falls back through QACR like any
other dispatch. Unavailable models, quota cooldown expiry (not just permanent exhaustion),
fallback exhaustion (all candidates cooled down at once resolves to `blocked`, not a
crash), high coupling, policy-preserving feedback; no test can make the router select a
harness/model the user never configured; load actually spreads across two configured
harnesses with equal headroom rather than always picking the first.
