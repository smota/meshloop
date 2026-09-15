# Meshloop Documentation

Meshloop is a deterministic local orchestration runtime for AI coding agents. It provides task decomposition, multi-language AST context reduction (7 languages), Git worktree isolation, and compiler-driven verification without background daemons.

---

## Operator Surface Reference

Meshloop operations can be triggered through terminal slash commands, MCP tools, or direct CLI verbs:

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

## Getting Started

```mermaid
flowchart LR
  Doctor["1. /meshloop:doctor\n(Verify environment)"] --> Plan["2. /meshloop:plan\n(Generate task DAG)"]
  Plan --> Review["3. /meshloop:review-plan\n(Accept / Decline / Adjust)"]
  Review --> Run["4. /meshloop:run\n(Execute in isolated worktrees)"]
  Run --> Accept["5. /meshloop:accept\n(Verify compiler & test evidence)"]
  Accept --> Integrate["6. meshloop integrate\n(Merge to target branch)"]
```

1. **[Installation & Setup](install.md)** — Step-by-step setup, `meshloop.toml` configuration, and MCP server integration.
2. **[Getting Started Guide](start.md)** — Detailed walkthrough of the in-session operator loop.
3. **[Product Brief & Use Cases](product/brief.md)** — Problem statement, stack positioning, and 4 concrete environments.
4. **[Skills Catalog](../skills/README.md)** — Reference for all operator skills and slash commands.

---

## System Overview

```mermaid
flowchart TB
  subgraph L1 ["1. Intelligence Layer (LLMs)"]
    M["Claude · GPT · Gemini · DeepSeek · Qwen (Ollama)"]
  end

  subgraph L2 ["2. Developer Surface"]
    CLI["CLI Agents (Claude Code, Codex, Agy) · IDEs (Cursor) · Cloud Sandboxes · CI/CD"]
  end

  subgraph L3 ["3. MESHLOOP Runtime (Rust Engine)"]
    D["1. DAG Decomposition & Tier Allocation"]
    C["2. AST Pruning (7 Languages) & Prompt Cache Normalization"]
    W["3. Git Worktree Isolation & Win32 Job Objects / POSIX PGID"]
    V["4. Deterministic Verification & Self-Repair (Lyapunov phi Lattice)"]
    D --> C --> W --> V
  end

  subgraph L4 ["4. Host Operating System & Workspace"]
    Host["Shared Git Repository · Compilers · Linters · Test Suites"]
  end

  L1 --> L2
  L2 --> L3
  L3 --> L4
```

---

## Documentation Index

### Core Architecture
- **[Architecture Overview](architecture/overview.md)** — Hexagonal boundaries, crate responsibilities, state machine, and glossary.
- **[Modular Architecture & Concurrency](architecture/modern-modular-architecture.md)** — Multi-worker execution ($N \in [1, 16]$), Job Objects, and self-repair convergence.
- **[Component Boundaries](architecture/boundaries.md)** — Allowed dependencies and architectural invariants per crate.
- **[Execution Lifecycle](architecture/execution-lifecycle.md)** — Formal task lifecycle and attempt state machine.
- **[Threat Model & Security](architecture/threat-model.md)** — Fail-closed isolation, credentials, and network posture.
- **[Architecture Decision Records (ADRs)](architecture/adr/README.md)** — Historical decision index from ADR 0001 to ADR 0029.

### Engineering & Operations
- **[Contributing Guide](../CONTRIBUTING.md)** — Workspace layout, adding language support, and extending harnesses.
- **[Testing Strategy](engineering/testing.md)** — Running `xtask check`, `xtask bench`, and isolation tests.
- **[Measurement & Benchmarking](engineering/benchmarking.md)** — SPEC-ML-BENCH-001 metrics, thresholds, and scorecard validation.
