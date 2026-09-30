# Installation and Setup

Meshloop is the multi-agent development optimization and loop engineering runtime on **native Windows 10/11 and Linux**. It operates as a single standalone Rust binary with zero background daemons, zero external multiplexers, and stores no credentials.

---

## Quick Reference: Operator Surface & Command Mapping

Once installed, Meshloop operations can be triggered via terminal slash commands, MCP tools, or direct CLI verbs:

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
model_ref = "codex"
model_tier = "top"
```

- **Database:** Local execution state is stored in `.meshloop/state.sqlite` (SQLite WAL mode).
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

Expected output:
```json
{
  "daemonless": true,
  "live_transport": "direct-cli",
  "git_available": true,
  "harnesses_ready": true
}
```

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
