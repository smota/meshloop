//! Quota-Aware Capability Routing (ADR 0009 / runtime-design.md §5). Strategy pattern
//! (design-patterns.md): scoring is a weighted sum over pluggable `RoutingSignal`s, so
//! adding a signal, harness, or model tier never touches this module's control flow.

use std::collections::HashMap;
use std::time::SystemTime;

use meshloop_domain::capability::{HarnessProfile, QuotaState};
use meshloop_domain::policy::{CouplingPenalty, ModelCapabilityTier, tier_fits};
use meshloop_domain::task_graph::Tier;

use crate::ports::{FeedbackKey, RoutingFeedbackStore, TierKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub harness: String,
    pub model_ref: String,
    pub model_tier: ModelCapabilityTier,
}

pub struct RoutingContext<'a> {
    pub task_tier: Tier,
    pub coupling_penalty: CouplingPenalty,
    pub preferred_harness: Option<&'a str>,
    pub headroom: &'a HashMap<String, f64>,
    pub feedback: &'a dyn RoutingFeedbackStore,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalScore(pub f64);

pub trait RoutingSignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore;
}

/// Soft preference against wasting a scarce top-tier model's quota on a low-tier task.
/// Adequacy itself (never assign Tier 3 to a model that can't handle it) is a hard filter
/// in `Router::select`, not a signal — that distinction matters: a signal can be outweighed,
/// a capability requirement cannot.
pub struct TierEfficiencySignal;
impl RoutingSignal for TierEfficiencySignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore {
        let waste = match (ctx.task_tier, candidate.model_tier) {
            (Tier::Tier1, ModelCapabilityTier::TopTier) => 1.0,
            (Tier::Tier1, ModelCapabilityTier::MidTier) => 0.5,
            (Tier::Tier2, ModelCapabilityTier::TopTier) => 0.5,
            _ => 0.0,
        };
        SignalScore(-waste)
    }
}

pub struct HistoricalSuccessSignal;
impl RoutingSignal for HistoricalSuccessSignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore {
        let key = FeedbackKey {
            harness: candidate.harness.clone(),
            model_ref: candidate.model_ref.clone(),
            tier: TierKey::from(ctx.task_tier),
        };
        let counters = ctx.feedback.counters(&key).unwrap_or_default();
        let total = counters.success + counters.failure;
        if total == 0 {
            SignalScore(0.5)
        } else {
            SignalScore(counters.success as f64 / total as f64)
        }
    }
}

pub struct LoadBalanceSignal;
impl RoutingSignal for LoadBalanceSignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore {
        // Missing headroom is omitted (same 0.0 contribution for every candidate that
        // lacks an observation). Never invent full capacity (the old unwrap_or(1.0)).
        match ctx.headroom.get(&candidate.harness) {
            Some(h) => SignalScore(*h),
            None => SignalScore(0.0),
        }
    }
}

pub struct CouplingSignal;
impl RoutingSignal for CouplingSignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore {
        let magnitude = ctx.coupling_penalty.0 as f64 * 0.1;
        match ctx.preferred_harness {
            Some(h) if h == candidate.harness => SignalScore(magnitude),
            Some(_) => SignalScore(-magnitude),
            None => SignalScore(0.0),
        }
    }
}

/// Restless-bandit exploration plus online-knapsack density.
///
/// Each harness is an arm whose state evolves whether or not it is pulled
/// (cooldown timers, sliding quota windows). Historical success stays in
/// [`HistoricalSuccessSignal`]; this signal is the information-value bonus and
/// the remaining-window density. Missing headroom contributes 0 (never invents
/// scarcity or abundance — ADR 0009).
pub struct RestlessBanditSignal {
    /// UCB-style exploration constant. Default 0.7.
    pub c: f64,
}

impl Default for RestlessBanditSignal {
    fn default() -> Self {
        Self { c: 0.7 }
    }
}

impl RoutingSignal for RestlessBanditSignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore {
        let key = FeedbackKey {
            harness: candidate.harness.clone(),
            model_ref: candidate.model_ref.clone(),
            tier: TierKey::from(ctx.task_tier),
        };
        let counters = ctx.feedback.counters(&key).unwrap_or_default();
        let n = f64::from(counters.success + counters.failure);
        let explore = self.c * (1.0 / (n + 1.0)).sqrt();
        let density = match ctx.headroom.get(&candidate.harness) {
            Some(h) => 0.5 * h.clamp(0.0, 1.0),
            None => 0.0,
        };
        SignalScore(explore + density)
    }
}

pub struct Router {
    signals: Vec<Box<dyn RoutingSignal>>,
}

impl Default for Router {
    fn default() -> Self {
        Self {
            signals: vec![
                Box::new(TierEfficiencySignal),
                Box::new(HistoricalSuccessSignal),
                Box::new(LoadBalanceSignal),
                Box::new(CouplingSignal),
                Box::new(RestlessBanditSignal::default()),
            ],
        }
    }
}

impl Router {
    pub fn with_signals(signals: Vec<Box<dyn RoutingSignal>>) -> Self {
        Self { signals }
    }

    /// QACR steps 1-4. Step 5 (dispatch, with visible fallback on failure) belongs to the
    /// orchestrator, since it has side effects this pure selection function must not have.
    /// Returns candidates best-first; an empty result means every configured candidate was
    /// filtered out (unfit or in cooldown) and the caller must resolve to `Blocked`, never guess.
    pub fn select<'a>(
        &self,
        configured: &'a [Candidate],
        profiles: &HashMap<String, HarnessProfile>,
        quotas: &HashMap<String, QuotaState>,
        now: SystemTime,
        ctx: &RoutingContext,
    ) -> Vec<&'a Candidate> {
        let mut scored: Vec<(&Candidate, f64)> = configured
            .iter()
            .filter(|c| tier_fits(ctx.task_tier, c.model_tier))
            .filter(|c| {
                profiles
                    .get(&c.harness)
                    .map(|p| p.is_dispatchable())
                    .unwrap_or(false)
            })
            .filter(|c| {
                quotas
                    .get(&c.harness)
                    .map(|q| q.is_available(now))
                    .unwrap_or(true)
            })
            .map(|c| {
                let score = self.signals.iter().map(|s| s.score(c, ctx).0).sum();
                (c, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(c, _)| c).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::capability::Compatibility;
    use meshloop_domain::policy::CouplingPenalty;

    fn profile(compatible: bool, cancellable: bool) -> HarnessProfile {
        HarnessProfile {
            harness: "h".into(),
            version: "1".into(),
            compatibility: if compatible {
                Compatibility::Compatible
            } else {
                Compatibility::Unsupported
            },
            supports_noninteractive: true,
            supports_structured_output: true,
            supports_cancellation: cancellable,
        }
    }

    fn ctx<'a>(
        tier: Tier,
        headroom: &'a HashMap<String, f64>,
        feedback: &'a dyn RoutingFeedbackStore,
    ) -> RoutingContext<'a> {
        RoutingContext {
            task_tier: tier,
            coupling_penalty: CouplingPenalty(0),
            preferred_harness: None,
            headroom,
            feedback,
        }
    }

    #[test]
    fn never_selects_a_candidate_outside_the_configured_set() {
        let configured = vec![Candidate {
            harness: "claude-code".into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }];
        let mut profiles = HashMap::new();
        profiles.insert("claude-code".into(), profile(true, true));
        profiles.insert("codex".into(), profile(true, true)); // configured elsewhere, not here
        let quotas = HashMap::new();
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].harness, "claude-code");
    }

    #[test]
    fn tier3_never_selects_a_non_top_tier_model() {
        let configured = vec![Candidate {
            harness: "codex".into(),
            model_ref: "mini".into(),
            model_tier: ModelCapabilityTier::MidTier,
        }];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let quotas = HashMap::new();
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier3, &headroom, &feedback),
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn unsupported_harness_is_filtered_out() {
        let configured = vec![Candidate {
            harness: "codex".into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(false, true));
        let quotas = HashMap::new();
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn candidate_in_open_cooldown_is_filtered_out() {
        let configured = vec![Candidate {
            harness: "codex".into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let mut quotas = HashMap::new();
        let mut q = QuotaState::default();
        q.on_capacity_exhausted(SystemTime::now(), std::time::Duration::from_secs(3600));
        quotas.insert("codex".into(), q);
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn all_candidates_cooled_down_yields_empty_not_a_panic() {
        let configured = vec![
            Candidate {
                harness: "codex".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "claude-code".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        profiles.insert("claude-code".into(), profile(true, true));
        let mut quotas = HashMap::new();
        for h in ["codex", "claude-code"] {
            let mut q = QuotaState::default();
            q.on_capacity_exhausted(SystemTime::now(), std::time::Duration::from_secs(3600));
            quotas.insert(h.into(), q);
        }
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn load_spreads_toward_the_harness_with_more_headroom() {
        let configured = vec![
            Candidate {
                harness: "codex".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "claude-code".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        profiles.insert("claude-code".into(), profile(true, true));
        let quotas = HashMap::new();
        let mut headroom = HashMap::new();
        headroom.insert("codex".into(), 0.1);
        headroom.insert("claude-code".into(), 0.9);
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected[0].harness, "claude-code");
    }

    #[test]
    fn historical_success_reorders_but_never_adds_candidates() {
        let configured = vec![Candidate {
            harness: "codex".into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let quotas = HashMap::new();
        let headroom = HashMap::new();
        let mut feedback = crate::ports::InMemoryFeedbackStore::default();
        let key = FeedbackKey {
            harness: "codex".into(),
            model_ref: "m".into(),
            tier: TierKey::Tier1,
        };
        feedback.record_outcome(&key, true).unwrap();
        feedback.record_outcome(&key, true).unwrap();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].harness, "codex");
    }

    #[test]
    fn restless_bandit_prefers_an_unused_arm_over_a_failing_one() {
        let configured = vec![
            Candidate {
                harness: "failed".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "fresh".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("failed".into(), profile(true, true));
        profiles.insert("fresh".into(), profile(true, true));
        let quotas = HashMap::new();
        let headroom = HashMap::new();
        let mut feedback = crate::ports::InMemoryFeedbackStore::default();
        let key = FeedbackKey {
            harness: "failed".into(),
            model_ref: "m".into(),
            tier: TierKey::Tier1,
        };
        for _ in 0..5 {
            feedback.record_outcome(&key, false).unwrap();
        }
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected[0].harness, "fresh");
    }

    #[test]
    fn restless_bandit_density_prefers_the_arm_with_more_window() {
        let configured = vec![
            Candidate {
                harness: "scarce".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "plenty".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("scarce".into(), profile(true, true));
        profiles.insert("plenty".into(), profile(true, true));
        let quotas = HashMap::new();
        let mut headroom = HashMap::new();
        headroom.insert("scarce".into(), 0.1);
        headroom.insert("plenty".into(), 0.9);
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::with_signals(vec![Box::new(RestlessBanditSignal::default())]);
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected[0].harness, "plenty");
    }
}
