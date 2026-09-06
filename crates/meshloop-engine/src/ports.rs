//! Trait definitions only (Ports and Adapters — design-patterns.md). No concrete
//! subprocess, Herdr, or SQLite code belongs here; meshloop-adapters implements these.

use std::collections::HashMap;

use meshloop_domain::capability::{HarnessError, HarnessProfile};
use meshloop_domain::evidence::{AttemptId, CandidateRef, Evidence};
use meshloop_domain::task_graph::Tier;

use crate::agent::AgentSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessHandle {
    pub attempt_id: AttemptId,
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

/// Repository pattern (design-patterns.md): engine depends only on this trait, never on
/// rusqlite types.
pub trait EvidenceStore {
    fn record(&mut self, evidence: Evidence) -> Result<(), StoreError>;
    fn evidence_for(&self, candidate: &CandidateRef) -> Vec<Evidence>;
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
    fn record_outcome(&mut self, key: &FeedbackKey, success: bool);
    fn counters(&self, key: &FeedbackKey) -> FeedbackCounters;
}

/// An in-memory `RoutingFeedbackStore` — the canonical fake for this port
/// (design-patterns.md's Test double / fake object pattern), reused by router and
/// orchestrator tests rather than each test rolling its own mock.
#[derive(Debug, Default)]
pub struct InMemoryFeedbackStore {
    counters: HashMap<FeedbackKey, FeedbackCounters>,
}

impl RoutingFeedbackStore for InMemoryFeedbackStore {
    fn record_outcome(&mut self, key: &FeedbackKey, success: bool) {
        let entry = self.counters.entry(key.clone()).or_default();
        if success {
            entry.success += 1;
        } else {
            entry.failure += 1;
        }
    }

    fn counters(&self, key: &FeedbackKey) -> FeedbackCounters {
        self.counters.get(key).copied().unwrap_or_default()
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

    fn evidence_for(&self, candidate: &CandidateRef) -> Vec<Evidence> {
        self.evidence
            .iter()
            .filter(|e| e.candidate() == candidate)
            .cloned()
            .collect()
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
        assert_eq!(store.counters(&key), FeedbackCounters::default());
        store.record_outcome(&key, true);
        store.record_outcome(&key, false);
        assert_eq!(
            store.counters(&key),
            FeedbackCounters {
                success: 1,
                failure: 1
            }
        );
    }
}
