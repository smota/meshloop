//! Trait definitions only (Ports and Adapters — design-patterns.md). No concrete
//! subprocess, Herdr, or SQLite code belongs here; meshloop-adapters implements these.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use meshloop_domain::capability::{HarnessError, HarnessProfile, QuotaState};
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
    pub output_redacted: String,
    pub worktree_changed: bool,
}

/// runtime-design.md §2: probe/invoke/cancel/collect, none assuming undiscovered behavior.
pub trait HarnessCapabilities {
    fn probe(&self) -> Result<HarnessProfile, HarnessError>;
    fn invoke(&self, spec: &AgentSpec) -> Result<HarnessHandle, HarnessError>;
    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError>;
    fn collect(&self, handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError>;
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
    Io(String),
    Corrupt(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    CommandFailed { stderr: String },
    Io(String),
    Conflict,
    Dirty,
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
    fn commit_all(&self, worktree: &Path, message: &str) -> Result<String, WorkspaceError>;
    fn merge_in_worktree(&self, worktree: &Path, from_ref: &str) -> Result<(), WorkspaceError>;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub graph_id: String,
    pub plan_state: PlanState,
    pub run_base: String,
    pub integrate_ref: String,
    pub plan_json: String,
    pub plan_sha256: String,
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
}

/// Repository pattern (design-patterns.md): engine depends only on this trait, never on
/// rusqlite types.
pub trait EvidenceStore {
    fn record(&mut self, evidence: Evidence) -> Result<(), StoreError>;
    fn evidence_for(&self, candidate: &CandidateRef) -> Result<Vec<Evidence>, StoreError>;
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

pub trait RunStore: EvidenceStore + RoutingFeedbackStore + EventLog + QuotaStore {
    fn save_run(&mut self, row: &RunRow) -> Result<(), StoreError>;
    fn load_run(&self, graph_id: &str) -> Result<Option<RunRow>, StoreError>;
    fn latest_run(&self) -> Result<Option<RunRow>, StoreError>;
    fn list_runs(&self) -> Result<Vec<RunRow>, StoreError>;
    fn next_attempt_id(&self) -> Result<AttemptId, StoreError>;
    fn save_attempt(&mut self, row: &AttemptRow) -> Result<(), StoreError>;
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
    fn graph_from_run(&self, graph_id: &str) -> Result<TaskGraph, StoreError> {
        let row = self
            .load_run(graph_id)?
            .ok_or_else(|| StoreError::Corrupt(format!("no run {graph_id}")))?;
        serde_json::from_str(&row.plan_json).map_err(|e| StoreError::Corrupt(e.to_string()))
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
