//! Orchestration saga (design-patterns.md): the single coordinator for QACR's step 5
//! (dispatch with visible fallback). Adapters and agents never decide the next step or
//! talk to each other directly.

use std::collections::HashMap;

use meshloop_domain::capability::HarnessError;
use meshloop_domain::state::TaskState;
use meshloop_domain::task_graph::Tier;

use crate::agent::AgentSpec;
use crate::ports::{
    FeedbackKey, HarnessCapabilities, HarnessOutcome, RoutingFeedbackStore, TierKey,
};
use crate::router::Candidate;

#[derive(Debug)]
pub enum OrchestratorError {
    UnknownHarness(String),
    Unsupported,
}

#[derive(Debug)]
pub struct DispatchResult {
    pub state: TaskState,
    pub outcome: Option<HarnessOutcome>,
    pub used_harness: Option<String>,
}

/// Drives QACR's fallback loop over an already-ordered (best-first) candidate list from
/// `Router::select`. On `CapacityExhausted`/`Timeout`/`ProcessFault` it records feedback
/// and tries the next candidate — visibly, since each attempt is observable by the
/// caller's event log, never silently. Exhausting every candidate resolves to `Blocked`,
/// never a crash or a silent stall.
pub fn dispatch_with_fallback(
    harnesses: &HashMap<String, &dyn HarnessCapabilities>,
    candidates: &[&Candidate],
    tier: Tier,
    spec_for: impl Fn(&Candidate) -> AgentSpec,
    feedback: &mut dyn RoutingFeedbackStore,
) -> Result<DispatchResult, OrchestratorError> {
    for candidate in candidates {
        let harness = harnesses
            .get(&candidate.harness)
            .ok_or_else(|| OrchestratorError::UnknownHarness(candidate.harness.clone()))?;
        let spec = spec_for(candidate);
        let key = FeedbackKey {
            harness: candidate.harness.clone(),
            model_ref: candidate.model_ref.clone(),
            tier: TierKey::from(tier),
        };

        let invoke_result = harness.invoke(&spec);
        let outcome = match invoke_result {
            Ok(handle) => harness.collect(&handle),
            Err(e) => Err(e),
        };

        match outcome {
            Ok(outcome) => {
                feedback.record_outcome(&key, true);
                return Ok(DispatchResult {
                    state: TaskState::Verifying,
                    outcome: Some(outcome),
                    used_harness: Some(candidate.harness.clone()),
                });
            }
            Err(HarnessError::CapacityExhausted { .. })
            | Err(HarnessError::Timeout)
            | Err(HarnessError::ProcessFault { .. }) => {
                feedback.record_outcome(&key, false);
                continue;
            }
            Err(HarnessError::Unsupported) => return Err(OrchestratorError::Unsupported),
        }
    }

    Ok(DispatchResult {
        state: TaskState::Blocked,
        outcome: None,
        used_harness: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{HarnessHandle, InMemoryFeedbackStore};
    use meshloop_domain::evidence::AttemptId;
    use meshloop_domain::policy::ModelCapabilityTier;
    use std::path::PathBuf;
    use std::time::Duration;

    struct ScriptedHarness {
        script: std::cell::RefCell<Vec<Result<HarnessOutcome, HarnessError>>>,
    }

    impl HarnessCapabilities for ScriptedHarness {
        fn probe(&self) -> Result<meshloop_domain::capability::HarnessProfile, HarnessError> {
            unimplemented!()
        }
        fn invoke(&self, _spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
            Ok(HarnessHandle {
                attempt_id: AttemptId(1),
            })
        }
        fn cancel(&self, _handle: &HarnessHandle) -> Result<(), HarnessError> {
            Ok(())
        }
        fn collect(&self, _handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
            self.script.borrow_mut().remove(0)
        }
    }

    fn candidate(harness: &str) -> Candidate {
        Candidate {
            harness: harness.into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }
    }

    fn spec_for(_c: &Candidate) -> AgentSpec {
        AgentSpec {
            task_id: meshloop_domain::task_graph::TaskId(1),
            attempt_id: AttemptId(1),
            harness: "x".into(),
            model_ref: "m".into(),
            worktree_path: PathBuf::from("/tmp"),
            prompt: "p".into(),
            timeout: Duration::from_secs(60),
        }
    }

    #[test]
    fn capacity_exhausted_falls_back_to_next_candidate_visibly() {
        let primary = ScriptedHarness {
            script: std::cell::RefCell::new(vec![Err(HarnessError::CapacityExhausted {
                retry_after: Duration::from_secs(60),
            })]),
        };
        let fallback = ScriptedHarness {
            script: std::cell::RefCell::new(vec![Ok(HarnessOutcome {
                exit_code: 0,
                output_redacted: "done".into(),
                worktree_changed: true,
            })]),
        };
        let mut harnesses: HashMap<String, &dyn HarnessCapabilities> = HashMap::new();
        harnesses.insert("codex".into(), &primary);
        harnesses.insert("claude-code".into(), &fallback);

        let c1 = candidate("codex");
        let c2 = candidate("claude-code");
        let candidates = vec![&c1, &c2];
        let mut feedback = InMemoryFeedbackStore::default();

        let result = dispatch_with_fallback(
            &harnesses,
            &candidates,
            Tier::Tier1,
            spec_for,
            &mut feedback,
        )
        .expect("should not error");

        assert_eq!(result.state, TaskState::Verifying);
        assert_eq!(result.used_harness, Some("claude-code".into()));

        let failed_key = FeedbackKey {
            harness: "codex".into(),
            model_ref: "m".into(),
            tier: TierKey::Tier1,
        };
        assert_eq!(feedback.counters(&failed_key).failure, 1);
    }

    #[test]
    fn every_candidate_exhausted_resolves_to_blocked_not_a_panic() {
        let h = ScriptedHarness {
            script: std::cell::RefCell::new(vec![Err(HarnessError::Timeout)]),
        };
        let mut harnesses: HashMap<String, &dyn HarnessCapabilities> = HashMap::new();
        harnesses.insert("codex".into(), &h);
        let c1 = candidate("codex");
        let candidates = vec![&c1];
        let mut feedback = InMemoryFeedbackStore::default();

        let result = dispatch_with_fallback(
            &harnesses,
            &candidates,
            Tier::Tier1,
            spec_for,
            &mut feedback,
        )
        .expect("should not error");

        assert_eq!(result.state, TaskState::Blocked);
        assert!(result.outcome.is_none());
    }

    #[test]
    fn unsupported_is_a_caller_bug_not_a_fallback_case() {
        let h = ScriptedHarness {
            script: std::cell::RefCell::new(vec![Err(HarnessError::Unsupported)]),
        };
        let mut harnesses: HashMap<String, &dyn HarnessCapabilities> = HashMap::new();
        harnesses.insert("codex".into(), &h);
        let c1 = candidate("codex");
        let candidates = vec![&c1];
        let mut feedback = InMemoryFeedbackStore::default();

        let result = dispatch_with_fallback(
            &harnesses,
            &candidates,
            Tier::Tier1,
            spec_for,
            &mut feedback,
        );
        assert!(matches!(result, Err(OrchestratorError::Unsupported)));
    }
}
