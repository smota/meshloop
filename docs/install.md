# Installation and Setup

Meshloop is the multi-agent development optimization and loop engineering runtime on **native Windows 10/11 and Linux**. It operates as a single standalone Rust binary with zero background daemons, zero external multiplexers, and stores no credentials.

---

## Quick Reference: Operator Surface & Command Mapping

Once installed, Meshloop operations can be triggered via terminal slash commands, MCP tools, or direct CLI verbs:

| Canonical ID | Slash Command | MCP Tool | CLI Equivalent | Stage & Function |
| :--- | :--- | :--- | :--- | :--- |
| `meshloop:doctor` | `/meshloop:doctor` | `meshloop_doctor` | `meshloop doctor` | **Diagnostics:** Loads `meshloop.toml` and runs each selected harness's bounded version probe; reports per-harness readiness. |
| `meshloop:plan` | `/meshloop:plan` | `meshloop_plan` | `meshloop plan --objective "<text>"` | **Planning:** Decomposes an objective into a validated DAG (`meshloop-plan.json`). |
| `meshloop:review-plan` | `/meshloop:review-plan` | `meshloop_review_plan` | `meshloop review-plan --plan <path> (exactly one of --accept, --decline, --adjust)` | **Human Gate:** Requires exactly one of `--accept`, `--decline`, or `--adjust`. |
| `meshloop:run` | `/meshloop:run` | `meshloop_run` | `meshloop run` | **Execution:** New runs use `<repo>/.meshloop/worktrees/<run-id>` after a Git layout probe. Existing runs stay pinned. Override with `--worktree-base` or `[workspace] root`. |
| `meshloop:status` | `/meshloop:status` | `meshloop_status` | `meshloop status` | **Inspection:** Reports task states, active attempts, and execution history. |
| `meshloop:cancel` | — | `meshloop_cancel` | `meshloop cancel --driver <id>` | **Cancellation:** Stops a driver without merging. |
| `meshloop:inspect` | — | `meshloop_inspect` | `meshloop inspect --driver <id>` | **Inspection:** Shows one driver's attempts, evidence, and worktree. |
| `meshloop:watch` | — | — | `meshloop watch --graph <id>` | **Watch:** Polls a graph until it settles. Interval defaults to 5s; `--timeout 0` waits without a limit. |
| `meshloop:accept` | `/meshloop:accept` | `meshloop_accept` | `meshloop accept` | **Verification:** Approves deterministic test evidence and git diff for a completed task. A verify gate on tier 1 or 2 accepts after deterministic checks; every other node stops at `AwaitingReview` until `meshloop accept`; the tier selects the harness and model only. |
| `meshloop:resume` | — | `meshloop_resume` | `meshloop resume` | **Continuation:** Resumes execution or restarts failed tasks without replanning. |
| `meshloop:integrate` | — | `meshloop_integrate` | `meshloop integrate` | **Integration:** Merges verified worktree changes into the target branch. |
| `meshloop:orchestrate` | `/meshloop:orchestrate` | `meshloop_orchestrate` | `meshloop orchestrate` | **Review:** Synthesizes cross-model feedback between two distinct agents. |
| `meshloop:roles` | `/meshloop:roles` | `meshloop_roles` | `meshloop roles` | **Catalog:** Lists bundled agent roles and capabilities. |
| `meshloop:mutate-plan` | — | `meshloop_mutate_plan` | `meshloop mutate-plan (--mutation <json> or --mutation-file <path>)` | **Replan:** Applies one upstream graph mutation. `--require-review` returns the plan to review. |
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
# `{prompt}` is the prompt text when `prompt_via` is `arg` (and, for `stdin`, only if
# the template contains `{prompt}`). `prompt_via` is `file` (default), `arg`, or `stdin`.
# Meshloop ships no per-harness flags (ADR 0003); take these from your CLI's docs.
invoke_args_template = []
model_ref = "codex"
model_tier = "top"
```

A harness with an empty `invoke_args_template` is probed but never dispatched, so this
sample is **not runnable as-is**. Every probe is bounded by `[limits] probe_timeout_seconds`
(default 15); a version call that opens an interactive UI is reported as a probe timeout.
Run `meshloop doctor --config meshloop.toml` to see each harness's readiness.

Verified templates live in [`config/meshloop.example.toml`](../config/meshloop.example.toml).
They were copied from each CLI's own help and are not a claim that live dispatch works:

- grok `grok 1.0.70 (f50904eaee6c) [stable]`, `grok --help` shows `--prompt-file <PATH>`: `["--prompt-file", "{prompt_file}"]` with default `prompt_via = "file"`.
- claude `2.1.316 (Claude Code)`, `claude --help` shows `--print` and argument `prompt`: `prompt_via = "arg"`, `["--print", "{prompt}"]`.
- codex `codex-cli 0.165.0`. `codex --help` lists `exec`. `codex exec --help` shows that `-` reads stdin: `prompt_via = "stdin"`, `["exec", "-"]`.
- pi `1.4.0`, `pi --help` shows `-p` / `--print` and example `pi -p "..."`: `prompt_via = "arg"`, `["-p", "{prompt}"]`.
- agy `1.4.12`, `agy --help` shows `--print` but not where the prompt text goes. Not verified; no template.

These templates do not pass `{model_ref}`, so each CLI runs its default model. To choose
the model, or to route several models of one harness with a `models` list, see
[Model selection](engineering/harnesses.md#model-selection).

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
- **Workspaces:** New runs use `<repo>/.meshloop/worktrees/<run-id>` after a Git layout probe. Existing runs stay pinned. Override with `--worktree-base` or `[workspace] root`.

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
/meshloop:plan → /meshloop:review-plan → /meshloop:run → meshloop accept
```

---

## System Invariants

- **No background daemons:** Meshloop executes on demand and exits cleanly.
- **No credential persistence:** No tokens or secrets are written to disk.
- **Fail-closed isolation:** Worker attempts run in isolated Git worktrees. Your working branch remains untouched until explicit integration (`meshloop integrate --into`).
- **Deterministic verification:** Compiler and linter exit codes are the check. A verify gate on tier 1 or 2 accepts after deterministic checks; every other node stays in `AwaitingReview` until `meshloop accept`. The tier selects the harness and model only. A sentence in a node description such as "HUMAN GATE" is not enforced. Dependent nodes do not start until the predecessor is accepted and `meshloop resume` merges it. `meshloop integrate` lands that result on the target branch and is a separate step.
