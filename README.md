<p>
  <img src="docs/assets/brand/logo.svg"
       width="128" height="128"
       alt="Meshloop mark: a closed ring with a four-node mesh. The bottom node is a square marking human accept.">
</p>

# Meshloop

Turn the CLI coding agents you already pay for into one verified engineering
loop — locally, without storing credentials.

[![License](https://img.shields.io/github/license/smota/meshloop?style=flat-square)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.1.0-informational?style=flat-square)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.98-orange?style=flat-square&logo=rust)](rust-toolchain.toml)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](#status)
[![R1](https://img.shields.io/badge/R1-fixture--backed-C4843C?style=flat-square)](#status)
[![unsafe](https://img.shields.io/badge/unsafe-forbidden-1A6B66?style=flat-square)](Cargo.toml)

> **Release 1** is a fixture-backed, single-writer closed loop on native Windows.
> Completing a live Claude Code, Codex, Pi, Grok, or Agy dispatch is **not** the
> R1 stamp. Concurrency is 1. No packaging. WSL2 is unverified.

## Why

**Idle subscriptions.** Codex, Claude Code, Pi, Grok, and Agy are flat-rate
CLI subscriptions, not metered APIs. Used one at a time, that capacity sits idle
or burns unevenly.

**Unverified scripts.** Ad-hoc glue can spawn a session. It does not plan a
task graph, isolate the diff, or recover when the session dies mid-task.

**Local, human-gated.** Meshloop plans one engineering objective, runs workers
in git worktrees, verifies the diff, and will not merge until you accept. It
uses sessions you already authenticated. It does not store credentials.

The five names above are the **intended configured set**, not a compatibility
matrix. Selection is not discovery, and discovery is not invoke.

## Status

| Capability | Release 1 (0.1.0) |
|---|---|
| Fixture-backed `plan` → `run --accept-plan` → verify git diff → `accept` → `resume` → `integrate` | Yes, native Windows |
| Commands: `plan`, `run`, `status`, `resume`, `cancel`, `inspect`, `accept`, `integrate`, `roles`, `doctor`, `orchestrate`, `mcp` | Present |
| Human gates: `--accept-plan`, `accept --as`, `integrate --accept-integrate` | Required |
| Credentials | Not stored |
| Current branch | Untouched until explicit integrate |
| Live Claude / Codex / Pi / Grok / Agy dispatch | **Not an R1 stamp.** Opt-in `--allow-live-harness` only |
| Live Herdr pane split | Fail-closed; tests never split panes |
| Concurrency | 1 |
| Packaging / crates.io | No (`publish = false`) |
| WSL2 / Linux / macOS | Unverified |
| Runtime ADRs 0001 / 0003 / 0005 / 0007 / 0009 / 0016 / 0017 | Proposed (human acceptance pending) |

Source of truth for what exists: [implementation status](docs/engineering/implementation-status.md).
R1 subset: [ADR 0016](docs/architecture/adr/0016-r1-closed-loop.md).

## Safety

- **No credential store.** Config must not contain API keys. Meshloop invokes
  CLIs you already logged into.
- **`run` does not merge** onto your current branch. Workers use extra git
  worktrees. Default base: `<repo-parent>/.meshloop-worktrees/<repo-dir>/`.
- **Integrate is explicit:** `meshloop integrate --graph <id> --into <ref> --accept-integrate`.
- **One worker.** It will not fan out five live agents against your tree.
- **No Windows service.** Foreground CLI; it exits.
- **Throwaway first.** Use a disposable git repo, not a production checkout.

`--as` on `accept` is an **audit label** (who accepted), not a git author rewrite.

## Try it (fixture demo)

Native Windows, Rust 1.98, git, Cargo. This path uses the **dummy** worker
`fixture_harness`. It is not Claude or Codex.

```text
git clone https://github.com/smota/meshloop.git
cd meshloop
cargo build -p meshloop-adapters --bin fixture_harness
cargo build -p meshloop-cli
cargo run -p meshloop-cli -- --help
cargo run -p meshloop-cli -- plan --objective "Add a hello.txt file" --config config/meshloop.example.toml
cargo run -p meshloop-cli -- run --plan meshloop-plan.json --accept-plan --config config/meshloop.example.toml
cargo run -p meshloop-cli -- status
cargo run -p meshloop-cli -- accept --task 1 --as sam
cargo run -p meshloop-cli -- resume
```

Default store: `.meshloop/state.sqlite` (gitignored in this repo). Worktrees are
kept. Do **not** pass `--allow-live-harness` on this path.

Full fixture loop, leftover paths, and optional integrate:
**[Getting started](docs/start.md)**.

No-args `meshloop` prints **status**, not help.

## What Release 1 is not

- Not a cloud service, credential manager, or background service
- Not `cargo install`, crates.io, Homebrew, or a GitHub Release binary
- Not a verified multi-harness runtime
- Not a Herdr fork, and not a replacement for the CLIs you already use
- Not automatic merge to `main`

Herdr is **not** required for the fixture demo. Live workers need Herdr **and**
`--allow-live-harness`. That path is opt-in and unstamped.

## Documentation

Start at the **[docs hub](docs/README.md)**.

| If you want | Read |
|---|---|
| Problem and value | [Product brief](docs/product/brief.md) |
| Operator steps | [Getting started](docs/start.md) |
| Call Meshloop from an agent session | [Skills / session control plane](skills/README.md) |
| What was actually built | [Implementation status](docs/engineering/implementation-status.md) |

## Development

Pinned Rust **1.98.0** with rustfmt and Clippy. This workstation uses mise; no
global tool installation is part of contributing.

```text
cargo run -p xtask -- check
cargo run -p xtask -- smoke
cargo run -p xtask -- bundle
```

`check` is the acceptance bar: fmt, clippy `-D warnings`, `cargo test --workspace`.
See [testing](docs/engineering/testing.md) and [AGENTS.md](AGENTS.md).

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
