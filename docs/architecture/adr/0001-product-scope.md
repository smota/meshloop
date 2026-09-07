# 0001 Product scope, platforms, and operating surface

- Status: Accepted
- Implementation: implemented — native Windows; agent-session UX (skills + local MCP); compiled `meshloop` binary is the engine/saga (ML-014). WSL2 Linux side unverified. crates.io 0.1.0 published (ADR 0018). Prebuilt GitHub Release binaries not started.
- Date: 2026-09-05
- Accepted: 2026-09-06
- Author/executor: Claude (Sonnet 5) consolidated; Grok rewrote operating-surface text under human product direction
- Decision owner: Samuel
- Approval evidence: user approved the 2026-09-06 launch plan (agent session is UX; binary is engine; live Herdr is the product path)
- Supersedes: none
- Superseded by: none

## Context and constraints
Supported OS matrix, toolchain, packaging, and the operating surface all need one coherent
decision rather than three separate ones — they are facets of "what Meshloop is and runs
on/as," not independent tradeoffs. Development happens on Windows with Linux available via
WSL2 on that same host, not a separate bare-metal Linux machine. No service installation is
authorized.

Operators work *inside* Codex, Claude Code, Pi, Grok, or Agy. The CLI is reachable for
debug; it is not the product front door.

## Alternatives
Foreground CLI versus resident daemon; native Windows plus separate bare-metal Linux versus
Windows-plus-WSL2 as one Tier A platform pairing; CLI as sole *product* surface versus
agent-session skills/MCP as UX with the binary as engine.

## Decision
Adopt Meshloop as the product and binary name. Tier A is Windows 10/11 native and Linux via
WSL2 on the same host; macOS is Tier B, best-effort. v1 runs foreground for one task graph
and exits — no daemon.

The compiled `meshloop` binary is the **sole engine** (the saga lives here; skills contain
no orchestration). The **operator surface** for launch is a `meshloop:` skill pack plus
local MCP (`meshloop mcp`), with slash `/meshloop:plan`. CLI verbs remain for debug and
tests. Full platform/toolchain detail is in runtime-design.md §1.

## Consequences
Launch docs lead with an in-session loop (`/meshloop:doctor` → plan → live worker pane).
A stranger who only reads `cargo run` + fixture is reading the CI appendix, not the
product. WSL2 remains a residual, not a launch stamp.

## Verification
See runtime-design.md §1 and `docs/start.md`. Native Windows is the verified Tier A path.
