//! The three evidence kinds from ADR 0007 / runtime-design.md §4. Each is bound to the
//! exact candidate revision and attempt id it validates.

use serde::{Deserialize, Serialize};

use crate::task_graph::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttemptId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRef {
    pub task_id: TaskId,
    pub attempt_id: AttemptId,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicEvidence {
    pub candidate: CandidateRef,
    pub tool: String,
    pub tool_version: String,
    pub exit_code: i32,
    pub output_redacted: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelReviewEvidence {
    pub candidate: CandidateRef,
    pub harness: String,
    pub model_ref: String,
    pub verdict: ReviewVerdict,
    pub rationale_redacted: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVerdict {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanAcceptanceEvidence {
    pub candidate: CandidateRef,
    pub accepted_by: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Evidence {
    Deterministic(DeterministicEvidence),
    ModelReview(ModelReviewEvidence),
    HumanAcceptance(HumanAcceptanceEvidence),
}

impl Evidence {
    pub fn candidate(&self) -> &CandidateRef {
        match self {
            Evidence::Deterministic(e) => &e.candidate,
            Evidence::ModelReview(e) => &e.candidate,
            Evidence::HumanAcceptance(e) => &e.candidate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredEvidence {
    DeterministicOnly,
    DeterministicAndModelReview,
    DeterministicAndHumanAcceptance,
}

/// Whether a candidate's collected evidence satisfies its tier's requirement. This is the
/// mechanical half of ADR 0007's gate: it never judges evidence *content*, only presence —
/// a Tier 3 candidate can never reach `accepted` on model review alone, per that ADR.
pub fn satisfies(required: RequiredEvidence, evidence: &[Evidence]) -> bool {
    let has = |pred: &dyn Fn(&Evidence) -> bool| evidence.iter().any(pred);
    let has_deterministic = has(&|e| matches!(e, Evidence::Deterministic(_)));
    if !has_deterministic {
        return false;
    }
    match required {
        RequiredEvidence::DeterministicOnly => true,
        RequiredEvidence::DeterministicAndModelReview => {
            has(&|e| matches!(e, Evidence::ModelReview(r) if r.verdict == ReviewVerdict::Pass))
        }
        RequiredEvidence::DeterministicAndHumanAcceptance => {
            has(&|e| matches!(e, Evidence::HumanAcceptance(_)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> CandidateRef {
        CandidateRef {
            task_id: TaskId(1),
            attempt_id: AttemptId(1),
            revision: "deadbeef".into(),
        }
    }

    fn deterministic() -> Evidence {
        Evidence::Deterministic(DeterministicEvidence {
            candidate: candidate(),
            tool: "cargo test".into(),
            tool_version: "1.98.0".into(),
            exit_code: 0,
            output_redacted: "ok".into(),
        })
    }

    #[test]
    fn deterministic_alone_is_never_enough_for_tier3() {
        let evidence = vec![deterministic()];
        assert!(!satisfies(
            RequiredEvidence::DeterministicAndHumanAcceptance,
            &evidence
        ));
    }

    #[test]
    fn tier3_requires_human_acceptance_even_with_model_review() {
        let evidence = vec![
            deterministic(),
            Evidence::ModelReview(ModelReviewEvidence {
                candidate: candidate(),
                harness: "claude-code".into(),
                model_ref: "configured-model".into(),
                verdict: ReviewVerdict::Pass,
                rationale_redacted: "looks fine".into(),
            }),
        ];
        assert!(!satisfies(
            RequiredEvidence::DeterministicAndHumanAcceptance,
            &evidence
        ));
    }

    #[test]
    fn human_acceptance_satisfies_tier3() {
        let evidence = vec![
            deterministic(),
            Evidence::HumanAcceptance(HumanAcceptanceEvidence {
                candidate: candidate(),
                accepted_by: "samuel".into(),
                accepted_at: "2026-09-05T00:00:00Z".into(),
            }),
        ];
        assert!(satisfies(
            RequiredEvidence::DeterministicAndHumanAcceptance,
            &evidence
        ));
    }

    #[test]
    fn no_evidence_never_satisfies_anything() {
        assert!(!satisfies(RequiredEvidence::DeterministicOnly, &[]));
    }

    #[test]
    fn failed_model_review_does_not_satisfy_tier1_2() {
        let evidence = vec![
            deterministic(),
            Evidence::ModelReview(ModelReviewEvidence {
                candidate: candidate(),
                harness: "codex".into(),
                model_ref: "configured-model".into(),
                verdict: ReviewVerdict::Fail,
                rationale_redacted: "found a bug".into(),
            }),
        ];
        assert!(!satisfies(
            RequiredEvidence::DeterministicAndModelReview,
            &evidence
        ));
    }
}
