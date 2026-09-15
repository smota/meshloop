# Install and Setup

Verified paths: **Windows 10/11 & Linux**, Meshloop **0.1.0** on [crates.io](https://crates.io/crates/meshloop-cli).  
**Pure daemonless direct-CLI dispatch in isolated Git worktrees.** Zero background daemons, zero external multiplexers.

You can operate Meshloop:
1. **From inside terminal coding agents** (Claude Code, Codex, Pi, Grok, Agy) using slash skills or local MCP.
2. **From the CLI directly** in any terminal (PowerShell, Bash, Zsh) or CI/CD runner.

After this page: **[Getting started](start.md)** (the in-session engineering loop).

---

## Prerequisites

| Requirement | Command to Verify | Purpose |
| :--- | :--- | :--- |
| **Cargo & Rust toolchain** | `cargo --version` (Rust 1.98+) | To install the binary via crates.io |
| **Git** | `git --version` | For repository worktree isolation |
| **At least one model source** | CLI (`claude`, `codex`, `agy`), API key, or local Ollama | Execution engine |

> [!NOTE]
> Meshloop **does not store credentials**. It directly uses your existing CLI logins, environment API keys (`GEMINI_API_KEY`, `ANTHROPIC_API_KEY`), or a local Ollama endpoint (`http://localhost:11434`).

---

## 1. Install the Engine (Once per Machine)

```bash
cargo install meshloop-cli --locked
meshloop --version
```

This installs `meshloop` in your Cargo binary directory (`%USERPROFILE%\.cargo\bin` on Windows or `~/.cargo/bin` on Linux).

---

## 2. Initialize in a Target Repo

The **target** is the Git repository where Meshloop will plan, isolate, and verify code:

```bash
mkdir my-project
cd my-project
git init
git config user.email "you@example.com"
git config user.name "Your Name"
echo "# My Project" > README.md
git add . && git commit -m "chore: initial commit"
```

*(Ensure at least one commit exists so Git worktrees have a valid base commit).*

---

## 3. Configuration (`meshloop.toml`)

Copy [config/meshloop.example.toml](../config/meshloop.example.toml) to `meshloop.toml` in your project root, or create a minimal one:

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

* **Storage:** State is stored in `.meshloop/state.sqlite` (SQLite WAL).
* **Worktrees:** Isolated workspaces are automatically created in `.meshloop-worktrees/<task-id>`.

---

## 4. Install Operator Skills & MCP (Optional)

From your target repository root:

```bash
meshloop bundle --dest .
```

This exports the version-locked operator pack:
- `skills/meshloop-*/SKILL.md` (for origin CLI sessions like Claude Code, Codex, Agy)
- `meshloop-mcp-tools.json` (for Model Context Protocol clients like Cursor or Claude Desktop)

For MCP integration, point your client to the compiled binary:

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

---

## 5. Validate with `meshloop doctor`

Run the diagnostic check to ensure your environment is 100% ready:

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

Now you are ready to start: proceed to **[Getting started](start.md)** and run:
```bash
/meshloop:plan → /meshloop:review-plan → /meshloop:run
```

---

## What Meshloop Will NEVER Do

- Install background services or permanent system daemons.
- Store or transmit your API keys or passwords.
- Auto-merge unverified code directly into your active working branch.
- Rely on unverified model self-reports instead of deterministic compiler/linter test runs.
