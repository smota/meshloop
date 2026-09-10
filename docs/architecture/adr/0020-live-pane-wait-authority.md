# 0020 Live Herdr pane is the wait authority

- Status: Accepted
- Implementation: implemented
- Date: 2026-09-08
- Accepted: 2026-09-08
- Author/executor: Grok, under human product direction
- Decision owner: Samuel
- Approval evidence: user report 2026-09-08 that `/meshloop:status` showed Failed after `agent_prompt_stalled` while the Grok worker pane was still working; engine retried Claude (`agent_not_ready`) and went FailedTerminal
- Supersedes: none
- Superseded by: none

## Context
execution-lifecycle.md: `Running → Failed` via `HarnessCrashedOrTimeout` only when the
**owned process tree is confirmed terminated**. Herdr workers have no PID;
the pane/agent is that process tree.

`herdr agent prompt --wait` has a 5s first-change gate (`agent_prompt_stalled`).
Meshloop treated that as a crash, appended Failed, retried another harness, and
left the original pane thinking. Status (the event ledger) and Herdr (the pane)
then showed two clocks.

ADR 0019 already forbids using that gate as a plan-dispatch failure. The same
split happens on `run`.

## Decision
1. `invoke` places the pane, starts the agent, and **submits** the prompt.
   `collect` waits. Pane id is stored before the wait so status can show it.
2. Wait while Herdr reports `working` or `blocked`. `task_timeout_seconds` is
   an **inactivity** bound (never started working), not a cap on thinking time.
3. `HarnessCrashedOrTimeout` only after the pane/agent is gone. Do not in-run
   fallback while a previous attempt pane is live.
4. `status` overlays `pane_id` and `live` from Herdr. Failed + live pane is
   called out; `resume` waits on that pane and harvests via `LiveWorkerSettled`
   (`Failed → Verifying`). Dependents blocked on a recovered upstream are
   cleared with `DependencyCleared` (`Blocked → Pending`).
5. `crash_fail` / resume must not treat `pid: None` as dead when `pane_id` is
   live.

## Consequences
Fixture subprocess wait is unchanged (PID + kill on timeout). Live workers can
run longer than `task_timeout_seconds` while Herdr says `working`. Cancel still
closes the pane.

## Verification
Unit: `keep_waiting_for_agent` never times out on `working`/`blocked`; times out
only when never working. Domain: `Failed + LiveWorkerSettled → Verifying`,
`Blocked + DependencyCleared → Pending`. `xtask check`.
