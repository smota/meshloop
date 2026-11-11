//! Trait definitions only (Ports and Adapters — design-patterns.md). No concrete
//! subprocess, Herdr, or SQLite code belongs here; meshloop-adapters implements these.
//! `ScriptedDecisionPort` is an in-memory test double with no I/O, not an adapter.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use meshloop_domain::capability::{HarnessError, HarnessProfile, QuotaState};
use meshloop_domain::decision::{Confidence, DecisionOutcome};
use meshloop_domain::digest::ArtifactDigest;
use meshloop_domain::evidence::{AttemptId, CandidateRef, DeterministicEvidence, Evidence};
use meshloop_domain::state::{Event, PlanState, TaskState};
use meshloop_domain::task_graph::{TaskGraph, TaskId, Tier};

use crate::agent::AgentSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRecord {
    pub graph_id: String,
    pub task_id: TaskId,
    pub attempt_id: Option<AttemptId>,
    pub from: TaskState,
    pub to: TaskState,
    pub event: Event,
    pub reason: Option<String>,
    pub executor: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessHandle {
    pub attempt_id: AttemptId,
    pub pid: Option<u32>,
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessOutcome {
    pub exit_code: i32,
    /// Redacted stdout. Planner JSON is parsed from this field alone.
    pub output_redacted: String,
    /// Redacted stderr. Empty when the worker wrote none.
    pub stderr_redacted: String,
    pub worktree_changed: bool,
}

/// runtime-design.md §2: probe/invoke/cancel/collect, none assuming undiscovered behavior.
/// `Sync` so the orchestrator can probe every configured harness concurrently.
pub trait HarnessCapabilities: Sync {
    fn probe(&self) -> Result<HarnessProfile, HarnessError>;
    fn invoke(&self, spec: &AgentSpec) -> Result<HarnessHandle, HarnessError>;
    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError>;
    fn collect(&self, handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError>;
    /// Explicit opt-in: provisional drivers must never call a blocking fallback.
    fn supports_nonblocking_collection(&self) -> bool {
        false
    }
    /// Cancel only the exact retained owner and prove its complete tree quiescent.
    /// Unsupported adapters must not fall back to recorded PID cancellation.
    fn cancel_owned(
        &self,
        _handle: &HarnessHandle,
        _timeout: Duration,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::Unsupported)
    }
    /// Non-blocking poll for attempt outcome (ADR 0024).
    /// Returns Ok(Some(outcome)) when finished, Ok(None) when still executing, or Err on fault/timeout.
    fn try_collect(&self, handle: &HarnessHandle) -> Result<Option<HarnessOutcome>, HarnessError> {
        self.collect(handle).map(Some)
    }
    /// Whether the attempt's process/pane is still live. Herdr workers have no PID;
    /// the pane/agent is the process tree (execution-lifecycle.md).
    fn session_live(&self, handle: &HarnessHandle) -> LiveCheck {
        let _ = handle;
        LiveCheck::Ambiguous
    }

    fn pane_for_worktree(&self, worktree: &Path) -> Option<String> {
        let _ = worktree;
        None
    }

    /// OS image name of the process `invoke` spawns (e.g. `pwsh.exe`), persisted with the
    /// attempt pid so another process can tell a live worker from a reused pid. `None` when
    /// the harness has no process of its own (fixtures, panes).
    fn process_image(&self) -> Option<String> {
        None
    }

    /// Whether `invoke` passes the candidate's `model_ref` to the harness: true when the
    /// harness's argument template substitutes `{model_ref}` (#139). The engine never sees
    /// the template. Recorded once per attempt as `attempts.model_passed`.
    fn passes_model_ref(&self) -> bool {
        false
    }

    /// How this harness dispatches. No default: every harness states it, so a native
    /// harness (#147) cannot inherit `Subprocess`. The priors oracle gives a candidate
    /// whose harness is not a subprocess no prior (ADR 0038 #139 amendment, rule 3).
    fn dispatch_mode(&self) -> meshloop_domain::advisory::DispatchMode;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHandle {
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Exited(i32),
}

/// runtime-design.md §3: CLI-subprocess only in v1; panes are never a security boundary.
pub trait HerdrSessionPort {
    fn spawn(&self, spec: &AgentSpec) -> Result<SessionHandle, HarnessError>;
    fn status(&self, handle: &SessionHandle) -> Result<SessionStatus, HarnessError>;
    fn cancel(&self, handle: &SessionHandle) -> Result<(), HarnessError>;
    fn cleanup(&self, handle: &SessionHandle) -> Result<(), HarnessError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    ReadOnly,
    SchemaUpgradeRequired,
    LegacyOwnershipUnresolved,
    PlanRevisionMismatch,
    LifecycleBusy,
    /// The exclusive repository fence is held by another writer. Retry later.
    FenceBusy,
    /// Compare-and-append refused a task event (issue #167): `event` is not a legal
    /// transition from `from`, or `from` is not the task's persisted state, because
    /// another writer (a `cancel`, another driver) appended first. Nothing was written.
    /// The writer's view is stale; it must re-read the ledger, not retry blindly.
    TransitionRefused {
        task_id: TaskId,
        /// The task's state in the ledger at append time. `None` when it has no events.
        persisted: Option<TaskState>,
        from: TaskState,
        event: Event,
    },
    Io(String),
    Corrupt(String),
    /// The caller passed a value the store refuses to record (e.g. a bad `--as`
    /// identity). Nothing was written; the stored data is fine.
    InvalidInput(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    CommandFailed {
        stderr: String,
    },
    Io(String),
    Conflict,
    Dirty,
    /// A worktree's `.git` link no longer points at its own admin dir (worker output
    /// modified it). Git was not run there; `path` is that `.git` (#180).
    GitLinkModified {
        path: std::path::PathBuf,
        reason: String,
    },
    /// An entry named `.git` below the worktree's top level (a nested repository or
    /// link); `path` is worktree-relative. Nothing was staged (#180).
    NestedGit {
        path: std::path::PathBuf,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    Io(String),
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSummary {
    pub base: String,
    pub head: String,
    pub files: Vec<String>,
    pub stat_redacted: String,
    /// Gitlink (submodule) entries new or changed in `head` against `base` (#180).
    pub gitlinks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessHint {
    pub pid: u32,
    pub image_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveCheck {
    Live,
    Dead,
    Ambiguous,
}

pub trait ProcessView {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck;
}

pub trait CheckRunner {
    fn run(
        &self,
        worktree: &Path,
        argv: &[String],
        timeout: Duration,
    ) -> Result<DeterministicEvidence, CheckError>;
}

pub trait WorkspacePort {
    fn repo_root(&self) -> &Path;
    fn add_worktree_from(
        &self,
        path: &Path,
        branch: &str,
        start_point: &str,
    ) -> Result<(), WorkspaceError>;
    fn head(&self, worktree: &Path) -> Result<String, WorkspaceError>;
    fn diff_against(&self, worktree: &Path, base: &str) -> Result<DiffSummary, WorkspaceError>;
    fn status_porcelain(&self, worktree: &Path) -> Result<String, WorkspaceError>;
    /// Stages and commits everything in `worktree`. Refuses (`NestedGit`) a nested
    /// `.git` entry or a gitlink not in `base`, the attempt's start point (#180).
    fn commit_all(
        &self,
        worktree: &Path,
        base: &str,
        message: &str,
    ) -> Result<String, WorkspaceError>;
    fn merge_in_worktree(&self, worktree: &Path, from_ref: &str) -> Result<(), WorkspaceError>;
    /// Replays the commits in `base..head` onto `worktree`'s HEAD, oldest first. An empty
    /// range is a no-op. On conflict the cherry-pick is aborted and `Conflict` returned.
    fn cherry_pick_range(
        &self,
        worktree: &Path,
        base: &str,
        head: &str,
    ) -> Result<(), WorkspaceError>;
    fn branch_exists(&self, branch: &str) -> Result<bool, WorkspaceError>;
    fn worktree_exists(&self, path: &Path) -> bool;
    fn prune(&self) -> Result<(), WorkspaceError>;
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, WorkspaceError>;
    fn checkout_ref(&self, git_ref: &str) -> Result<(), WorkspaceError>;
    fn merge_ff_only(&self, from: &str) -> Result<(), WorkspaceError>;
    fn merge_no_ff(&self, from: &str) -> Result<(), WorkspaceError>;
    fn current_head(&self) -> Result<String, WorkspaceError>;
    fn reset_hard(&self, worktree: &Path, rev: &str) -> Result<(), WorkspaceError>;
    fn remove_file(&self, worktree: &Path, rel: &str) -> Result<(), WorkspaceError>;
    fn unified_diff(&self, worktree: &Path, base: &str) -> Result<String, WorkspaceError>;
    fn remove_worktree(&self, path: &Path) -> Result<(), WorkspaceError>;
    fn delete_branch(&self, branch: &str) -> Result<(), WorkspaceError>;
    /// Symbolic ref the main checkout is on (`refs/heads/<name>`), or `None` when
    /// HEAD is detached or unreadable. Read-only.
    fn current_branch(&self) -> Result<Option<String>, WorkspaceError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewError {
    Transport(String),
    OriginPane,
    Timeout,
}

/// Live Herdr worker/reviewer transport. Never split the origin supervisor pane.
pub trait ReviewTransport {
    fn split_pane(&self, cwd: &Path, avoid_pane: Option<&str>) -> Result<String, ReviewError>;
    fn start_agent(&self, name: &str, kind: &str, pane_id: &str) -> Result<(), ReviewError>;
    fn prompt_and_wait(
        &self,
        target: &str,
        prompt: &str,
        timeout_ms: u64,
    ) -> Result<String, ReviewError>;
    fn read_output(&self, target: &str) -> Result<String, ReviewError>;
    fn close_pane(&self, pane_id: &str) -> Result<(), ReviewError>;
}

/// Detached `run` driver identity persisted beside the run row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverLiveness {
    pub graph_id: String,
    pub pid: u32,
    pub started_at: String,
    pub heartbeat_at: String,
    pub exit_code: Option<i32>,
    pub log_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub graph_id: String,
    pub plan_state: PlanState,
    pub run_base: String,
    pub integrate_ref: String,
    pub plan_json: String,
    pub plan_id: String,
    pub created_at: String,
    pub review_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRow {
    pub attempt_id: AttemptId,
    pub graph_id: String,
    pub task_id: TaskId,
    pub harness: Option<String>,
    pub model_ref: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub pid: Option<u32>,
    pub image_name: Option<String>,
    pub pane_id: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub outcome: Option<String>,
    /// Whether dispatch passed `model_ref` to the harness (#139). Written when the row is
    /// created and never changed after; `None` on rows written before #139.
    pub model_passed: Option<bool>,
    /// The dispatch mode the row records. A placeholder until the store has a mode
    /// column (#147); read only through `DispatchMode::of_recorded_attempt`.
    pub dispatch: meshloop_domain::advisory::RecordedDispatch,
}

/// One worker attempt routed while advisory priors were in effect (#139). Written in the
/// attempt row's transaction. Never an Evidence row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryRouting {
    pub attempt_id: AttemptId,
    pub priors_sha256: String,
    /// The dispatched candidate differs from the one dispatch would pick with the oracle
    /// off, already-spent candidates excluded in both.
    pub changed_top: bool,
}

/// A promoted priors file as routing reads it: one read of the latest ledger row,
/// minus every entry that row's `accept` ignored and every entry the loader ignores.
#[derive(Debug, Clone, PartialEq)]
pub struct PriorsSnapshot {
    /// The ledger row's sequence number.
    pub sequence: i64,
    pub priors_sha256: String,
    pub entries: Vec<meshloop_domain::advisory::PriorEntry>,
}

/// Engine port for promoted advisory priors (ADR 0038 #139 amendment, rule 4).
/// `Ok(None)` means no priors are in effect: no row, a revoke, or a file-level fault.
/// Routing never re-resolves provenance; the decision made at `accept` is final.
pub trait AdvisoryPriorsSource {
    fn snapshot(&self) -> Result<Option<PriorsSnapshot>, StoreError>;
    /// Reports a `snapshot` fault the engine routes past without priors. Adapters
    /// warn and record telemetry; the engine does no I/O.
    fn report_fault(&self, fault: &StoreError);
}

/// Repository pattern (design-patterns.md): engine depends only on this trait, never on
/// rusqlite types.
pub trait EvidenceStore {
    fn record(&mut self, evidence: Evidence) -> Result<(), StoreError>;
    fn evidence_for(&self, candidate: &CandidateRef) -> Result<Vec<Evidence>, StoreError>;
    /// Every evidence row recorded for an attempt, whatever revision it was bound to
    /// (read-only display; gating still uses the exact-candidate `evidence_for`).
    fn evidence_for_attempt(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Vec<Evidence>, StoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedbackKey {
    pub harness: String,
    pub model_ref: String,
    pub tier: TierKey,
}

/// A hashable mirror of `Tier` — `Tier` itself stays free of derive bloat it doesn't need
/// elsewhere in meshloop-domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TierKey {
    Tier1,
    Tier2,
    Tier3,
}

impl From<Tier> for TierKey {
    fn from(t: Tier) -> Self {
        match t {
            Tier::Tier1 => TierKey::Tier1,
            Tier::Tier2 => TierKey::Tier2,
            Tier::Tier3 => TierKey::Tier3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedbackCounters {
    pub success: u32,
    pub failure: u32,
}

pub trait RoutingFeedbackStore {
    fn record_outcome(&mut self, key: &FeedbackKey, success: bool) -> Result<(), StoreError>;
    fn counters(&self, key: &FeedbackKey) -> Result<FeedbackCounters, StoreError>;
}

pub trait EventLog {
    fn append(&mut self, record: TransitionRecord) -> Result<(), StoreError>;
    fn records_for_graph(&self, graph_id: &str) -> Result<Vec<TransitionRecord>, StoreError>;
}

pub trait QuotaStore {
    fn load_quota(&self, harness: &str) -> Result<QuotaState, StoreError>;
    fn save_quota(&mut self, harness: &str, state: &QuotaState) -> Result<(), StoreError>;
}

/// Process-local oracle budget in integer micro-dollars. Not persisted and not a
/// routing arm. A zero cost consults the oracle without changing `remaining`.
#[derive(Debug)]
pub struct SessionSpendLedger {
    remaining: Cell<u64>,
    cost: u64,
}

impl SessionSpendLedger {
    pub fn new(budget_micro_dollars: u64, cost_micro_dollars: u64) -> Self {
        Self {
            remaining: Cell::new(budget_micro_dollars),
            cost: cost_micro_dollars,
        }
    }

    pub fn remaining(&self) -> u64 {
        self.remaining.get()
    }

    pub fn cost(&self) -> u64 {
        self.cost
    }

    /// Subtracts `cost` when the budget covers it. Insufficient budget fails open:
    /// `remaining` is left unchanged and the caller must not treat that as an error.
    #[must_use]
    pub fn try_charge(&self) -> bool {
        if self.cost == 0 {
            return true;
        }
        let remaining = self.remaining.get();
        if remaining < self.cost {
            return false;
        }
        self.remaining.set(remaining - self.cost);
        true
    }
}

pub trait RunStore:
    EvidenceStore + RoutingFeedbackStore + EventLog + QuotaStore + crate::workspace::RuntimeLedger
{
    fn save_run(&mut self, row: &RunRow) -> Result<(), StoreError>;
    fn save_run_and_events(
        &mut self,
        row: &RunRow,
        events: &[TransitionRecord],
    ) -> Result<(), StoreError>;
    fn load_run(&self, graph_id: &str) -> Result<Option<RunRow>, StoreError>;
    fn latest_run(&self) -> Result<Option<RunRow>, StoreError>;
    fn list_runs(&self) -> Result<Vec<RunRow>, StoreError>;
    fn next_attempt_id(&self) -> Result<AttemptId, StoreError>;
    fn save_attempt(&mut self, row: &AttemptRow) -> Result<(), StoreError>;
    /// `save_attempt`, plus the attempt's `advisory_routing` row in the same transaction
    /// when priors were in effect. A store that cannot record routing refuses it.
    fn save_attempt_routed(
        &mut self,
        row: &AttemptRow,
        routing: Option<&AdvisoryRouting>,
    ) -> Result<(), StoreError> {
        match routing {
            None => self.save_attempt(row),
            Some(_) => Err(StoreError::Io(
                "this store cannot record advisory routing".into(),
            )),
        }
    }
    fn load_attempt(&self, attempt_id: AttemptId) -> Result<Option<AttemptRow>, StoreError>;
    fn attempts_for_task(
        &self,
        graph_id: &str,
        task_id: TaskId,
    ) -> Result<Vec<AttemptRow>, StoreError>;
    fn latest_attempt_for_task(
        &self,
        graph_id: &str,
        task_id: TaskId,
    ) -> Result<Option<AttemptRow>, StoreError>;
    fn attempts_for_graph(&self, graph_id: &str) -> Result<Vec<AttemptRow>, StoreError>;
    fn reset_graph_execution(&mut self, graph_id: &str) -> Result<(), StoreError>;
    fn update_attempt_pid(
        &mut self,
        attempt_id: AttemptId,
        pid: Option<u32>,
        image_name: Option<&str>,
    ) -> Result<(), StoreError>;
    fn update_attempt_pane(
        &mut self,
        attempt_id: AttemptId,
        pane_id: Option<&str>,
    ) -> Result<(), StoreError>;
    fn upsert_driver(&mut self, _row: &DriverLiveness) -> Result<(), StoreError> {
        Ok(())
    }
    fn record_driver_heartbeat(
        &mut self,
        _graph_id: &str,
        _pid: u32,
        _at: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    fn record_driver_exit(
        &mut self,
        _graph_id: &str,
        _pid: u32,
        _code: i32,
        _at: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    fn load_driver(&self, _graph_id: &str) -> Result<Option<DriverLiveness>, StoreError> {
        Ok(None)
    }
    fn graph_from_run(&self, graph_id: &str) -> Result<TaskGraph, StoreError> {
        let row = self
            .load_run(graph_id)?
            .ok_or_else(|| StoreError::Corrupt(format!("no run {graph_id}")))?;
        serde_json::from_str(&row.plan_json).map_err(|e| StoreError::Corrupt(e.to_string()))
    }
    fn integrity_check(&self) -> Result<bool, StoreError> {
        Ok(true)
    }
    fn event_count(&self) -> Result<usize, StoreError> {
        Ok(0)
    }

    /// In-process oracle spend, if this store keeps a ledger. Not a schema field.
    /// Test doubles return `None` and therefore never charge.
    fn session_spend(&self) -> Option<&SessionSpendLedger> {
        None
    }

    /// Promoted advisory priors, if this store keeps a promotion ledger (#139).
    /// Test doubles return `None`, so routing is unchanged.
    fn advisory_priors(&self) -> Option<&dyn AdvisoryPriorsSource> {
        None
    }
}

/// An in-memory `RoutingFeedbackStore` — the canonical fake for this port
/// (design-patterns.md's Test double / fake object pattern), reused by router and
/// orchestrator tests rather than each test rolling its own mock.
#[derive(Debug, Default)]
pub struct InMemoryFeedbackStore {
    counters: HashMap<FeedbackKey, FeedbackCounters>,
}

impl RoutingFeedbackStore for InMemoryFeedbackStore {
    fn record_outcome(&mut self, key: &FeedbackKey, success: bool) -> Result<(), StoreError> {
        let entry = self.counters.entry(key.clone()).or_default();
        if success {
            entry.success += 1;
        } else {
            entry.failure += 1;
        }
        Ok(())
    }

    fn counters(&self, key: &FeedbackKey) -> Result<FeedbackCounters, StoreError> {
        Ok(self.counters.get(key).copied().unwrap_or_default())
    }
}

/// The canonical in-memory `EvidenceStore` fake.
#[derive(Debug, Default)]
pub struct InMemoryEvidenceStore {
    evidence: Vec<Evidence>,
}

impl EvidenceStore for InMemoryEvidenceStore {
    fn record(&mut self, evidence: Evidence) -> Result<(), StoreError> {
        self.evidence.push(evidence);
        Ok(())
    }

    fn evidence_for(&self, candidate: &CandidateRef) -> Result<Vec<Evidence>, StoreError> {
        Ok(self
            .evidence
            .iter()
            .filter(|e| e.candidate() == candidate)
            .cloned()
            .collect())
    }

    fn evidence_for_attempt(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Vec<Evidence>, StoreError> {
        Ok(self
            .evidence
            .iter()
            .filter(|e| {
                let c = e.candidate();
                c.task_id == task_id && c.attempt_id == attempt_id
            })
            .cloned()
            .collect())
    }
}

/// Failure of a decision port. Not a domain `DecisionError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionPortError {
    EmptyQuery,
    BatchTooLarge {
        len: usize,
    },
    Unscripted {
        fingerprint: String,
    },
    /// The priors oracle has no prior for this arm (#139): the router keeps the local mean.
    NoPrior,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceQuery {
    pub prompt: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoulQuery {
    pub proposition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreQuery {
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchNoulQuery {
    pub propositions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleQuery {
    pub arm: String,
    pub pulls: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OracleOutcome {
    pub prior: Confidence,
}

/// System 1 choice, noul, score, and batched noul queries. No transport.
pub trait DecisionPort {
    fn evaluate_choice(&self, query: &ChoiceQuery) -> Result<DecisionOutcome, DecisionPortError>;
    fn evaluate_noul(&self, query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError>;
    fn evaluate_score(&self, query: &ScoreQuery) -> Result<DecisionOutcome, DecisionPortError>;
    fn evaluate_batch_noul(
        &self,
        query: &BatchNoulQuery,
    ) -> Result<Vec<DecisionOutcome>, DecisionPortError>;
}

/// Prior for a predictive-success arm. Not a routing arm.
pub trait PredictiveSuccessOracle {
    fn predict(&self, query: &OracleQuery) -> Result<OracleOutcome, DecisionPortError>;
}

const MAX_BATCH_PROPOSITIONS: usize = 24;

/// SHA-256 hex of `label`, then `\n` and each part, with no trailing newline.
pub fn decision_fingerprint(label: &str, parts: &[&str]) -> String {
    let mut bytes =
        Vec::with_capacity(label.len() + parts.iter().map(|part| 1 + part.len()).sum::<usize>());
    bytes.extend_from_slice(label.as_bytes());
    for part in parts {
        bytes.push(b'\n');
        bytes.extend_from_slice(part.as_bytes());
    }
    ArtifactDigest::sha256(&bytes).digest
}

/// In-memory test double for [`DecisionPort`] and [`PredictiveSuccessOracle`].
/// No file, network, clock, or process I/O. Not an adapter.
#[derive(Debug, Default)]
pub struct ScriptedDecisionPort {
    outcomes: HashMap<String, DecisionOutcome>,
    oracles: HashMap<String, OracleOutcome>,
}

impl ScriptedDecisionPort {
    pub fn insert(&mut self, fingerprint: String, outcome: DecisionOutcome) {
        self.outcomes.insert(fingerprint, outcome);
    }

    pub fn insert_oracle(&mut self, fingerprint: String, outcome: OracleOutcome) {
        self.oracles.insert(fingerprint, outcome);
    }

    fn scripted(&self, fingerprint: String) -> Result<DecisionOutcome, DecisionPortError> {
        self.outcomes
            .get(&fingerprint)
            .cloned()
            .ok_or(DecisionPortError::Unscripted { fingerprint })
    }
}

impl DecisionPort for ScriptedDecisionPort {
    fn evaluate_choice(&self, query: &ChoiceQuery) -> Result<DecisionOutcome, DecisionPortError> {
        if query.prompt.is_empty()
            || query.options.is_empty()
            || query.options.iter().any(String::is_empty)
        {
            return Err(DecisionPortError::EmptyQuery);
        }
        let mut parts = Vec::with_capacity(1 + query.options.len());
        parts.push(query.prompt.as_str());
        parts.extend(query.options.iter().map(String::as_str));
        self.scripted(decision_fingerprint("choice", &parts))
    }

    fn evaluate_noul(&self, query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError> {
        if query.proposition.is_empty() {
            return Err(DecisionPortError::EmptyQuery);
        }
        self.scripted(decision_fingerprint("noul", &[query.proposition.as_str()]))
    }

    fn evaluate_score(&self, query: &ScoreQuery) -> Result<DecisionOutcome, DecisionPortError> {
        if query.subject.is_empty() {
            return Err(DecisionPortError::EmptyQuery);
        }
        self.scripted(decision_fingerprint("score", &[query.subject.as_str()]))
    }

    fn evaluate_batch_noul(
        &self,
        query: &BatchNoulQuery,
    ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
        if query.propositions.is_empty() || query.propositions.iter().any(String::is_empty) {
            return Err(DecisionPortError::EmptyQuery);
        }
        if query.propositions.len() > MAX_BATCH_PROPOSITIONS {
            return Err(DecisionPortError::BatchTooLarge {
                len: query.propositions.len(),
            });
        }
        let parts: Vec<&str> = query.propositions.iter().map(String::as_str).collect();
        let outcome = self.scripted(decision_fingerprint("batch-noul", &parts))?;
        // The map stores one outcome per fingerprint. Apply it to every proposition.
        Ok(vec![outcome; query.propositions.len()])
    }
}

impl PredictiveSuccessOracle for ScriptedDecisionPort {
    fn predict(&self, query: &OracleQuery) -> Result<OracleOutcome, DecisionPortError> {
        if query.arm.is_empty() {
            return Err(DecisionPortError::EmptyQuery);
        }
        let pulls = query.pulls.to_string();
        let fingerprint = decision_fingerprint("oracle", &[query.arm.as_str(), pulls.as_str()]);
        self.oracles
            .get(&fingerprint)
            .copied()
            .ok_or(DecisionPortError::Unscripted { fingerprint })
    }
}

#[cfg(test)]
mod tests {
    use meshloop_domain::decision::SanitizedReason;
    use meshloop_domain::digest::ArtifactDigest;

    use super::*;

    fn spec_digest(label: &str, parts: &[&str]) -> String {
        let mut bytes = label.as_bytes().to_vec();
        for part in parts {
            bytes.push(b'\n');
            bytes.extend_from_slice(part.as_bytes());
        }
        ArtifactDigest::sha256(&bytes).digest
    }

    #[test]
    fn in_memory_feedback_store_accumulates_counters() {
        let mut store = InMemoryFeedbackStore::default();
        let key = FeedbackKey {
            harness: "claude-code".into(),
            model_ref: "configured".into(),
            tier: TierKey::Tier2,
        };
        assert_eq!(store.counters(&key).unwrap(), FeedbackCounters::default());
        store.record_outcome(&key, true).unwrap();
        store.record_outcome(&key, false).unwrap();
        assert_eq!(
            store.counters(&key).unwrap(),
            FeedbackCounters {
                success: 1,
                failure: 1
            }
        );
    }

    #[test]
    fn decision_port_rejects_empty_queries_without_inserting() {
        let port = ScriptedDecisionPort::default();

        assert_eq!(
            port.evaluate_choice(&ChoiceQuery {
                prompt: String::new(),
                options: vec!["keep".into()],
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            port.evaluate_choice(&ChoiceQuery {
                prompt: "pick".into(),
                options: Vec::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            port.evaluate_choice(&ChoiceQuery {
                prompt: "pick".into(),
                options: vec!["keep".into(), String::new()],
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            port.evaluate_noul(&NoulQuery {
                proposition: String::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            port.evaluate_score(&ScoreQuery {
                subject: String::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            port.evaluate_batch_noul(&BatchNoulQuery {
                propositions: Vec::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            port.evaluate_batch_noul(&BatchNoulQuery {
                propositions: vec!["ok".into(), String::new()],
            }),
            Err(DecisionPortError::EmptyQuery)
        );

        assert_eq!(MAX_BATCH_PROPOSITIONS, 24);
        let too_many: Vec<String> = (0..=MAX_BATCH_PROPOSITIONS)
            .map(|i| format!("p{i}"))
            .collect();
        assert_eq!(too_many.len(), 25);
        assert_eq!(
            port.evaluate_batch_noul(&BatchNoulQuery {
                propositions: too_many,
            }),
            Err(DecisionPortError::BatchTooLarge { len: 25 })
        );

        let at_limit: Vec<String> = (0..MAX_BATCH_PROPOSITIONS)
            .map(|i| format!("p{i}"))
            .collect();
        let limit_parts: Vec<&str> = at_limit.iter().map(String::as_str).collect();
        let limit_fp = decision_fingerprint("batch-noul", &limit_parts);
        assert_eq!(
            port.evaluate_batch_noul(&BatchNoulQuery {
                propositions: at_limit,
            }),
            Err(DecisionPortError::Unscripted {
                fingerprint: limit_fp,
            })
        );

        assert_eq!(
            port.predict(&OracleQuery {
                arm: String::new(),
                pulls: 1,
            }),
            Err(DecisionPortError::EmptyQuery)
        );

        assert!(port.outcomes.is_empty());
        assert!(port.oracles.is_empty());
    }

    #[test]
    fn decision_port_choice_fingerprint_returns_only_scripted_outcome() {
        let query = ChoiceQuery {
            prompt: "pick".into(),
            options: vec!["keep".into(), "drop".into()],
        };
        let same = ChoiceQuery {
            prompt: "pick".into(),
            options: vec!["keep".into(), "drop".into()],
        };
        let parts = ["pick", "keep", "drop"];
        let fingerprint = decision_fingerprint("choice", &parts);
        assert_eq!(fingerprint, decision_fingerprint("choice", &parts));
        assert_eq!(fingerprint, spec_digest("choice", &parts));
        assert_eq!(fingerprint.len(), 64);
        assert!(
            fingerprint
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(
            fingerprint,
            ArtifactDigest::sha256(b"choice\npick\nkeep\ndrop\n").digest
        );

        let outcome = DecisionOutcome::try_selected("keep").unwrap();
        let mut port = ScriptedDecisionPort::default();
        port.insert(fingerprint.clone(), outcome.clone());
        assert_eq!(port.evaluate_choice(&query).unwrap(), outcome);
        assert_eq!(port.evaluate_choice(&same).unwrap(), outcome);

        let other = ChoiceQuery {
            prompt: "pick".into(),
            options: vec!["drop".into(), "keep".into()],
        };
        let other_fp = decision_fingerprint("choice", &["pick", "drop", "keep"]);
        assert_ne!(other_fp, fingerprint);
        assert_eq!(
            port.evaluate_choice(&other),
            Err(DecisionPortError::Unscripted {
                fingerprint: other_fp,
            })
        );
    }

    #[test]
    fn decision_port_choice_prompt_change_is_unscripted() {
        let fingerprint = decision_fingerprint("choice", &["pick", "keep"]);
        let mut port = ScriptedDecisionPort::default();
        port.insert(fingerprint, DecisionOutcome::Boolean(true));
        let changed_fp = decision_fingerprint("choice", &["rick", "keep"]);
        assert_ne!(
            changed_fp,
            decision_fingerprint("choice", &["pick", "keep"])
        );
        assert_eq!(
            port.evaluate_choice(&ChoiceQuery {
                prompt: "rick".into(),
                options: vec!["keep".into()],
            }),
            Err(DecisionPortError::Unscripted {
                fingerprint: changed_fp,
            })
        );
    }

    #[test]
    fn decision_port_choice_returns_scripted_abstain() {
        let reason = SanitizedReason::try_new("needs a human").unwrap();
        let outcome = DecisionOutcome::abstain(reason.clone());
        let fingerprint = decision_fingerprint("choice", &["pick", "keep"]);
        let mut port = ScriptedDecisionPort::default();
        port.insert(fingerprint, outcome);
        match port
            .evaluate_choice(&ChoiceQuery {
                prompt: "pick".into(),
                options: vec!["keep".into()],
            })
            .unwrap()
        {
            DecisionOutcome::Abstain(got) => assert_eq!(got.as_str(), reason.as_str()),
            other => panic!("expected Abstain, got {other:?}"),
        }
    }

    #[test]
    fn decision_port_predict_matches_arm_and_pulls() {
        let fingerprint = decision_fingerprint("oracle", &["repair", "10"]);
        assert_eq!(fingerprint, spec_digest("oracle", &["repair", "10"]));
        let prior = Confidence::try_new(0.5).unwrap();
        let mut port = ScriptedDecisionPort::default();
        port.insert_oracle(fingerprint, OracleOutcome { prior });
        assert_eq!(
            port.predict(&OracleQuery {
                arm: "repair".into(),
                pulls: 10,
            })
            .unwrap()
            .prior,
            prior
        );
        let changed_fp = decision_fingerprint("oracle", &["repair", "11"]);
        assert_eq!(
            port.predict(&OracleQuery {
                arm: "repair".into(),
                pulls: 11,
            }),
            Err(DecisionPortError::Unscripted {
                fingerprint: changed_fp,
            })
        );
    }

    #[test]
    fn decision_port_noul_score_and_batch_use_fingerprints() {
        let yes = DecisionOutcome::Boolean(true);
        let scored = DecisionOutcome::try_score(1.5).unwrap();
        let mut port = ScriptedDecisionPort::default();
        port.insert(decision_fingerprint("noul", &["safe"]), yes.clone());
        port.insert(decision_fingerprint("score", &["diff"]), scored.clone());
        port.insert(decision_fingerprint("batch-noul", &["a", "b"]), yes.clone());

        assert_eq!(
            port.evaluate_noul(&NoulQuery {
                proposition: "safe".into(),
            })
            .unwrap(),
            yes
        );
        assert_eq!(
            port.evaluate_score(&ScoreQuery {
                subject: "diff".into(),
            })
            .unwrap(),
            scored
        );
        assert_eq!(
            port.evaluate_batch_noul(&BatchNoulQuery {
                propositions: vec!["a".into(), "b".into()],
            })
            .unwrap(),
            vec![yes.clone(), yes]
        );
        assert_eq!(
            port.evaluate_noul(&NoulQuery {
                proposition: "safer".into(),
            }),
            Err(DecisionPortError::Unscripted {
                fingerprint: decision_fingerprint("noul", &["safer"]),
            })
        );
    }
}
