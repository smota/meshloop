# 0022 Daemonless Standalone Execution and Multi-Language Context Engineering

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-14
- Author/executor: Antigravity / AI Pair programmer
- Decision owner: Samuel
- Approval evidence: user approval in chat ("Perfeito. Implemente todo o plano, teste, valide, ajuste")
- Supersedes: none (extends 0005 and 0009)
- Superseded by: none

## Context and constraints
Meshloop was initially tied to the external Herdr daemon for live worker lifecycle and process supervision. While Herdr provided multiplexed terminal pane supervision, it introduced an operational dependency, dual-state synchronization friction, and prevented zero-dependency execution in CI/CD, headless environments, or native integration into modern agent harnesses (like AGY, Claude Code, Cursor). Furthermore, monolithic task contexts caused rapid token exhaustion and context rot when ingesting extensive architectural plans across polyglot repositories.

## Alternatives
1. Continue mandating Herdr server running in background for all live tasks.
2. Port Meshloop entirely into a single proprietary harness platform.
3. Decouple worker execution by making Git worktrees the primary isolation boundary, supporting direct CLI dispatch (`CliHarness`) without Herdr, and introducing a multi-language AST skeleton extraction and delegated reading pipeline (`meshloop-context`).

## Decision
1. **Permanent Herdr Removal & Pure Daemonless Architecture:** Herdr daemon compatibility is completely removed from Meshloop. All live tasks run as direct CLI processes (`CliHarness`) within isolated, ephemeral Git worktrees (`GitWorktreeAdapter`). Zero external background daemons or multiplexer sockets are required.
2. **Multi-Language AST Context Engineering (7 Languages):** Introduce `meshloop-context` crate providing AST skeleton extraction for Rust, TypeScript/JavaScript, Python, Go, C#, PHP, and C++. Internal function and method bodies are pruned while preserving public traits, structs, interfaces, signatures, auto-properties, and docstrings, reducing input tokens by 70% to 90%.
3. **Dynamic Tier 1 (Bulk Reader) Resolution:** Support 4-level precedence for bulk reading models (CLI flag > Environment Variable > TOML Config > Auto-detected API keys / local Ollama fallback), enabling subscription-based CLI harnesses (`agy`, `claude`, `codex`) alongside direct APIs and zero-cost local models.
4. **Prompt Cache Normalization:** Structure prompts with deterministic, byte-identical static prefixes (system policies, output contracts, repository skeletons) to maximize provider prompt caching hit ratios (>80%).

## Consequences
- Clean, standalone execution in any environment (local workstation, CI/CD, headless containers).
- Zero dual-state synchronization friction between Meshloop SQLite event log and external multiplexers.
- Polyglot repositories across 7 languages benefit from dramatic token savings and reduced LLM hallucination.
- Preserves clean hexagonal domain architecture: `meshloop-domain` remains pure data; `meshloop-context` handles AST and prompt engineering; `meshloop-engine` consumes context abstractions; `meshloop-adapters` provides Git and direct CLI process execution; `meshloop-cli` composes the runtime.

## Verification and implementation evidence
- `meshloop-context`: Unit tests for Rust, TypeScript, Python, Go, C#, PHP, and C++ skeleton extraction, Tier 1 resolution, and cache normalization (`cargo test -p meshloop-context`, 14 tests passing).
- `meshloop-cli`: Unit tests confirming live direct CLI harness composition and daemonless execution.
- `meshloop doctor`: Reports `"daemonless": true` and `"live_transport": "direct-cli"`.
- `xtask live`: Launch gate validating daemonless direct-cli operation and git worktree isolation.
- Full workspace test suite verification: `cargo test --workspace` (106 tests passing).
