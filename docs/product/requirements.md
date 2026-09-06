# Initial requirements

These are proposed product acceptance targets, not implemented behavior.

- ML-001: Validate a versioned task graph, reject cycles and missing dependencies.
- ML-002: Resolve only verified harness capabilities and explicit model configuration.
- ML-003: Bound worker concurrency, time, retries, cancellation, and fallback.
- ML-004: Isolate worker changes and enforce one integration owner.
- ML-005: Bind verification evidence to the exact candidate and base revision.
- ML-006: Reconcile durable state with actual worktrees/processes after interruption.
- ML-007: Reject unauthorized scope expansion from prompts, logs, and agent output.
- ML-008: Keep secrets out of configuration, diagnostics, persistence, and test data.
- ML-009: Distinguish local success, review acceptance, integration, and publication.
- ML-010: Report failures and unsupported capabilities explicitly without silent skipping.
- ML-011: Select harness and model per task from verified capability, risk tier, and
  observed subscription quota/rate-limit state — never dollar cost, never a hardcoded
  default, never a candidate the user did not configure.
- ML-012: Construct each agent's prompt only from its task node's declared context, and
  verify its worktree diff directly rather than trusting the harness process's own report
  of success.
- ML-013: Require explicit human acceptance of a produced task graph before scheduling any
  of its nodes, for any objective at or above the lowest risk tier — decomposition quality
  is not mechanically verifiable.
- ML-014: Treat the compiled CLI as the sole functional entry point to every capability;
  any convenience wrapper (skill or otherwise) must add zero engine-side logic or dependency.

Each implementation task must add concrete scenarios and link applicable ADRs before coding.
