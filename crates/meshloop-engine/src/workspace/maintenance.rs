//! Opt-in maintenance selection. This function does not delete anything.
//! Seven-day age is recorded as advisory and never selects or excludes a candidate.

use std::time::Duration;

use meshloop_domain::digest::sha256_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceCandidate {
    pub id: String,
    pub eligible: bool,
    pub landed: bool,
    pub failed: bool,
    pub cancelled: bool,
    pub evidence: bool,
    pub has_dependency: bool,
    pub outcome_known: bool,
    pub age_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaintenanceBudget {
    pub max_considered: usize,
    pub max_elapsed: Duration,
    pub max_deletions: usize,
}

impl Default for MaintenanceBudget {
    fn default() -> Self {
        Self {
            max_considered: 20,
            max_elapsed: Duration::from_secs(5),
            max_deletions: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MaintenancePlan {
    pub apply: bool,
    pub digest: String,
    pub selected: Vec<String>,
    pub retained: Vec<MaintenanceRetained>,
    pub cursor: Option<String>,
    pub advisory: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MaintenanceRetained {
    pub id: String,
    pub reason: String,
}

pub fn maintenance_plan(
    candidates: &[MaintenanceCandidate],
    budget: MaintenanceBudget,
    elapsed: Duration,
    opt_in: bool,
    bound_digest: Option<&str>,
) -> MaintenancePlan {
    let digest = digest_of(candidates, budget);
    let mut selected = Vec::new();
    let mut retained = Vec::new();
    let mut advisory = Vec::new();
    let mut cursor = None;
    if elapsed >= budget.max_elapsed {
        cursor = candidates.first().map(|candidate| candidate.id.clone());
    } else {
        for candidate in candidates.iter().take(budget.max_considered) {
            if candidate.age_days.is_some_and(|age| age >= 7) {
                advisory.push(candidate.id.clone());
            }
            if let Some(reason) = retain_reason(candidate) {
                retained.push(MaintenanceRetained {
                    id: candidate.id.clone(),
                    reason: reason.to_string(),
                });
                continue;
            }
            if selected.len() < budget.max_deletions {
                selected.push(candidate.id.clone());
            }
        }
        if candidates.len() > budget.max_considered {
            cursor = candidates
                .get(budget.max_considered)
                .map(|candidate| candidate.id.clone());
        } else if let Some(skipped) =
            candidates
                .iter()
                .take(budget.max_considered)
                .find(|candidate| {
                    retain_reason(candidate).is_none()
                        && !selected.iter().any(|id| id == &candidate.id)
                })
        {
            cursor = Some(skipped.id.clone());
        }
    }
    let fresh = bound_digest.is_some_and(|bound| bound == digest);
    MaintenancePlan {
        apply: opt_in && fresh && !selected.is_empty(),
        digest,
        selected,
        retained,
        cursor,
        advisory,
    }
}

fn retain_reason(candidate: &MaintenanceCandidate) -> Option<&'static str> {
    if candidate.evidence {
        Some("evidence")
    } else if candidate.failed {
        Some("failed")
    } else if candidate.cancelled {
        Some("cancelled")
    } else if candidate.has_dependency {
        Some("dependency")
    } else if !candidate.outcome_known {
        Some("outcome-unknown")
    } else if !candidate.landed {
        Some("not-landed")
    } else if !candidate.eligible {
        Some("not-eligible")
    } else {
        None
    }
}

fn digest_of(candidates: &[MaintenanceCandidate], budget: MaintenanceBudget) -> String {
    let mut bytes = Vec::new();
    for candidate in candidates {
        bytes.extend(
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                candidate.id,
                candidate.eligible,
                candidate.landed,
                candidate.failed,
                candidate.cancelled,
                candidate.evidence,
                candidate.has_dependency,
                candidate.outcome_known,
                candidate
                    .age_days
                    .map(|age| age.to_string())
                    .unwrap_or_default()
            )
            .bytes(),
        );
    }
    bytes.extend(
        format!(
            "budget\t{}\t{}\t{}\n",
            budget.max_considered,
            budget.max_elapsed.as_secs(),
            budget.max_deletions
        )
        .bytes(),
    );
    sha256_hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str) -> MaintenanceCandidate {
        MaintenanceCandidate {
            id: id.to_string(),
            eligible: true,
            landed: true,
            failed: false,
            cancelled: false,
            evidence: false,
            has_dependency: false,
            outcome_known: true,
            age_days: None,
        }
    }

    fn budget() -> MaintenanceBudget {
        MaintenanceBudget::default()
    }

    #[test]
    fn maintenance_budget_selects_one_landed_candidate_and_retains_the_rest() {
        let mut failed = candidate("failed");
        failed.failed = true;
        failed.age_days = Some(30);
        let mut cancelled = candidate("cancelled");
        cancelled.cancelled = true;
        let mut evidence = candidate("evidence");
        evidence.evidence = true;
        let mut dependency = candidate("dependency");
        dependency.has_dependency = true;
        let mut unlanded = candidate("unlanded");
        unlanded.landed = false;
        let mut unknown = candidate("unknown");
        unknown.outcome_known = false;
        let plan = maintenance_plan(
            &[
                candidate("keep"),
                failed,
                cancelled,
                evidence,
                dependency,
                unlanded,
                unknown,
            ],
            budget(),
            Duration::from_secs(0),
            false,
            None,
        );
        assert_eq!(plan.selected, vec!["keep".to_string()]);
        assert!(!plan.apply);
        assert_eq!(plan.advisory, vec!["failed".to_string()]);
        assert!(plan.cursor.is_none());
        let reasons: Vec<_> = plan
            .retained
            .iter()
            .map(|row| (row.id.as_str(), row.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                ("failed", "failed"),
                ("cancelled", "cancelled"),
                ("evidence", "evidence"),
                ("dependency", "dependency"),
                ("unlanded", "not-landed"),
                ("unknown", "outcome-unknown"),
            ]
        );
    }

    #[test]
    fn maintenance_budget_stops_at_the_window_and_the_deadline() {
        let many: Vec<_> = (0..21)
            .map(|index| candidate(&format!("c{index}")))
            .collect();
        let plan = maintenance_plan(&many, budget(), Duration::from_secs(0), true, None);
        assert_eq!(plan.selected, vec!["c0".to_string()]);
        assert_eq!(plan.cursor.as_deref(), Some("c20"));
        assert!(!plan.apply);
        let expired = maintenance_plan(
            &many,
            budget(),
            Duration::from_secs(5),
            true,
            Some(&plan.digest),
        );
        assert!(expired.selected.is_empty());
        assert_eq!(expired.cursor.as_deref(), Some("c0"));
        assert!(!expired.apply);
        let opted = maintenance_plan(
            &[candidate("only")],
            budget(),
            Duration::from_secs(0),
            true,
            Some(
                &maintenance_plan(&[candidate("only")], budget(), Duration::ZERO, false, None)
                    .digest,
            ),
        );
        assert!(opted.apply);
        assert_eq!(opted.selected, vec!["only".to_string()]);
        let stale = maintenance_plan(
            &[candidate("only")],
            budget(),
            Duration::ZERO,
            true,
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
        assert!(!stale.apply);
        assert_eq!(stale.selected, vec!["only".to_string()]);
    }
}
