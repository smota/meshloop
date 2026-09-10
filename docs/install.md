# Install and setup

Verified path: **native Windows 10/11**, Herdr **0.8**, Meshloop **0.1.0** on
[crates.io](https://crates.io/crates/meshloop-cli). WSL2, macOS, and prebuilt
GitHub Release binaries are unverified.

You operate **from inside** Claude Code, Codex, Pi, Grok, or Agy. The binary is
the engine. Skills and local MCP are the operator surface. Do not run the loop
against the Meshloop product clone.

After this page: **[Getting started](start.md)** (the in-session loop).

## Prerequisites

| Need | Check |
|---|---|
| Cargo on `PATH` | `cargo --version` (Rust 1.98+ to *install*; you are not building Meshloop) |
| `git` | `git --version` |
| Herdr 0.8 | `herdr status` → running |
| At least one harness CLI, already logged in | `codex`, `claude`, `grok`, `pi`, or `agy` |

Meshloop does not store credentials. It uses logins you already have.

## 1. Install the engine (once per machine)

```text
cargo install meshloop-cli --locked
meshloop --version
```

That puts `meshloop.exe` in `%USERPROFILE%\.cargo\bin`. If `meshloop` is not
found, add that directory to `PATH`.

There is no installer and no Windows service.

## 2. Create a throwaway target repo

The **target** is the git repo Meshloop will plan and isolate. Use a disposable
repo the first time.

```text
mkdir meshloop-lab
cd meshloop-lab
git init
git config user.email "you@example.com"
git config user.name "you"
```

Seed at least one commit so worktrees have a base (a `README.md` is enough).

## 3. Config — only harnesses you have

Copy [config/meshloop.example.toml](../config/meshloop.example.toml) to
`meshloop.toml` in the target repo, **or** start from this minimum and keep
only kinds you are logged into:

```toml
selected_harnesses = ["codex"]

[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 300

[verify]
verify_command = []

[harnesses.codex]
kind = "codex"
executable = "codex"
model_ref = "codex"
model_tier = "top"
```

`kind` is the Herdr `--kind`. No API keys. Default store:
`.meshloop/state.sqlite`. Worktrees:
`<repo-parent>/.meshloop-worktrees/`.

## 4. Session pack (skills + MCP catalog)

From the **target** repo:

```text
meshloop bundle --dest .
```

This writes a version-locked pack:

```text
skills/meshloop-*/SKILL.md
meshloop-mcp-tools.json
README.md
```

Copy each `skills/meshloop-*` directory into the skill folder your **origin**
harness already loads in this repo (project-local, not a global Meshloop
write). Confirm against that product’s docs. Slash `/meshloop:plan` works once
the origin session can see those `SKILL.md` files.

Optional local MCP (stdio). Point the origin harness at the same binary:

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

MCP tool names are `meshloop_plan`, not `plan`. Unprefixed `plan` / `reviewer`
/ `scout` tools are rejected.

## 5. Open the origin pane and doctor

1. `herdr status` must show running. If not, start Herdr 0.8 and **stop**.
2. Open Claude Code, Codex, Pi, Grok, or Agy **in the target repo**. This pane
   is `meshloop:origin`. It must not implement the work.
3. Slash `/meshloop:doctor` (or `meshloop doctor --json`).

You want `herdr_server_running: true` and `origin_session` = **this** pane.
If unset, export:

```text
MESHLOOP_ORIGIN_HARNESS=codex
MESHLOOP_ORIGIN_SESSION=w3:p1
```

Use the harness and pane id doctor reported. Doctor does not split panes.

Then go to **[Getting started](start.md)** and run
`/meshloop:plan` → `/meshloop:review-plan` → `/meshloop:run`.

## What Meshloop will not do

- Write a Windows service or global config
- Store API keys
- Merge onto a branch you did not name (`integrate --into --accept-integrate`)
- Split the origin pane or add tabs to the origin space (if a planner/worker/reviewer appears **here**, stop). Live agents use a Meshloop-owned Herdr workspace.

## Builders (clone the product)

Only if you are changing Meshloop itself:

```text
git clone https://github.com/smota/meshloop.git
cd meshloop
cargo build -p meshloop-cli
cargo run -p xtask -- check
```

Still run the **loop** in a throwaway target repo, not in this clone.
See [Contributing](../CONTRIBUTING.md).
