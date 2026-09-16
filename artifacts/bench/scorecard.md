# Executive Benchmark Scorecard (SPEC-ML-BENCH-001)

| Metric | Target | Observed | Status |
|---|---|---|---|
| `ctx.tokens.reduction_pct` | >= 65.0% | 67.7% (7 languages) | **PASS** |
| `ctx.parse.latency_ms.p95` | <= 15.0ms | 0.013 ms | **PASS** |
| `ctx.cache.stale_hit_rate_pct` | = 0.0% | 0.0% | **PASS** |
| `quant.index.compression_ratio` | >= 4.0x | 5.5x | **PASS** |
| `quant.search.latency_us.p95` | <= 500.0us | 332.0 us | **PASS** |
| `quant.recall_at_k` | >= 95.0% | 100.0% | **PASS** |
| `orch.txn.wal_commit_ms.p95` | <= 10.0ms | 1.99 ms | **PASS** |
| `conc.wal.write_contention_ms` | <= 15.0ms | 1.63 ms | **PASS** |
| `conc.git_admin.lock_contention_ms` | <= 200.0ms | 80.66 ms | **PASS** |
| `conc.throughput_gain` | >= 1.0x | 1.93x | **PASS** |
| `iso.redact.pass_rate_pct` | = 100.0% | 100.0% | **PASS** |
| `iso.worktree.leak_count` | = 0.0 | 0 | **PASS** |
| `iso.gate.bypass_count` | = 0.0 | 0 | **PASS** |
| `iso.crash.recovery_fidelity` | = 1.0 | 1.0 (100% integrity + fold) | **PASS** |
| `conv.self_repair.success_rate` | >= 80.0% | 100.0% | **PASS** |
| `conv.lyapunov.monotonic_reduction_pct` | = 100.0% | 100.0% (2/2 pairs) | **PASS** |
| `conv.oscillation.detected_count` | >= 1.0 | 1 cycle(s) detected | **PASS** |
| `conv.rollback.count` | >= 1.0 | 1 rollback(s) triggered | **PASS** |
| `conv.self_repair.convergence_ms.p95` | <= 1500.0ms | 1027.4 ms | **PASS** |
| `iso.repair_rollback.fidelity` | = 1.0 | 1.0 (SHA match + clean tree after reset_hard) | **PASS** |
| `slice.build_avoidance_rate` | >= 50.0% | 66.7% | **PASS** |
| `orch.schedule.overhead_ms.p95` | <= 25.0ms | 0.094 ms (7 manifests) | **PASS** |
| `conc.orphan_process_count` | = 0.0 | 0 | **PASS** |
| `iso.ancestor_propagation.pass_rate_pct` | = 100.0% | 100.0% | **PASS** |
| `orch.wave.dispatch_overhead_ms` | <= 1500.0ms | 862.1 ms | **PASS** |
| `iso.mutation_rollback.fidelity` | = 1.0 | 1.0 (7/7 negative controls rolled back) | **PASS** |
| `orch.mutation.overhead_ms.p95` | <= 15.0ms | 1.894 ms (6 manifests) | **PASS** |
| `smoke.wall_clock_s` | <= 5.0s | 3.88 s | **PASS** |
| `conv.lattice.reduction_rate` | > 0 | 1.0 Delta Phi/round | **PASS** |

**Overall Gate Status:** PASS (28/28 metrics within canonical thresholds)
