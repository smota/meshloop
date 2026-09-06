# Architecture source and adaptation

Source: user-supplied System Architecture.docx, titled System Architecture & Technical
Specification: Autonomous Engineering Loop Engine (mesh-loop), labeled v2.0 Refined.
The source is retained outside this repository; it has not been copied or published.
Its implementation approval label was treated as document content, not user authorization.

Retained concepts: local Rust engine, authenticated CLI agents, Herdr sessions, task
graphs, progressive rigor, verification, bounded fallback, integration, local feedback.

Adaptations approved during planning: four architectural boundaries; ADR lifecycle;
fully AI-coded implementation with human decisions; five harnesses; no required skills;
Apache-2.0; explicit isolation and evidence. Model names, CLI flags, dependency versions,
fixed three-way decomposition, platform claims, and correctness guarantees were not adopted.

General engineering practices were extracted from Ativaly AGENTS.md sections 6, 7,
13-18 and its thin adapters: boundary validation, error handling, risk classification,
testing, specification, ownership, and honest review evidence. No Ativaly skills, domain
rules, source code, framework scripts, deployment topology, or credentials were copied.

Approval source: the user's Meshloop planning conversation and subsequent instruction
to initialize. Runtime ADR proposals remain subject to their own acceptance gates.

## Revision history (all 2026-09-05, same day)
No content beyond what is listed as retained concepts above was ever drawn from the docx in
any pass below; every pass left all touched content at Status: Proposed.

1. **Revision pass**: elaborated the initial ADR stubs into concrete, implementable
   proposals after an independent re-analysis of the docx.
2. **Consolidation pass**: reframed the core problem around Meshloop as a multi-harness
   coordinator doing model selection across subscription harnesses, not a generic
   "AI agents run one at a time" story; added the harness error taxonomy and named the
   routing framework (QACR); specified the previously-missing agent/dispatch shape.
3. **Gap-closure pass**: added QACR's `RoutingSignal` extensibility point; specified that
   decomposition is itself an agent dispatch with a plan-review gate (the previously-missing
   "orchestration planner"); stated the CLI as the sole operating surface, distinct from the
   already-rejected skill-pack dependency.
4. **Skill-naming correction**: fixed an over-broad naming rule that would have blocked the
   correct `mesh-loop-*` kebab-case convention; named the two optional skills concretely as
   `mesh-loop-planner`/`mesh-loop-executor`.
5. **ADR cleanup pass**: merged 12 proposed ADRs down to 5 (0001, 0003, 0005, 0007, 0009),
   each short and pointing to the new runtime-design.md for mechanism detail, per the ADR
   README's own rule that ADRs explain "why" and baseline docs describe "how." The 7 merged
   IDs (0004, 0006, 0008, 0010, 0013, 0014, 0015) are preserved as short withdrawn stubs,
   never deleted or reused. Also corrected Tier A's Linux target to WSL2 on the Windows
   development host, not a separate bare-metal machine, and added a packaging phase to
   implementation-plan.md.
