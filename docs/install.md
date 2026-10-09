# Installation and Setup

Meshloop is the multi-agent development optimization and loop engineering runtime on **native Windows 10/11 and Linux**. It operates as a single standalone Rust binary with zero background daemons, zero external multiplexers, and stores no credentials.

---

## Quick Reference: Operator Surface & Command Mapping

Once installed, Meshloop operations can be triggered via terminal slash commands, MCP tools, or direct CLI verbs:

| Canonical ID | Slash Command | MCP Tool | CLI Equivalent | Stage & Function |
| :--- | :--- | :--- | :--- | :--- |
| `meshloop:doctor` | `/meshloop:doctor` | `meshloop_doctor` | `meshloop doctor` | **Diagnostics:** Loads `meshloop.toml` and runs each selected harness's bounded version probe; reports per-harness readiness. |
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
| `meshloop:ast-skeleton` | — | `meshloop_ast_skeleton` | `meshloop ast-skeleton --path <file>` | **Context:** Returns one file's AST skeleton (bodies elided) with approximate token counts. Read-only; confined to the working directory. |
| `meshloop:symbol-lookup` | — | `meshloop_symbol_lookup` | `meshloop symbol-lookup --query <text>` | **Context:** Ranks declared symbols across the working directory with the quantized signature index; reports index build and search time separately. |

---

## Prerequisites

| Requirement | Verification Command | Purpose |
| :--- | :--- | :--- |
| **Cargo & Rust Toolchain** | `cargo --version` (Rust 1.98+) | Installs and compiles the binary |
| **Git** | `git --version` | Manages isolated worktrees and diff verification |
| **At least one agent harness or model** | CLI (`claude`, `codex`, `agy`), API keys, or local Ollama | Task executor |

> [!NOTE]
> Meshloop **does not store credentials**. It directly uses existing CLI logins, environment variables (e.g., `GEMINI_API_KEY`, `ANTHROPIC_API_KEY`), or local endpoints (e.g., Ollama at `http://localhost:11434`).

---

## Step 1: Install the Meshloop Engine (Machine-wide)

Install the compiled binary from crates.io:

```bash
cargo install meshloop-cli --locked
```

Verify that the executable is accessible in your `PATH`:

```bash
meshloop --version
```

On Windows, the binary is placed in `%USERPROFILE%\.cargo\bin\meshloop.exe`.  
On Linux/macOS, it is placed in `~/.cargo/bin/meshloop`.

---

## Step 2: Initialize in Your Project Repository

The **target repository** is the Git repository where Meshloop will execute and verify tasks:

```bash
cd /path/to/your/project
```

Ensure the repository is a Git repo with at least one commit so that worktrees have a valid base commit:

```bash
git init
git add . && git commit -m "chore: initial commit"
```

---

## Step 3: Configure `meshloop.toml`

Create a `meshloop.toml` file in your repository root, or copy from [`config/meshloop.example.toml`](../config/meshloop.example.toml):

```toml
selected_harnesses = ["codex"]

[limits]
max_concurrent_workers = 2
max_retries = 3
task_timeout_seconds = 300

[verify]
verify_command = ["cargo", "check"]

[harnesses.codex]
kind = "codex"
executable = "codex"
# Read-only probe; must exit without interaction. Defaults to ["--version"].
version_args = ["--version"]
# Required for live dispatch: argv that runs one prompt non-interactively.
# `{prompt_file}` becomes the path of a file holding the prompt; `{model_ref}` the model.
# Meshloop ships no per-harness flags (ADR 0003); take these from your CLI's docs.
invoke_args_template = []
model_ref = "codex"
model_tier = "top"
```

A harness with an empty `invoke_args_template` is probed but never dispatched, so this
sample is **not runnable as-is**. Every probe is bounded by `[limits] probe_timeout_seconds`
(default 15); a version call that opens an interactive UI is reported as a probe timeout.
Run `meshloop doctor --config meshloop.toml` to see each harness's readiness.

- **Planner harness:** `plan` routes the planner as a Tier3 task, so by default only a
  harness with `model_tier = "top"` can plan. To plan with a cheaper model, name it, or
  lower the routing tier (set one key, not both):

  ```toml
  [planner]
  harness = "codex"   # a selected harness, whatever its model_tier
  # tier = "Tier2"    # or: route planning as Tier1, Tier2 or Tier3 (default)
  ```

  Keep `model_tier` honest: it is recorded with the plan (`plan --json` reports
  `data.planner`), and `doctor --json` lists the harnesses that can plan under
  `data.planner.eligible`.

- **Database:** Local execution state is stored in `.meshloop/state.sqlite` (SQLite WAL mode).
  **Ignore it in Git:** add `/.meshloop/` to your repository's `.gitignore`, or run
  `meshloop bundle --dest . --gitignore` to append a marked, idempotent block
  (`# >>> meshloop >>>` … `# <<< meshloop <<<`). Without `--gitignore`, `bundle` never edits
  `.gitignore`; it reports `gitignore: missing` and prints the line to add. On first store
  creation `plan`/`run` also write `.meshloop/.gitignore` (`*`) so the store never shows as
  untracked. `meshloop doctor --json` reports `store_ignored`.
- **Workspaces:** Isolated task executions occur in `.meshloop-worktrees/<task-id>`.

---

## Step 4: Export Operator Skills & MCP Catalog

From your target repository root, run the bundled packager:

```bash
meshloop bundle --dest .
```

This generates:
- `skills/meshloop-*/SKILL.md`: Skill definitions for CLI agent sessions (Claude Code, Codex, Agy, Pi, Grok).
- `meshloop-mcp-tools.json`: Tool catalog for Model Context Protocol clients.

---

## Skills Manager development copies

For the local development installation, `skills/meshloop-*` in the Meshloop
checkout is the authoritative source. Skills Manager owns the imported library
copies, their metadata, the **Meshloop** preset, and agent/project deployments.
Local import copies files; a recorded local source path is not a live link.
Samuel accepted this copy-based workflow on 2026-09-25, with the requirement that
changes to Meshloop skills also refresh the copies used by Skills Manager.

Environment Contract owns environment declarations and audits this integration
through the Skills Manager CLI. The development executable may point to the
checkout's release build; that link does not refresh skill copies or rebuild the
binary after source changes. Keep the embedded session pack and executable
compatible with the changed skills, following ADR 0018.

Whenever a skill or a file it uses changes:

1. Inspect the installed Skills Manager CLI, configured library, existing skill
   records, **Meshloop** preset membership, and current deployments. Preserve IDs,
   unrelated skills, local edits, and activation choices. Reuse the existing preset.
2. Refresh each affected library copy from its individual `skills/meshloop-*`
   directory through the Manager's supported local re-import workflow. Check the
   installed CLI help and behavior first: do not assume that an update command for
   Git sources refreshes local sources, or that re-import always preserves identity.
   If it would duplicate records or overwrite divergent library edits, report the
   conflict before proceeding. Keep all bundled operator skills in the preset.
3. Verify file inventories and content hashes between the source and each refreshed
   library copy, excluding Manager-owned metadata. Confirm that membership and
   record identity were preserved and no duplicate skills were created.
4. Inspect already activated agent/project destinations. Refresh stale copies
   through Skills Manager within the authorized scope; verify linked destinations
   resolve to the refreshed library. New activations remain a separate choice.
   Library refresh alone is not evidence that every deployed copy is current.
5. Record source revision (and dirty state), affected skill IDs, refreshed library
   entries, checked destinations, validation results, and any pending work in the
   task's completion report. If the Manager is unavailable or a destination cannot
   be refreshed, explicitly report it as pending rather than claiming full sync.

Use the Manager's supported interfaces for mutations; retain its ownership of
metadata and distribution. Do not edit its SQLite database or replace library
copies with development junctions as part of this workflow. Before a refresh,
retain enough prior content and metadata to restore the affected entries through
supported Manager operations if verification fails.

This is a maintenance requirement, not an automatic synchronization mechanism or
an installation receipt. Recording it does not itself import or activate skills.

---

## Step 5: Configure MCP Client (Cursor, Claude Desktop, etc.)

To use Meshloop tools inside an MCP-compliant IDE or client, add `meshloop mcp` to your MCP configuration file:

```json
{
  "mcpServers": {
    "meshloop": {
      "command": "meshloop",
      "args": ["mcp"]
    }
  }
}
```

The MCP server runs over standard I/O (`stdio`), connects to the local `meshloop` CLI, and shuts down cleanly when the client closes the session.

---

## Step 6: Verify Environment with `meshloop doctor`

Run the diagnostic check:

```bash
meshloop doctor
```

`doctor` finds `meshloop.toml` (or takes `--config <path>`), loads it, and probes every
selected harness in parallel. It exits non-zero, with `ok: false`, when no config is found,
the config is invalid, or no selected harness is dispatchable. Abridged `--json` output:
```json
{
  "ok": true,
  "data": {
    "daemonless": true,
    "live_transport": "direct-cli",
    "config": "meshloop.toml",
    "harnesses_ready": true,
    "harnesses": [
      { "name": "codex", "version": "<reported>", "dispatchable": true, "reason": null }
    ]
  }
}
```
A harness that is not ready carries a `reason`, such as a probe timeout, a failed version
call, or a missing `invoke_args_template`.

When `doctor` passes, proceed to the **[Getting Started Guide](start.md)** to run the planning and execution loop:
```text
/meshloop:plan → /meshloop:review-plan → /meshloop:run
```

---

## System Invariants

- **No background daemons:** Meshloop executes on demand and exits cleanly.
- **No credential persistence:** No tokens or secrets are written to disk.
- **Fail-closed isolation:** Worker attempts run in isolated Git worktrees. Your working branch remains untouched until explicit integration (`meshloop integrate --into`).
- **Deterministic verification:** Compiler and linter exit codes govern acceptance.
