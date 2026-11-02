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

/// Machine-verifiable Git commit and worktree export metadata emitted upon task completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitExport {
    pub worktree_path: String,
    pub branch_ref: String,
    pub commit_sha: String,
    pub patch_sha256: String,
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

/// True iff every `DeterministicEvidence` row has `exit_code == 0`.
/// Vacuous-false: no deterministic rows → false (same as `satisfies(DeterministicOnly, &[])`).
pub fn all_deterministic_passed(evidence: &[Evidence]) -> bool {
    let rows: Vec<_> = evidence
        .iter()
        .filter_map(|e| match e {
            Evidence::Deterministic(d) => Some(d),
            _ => None,
        })
        .collect();
    !rows.is_empty() && rows.iter().all(|d| d.exit_code == 0)
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
    fn failed_exit_code_does_not_pass_all_deterministic() {
        let evidence = vec![Evidence::Deterministic(DeterministicEvidence {
            candidate: candidate(),
            tool: "git-diff".into(),
            tool_version: "n/a".into(),
            exit_code: 1,
            output_redacted: "empty".into(),
        })];
        assert!(satisfies(RequiredEvidence::DeterministicOnly, &evidence));
        assert!(!all_deterministic_passed(&evidence));
    }

    #[test]
    fn mixed_deterministic_rows_fail_if_any_exit_nonzero() {
        let evidence = vec![
            deterministic(),
            Evidence::Deterministic(DeterministicEvidence {
                candidate: candidate(),
                tool: "cargo".into(),
                tool_version: "n/a".into(),
                exit_code: 1,
                output_redacted: "fail".into(),
            }),
        ];
        assert!(!all_deterministic_passed(&evidence));
    }

    #[test]
    fn empty_evidence_fails_all_deterministic_passed() {
        assert!(!all_deterministic_passed(&[]));
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

    // Externally tagged payloads of today's shapes. A new required field fails `from_str`.
    #[test]
    fn handwritten_model_review_and_deterministic_payloads_round_trip() {
        let model_review = r#"{
            "ModelReview": {
                "candidate": {"task_id": 1, "attempt_id": 1, "revision": "deadbeef"},
                "harness": "noul",
                "model_ref": "scripted",
                "verdict": "Pass",
                "rationale_redacted": "p=0.95"
            }
        }"#;
        let deterministic_payload = r#"{
            "Deterministic": {
                "candidate": {"task_id": 1, "attempt_id": 1, "revision": "deadbeef"},
                "tool": "cargo test",
                "tool_version": "1.98.0",
                "exit_code": 0,
                "output_redacted": "ok"
            }
        }"#;
        assert_eq!(
            round_trip_payload(model_review),
            Evidence::ModelReview(ModelReviewEvidence {
                candidate: candidate(),
                harness: "noul".into(),
                model_ref: "scripted".into(),
                verdict: ReviewVerdict::Pass,
                rationale_redacted: "p=0.95".into(),
            })
        );
        assert_eq!(round_trip_payload(deterministic_payload), deterministic());
    }

    fn round_trip_payload(payload: &str) -> Evidence {
        let parsed: Evidence = serde_json::from_str(payload).expect("handwritten payload");
        let encoded = serde_json::to_string(&parsed).expect("serialize");
        let again: Evidence = serde_json::from_str(&encoded).expect("round trip");
        assert_eq!(parsed, again);
        parsed
    }
}
