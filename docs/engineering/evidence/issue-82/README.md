# Issue 82: portable measurement evidence and handover

This is a sanitized, durable evidence snapshot for another machine, dated 2026-11-01,
archived under [issue #88](https://github.com/smota/meshloop/issues/88).
[Issue #82](https://github.com/smota/meshloop/issues/82) is **closed** and
[PR #87](https://github.com/smota/meshloop/pull/87) is **merged** at
`477c30a` on `main`. Main merge CI [41317910822](https://github.com/smota/meshloop/actions/runs/41317910822)
succeeded. These administrative and CI outcomes do not establish the cause of earlier
failures or erase them. No root cause or additional supported optimization was established.

## Source revisions and retained outcomes

| Revision | Evidence and outcome |
|---|---|
| `7081d6080722633699213f11e7dad56db71fff3c` | A baseline for both isolated comparisons. |
| `ed32d235707e5b0f0c1fa8edd9608e29a44c1525` | B in completed local comparison. |
| `2f495950e06095c310fcd3d44fa225d98217536f` | Corrected concurrent timer boundary, retained chronological WAL samples, corrected benchmark documentation; remote B. Local canonical 27/28: self-repair 1604.2 ms >1500 ms; WAL 1.1023 ms. |
| `f3d3baed17e12b055702d0858680e3163c6ac848` | One temporary remote diagnostic dispatch. |
| `1e23c5eae90fe5f0d310a1a0cb364ce3ee025593` | First subsequent commit removed temporary diagnostic job. Final candidate tree `6e112716f4dbb623f443972126df6c2ca6b984c6`. |
| `3eeed70df50600d281c468aa930fbf80b7817f0c` | Final PR test merge; tree identical to final candidate. |

| Actions run | Retained result |
|---|---|
| [41246944440](https://github.com/smota/meshloop/actions/runs/41246944440), push at `2f49595` | Windows benchmark 27/28: WAL 10.0462 ms >10 ms. |
| [41246965285](https://github.com/smota/meshloop/actions/runs/41246965285), PR test merge `11e0809bdaa856e3053aecbe3c9b408c63430a15` | Windows benchmark 24/28: WAL 521.3522 ms, contention 63.17 ms, throughput 0.91x, repair 2691.2 ms. Test-merge tree equals `2f49595` tree. |
| [41250961308](https://github.com/smota/meshloop/actions/runs/41250961308), dispatch push | Passed. |
| [41250964703](https://github.com/smota/meshloop/actions/runs/41250964703), dispatch PR | Isolated diagnostic succeeded; canonical Windows benchmark 26/28 failed WAL 16.29 ms and throughput 0.85x. |
| [41251359828](https://github.com/smota/meshloop/actions/runs/41251359828), final candidate push | Windows benchmark 26/28 failed WAL 11.4256 ms >10 ms and mutation 18.047 ms >15 ms. |
| [41251363796](https://github.com/smota/meshloop/actions/runs/41251363796), final candidate PR | Windows and Ubuntu passed. |

Ubuntu and Windows workspace checks/smoke passed in the listed failed benchmark runs.
A passing PR, push, or main run does not substitute for another run's retained failure.
The final four OS artifacts are preserved numerically in [results.json](results.json).
Main merge CI is linked above; its raw artifacts are not part of this numeric snapshot.

## Completed diagnostic limits

Both frozen batches are **consumed and must never repeat**, including on another machine:

- Local: six pairs AB/BA/AB/BA/AB/BA, 12 runs, 50 operations each, no warmups/retries.
  Paired B−A p95 median +0.2355 ms, range −1.9639 to +2.1868 ms; four higher/two lower.
- Remote: same six pair orders, 12 product runs plus six Python SQLite storage controls,
  50 operations each, no warmups/retries, single Actions run 41250964703 attempt 1.
  Paired B−A p95 median −0.08075 ms, range −4.0102 to +2.4067 ms; four lower/two higher.
  Controls ranged 4.1309–5.8002 ms; their different SQLite library/workload prevents treating
  them as comparable product latency or gate acceptance.

Both batches had a 30-minute cap and fresh fixtures on one host/volume per batch.
They did not recreate the earlier failing runner plus full workload; no CI cause follows.
The temporary `issue82-wal-storage-diagnostic` CI job was removed: **never restore it or
manually rerun its jobs**. Diagnostic source files remaining in Git are historical tools,
not permission to execute. The proposed eight-case repair experiment was never executed
or approved. No overnight automation remains; do not recreate it.

Do not relax thresholds, workload, sample count, guard behavior, or approval requirements.
No new measurements, source changes, speculative retries, or corrective fix are authorized
by this handover. New work requires a separately scoped GitHub follow-up and a reviewed,
bounded investigation contract before execution; #82 remains historical context.

## Portable evidence and recovery

`results.json` contains 12 local and 18 remote raw chronological sample sets, one local
canonical run, four final canonical OS runs, and two earlier CI WAL sets. All 37 sets
contain 50 finite nonnegative samples; sorted index 47 exactly matches retained p95.
Metrics, targets, statuses, phases and relevant source identities are retained. Original
relative source names and SHA-256 hashes identify the unsanitized source bytes; hashes
are provenance, not hashes of the sanitized projection. Local fixture paths, hostnames,
environment configuration, full logs and private sessions are excluded.

Full original logs, process/environment receipts and manifests remain in the original
machine's ignored `.agent-runs/issue-82/`; Git clone will not transfer them. The ignored
checkpoint contains historical OPEN/DRAFT statements superseded by the current status
above. Original local AgentFlow configuration references ignored files and is intentionally
not portable: use this machine's local paths and native tools; do not copy or reconstruct
that configuration or advance unrelated issue #74 state.

For available GitHub artifacts, run from the new checkout with authenticated `gh`.
Downloads are read-only and do not execute or rerun benchmarks:

```powershell
$runIds = 41246944440,41246965285,41250961308,41250964703,41251359828,41251363796,41317910822
foreach ($runId in $runIds) {
  gh run view $runId --repo smota/meshloop --json headSha,conclusion,event,url
  gh run download $runId --repo smota/meshloop --dir ".agent-runs/issue-82/download-$runId"
}
```

Artifacts can expire or be absent. If download fails, retain that limitation and use
this tracked numeric snapshot plus linked run/PR/issue records. Request specific archived
original files from the original machine when necessary; never rerun a consumed batch to
replace missing evidence. Review retrieved logs privately before publishing any excerpts.

## Copy-ready prompt for the other machine

```text
Resume from docs/engineering/evidence/issue-82/README.md and results.json in the
current Meshloop checkout. Read AGENTS.md and the relevant engineering workflow,
benchmark specification and accepted ADRs. Inspect branch/working-tree state and
preserve unrelated changes. Recheck live GitHub status: #82 was closed, PR87 merged
at 477c30a, main CI 41317910822 succeeded; earlier failures remain unresolved evidence.
Use local paths and authenticated native tools on this machine. Do not import the
original machine's AgentFlow configuration, ignored sessions or issue #74 state.
First inspect the tracked evidence and, if needed, download existing Actions artifacts
using the documented read-only commands. Report missing/expired artifacts honestly.
Never repeat the consumed local 12 or remote 12+6 diagnostic batches, restore the removed
CI job, manually rerun diagnostic jobs, recreate automation, weaken guards/thresholds,
or infer a root cause from passing runs. No benchmark/source execution is authorized
by this prompt. Before any new investigation, open a separately scoped GitHub follow-up
under repository policy and obtain review of a bounded protocol, evidence requirements,
execution budget and stop conditions. Use low effort for bounded discovery, medium
effort for builders, independent review and one writer per shared surface under the
repository workflow. Report what existing evidence supports and its
limits; request user direction for any execution outside that contract.
```
