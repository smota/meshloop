//! Git-diff verification helpers. Harness exit code is never the gate.

use meshloop_domain::decision::{Confidence, DecisionOutcome};
use meshloop_domain::diagnostic::{DiagnosticLattice, parse_diagnostics};
use meshloop_domain::evidence::{
    CandidateRef, Evidence, ModelReviewEvidence, RequiredEvidence, ReviewVerdict,
    all_deterministic_passed, satisfies,
};
use meshloop_domain::task_graph::Tier;

use crate::ports::{DecisionPort, DecisionPortError, DiffSummary, NoulQuery};

/// Prefix a check's redacted output with a lattice fingerprint so the event log
/// carries a deterministic diagnostic identity without storing raw stderr.
pub fn annotate_with_lattice(output_redacted: &str) -> (DiagnosticLattice, String) {
    let lattice = parse_diagnostics(output_redacted);
    let annotated = format!(
        "lattice={:016x} syntax={} type={} test={} blocking={}\n{output_redacted}",
        lattice.fingerprint(),
        lattice.progress().syntax,
        lattice.progress().type_errors,
        lattice.progress().tests,
        lattice.blocking(),
    );
    (lattice, annotated)
}

pub fn git_diff_exit_code(diff: &DiffSummary, empty_diff_ok: bool) -> i32 {
    if diff.files.is_empty() && !empty_diff_ok {
        1
    } else {
        0
    }
}

/// Paths from `git diff --name-only`, `/` separators. A rule ending in `/` is a prefix.
pub fn path_allowed(changed: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let changed = changed.replace('\\', "/");
    allowed.iter().any(|rule| {
        let rule = rule.replace('\\', "/");
        if rule.ends_with('/') {
            changed.starts_with(&rule)
        } else {
            changed == rule
        }
    })
}

pub fn all_paths_allowed(files: &[String], allowed: &[String]) -> bool {
    allowed.is_empty() || files.iter().all(|f| path_allowed(f, allowed))
}

/// Name prefix of Meshloop's per-attempt prompt files in a worktree.
pub const PROMPT_FILE_PREFIX: &str = ".meshloop-prompt-";

/// Changed paths a worker may never deliver, whatever `allowed_paths` says: any path
/// component naming a prompt file. A committed (symlink) prompt file could make a
/// later attempt write through it (#180).
pub fn reserved_paths(files: &[String]) -> Vec<&String> {
    files
        .iter()
        .filter(|file| {
            file.split(['/', '\\'])
                .any(|part| part.to_ascii_lowercase().starts_with(PROMPT_FILE_PREFIX))
        })
        .collect()
}

pub fn verification_passed(rows: &[Evidence]) -> bool {
    all_deterministic_passed(rows) && satisfies(RequiredEvidence::DeterministicOnly, rows)
}

/// Review of one candidate diff. `Abstain` carries no evidence row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateReview {
    Pass(Evidence),
    Fail(Evidence),
    Abstain,
}

/// Pure dual-threshold gate. Tier 3 always abstains and builds no model-review row.
/// `None` abstains. `p <= 0.05` fails. Checks passed and `p >= 0.95` pass.
pub fn evaluate_candidate_diff(
    tier: Tier,
    checks_passed: bool,
    acceptability: Option<Confidence>,
    candidate: CandidateRef,
) -> CandidateReview {
    match tier {
        Tier::Tier3 => CandidateReview::Abstain,
        Tier::Tier1 | Tier::Tier2 => {
            let Some(confidence) = acceptability else {
                return CandidateReview::Abstain;
            };
            let p = confidence.get();
            if p <= 0.05 {
                return CandidateReview::Fail(model_review(candidate, ReviewVerdict::Fail, p));
            }
            if checks_passed && p >= 0.95 {
                return CandidateReview::Pass(model_review(candidate, ReviewVerdict::Pass, p));
            }
            CandidateReview::Abstain
        }
    }
}

/// `P(acceptable)` from a noul port. Does not consult tier.
/// A unit-interval score is `Some`. Abstain, boolean, selected, and any other score are `None`.
pub fn acceptability_from_noul(
    port: &dyn DecisionPort,
    proposition: &str,
) -> Result<Option<Confidence>, DecisionPortError> {
    let outcome = port.evaluate_noul(&NoulQuery {
        proposition: proposition.to_owned(),
    })?;
    Ok(match outcome {
        DecisionOutcome::Score(score) => Confidence::try_new(score.get()).ok(),
        DecisionOutcome::Abstain(_)
        | DecisionOutcome::Boolean(_)
        | DecisionOutcome::Selected(_) => None,
    })
}

fn model_review(candidate: CandidateRef, verdict: ReviewVerdict, p: f64) -> Evidence {
    Evidence::ModelReview(ModelReviewEvidence {
        candidate,
        harness: "noul".into(),
        model_ref: "scripted".into(),
        verdict,
        rationale_redacted: format!("p={p}"),
    })
}

#[cfg(test)]
mod tests {
    use meshloop_domain::decision::SanitizedReason;
    use meshloop_domain::evidence::AttemptId;
    use meshloop_domain::task_graph::TaskId;

    use crate::ports::{ScriptedDecisionPort, decision_fingerprint};

    use super::*;

    #[test]
    fn prompt_file_paths_are_reserved_anywhere_in_a_change() {
        let files = vec![
            "src/lib.rs".to_string(),
            ".meshloop-prompt-7".to_string(),
            "nested/.meshloop-prompt-2".to_string(),
            "docs\\.meshloop-prompt-x\\a".to_string(),
            "meshloop-prompt-1".to_string(),
            ".MeshLoop-Prompt-9".to_string(),
        ];
        assert_eq!(
            reserved_paths(&files),
            vec![&files[1], &files[2], &files[3], &files[5]]
        );
        assert!(reserved_paths(&["README.md".to_string()]).is_empty());
    }

    #[test]
    fn empty_diff_fails_unless_flagged() {
        let diff = DiffSummary {
            base: "a".into(),
            head: "a".into(),
            files: vec![],
            stat_redacted: String::new(),
            gitlinks: vec![],
        };
        assert_eq!(git_diff_exit_code(&diff, false), 1);
        assert_eq!(git_diff_exit_code(&diff, true), 0);
    }

    #[test]
    fn allowed_paths_prefix_and_exact() {
        assert!(path_allowed("src/lib.rs", &["src/".into()]));
        assert!(!path_allowed("README.md", &["src/".into()]));
        assert!(path_allowed("README.md", &["README.md".into()]));
    }

    #[test]
    fn lattice_annotation_is_stable_across_worktree_paths() {
        let a = "error[E0308]: mismatched types\n --> src/main.rs:2:5\n";
        let b = "error[E0308]: mismatched types\n --> C:\\wt\\task-1\\src\\main.rs:99:1\n";
        let (la, sa) = annotate_with_lattice(a);
        let (lb, sb) = annotate_with_lattice(b);
        assert_eq!(la.fingerprint(), lb.fingerprint());
        assert!(sa.starts_with("lattice="));
        assert!(sb.starts_with("lattice="));
        assert_eq!(&sa[..23], &sb[..23]);
    }

    fn candidate() -> CandidateRef {
        CandidateRef {
            task_id: TaskId(7),
            attempt_id: AttemptId(3),
            revision: "cafebabe".into(),
        }
    }

    fn confidence(p: f64) -> Confidence {
        Confidence::try_new(p).expect("unit interval")
    }

    fn model_row(verdict: ReviewVerdict, rationale: &str) -> Evidence {
        Evidence::ModelReview(ModelReviewEvidence {
            candidate: candidate(),
            harness: "noul".into(),
            model_ref: "scripted".into(),
            verdict,
            rationale_redacted: rationale.into(),
        })
    }

    #[test]
    fn candidate_diff_tier1_checks_passed_p_0_95_passes() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier1, true, Some(confidence(0.95)), candidate()),
            CandidateReview::Pass(model_row(ReviewVerdict::Pass, "p=0.95"))
        );
    }

    #[test]
    fn candidate_diff_tier2_checks_passed_p_1_passes() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier2, true, Some(confidence(1.0)), candidate()),
            CandidateReview::Pass(model_row(ReviewVerdict::Pass, "p=1"))
        );
    }

    #[test]
    fn candidate_diff_tier1_checks_passed_p_0_05_fails() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier1, true, Some(confidence(0.05)), candidate()),
            CandidateReview::Fail(model_row(ReviewVerdict::Fail, "p=0.05"))
        );
    }

    #[test]
    fn candidate_diff_tier2_checks_failed_p_0_99_abstains() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier2, false, Some(confidence(0.99)), candidate()),
            CandidateReview::Abstain
        );
    }

    #[test]
    fn candidate_diff_tier1_checks_passed_p_0_50_abstains() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier1, true, Some(confidence(0.50)), candidate()),
            CandidateReview::Abstain
        );
    }

    #[test]
    fn candidate_diff_tier1_checks_passed_none_abstains() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier1, true, None, candidate()),
            CandidateReview::Abstain
        );
    }

    #[test]
    fn candidate_diff_tier3_checks_passed_p_1_abstains() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier3, true, Some(confidence(1.0)), candidate()),
            CandidateReview::Abstain
        );
    }

    #[test]
    fn candidate_diff_tier3_checks_passed_p_0_abstains() {
        assert_eq!(
            evaluate_candidate_diff(Tier::Tier3, true, Some(confidence(0.0)), candidate()),
            CandidateReview::Abstain
        );
    }

    #[test]
    fn candidate_diff_acceptability_from_noul_maps_score_and_abstain() {
        let mut port = ScriptedDecisionPort::default();
        let scored = "candidate diff is acceptable";
        port.insert(
            decision_fingerprint("noul", &[scored]),
            DecisionOutcome::try_score(0.95).expect("finite score"),
        );
        assert_eq!(
            acceptability_from_noul(&port, scored).expect("scripted score"),
            Some(confidence(0.95))
        );

        let abstained = "candidate diff is unscored";
        port.insert(
            decision_fingerprint("noul", &[abstained]),
            DecisionOutcome::abstain(SanitizedReason::try_new("no score").expect("reason")),
        );
        assert_eq!(
            acceptability_from_noul(&port, abstained).expect("scripted abstain"),
            None
        );
    }
}
