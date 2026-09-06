# Getting started

**Audience:** an operator on **native Windows** with Rust 1.98 and Cargo.
This is the fixture demo. It is not live Claude Code, Codex, Pi, Grok, or Agy.

Herdr is **not** required here. Do **not** pass `--allow-live-harness`.
WSL2, macOS, Linux, and packaged installs are unverified. Concurrency is 1.

If you only wanted to know whether Meshloop will drive your daily Claude or
Codex session tonight: **not as an R1 stamp.** You can still inspect the loop.

## What this will and will not touch

- Default store: `.meshloop/state.sqlite` in the target git repo.
- Worktrees: `<repo-parent>/.meshloop-worktrees/<repo-dir>/` (override with
  `--worktree-base`). They are **kept**. `run` does **not** merge to your
  current branch.
- Integrate is a separate command and requires `--accept-integrate`.
- No credentials are stored. The example config ships none; do not add API keys.
- `--as` on `accept` is an audit label, not a git author rewrite.
- Meshloop does not install a Windows service.

Use a **throwaway git repo** or this clone — not a production checkout — the
first time. This repository already gitignores `.meshloop/` and
`.meshloop-worktrees/`. Meshloop does not rewrite a target repo’s gitignore.

## 1. Clone and build the dummy worker

```text
git clone https://github.com/smota/meshloop.git
cd meshloop
cargo build -p meshloop-adapters --bin fixture_harness
cargo build -p meshloop-cli
cargo run -p meshloop-cli -- --help
```

You should see Release 1, the command list, default store
`.meshloop/state.sqlite`, that worktrees are kept, that `run` does not merge,
and that non-fixture harnesses need `--allow-live-harness`.

`meshloop` with no arguments prints **status**, not help.

## 2. Plan, run, accept

Stay in the clone for the shortest path. Example config selects **only**
`fixture` and points at `target/debug/fixture_harness.exe`. You are not
authorizing Claude.

```text
cargo run -p meshloop-cli -- plan --objective "Add a hello.txt file" --config config/meshloop.example.toml
```

Read the plan. Meshloop checks that the graph is valid. It does **not** check
that the breakdown is a good idea. Nothing is scheduled until you pass
`--accept-plan`.

```text
cargo run -p meshloop-cli -- run --plan meshloop-plan.json --accept-plan --config config/meshloop.example.toml
cargo run -p meshloop-cli -- status
```

Expected: the dummy worker ran in a **separate worktree**. Your current branch
is unchanged. Status should show a task waiting for human accept. Verification
is git-diff (plus an optional `verify_command`; the example leaves it empty).

```text
cargo run -p meshloop-cli -- accept --task 1 --as sam
cargo run -p meshloop-cli -- resume
cargo run -p meshloop-cli -- status
```

Replace `sam` with your name or handle. `you` is not a magic keyword.

## 3. Optional integrate

Only if you want the result on a branch you name:

```text
cargo run -p meshloop-cli -- integrate --graph <id> --into <ref> --accept-integrate --config config/meshloop.example.toml
```

Without `--accept-integrate`, it must refuse.

## 4. Doctor and leftovers

```text
cargo run -p meshloop-cli -- doctor --json
cargo run -p meshloop-cli -- roles --json
```

R1 “healthy” means the fixture path and store are usable on native Windows —
not that live harnesses are ready. Redact anything sensitive before sharing
JSON.

Leftovers you may delete after a throwaway run:

- `.meshloop/` in the target repo
- worktrees under `<repo-parent>/.meshloop-worktrees/`
- the temp repo itself, if you used one

If you followed these steps against this clone only, your other projects were
not touched.

## Live workers (not the R1 stamp)

To invoke a real CLI agent you must:

1. Configure a non-fixture harness yourself (executable + argv template; no
   credentials in Meshloop config).
2. Have Herdr available for pane transport.
3. Pass `--allow-live-harness` (and origin session flags where required).

Tests never split live panes. Completing that path is **not** required to
believe Release 1. Fail-closed without the flag is expected.

Call Meshloop from an agent session: [skills](../skills/README.md)
(`/meshloop:plan`, MCP `meshloop mcp`). Origin session is supervisor-only.

## Next

- [Docs hub](README.md)
- [Product brief](product/brief.md)
- [ADR 0016](architecture/adr/0016-r1-closed-loop.md)
