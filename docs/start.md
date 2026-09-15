# Getting Started

Install and first-time setup: **[Install and Setup](install.md)** (`cargo install meshloop-cli --locked`, then `meshloop bundle`).

You operate Meshloop directly from your existing terminal agent (Claude Code, Codex, Pi, Grok, Agy) as `meshloop:origin`. Meshloop executes worker agents as direct CLI subprocesses in isolated ephemeral Git worktrees (`.meshloop-worktrees/<task-id>`), with multi-language AST context reduction and deterministic prompt caching. No background daemons or multiplexer servers are required.

---

## Operator Surface Reference

Meshloop operations can be triggered through terminal slash commands, MCP tools, or direct CLI verbs:

| Canonical ID | Slash Command | MCP Tool | CLI Equivalent | Stage & Function |
| :--- | :--- | :--- | :--- | :--- |
| `meshloop:doctor` | `/meshloop:doctor` | `meshloop_doctor` | `meshloop doctor` | **Diagnostics:** Validates environment, worktree isolation, and configured harnesses. |
| `meshloop:plan` | `/meshloop:plan` | `meshloop_plan` | `meshloop plan` | **Planning:** Decomposes objective into a validated DAG (`meshloop-plan.json`). |
| `meshloop:review-plan` | `/meshloop:review-plan` | `meshloop_review_plan` | `meshloop review-plan` | **Human Gate:** Interactively review plan: Accept, Decline, or Adjust. |
| `meshloop:run` | `/meshloop:run` | `meshloop_run` | `meshloop run` | **Execution:** Runs workers in ephemeral Git worktrees (`.meshloop-worktrees/<task-id>`). |
| `meshloop:status` | `/meshloop:status` | `meshloop_status` | `meshloop status` | **Inspection:** Reports task states, active attempts, and execution history. |
| `meshloop:accept` | `/meshloop:accept` | `meshloop_accept` | `meshloop accept` | **Verification:** Approves deterministic test evidence and git diff for a completed task. |
| `meshloop:resume` | — | `meshloop_resume` | `meshloop resume` | **Continuation:** Resumes execution or restarts failed tasks without replanning. |
| `meshloop:integrate` | — | `meshloop_integrate` | `meshloop integrate` | **Integration:** Merges verified worktree changes into the target branch. |
| `meshloop:orchestrate` | `/meshloop:orchestrate` | `meshloop_orchestrate` | `meshloop orchestrate` | **Review:** Synthesizes cross-model feedback between two distinct agents. |
| `meshloop:roles` | `/meshloop:roles` | `meshloop_roles` | `meshloop roles` | **Catalog:** Lists bundled agent roles and capabilities. |
| `meshloop:mcp` | — | — | `meshloop mcp` | **Server:** Starts the local stdio JSON-RPC Model Context Protocol server. |
| `meshloop:bundle` | — | — | `meshloop bundle` | **Packager:** Exports bundled skills and MCP catalog to target repository. |

---

## The Engineering Loop

The standard workflow runs in a target Git repository:

| State | What you see | Action |
|---|---|---|
| **Ready** | `daemonless: true`, `live_transport: direct-cli`, harnesses verified | `/meshloop:doctor` then `/meshloop:plan` |
| **Plan in flight** | Planner executes; writes `meshloop-plan.json` | Review the generated task graph |
| **Review Plan** | Interactive 3-way decision prompt | `/meshloop:review-plan` — choose **Accept**, **Decline**, or **Adjust** |
| **Accept** | Plan accepted (`PlanAccepted`); no workers spawned yet | `/meshloop:run` |
| **Decline** | Plan declined (`PlanDeclined`); execution refused | `/meshloop:review-plan --adjust` to revise |
| **Adjust** | Planner re-executes with feedback; graph regenerated | Repeat review |
| **Run** | Worker executes in isolated worktree; main branch untouched | `/meshloop:accept` then `/meshloop:resume` |
| **Integrate** | All tasks verified and accepted | `meshloop integrate --into <branch> --accept-integrate` |

---

## Step-by-Step Walkthrough

### 0. Verification Before Starting
From [Install and Setup](install.md), ensure `meshloop` is on `PATH`, your project has `meshloop.toml`, and you exported the skills with `meshloop bundle --dest .`:

```bash
meshloop --version
meshloop doctor
```

### 1. Doctor — Verify Environment
```text
/meshloop:doctor
```
Ensures `daemonless: true` and `live_transport: "direct-cli"`. If doctor fails, check CLI harness paths in `meshloop.toml`.

### 2. Plan — Decompose Objective into Task Graph
```text
/meshloop:plan
```
Generates a Directed Acyclic Graph (DAG) of tasks written to `meshloop-plan.json`. No code is modified and no workers are dispatched at this stage.

### 3. Review Plan — Human Gate
```text
/meshloop:review-plan
```
Prompts for approval before execution:
- **Accept** (`--accept`): Advances plan to `PlanAccepted`. Ready for `/meshloop:run`.
- **Decline** (`--decline`): Transitions plan to `PlanDeclined`. Workers refuse to run.
- **Adjust** (`--adjust`): Re-invokes planner with feedback to regenerate `meshloop-plan.json`.

*(Supports Portuguese input aliases: `aceitar` -> Accept, `recusar` -> Decline, `ajustar` -> Adjust).*

### 4. Run — Worker Execution in Isolated Worktrees
```text
/meshloop:run
```
Dispatches worker agents in dedicated Git worktrees (`.meshloop-worktrees/<task-id>`). Your working branch is never modified during worker execution.

When a task completes, verify compiler and test evidence:
```bash
meshloop accept --task 1 --as your-name
meshloop resume
```

### 5. Integrate — Land Changes on Target Branch
Only explicit integration merges verified commits into your active branch:
```bash
meshloop integrate --graph <id> --into main --accept-integrate
```
Without `--accept-integrate`, integration is refused.

---

## CI / Fixture Mode (Offline Testing)

For CI pipelines or automated testing without live CLI agents:
```bash
cargo build -p meshloop-adapters --bin fixture_harness
meshloop plan --objective "Add a hello.txt file" --config config/meshloop.fixture.toml
meshloop run --plan meshloop-plan.json --accept-plan --config config/meshloop.fixture.toml --fixture-only
```
