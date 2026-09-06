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
[![Version](https://img.shields.io/badge/version-0.1.0-informational?style=flat-square)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.98-orange?style=flat-square&logo=rust)](rust-toolchain.toml)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](#status)
[![R1](https://img.shields.io/badge/R1-live--Herdr-1A6B66?style=flat-square)](#status)
[![unsafe](https://img.shields.io/badge/unsafe-forbidden-1A6B66?style=flat-square)](Cargo.toml)

> **Release 1** (native Windows + Herdr 0.8): you stay in Claude Code, Codex,
> Pi, Grok, or Agy. Meshloop opens **other** panes for planner, worker, and
> reviewers, verifies the git diff, and will not merge until you accept.
> Fixture subprocess is CI, not the product.

| You — this pane (`meshloop:origin`) | Meshloop — other Herdr panes |
|---|---|
| Direct. Never implement the work here. | Planner, worker (+ git worktree), reviewers |
| Never split. Bind `MESHLOOP_ORIGIN_SESSION` from `/meshloop:doctor`. | Isolation is the worktree, not the pane |

If a planner, worker, or reviewer appears **in this pane**, stop. Origin was
not injected.

## The loop (one story)

1. `/meshloop:doctor` — bind origin (this pane id). If Herdr is down, stop.
2. `/meshloop:plan` — planner pane **elsewhere**; graph file.
3. `/meshloop:review-plan` — **Accept / Decline / Adjust** (English prompt; answer in any language).
4. `/meshloop:run` — **after Accept only** — worker pane + worktree; **current branch unchanged**.
5. `/meshloop:accept` then `/meshloop:resume` — node merges into the **integrate worktree**, not your branch.
6. Optional `/meshloop:orchestrate` — two reviewer panes, never origin.
7. Optional `meshloop integrate --into --accept-integrate` — only step that lands on a branch you name.

Full steps, throwaway repo, and failure screens: **[Getting started](docs/start.md)**.

## Why

**Idle subscriptions.** Codex, Claude Code, Pi, Grok, and Agy are flat-rate
CLI subscriptions, not metered APIs.

**Unverified scripts.** Spawning a session is easy. Planning a graph, isolating
the diff, and recovering mid-task is not.

**Local, human-gated.** Meshloop uses logins you already have. It does not
store credentials. It does not merge to `main` by itself.

The five names are the **intended configured set**, not a compatibility matrix.

## Status

| Capability | Release 1 (0.1.0) |
|---|---|
| Loop above, live Herdr workers | Yes, native Windows + Herdr 0.8 (`xtask live` is the launch gate) |
| Commands | `plan`, `review-plan`, `run`, `status`, `resume`, `cancel`, `inspect`, `accept`, `integrate`, `roles`, `doctor`, `orchestrate`, `mcp` |
| Plan gate | `review-plan` Accept / Decline / Adjust (CLI shortcut: `run --accept-plan`) |
| Node / land gates | `accept --as` then `resume`; `integrate --accept-integrate` |
| Origin pane | Supervisor-only; never split |
| Fixture | CI double (`--fixture-only`) |
| Credentials / your branch | Not stored / untouched until integrate |
| Concurrency / packaging / WSL2 / queried quota | 1 / no / unverified / not queried |
| ADRs 0001 · 0003 · 0005 · 0007 · 0009 · 0016 · 0017 | Accepted |

## Safety

- No API keys in config. No Windows service.
- Workers use extra git worktrees under `<repo-parent>/.meshloop-worktrees/`.
- `--as` is an audit label, not a git author rewrite.
- Throwaway git repo first — **not** this product clone as the target tree.
- One writer at a time.

## Documentation

**[Docs hub](docs/README.md)** ·
**[Getting started](docs/start.md)** ·
**[Product brief](docs/product/brief.md)** ·
**[Skills](skills/README.md)**

## Development

Pinned Rust **1.98.0**. Mise-managed toolchain; no global installs.

```text
cargo build -p meshloop-cli
cargo run -p xtask -- check    # fmt, clippy, fixture tests
cargo run -p xtask -- smoke
cargo run -p xtask -- bundle
cargo run -p xtask -- live     # fails if Herdr is down
```

The binary is the **engine**. Skills/MCP are the **operator surface**.
No-args `meshloop` prints **status**, not help.

Live tests may split **non-origin** panes. See
[testing](docs/engineering/testing.md) and [AGENTS.md](AGENTS.md).

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
