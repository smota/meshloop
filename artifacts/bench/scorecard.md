# Executive Benchmark Scorecard (SPEC-ML-BENCH-001)

| Metric | Target | Observed | Status |
|---|---|---|---|
| `ctx.tokens.reduction_pct` | >= 65.0% | 67.7% (7 languages) | PASS |
| `ctx.parse.latency_ms` | < 15.0 ms | 0.010 ms | PASS |
| `quant.index.compression_ratio` | >= 4.0x | 4.6x | PASS |
| `quant.search.latency_us` | < 500 us | 218.0 us | PASS |
| `quant.recall_at_k` | >= 95.0% | 100.0% | PASS |
| `orch.txn.wal_commit_ms` | < 10.0 ms | 1.07 ms | PASS |
| `iso.redact.pass_rate_pct` | = 100% | 100.0% | PASS |
| `iso.worktree.leak_count` | = 0 | 0 | PASS |
| `conv.lattice.reduction_rate` | > 0 | 1.0 Delta Phi/round | PASS |
| `conv.self_repair.success_rate` | >= 80% | 100.0% | PASS |
| `conv.oscillation.detected_count` | > 0 | 1 cycle(s) detected | PASS |
| `conv.rollback.count` | > 0 | 1 rollback(s) triggered | PASS |
| `slice.build_avoidance_rate` | >= 50% | 66.7% | PASS |
| `conc.orphan_process_count` | = 0 | 0 | PASS |

**Overall Gate Status:** PASS (14/14 metrics within canonical thresholds)
