# 0029 Deterministic loop algorithms and quantized signature retrieval

- Status: Proposed
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Grok
- Reviewer: pending human
- Approval evidence: none
- Supersedes: none
- Superseded by: none

## Context and constraints
Meshloop's hexagon is real (domain / context / engine / adapters / CLI) but the inner
loop is still wiring: a failed `CheckRunner` marks the attempt `Failed` and dumps
stderr. QACR is a static weighted sum over counters. Context injection dumps every
skeleton it is given. That is orchestration, not a convergence algorithm.

A research vector index ([turbovec](https://github.com/RyanCodrai/turbovec), TurboQuant
/ ICLR 2026) was proposed as a cutting-edge tool. Meshloop must decide whether to take
it as a crate or take the *algorithmic properties* that actually fit this product.

Constraints that bound the decision:

- Workspace `unsafe_code = "forbid"` (would need a successor ADR to import SIMD kernels).
- No embedding model, no daemon, no extra runtime (ADR 0022).
- Domain stays pure; engine consumes ports; SQLite schema changes need ADR 0007 succession.
- R1 inner-loop self-repair is still a *proposed* saga change (architecture note's ADR 0026).
  This ADR ships the decision procedure without adding a `Repairing` state.

## Alternatives
1. **Depend on `turbovec`.** Online ingest, 1/2/4-bit TurboQuant, AVX2/AVX-512/NEON FastScan
   kernels, 10M×1536-d in ~4 GB. Rejected: Meshloop has no float32 embeddings; a local
   repo is 10³–10⁵ files not 10⁷ rows; the crate's value is SIMD over neural vectors we
   do not produce. It would be dead weight in the hexagon and requires `unsafe`.
2. **Keep wiring.** Rejected: the product gap is algorithms, not another adapter.
3. **In-tree, safe, data-oblivious quantization of AST signatures, plus a formal
   diagnostic lattice and a restless-bandit QACR signal.** Chosen.

## Decision or proposal
1. **Do not take `turbovec` as a dependency.** Implement the TurboQuant properties that
   matter here — data-oblivious codes, no train step, online add, length-renormalized
   inner product — over *feature-hashed AST signatures* in `meshloop-context::quant`
   (`SignatureIndex`, 1-bit / 2-bit, DIM=64 Walsh–Hadamard rotation, seed-stable).
2. **Diagnostic lattice** in `meshloop-domain::diagnostic`. Atoms are `(severity, code,
   basename, template_hash)` after ANSI/path/line stripping. Fingerprints are portable
   FNV-1a. Progress is lexicographic `(syntax, type, test, error, blocking)`.
3. **Convergence algorithm** in `meshloop-engine::converge`. `RepairSession::observe`
   is the only decision procedure for inner-loop repair. Continue iff φ strictly
   decreases; syntax regression or any φ increase is `Rollback` to the previous HEAD
   (`WorkspacePort::reset_hard`); a repeated fingerprint is `Stop { Oscillation }`;
   `max_rounds` observations including the first check is `BudgetExhausted`. No new
   `TaskState`. Negative constraints are deterministic strings of introduced codes.
4. **Syntactic impact slicing** in `meshloop-engine::slice`. Body-only edits (signature
   blob unchanged) skip a full dependent compile; signature changes name the files that
   must be re-checked, unioned with diagnostic paths and index neighbors.
5. **QACR restless bandit** as a new `RoutingSignal` (`RestlessBanditSignal`).
   Exploration bonus `c / sqrt(n+1)` plus remaining-window density `0.5 * headroom`.
   Missing headroom still contributes 0. Historical success stays its own signal.
   Feedback still cannot add a candidate the user did not configure.

Prompt construction ranks skeletons through `select_context` (budget 24, `allowed_paths`
pinned). Check output is annotated with `lattice=<hex>` in `verify::annotate_with_lattice`.
The saga does **not** auto-reinvoke the harness; that remains ADR 0026.

Mechanism: [runtime-design.md](../runtime-design.md) §6.

## Consequences
- Meshloop gains a deterministic error space, a non-oscillating repair policy, a
  local ANN over type signatures, and a QACR term that treats quota as a sliding
  knapsack — without a neural index, without `unsafe`, without a schema bump.
- `turbovec` remains a valid *future* optional feature if Meshloop ever stores
  model embeddings (Tier 1 reader vectors). That would be a successor ADR, a Cargo
  feature, and an exception to `forbid(unsafe_code)` scoped to that crate.
- Inner-loop re-invoke is deliberately not wired. `RepairSession` is the contract
  ADR 0026 should call.

## Verification and implementation evidence
- `cargo test -p meshloop-domain` — lattice parse, path-invariant fingerprint, syntax
  regression, template holes.
- `cargo test -p meshloop-context` — data-oblivious encode, FWHT involution, online add
  does not rewrite existing codes, search ranks the matching file, `select_context` pins
  allowed paths.
- `cargo test -p meshloop-engine` — converge accept/continue/rollback/oscillation/budget;
  slice body-only vs signature change; restless bandit unused-vs-failed and density;
  agent context budget keeps the query-relevant file.
- `cargo run -p xtask -- check` (fmt, clippy `-D warnings`, workspace tests) passed on
  native Windows after this change. Inner-loop re-invoke remains unwired (ADR 0026).
