# Executive Benchmark Scorecard (SPEC-ML-BENCH-001)

| Metric | Target | Observed | Status |
|---|---|---|---|
| `ctx.tokens.reduction_pct` | >= 65.0% | 67.7% (7 languages) | **PASS** |
| `ctx.parse.latency_ms.p95` | <= 15.0ms | 0.011 ms | **PASS** |
| `ctx.cache.stale_hit_rate_pct` | = 0.0% | 0.0% | **PASS** |
| `quant.index.compression_ratio` | >= 4.0x | 5.5x | **PASS** |
| `quant.search.latency_us.p95` | <= 500.0us | 485.0 us | **PASS** |
| `quant.recall_at_k` | >= 95.0% | 100.0% | **PASS** |
| `orch.txn.wal_commit_ms.p95` | <= 10.0ms | 1.24 ms | **PASS** |
| `conc.wal.write_contention_ms` | <= 15.0ms | 1.28 ms | **PASS** |
| `conc.git_admin.lock_contention_ms` | <= 200.0ms | 71.39 ms | **PASS** |
| `conc.throughput_gain` | >= 1.0x | 1.42x | **PASS** |
| `iso.redact.pass_rate_pct` | = 100.0% | 100.0% | **PASS** |
| `iso.worktree.leak_count` | = 0.0 | 0 | **PASS** |
| `iso.gate.bypass_count` | = 0.0 | 0 | **PASS** |
| `iso.crash.recovery_fidelity` | = 1.0 | 1.0 (100% integrity + fold) | **PASS** |
| `conv.self_repair.success_rate` | >= 80.0% | 100.0% | **PASS** |
| `slice.build_avoidance_rate` | >= 50.0% | 66.7% | **PASS** |
| `conc.orphan_process_count` | = 0.0 | 0 | **PASS** |
| `smoke.wall_clock_s` | <= 5.0s | 1.48 s | **PASS** |
| `conv.lattice.reduction_rate` | > 0 | 1.0 Delta Phi/round | **PASS** |
| `conv.oscillation.detected_count` | > 0 | 1 cycle(s) detected | **PASS** |
| `conv.rollback.count` | > 0 | 1 rollback(s) triggered | **PASS** |

**Overall Gate Status:** PASS (18/18 metrics within canonical thresholds)
