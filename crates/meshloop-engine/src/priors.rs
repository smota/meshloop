//! Advisory routing priors in the engine (ADR 0038, amendment 2026-11-10 for #139).
//!
//! [`PriorsOracle`] is a [`PredictiveSuccessOracle`] built from one
//! [`PriorsSnapshot`]. Its π enters routing only as the prior of
//! `fused_historical_mean`, which clips the local mean by ± 0.25 once. It is consulted
//! with `spend: None` and for worker routing only; planner routing keeps `oracle: None`.
//! It never adds or removes a candidate: `Router::select` filters before it scores.
//!
//! [`resolve_provenance`] checks an entry's references against the local ledger for
//! `priors accept` and `priors check`. Routing never calls it.

use std::collections::HashMap;

use meshloop_domain::advisory::{CheckResult, DispatchMode, PRIOR_EVIDENCE_FLOOR, PriorEntry};
use meshloop_domain::decision::Confidence;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::Event;
use meshloop_domain::task_graph::{TaskId, Tier};

use crate::ports::{
    DecisionPortError, OracleOutcome, OracleQuery, PredictiveSuccessOracle, PriorsSnapshot,
    RunStore, StoreError, TierKey,
};
use crate::router::oracle_arm;

/// Priors from one snapshot, keyed by the router's arm string.
#[derive(Debug, Clone, Default)]
pub struct PriorsOracle {
    arms: HashMap<String, Confidence>,
}

impl PriorsOracle {
    /// `dispatch` maps each configured harness to how it dispatches. An entry whose
    /// harness is missing there, or does not dispatch as a subprocess, gets no arm, so
    /// `predict` errs and the router keeps the local mean.
    pub fn new(snapshot: &PriorsSnapshot, dispatch: &HashMap<String, DispatchMode>) -> Self {
        let mut arms = HashMap::new();
        for entry in &snapshot.entries {
            // The loader enforces the floor; the oracle does not trust a snapshot that
            // skipped it.
            if entry.attempts < PRIOR_EVIDENCE_FLOOR || !entry.key.mode.admits_priors() {
                continue;
            }
            let candidate_mode = match dispatch.get(&entry.key.harness) {
                Some(mode) => *mode,
                None => continue,
            };
            // Exhaustive: a `Native` variant (#147) must decide here whether its
            // candidates may take a prior. Rule 3 says they may not.
            match candidate_mode {
                DispatchMode::Subprocess => {}
            }
            let Ok(prior) = Confidence::try_new(entry.pi()) else {
                continue;
            };
            arms.insert(
                oracle_arm(
                    &entry.key.harness,
                    &entry.key.model_ref,
                    TierKey::from(entry.key.tier),
                ),
                prior,
            );
        }
        Self { arms }
    }

    pub fn is_empty(&self) -> bool {
        self.arms.is_empty()
    }
}

impl PredictiveSuccessOracle for PriorsOracle {
    /// The prior for `query.arm`, whatever `pulls` is: π comes from the file's counts
    /// alone and local history only lowers its weight inside the fused mean.
    fn predict(&self, query: &OracleQuery) -> Result<OracleOutcome, DecisionPortError> {
        self.arms
            .get(&query.arm)
            .copied()
            .map(|prior| OracleOutcome { prior })
            .ok_or(DecisionPortError::NoPrior)
    }
}

/// What resolving an entry's references against the local ledger found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvenanceReport {
    /// References found in the local ledger, over every entry checked.
    pub resolved: u64,
    /// References not in the local ledger. Reported only: #140 promotes into fresh stores.
    pub unresolved: u64,
    /// `(entry index, unresolved references)` for each entry with any.
    pub unresolved_by_entry: Vec<(usize, u64)>,
    /// `(entry index, why)` for each entry a resolved reference contradicts.
    pub contradicted: Vec<(usize, String)>,
}

/// One resolved attempt as the ledger recorded it.
struct Recorded {
    harness: Option<String>,
    model_ref: Option<String>,
    tier: Tier,
    mode: DispatchMode,
    model_passed: Option<bool>,
    check_result: Option<CheckResult>,
}

/// Per-graph data read once per call.
struct GraphView {
    tiers: HashMap<TaskId, Tier>,
    events: HashMap<AttemptId, Vec<Event>>,
}

/// Resolves every reference of `entries` against `store`. A reference resolves when an
/// attempt with its id exists in its graph and task and the graph's plan is readable.
/// A resolved reference contradicts its entry when the recorded harness, model, task
/// tier (unset tiers dispatch as Tier2), dispatch mode, `model_passed` or check result
/// disagrees with the entry's key or the reference's `passed`. Read-only.
pub fn resolve_provenance(
    store: &dyn RunStore,
    entries: &[PriorEntry],
) -> Result<ProvenanceReport, StoreError> {
    let mut graphs: HashMap<String, Option<GraphView>> = HashMap::new();
    let mut report = ProvenanceReport::default();
    for entry in entries {
        let mut unresolved = 0u64;
        let mut contradiction: Option<String> = None;
        for reference in &entry.provenance {
            let Some(recorded) = lookup(store, &mut graphs, reference)? else {
                unresolved += 1;
                continue;
            };
            report.resolved += 1;
            if contradiction.is_some() {
                continue;
            }
            let expected = if reference.passed {
                CheckResult::Passed
            } else {
                CheckResult::Failed
            };
            let where_ = format!(
                "attempt {} (graph {}, task {})",
                reference.attempt_id, reference.graph_id, reference.task_id
            );
            contradiction = if recorded.harness.as_deref() != Some(entry.key.harness.as_str()) {
                Some(format!("{where_} ran another harness"))
            } else if recorded.model_ref.as_deref() != Some(entry.key.model_ref.as_str()) {
                Some(format!("{where_} ran another model"))
            } else if recorded.tier != entry.key.tier {
                Some(format!("{where_} ran a task of another tier"))
            } else if recorded.mode != entry.key.mode {
                Some(format!("{where_} was dispatched in another mode"))
            } else if recorded.model_passed != Some(true) {
                Some(format!("{where_} did not record model_passed true"))
            } else if recorded.check_result != Some(expected) {
                Some(format!(
                    "{where_} recorded check_result {}, not {}",
                    recorded.check_result.map_or("null", CheckResult::as_str),
                    expected.as_str()
                ))
            } else {
                None
            };
        }
        report.unresolved += unresolved;
        if unresolved > 0 {
            report.unresolved_by_entry.push((entry.index, unresolved));
        }
        if let Some(why) = contradiction {
            report.contradicted.push((entry.index, why));
        }
    }
    Ok(report)
}

fn lookup(
    store: &dyn RunStore,
    graphs: &mut HashMap<String, Option<GraphView>>,
    reference: &meshloop_domain::advisory::AttemptRef,
) -> Result<Option<Recorded>, StoreError> {
    let (Ok(task_id), Ok(attempt_id)) = (
        u32::try_from(reference.task_id),
        u32::try_from(reference.attempt_id),
    ) else {
        return Ok(None);
    };
    let (task_id, attempt_id) = (TaskId(task_id), AttemptId(attempt_id));
    let Some(row) = store.load_attempt(attempt_id)? else {
        return Ok(None);
    };
    if row.graph_id != reference.graph_id || row.task_id != task_id {
        return Ok(None);
    }
    if !graphs.contains_key(&reference.graph_id) {
        let view = graph_view(store, &reference.graph_id)?;
        graphs.insert(reference.graph_id.clone(), view);
    }
    let Some(Some(view)) = graphs.get(&reference.graph_id) else {
        return Ok(None);
    };
    let Some(tier) = view.tiers.get(&task_id).copied() else {
        return Ok(None);
    };
    let events = view.events.get(&attempt_id).cloned().unwrap_or_default();
    Ok(Some(Recorded {
        harness: row.harness,
        model_ref: row.model_ref,
        tier,
        mode: DispatchMode::of_recorded_attempt(),
        model_passed: row.model_passed,
        check_result: CheckResult::final_of(events),
    }))
}

fn graph_view(store: &dyn RunStore, graph_id: &str) -> Result<Option<GraphView>, StoreError> {
    if store.load_run(graph_id)?.is_none() {
        return Ok(None);
    }
    // An unreadable plan leaves the reference unresolved rather than failing the verb.
    let Ok(graph) = store.graph_from_run(graph_id) else {
        return Ok(None);
    };
    let tiers = graph
        .nodes
        .iter()
        .map(|n| (n.id, n.tier.unwrap_or(Tier::Tier2)))
        .collect();
    let mut events: HashMap<AttemptId, Vec<Event>> = HashMap::new();
    for record in store.records_for_graph(graph_id)? {
        if let Some(id) = record.attempt_id {
            events.entry(id).or_default().push(record.event);
        }
    }
    Ok(Some(GraphView { tiers, events }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::advisory::{AttemptRef, PriorKey};

    fn entry(harness: &str, model: &str, tier: Tier, attempts: u32, successes: u32) -> PriorEntry {
        PriorEntry {
            index: 0,
            key: PriorKey {
                harness: harness.into(),
                model_ref: model.into(),
                tier,
                mode: DispatchMode::Subprocess,
            },
            attempts,
            successes,
            provenance: (0..attempts)
                .map(|i| AttemptRef {
                    graph_id: "g".into(),
                    task_id: 1,
                    attempt_id: u64::from(i) + 1,
                    passed: i < successes,
                })
                .collect(),
        }
    }

    fn snapshot(entries: Vec<PriorEntry>) -> PriorsSnapshot {
        PriorsSnapshot {
            sequence: 1,
            priors_sha256: "0".repeat(64),
            entries,
        }
    }

    fn subprocess(names: &[&str]) -> HashMap<String, DispatchMode> {
        names
            .iter()
            .map(|n| (n.to_string(), DispatchMode::Subprocess))
            .collect()
    }

    fn predict(oracle: &PriorsOracle, arm: &str, pulls: u64) -> Option<f64> {
        oracle
            .predict(&OracleQuery {
                arm: arm.into(),
                pulls,
            })
            .ok()
            .map(|o| o.prior.get())
    }

    #[test]
    fn oracle_gives_pi_for_the_matching_arm_only() {
        let oracle = PriorsOracle::new(
            &snapshot(vec![entry("codex", "gpt", Tier::Tier2, 8, 6)]),
            &subprocess(&["codex"]),
        );
        assert_eq!(predict(&oracle, "codex\ngpt\nTier2", 0), Some(7.0 / 10.0));
        // Pulls never change π.
        assert_eq!(predict(&oracle, "codex\ngpt\nTier2", 40), Some(7.0 / 10.0));
        assert_eq!(predict(&oracle, "codex\ngpt\nTier1", 0), None);
        assert_eq!(predict(&oracle, "codex\nother\nTier2", 0), None);
        assert_eq!(predict(&oracle, "claude\ngpt\nTier2", 0), None);
        assert_eq!(
            oracle.predict(&OracleQuery {
                arm: String::new(),
                pulls: 0
            }),
            Err(DecisionPortError::NoPrior)
        );
    }

    #[test]
    fn oracle_refuses_an_entry_below_the_floor_even_if_a_snapshot_carries_it() {
        let oracle = PriorsOracle::new(
            &snapshot(vec![entry("codex", "gpt", Tier::Tier2, 4, 4)]),
            &subprocess(&["codex"]),
        );
        assert!(oracle.is_empty());
    }

    /// LP test 6 as far as it is testable before #147: a candidate whose harness has no
    /// subprocess dispatch (here: no known dispatch at all) gets no prior. #147 adds
    /// the native case when `DispatchMode::Native` exists, and the exhaustive match in
    /// `PriorsOracle::new` forces that decision.
    #[test]
    fn a_candidate_without_subprocess_dispatch_gets_no_prior() {
        let oracle = PriorsOracle::new(
            &snapshot(vec![entry("codex", "gpt", Tier::Tier2, 8, 8)]),
            &subprocess(&["claude"]),
        );
        assert_eq!(predict(&oracle, "codex\ngpt\nTier2", 0), None);
        assert!(oracle.is_empty());
    }
}
