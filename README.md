<p>
  <img src="docs/assets/brand/logo.svg"
       width="128" height="128"
       alt="Meshloop mark: a closed ring with a four-node mesh. The bottom node is a square marking human accept.">
</p>

# Meshloop

Turn the CLI coding agents you already pay for into one verified engineering
loop — locally, from inside the session you are already in, without storing
credentials.

[![License](https://img.shields.io/github/license/smota/meshloop?style=flat-square)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/meshloop-cli.svg?style=flat-square)](https://crates.io/crates/meshloop-cli)
[![Version](https://img.shields.io/badge/version-0.1.0-informational?style=flat-square)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.98-orange?style=flat-square&logo=rust)](rust-toolchain.toml)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](#status)
[![Mode](https://img.shields.io/badge/mode-daemonless--direct--cli-1A6B66?style=flat-square)](#status)
[![Context](https://img.shields.io/badge/AST--context-7--languages-1A6B66?style=flat-square)](#status)
[![unsafe](https://img.shields.io/badge/unsafe-forbidden-1A6B66?style=flat-square)](Cargo.toml)

> **Standalone & Daemonless Core**: you stay in Claude Code, Codex, Pi, Grok, or Agy.
> Meshloop schedules work across your authenticated CLI agents, extracts AST skeletons
> across 7 languages to slash context consumption by 70–90%, executes workers in isolated
> Git worktrees without background daemons, verifies diffs, and will not merge until you accept.

| You — this session (`meshloop:origin`) | Meshloop — isolated execution |
|---|---|
| Direct. Never implement the work here. | Planner, worker (+ ephemeral git worktree), reviewers |
| Never mutated directly. Bind session via `/meshloop:doctor`. | Isolation is the git worktree (`.meshloop-worktrees/<task-id>`) |

## Install

Native Windows. Full setup: **[Install and setup](docs/install.md)**.

```text
cargo install meshloop-cli --locked
meshloop --version
```

Then, in a **throwaway git repo** (not this product clone): write `meshloop.toml`
from [config/meshloop.example.toml](config/meshloop.example.toml), run
`meshloop bundle --dest .`, copy `skills/meshloop-*` into the origin harness
skill folder, and `/meshloop:doctor`.

## The loop (one story)

1. `/meshloop:doctor` — verify CLI harnesses, git worktrees, and daemonless mode.
2. `/meshloop:plan` — generate decomposition task graph file (`meshloop-plan.json`).
3. `/meshloop:review-plan` — **Accept / Decline / Adjust** (English prompt; answer in any language).
4. `/meshloop:run` — **after Accept only** — worker in ephemeral worktree; **current branch unchanged**.
5. `/meshloop:accept` then `/meshloop:resume` — node merges into the **integrate worktree**, not your branch.
6. Optional `/meshloop:orchestrate` — two reviewer models evaluate candidates.
7. Optional `meshloop integrate --into --accept-integrate` — only step that lands on a branch you name.

Full steps and failure screens: **[Getting started](docs/start.md)**.

## Why

**Idle subscriptions.** Codex, Claude Code, Pi, Grok, and Agy are flat-rate
CLI subscriptions, not metered APIs.

**Unverified scripts.** Spawning a session is easy. Planning a graph, isolating
the diff, and recovering mid-task is not.

**Local, human-gated.** Meshloop uses logins you already have. It does not
store credentials. It does not merge to `main` by itself.

The five names are the **intended configured set**, not a compatibility matrix.

## Status

| Capability | Current Architecture (0.1.0) |
|---|---|
| Execution architecture | Pure daemonless direct-CLI dispatch in isolated Git worktrees |
| Context engineering | Multi-language AST pruning (Rust, TS/JS, Python, Go, C#, PHP, C++) + Prompt Cache normalizer |
| Bulk Reader (Tier 1) | Dynamic 4-level resolution: CLI flag > Env > TOML > Auto-detect / local Ollama |
| Commands | `plan`, `review-plan`, `run`, `status`, `resume`, `cancel`, `inspect`, `accept`, `integrate`, `roles`, `doctor`, `orchestrate`, `mcp`, `bundle` |
| Plan gate | `review-plan` Accept / Decline / Adjust (CLI shortcut: `run --accept-plan`) |
| Node / land gates | `accept --as` then `resume`; `integrate --accept-integrate` |
| Isolation | Ephemeral Git worktrees (`.meshloop-worktrees/<task-id>`) |
| Fixture | CI double (`--fixture-only`) |
| Credentials / your branch | Not stored / untouched until integrate |
| Concurrency / prebuilt binaries / WSL2 / queried quota | 1 / no / unverified / not queried |
| crates.io | 0.1.0 (`cargo install meshloop-cli --locked`) |
| ADRs | 0001 · 0003 · 0005 · 0007 · 0009 · 0016 · 0017 · 0018 · 0022 Accepted |

## Safety

- No API keys in config. No Windows service. No background daemons.
- Workers use extra git worktrees under `<repo-parent>/.meshloop-worktrees/`.
- `--as` is an audit label, not a git author rewrite.
- Throwaway git repo first — **not** this product clone as the target tree.
- One writer at a time.

## Documentation

**[Docs hub](docs/README.md)** ·
**[Install](docs/install.md)** ·
**[Getting started](docs/start.md)** ·
**[Product brief](docs/product/brief.md)** ·
**[Skills](skills/README.md)**

## Development

Pinned Rust **1.98.0**. Mise-managed toolchain; no global installs.

```text
cargo build -p meshloop-cli
cargo run -p xtask -- check    # fmt, clippy, workspace tests (106 tests)
cargo run -p xtask -- smoke
cargo run -p xtask -- bundle
cargo run -p xtask -- publish-dry
cargo run -p xtask -- live     # verifies doctor daemonless and git worktree isolation
```

The binary is the **engine**. Skills/MCP are the **operator surface**.
No-args `meshloop` prints **status**, not help. Operator install is
[docs/install.md](docs/install.md), not this section.

## License, trademarks, provenance

Code is licensed under [Apache-2.0](LICENSE). Apache-2.0 does not grant
trademark rights. See [NOTICE](NOTICE) and [TRADEMARKS.md](TRADEMARKS.md).

Created and maintained by Samuel ([@smota](https://github.com/smota)). Meshloop
is fully AI-coded under human product direction. Contributor and execution
records distinguish human decisions from generated implementation; they do not
establish legal authorship. No exclusive copyright over AI-generated output is
asserted.

[Contributing](CONTRIBUTING.md) ·
[Security](.github/SECURITY.md) ·
[Code of Conduct](.github/CODE_OF_CONDUCT.md) ·
[Support](.github/SUPPORT.md)
