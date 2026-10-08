# Agent workflow and operating model

> Maintainers and agents. Canonical policy: [`AGENTS.md`](../../AGENTS.md).
> Hub: [docs/README.md](../README.md).

Every harness (Codex, Claude Code, Pi, Grok, Agy) works through the same operating model.
GitHub issues hold the backlog. AgentFlow SDLC governs how work moves from request to pull
request. Meshloop runs bounded engineering work. This file binds those three for this
repository. The generic AgentFlow process is in [`docs/agent-workflow.md`](../agent-workflow.md).

## Systems of record

| Concern | Where it lives |
|---|---|
| Backlog, requirements, roadmap | [GitHub issues](https://github.com/smota/meshloop/issues). Never repository files. |
| Issue format and labels | [`docs/issue-standards.md`](../issue-standards.md) (AgentFlow-managed) |
| Process, roles, gates | AgentFlow SDLC: `agent-workflow.config.json`, `roles/`, [`docs/roles/index.md`](../roles/index.md) |
| Engineering execution | Meshloop, built from this checkout |
| Durable decisions | [ADRs](../architecture/adr/README.md) |
| Delivered history | [Product log](../product/product-log.md), [implementation status](implementation-status.md), PRs |
| Scratch, plans, run state | Ignored `.agent-runs/`, `.meshloop/`, `.meshloop-worktrees/` |

## Backlog rules

1. Start every change from an issue. If none exists, create one that follows the issue
   standards: a Conventional Commit title prefix, the template sections, one type label,
   and `drafted-by:<runtime that actually drafted it>`.
2. Do not add backlog, roadmap, TODO, or "next steps" lists to repository files. File each
   follow-up as an issue and link it from the PR. Use the `limitation` label for documented
   residuals and non-goals.
3. Epics carry the `epic` label and a `## Feature Tracking` task list. Children start with
   `**Epic:** #<n>` and, when ordered, `**Blocked by:** #<n>`. Do not start a blocked issue.
4. Claim before writing. Check for an open PR or branch for the issue. Add
   `for-implementation:<agent>` and post one workflow-status comment, then update that
   comment in place. An issue has one writer at a time.
5. Change issue bodies with section-targeted edits, as the issue standards describe. Fold any
   clarifications humans give in comments back into the body.
6. Close issues through a PR whose body says `Implements #<n>` (the AgentFlow reference) and
   `Closes #<n>` (GitHub closes the issue on merge; `Implements` alone does not). After
   merging, tick the epic's tracking box. Reconcile issues that were delivered without a PR by commenting with the
   commit and the check evidence, then closing them.

## Delivery path

One accountable executor normally carries an issue through the AgentFlow roles: product
manager, analyst, architect, implementation planner, developer, tester, reviewer, technical
writer, and PR readiness. Skip a role only when it adds nothing for the risk involved. Every
harness gets the same role definitions:

- Claude Code: `/role-<name>` commands and `agentflow-*` skills under `.claude/`.
- Codex and Agy: `.agentflow/roles/<harness>/`, plus `.agents/skills/` for Codex.
- Pi: `/role-<name>` under `.pi/`. That directory is ignored, so regenerate it locally.

The architect role writes an ADR for consequential boundary changes, and a human accepts it
before dependent implementation starts. Branches are `work/<slug>` (`codex/<slug>` for Codex),
and PRs target `main`. Commit, push, merge, and release still need the authorization
described in `AGENTS.md`.

### Governed runs

Use an AgentFlow run when the change needs a frozen acceptance contract and recorded
evidence. Name the run `issue-<n>`:

```bash
agentflow-sdlc run source-plan issue-<n> --target .
agentflow-sdlc run start issue-<n> --goal issue:<n> --execute --setup-confirm <digest> --target .
agentflow-sdlc run freeze issue-<n> --execute --target .
agentflow-sdlc run verify issue-<n> --check workspace-check --execute --target .
agentflow-sdlc run status issue-<n> --target . --json
```

`workspace-check` runs `cargo run --locked -p xtask -- check`. Before freezing, add the
files the issue changes to `delivery.candidate.inputs` and point `delivery.contracts` at the
issue's acceptance file, as described in [run operations](../run-operations.md). Creating the
`agentflow-state` coordination ref requires authorizing the exact `source-plan` digest.

## Execution with Meshloop

Build the binary from this checkout. The installed `meshloop` may be older, and that build
is not equivalent evidence:

```bash
cargo build --locked -p meshloop-cli -p meshloop-adapters --bins
```

When an issue splits into independent nodes, the developer role drives Meshloop:

1. `meshloop plan --objective "#<n>: <title>" --json`, then
   `meshloop review-plan --plan <path> --accept|--decline|--adjust`.
2. `meshloop run --plan <path> --json`. Add `--detach` for long runs, then use
   `meshloop inspect --session-id <id> --json` and `meshloop cancel --session-id <id>`.
3. `meshloop status --json` reports nodes awaiting technical acceptance. Accept them with
   `meshloop accept --task <id> --as <identity>`, naming whoever actually accepted. Tier 3
   nodes always need human acceptance.
4. Integrate explicitly with `meshloop integrate --graph <id> --into work/<slug>`, then
   rerun the workspace check on the combined candidate. When one graph delivers several
   issues, tag each node with `"deliverable": "issue-<n>"` in the plan and land each issue on
   its own branch with `meshloop integrate --graph <id> --deliverable issue-<n> --into
   work/<slug>`: run it once without `--accept-integrate` to build the worktree for review,
   then again with it.

Meshloop owns its technical plan, worktrees, workers, and technical acceptance. AgentFlow
owns SDLC acceptance. A Meshloop completion receipt (`git_export`) is evidence, not a merge
or human approval. The optional AgentFlow `meshloop-engineering-cli` provider needs
machine-specific absolute paths and a pinned binary digest. Configure it only for a local
run and never commit it. See the
[integration profile](https://github.com/smota/agentflow-sdlc/blob/main/docs/process-autonomy/meshloop-minimal-integration.md)
for its qualified limits.

## Multi-agent coordination

- Each issue and branch has one writer. Concurrent writers use separate worktrees: Meshloop
  worktrees for plan nodes, or one Git worktree per harness. The integration owner serializes
  writes to shared files.
- Reviews come from a different executor than the developer. Record them with
  `for-review:<agent>` and `reviewed-by:<agent>`. Agreement between models supports a
  conclusion but does not prove it. Use `for-review:human` where `AGENTS.md` requires human
  review.
- Handoffs: governed runs use `agentflow-sdlc run handoff` and `run resume`. Other work
  records the task, executor, base revision, owned paths, changed files, checks,
  limitations, and next owner in the issue's workflow-status comment.
- Provenance labels name only runtimes that actually did the work.

## Maintaining the AgentFlow installation

`agent-framework-lock.json` lists the files AgentFlow owns. Do not edit them by hand; change
`agent-workflow.config.json` instead. To update:

```bash
agentflow-sdlc adopt plan --profile standard --target . --json
agentflow-sdlc config sync --target . --apply
agentflow-sdlc config check --target .
agentflow-sdlc sdlc validate --target .
```

Review the plan before applying it with `adopt apply`. Rollback receipts and recovery are
covered in [reliable delivery](../reliable-delivery.md).
