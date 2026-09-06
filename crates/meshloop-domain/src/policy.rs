//! Tier/coupling value types consumed by routing (meshloop-engine::router). No scoring
//! logic lives here — only the shared vocabulary scoring signals read.

use std::collections::HashSet;

use crate::task_graph::{TaskId, Tier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CouplingPenalty(pub u32);

/// File-overlap based coupling: two nodes touching any of the same paths are coupled.
/// Zero overlap is zero penalty; this is a signal input, not a routing decision.
pub fn coupling_penalty(a_paths: &HashSet<String>, b_paths: &HashSet<String>) -> CouplingPenalty {
    CouplingPenalty(a_paths.intersection(b_paths).count() as u32)
}

/// Whether a model tier is an acceptable fit for a task tier, per runtime-design.md §5's
/// "don't spend a scarce top-tier model's quota on a Tier 1 task; don't assign a Tier 3
/// task to a model that has never confirmed handling that tier" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelCapabilityTier {
    Lightweight,
    MidTier,
    TopTier,
}

pub fn tier_fits(task_tier: Tier, model_tier: ModelCapabilityTier) -> bool {
    match task_tier {
        Tier::Tier1 => true,
        Tier::Tier2 => model_tier >= ModelCapabilityTier::MidTier,
        Tier::Tier3 => model_tier == ModelCapabilityTier::TopTier,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequiredHumanAcceptance(pub bool);

pub fn requires_human_acceptance(tier: Tier) -> RequiredHumanAcceptance {
    RequiredHumanAcceptance(tier == Tier::Tier3)
}

#[derive(Debug, Clone)]
pub struct PlanScope {
    pub graph_id: String,
    pub touched_tasks: Vec<TaskId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coupling_counts_shared_paths_only() {
        let a: HashSet<String> = ["a.rs".into(), "b.rs".into()].into();
        let b: HashSet<String> = ["b.rs".into(), "c.rs".into()].into();
        assert_eq!(coupling_penalty(&a, &b), CouplingPenalty(1));
    }

    #[test]
    fn tier1_accepts_any_model_tier() {
        assert!(tier_fits(Tier::Tier1, ModelCapabilityTier::Lightweight));
        assert!(tier_fits(Tier::Tier1, ModelCapabilityTier::TopTier));
    }

    #[test]
    fn tier3_requires_top_tier_model() {
        assert!(!tier_fits(Tier::Tier3, ModelCapabilityTier::MidTier));
        assert!(tier_fits(Tier::Tier3, ModelCapabilityTier::TopTier));
    }

    #[test]
    fn only_tier3_requires_human_acceptance() {
        assert_eq!(
            requires_human_acceptance(Tier::Tier1),
            RequiredHumanAcceptance(false)
        );
        assert_eq!(
            requires_human_acceptance(Tier::Tier3),
            RequiredHumanAcceptance(true)
        );
    }
}
