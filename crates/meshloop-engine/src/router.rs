//! Quota-Aware Capability Routing (ADR 0009 / runtime-design.md §5). Strategy pattern
//! (design-patterns.md): scoring is a weighted sum over pluggable `RoutingSignal`s, so
//! adding a signal, harness, or model tier never touches this module's control flow.
//! `HistoricalSuccessSignal` may shrink its mean toward a predictive-success oracle
//! prior. That oracle is not a routing arm and cannot add or remove candidates.

use std::collections::HashMap;
use std::time::SystemTime;

use meshloop_domain::capability::{HarnessProfile, QuotaState};
use meshloop_domain::policy::{CouplingPenalty, ModelCapabilityTier, tier_fits};
use meshloop_domain::task_graph::Tier;

use crate::ports::{
    FeedbackKey, OracleQuery, PredictiveSuccessOracle, RoutingFeedbackStore, SessionSpendLedger,
    TierKey,
};

/// A configured candidate that routing filtered out, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub harness: String,
    /// The candidate's model; one harness may list several (issue #134).
    pub model_ref: String,
    pub reason: String,
}

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
    /// Predictive prior. Never a candidate and never consulted when `None`.
    pub oracle: Option<&'a dyn PredictiveSuccessOracle>,
    /// Charge before each oracle query. `None` predicts without spending.
    pub spend: Option<&'a SessionSpendLedger>,
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

/// The arm an oracle is queried with: `harness\nmodel_ref\nTierN`.
pub fn oracle_arm(harness: &str, model_ref: &str, tier: TierKey) -> String {
    format!("{harness}\n{model_ref}\n{tier:?}")
}

/// Local mean, optionally shrunk toward an oracle prior by at most 0.25.
/// Pulls are real feedback counts. A missing oracle, a declined charge, or a
/// predict error returns the local mean and never fails selection.
fn fused_historical_mean(mu_local: f64, successes: f64, n: u64, pi: f64) -> f64 {
    let raw = (successes + 4.0 * pi) / (n as f64 + 4.0);
    raw.clamp(mu_local - 0.25, mu_local + 0.25).clamp(0.0, 1.0)
}

impl RoutingSignal for HistoricalSuccessSignal {
    fn score(&self, candidate: &Candidate, ctx: &RoutingContext) -> SignalScore {
        let tier = TierKey::from(ctx.task_tier);
        let key = FeedbackKey {
            harness: candidate.harness.clone(),
            model_ref: candidate.model_ref.clone(),
            tier,
        };
        let counters = ctx.feedback.counters(&key).unwrap_or_default();
        let n = u64::from(counters.success + counters.failure);
        let mu_local = if n == 0 {
            0.5
        } else {
            f64::from(counters.success) / n as f64
        };
        let Some(oracle) = ctx.oracle else {
            return SignalScore(mu_local);
        };
        if let Some(spend) = ctx.spend
            && !spend.try_charge()
        {
            return SignalScore(mu_local);
        }
        let arm = oracle_arm(&candidate.harness, &candidate.model_ref, tier);
        match oracle.predict(&OracleQuery { arm, pulls: n }) {
            Ok(outcome) => SignalScore(fused_historical_mean(
                mu_local,
                f64::from(counters.success),
                n,
                outcome.prior.get(),
            )),
            Err(_) => SignalScore(mu_local),
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
/// Each (harness, model) candidate is an arm whose state evolves whether or not it is pulled
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

    /// Why each configured candidate fails `select`'s filters, in configuration order.
    /// Candidates that pass every filter are omitted. Used to make `NoCandidate` actionable.
    pub fn rejections(
        configured: &[Candidate],
        profiles: &HashMap<String, HarnessProfile>,
        quotas: &HashMap<String, QuotaState>,
        now: SystemTime,
        task_tier: Tier,
    ) -> Vec<Rejection> {
        configured
            .iter()
            .filter_map(|c| {
                let reason = if !tier_fits(task_tier, c.model_tier) {
                    format!(
                        "model_tier {:?} does not fit task {:?}",
                        c.model_tier, task_tier
                    )
                } else if let Some(r) = match profiles.get(&c.harness) {
                    Some(p) => p.rejection_reason(),
                    None => Some("not probed".into()),
                } {
                    r
                } else if quotas.get(&c.harness).is_some_and(|q| !q.is_available(now)) {
                    "in quota cooldown".into()
                } else {
                    return None;
                };
                Some(Rejection {
                    harness: c.harness.clone(),
                    model_ref: c.model_ref.clone(),
                    reason,
                })
            })
            .collect()
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
            reason: None,
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
            oracle: None,
            spend: None,
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
    fn absent_quota_record_stays_selectable() {
        let configured = vec![
            Candidate {
                harness: "codex".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "grok".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        profiles.insert("grok".into(), profile(true, true));
        let mut quotas = HashMap::new();
        let mut cooled = QuotaState::default();
        cooled.on_capacity_exhausted(SystemTime::now(), std::time::Duration::from_secs(3600));
        quotas.insert("codex".into(), cooled);
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let now = SystemTime::now();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            now,
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].harness, "grok");
        let rejected = Router::rejections(&configured, &profiles, &quotas, now, Tier::Tier1);
        assert!(rejected.iter().any(|r| r.harness == "codex"));
        assert!(rejected.iter().all(|r| r.harness != "grok"));
    }

    #[test]
    fn elapsed_open_quota_window_is_selectable() {
        let configured = vec![Candidate {
            harness: "codex".into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let opened = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let now = opened + std::time::Duration::from_secs(30);
        let mut stale = QuotaState::default();
        stale.on_capacity_exhausted(opened, std::time::Duration::from_secs(1));
        let mut quotas = HashMap::new();
        quotas.insert("codex".into(), stale);
        let headroom = HashMap::new();
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let router = Router::default();
        let selected = router.select(
            &configured,
            &profiles,
            &quotas,
            now,
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].harness, "codex");
        let rejected = Router::rejections(&configured, &profiles, &quotas, now, Tier::Tier1);
        assert!(rejected.is_empty());
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

    #[test]
    fn rejections_name_each_filtered_candidate_and_why() {
        let cand = |h: &str, t| Candidate {
            harness: h.into(),
            model_ref: "m".into(),
            model_tier: t,
        };
        let configured = vec![
            cand("ok", ModelCapabilityTier::TopTier),
            cand("weak", ModelCapabilityTier::MidTier),
            cand("hung", ModelCapabilityTier::TopTier),
            cand("missing", ModelCapabilityTier::TopTier),
        ];
        let mut profiles = HashMap::new();
        profiles.insert("ok".into(), profile(true, true));
        profiles.insert("weak".into(), profile(true, true));
        profiles.insert(
            "hung".into(),
            HarnessProfile::unsupported("hung", "probe timed out after 15s"),
        );
        let rejected = Router::rejections(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            Tier::Tier3,
        );
        let names: Vec<_> = rejected.iter().map(|r| r.harness.as_str()).collect();
        assert_eq!(names, ["weak", "hung", "missing"]);
        assert!(rejected[0].reason.contains("model_tier"));
        assert_eq!(rejected[1].reason, "probe timed out after 15s");
        assert_eq!(rejected[2].reason, "not probed");
    }

    #[test]
    fn one_harness_with_two_models_is_two_arms_with_separate_feedback() {
        // Issue #134: one harness entry lists two models; each is its own arm keyed by
        // (harness, model_ref, tier), so feedback on one never moves the other.
        let configured = vec![
            Candidate {
                harness: "codex".into(),
                model_ref: "gpt-a".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "codex".into(),
                model_ref: "gpt-b".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let headroom = HashMap::new();
        let mut feedback = crate::ports::InMemoryFeedbackStore::default();
        let key = |model: &str| FeedbackKey {
            harness: "codex".into(),
            model_ref: model.into(),
            tier: TierKey::Tier1,
        };
        for _ in 0..3 {
            feedback.record_outcome(&key("gpt-a"), false).unwrap();
        }
        feedback.record_outcome(&key("gpt-b"), true).unwrap();
        assert_eq!(feedback.counters(&key("gpt-a")).unwrap().failure, 3);
        assert_eq!(feedback.counters(&key("gpt-a")).unwrap().success, 0);
        assert_eq!(feedback.counters(&key("gpt-b")).unwrap().success, 1);
        assert_eq!(feedback.counters(&key("gpt-b")).unwrap().failure, 0);

        let selected = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        let arms: Vec<_> = selected
            .iter()
            .map(|c| (c.harness.as_str(), c.model_ref.as_str()))
            .collect();
        assert_eq!(arms, [("codex", "gpt-b"), ("codex", "gpt-a")]);

        // A model whose tier does not fit is rejected on its own, naming its model.
        let mut mixed = configured.clone();
        mixed[0].model_tier = ModelCapabilityTier::MidTier;
        let rejected = Router::rejections(
            &mixed,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            Tier::Tier3,
        );
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].harness, "codex");
        assert_eq!(rejected[0].model_ref, "gpt-a");
    }

    fn codex() -> Candidate {
        Candidate {
            harness: "codex".into(),
            model_ref: "m".into(),
            model_tier: ModelCapabilityTier::TopTier,
        }
    }

    fn scripted_prior(arm: &str, pulls: u64, pi: f64) -> crate::ports::ScriptedDecisionPort {
        use meshloop_domain::decision::Confidence;

        let pulls_decimal = pulls.to_string();
        let mut port = crate::ports::ScriptedDecisionPort::default();
        port.insert_oracle(
            crate::ports::decision_fingerprint("oracle", &[arm, pulls_decimal.as_str()]),
            crate::ports::OracleOutcome {
                prior: Confidence::try_new(pi).unwrap(),
            },
        );
        port
    }

    #[test]
    fn cold_arm_exploration_is_exactly_point_seven_and_oracle_is_not_a_routing_arm() {
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let headroom = HashMap::new();
        let cold = codex();
        let score =
            RestlessBanditSignal::default().score(&cold, &ctx(Tier::Tier1, &headroom, &feedback));
        assert_eq!(score, SignalScore(0.7));

        let configured = vec![
            cold,
            Candidate {
                harness: "claude-code".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        profiles.insert("claude-code".into(), profile(false, true));
        profiles.insert("jev".into(), profile(true, true));
        profiles.insert("oracle".into(), profile(true, true));
        let selected = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &ctx(Tier::Tier1, &headroom, &feedback),
        );
        let names: Vec<_> = selected.iter().map(|c| c.harness.as_str()).collect();
        assert_eq!(names, ["codex"]);
    }

    #[test]
    fn prior_clip_limits_fused_historical_mean() {
        let cases = [
            (0u32, 0u32, 1.0, 0.75),
            (0, 0, 0.0, 0.25),
            (0, 0, 0.5, 0.5),
            (4, 0, 0.0, 0.75),
            (4, 0, 1.0, 1.0),
        ];
        for (successes, failures, pi, expected) in cases {
            let mut feedback = crate::ports::InMemoryFeedbackStore::default();
            let key = FeedbackKey {
                harness: "codex".into(),
                model_ref: "m".into(),
                tier: TierKey::Tier1,
            };
            for _ in 0..successes {
                feedback.record_outcome(&key, true).unwrap();
            }
            for _ in 0..failures {
                feedback.record_outcome(&key, false).unwrap();
            }
            let pulls = u64::from(successes + failures);
            let arm = format!("codex\nm\n{:?}", TierKey::Tier1);
            let port = scripted_prior(&arm, pulls, pi);
            let headroom = HashMap::new();
            let mut routing = ctx(Tier::Tier1, &headroom, &feedback);
            routing.oracle = Some(&port);
            let score = HistoricalSuccessSignal.score(&codex(), &routing);
            assert_eq!(
                score,
                SignalScore(expected),
                "successes={successes} failures={failures} pi={pi}"
            );
        }
    }

    #[test]
    fn spend_exhaustion_keeps_cold_mean_and_the_candidate() {
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let headroom = HashMap::new();
        let ledger = crate::ports::SessionSpendLedger::new(0, 1);
        let port = scripted_prior("codex\nm\nTier1", 0, 1.0);
        let candidate = codex();
        let mut routing = ctx(Tier::Tier1, &headroom, &feedback);
        routing.oracle = Some(&port);
        routing.spend = Some(&ledger);
        assert_eq!(
            HistoricalSuccessSignal.score(&candidate, &routing),
            SignalScore(0.5)
        );
        assert_eq!(ledger.remaining(), 0);

        let configured = vec![candidate];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let selected = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &routing,
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].harness, "codex");
        assert_eq!(ledger.remaining(), 0);
    }

    #[test]
    fn one_successful_charge_fuses_a_cold_prior() {
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let headroom = HashMap::new();
        let ledger = crate::ports::SessionSpendLedger::new(1, 1);
        let port = scripted_prior("codex\nm\nTier1", 0, 1.0);
        let mut routing = ctx(Tier::Tier1, &headroom, &feedback);
        routing.oracle = Some(&port);
        routing.spend = Some(&ledger);
        assert_eq!(
            HistoricalSuccessSignal.score(&codex(), &routing),
            SignalScore(0.75)
        );
        assert_eq!(ledger.remaining(), 0);
    }

    fn priors_oracle(entries: &[(&str, &str, Tier, u32, u32)]) -> crate::priors::PriorsOracle {
        use meshloop_domain::advisory::{AttemptRef, DispatchMode, PriorEntry, PriorKey};
        let entries: Vec<PriorEntry> = entries
            .iter()
            .enumerate()
            .map(|(index, (h, m, tier, n, s))| PriorEntry {
                index,
                key: PriorKey {
                    harness: (*h).into(),
                    model_ref: (*m).into(),
                    tier: *tier,
                    mode: DispatchMode::Subprocess,
                },
                attempts: *n,
                successes: *s,
                provenance: (0..*n)
                    .map(|i| AttemptRef {
                        graph_id: "g".into(),
                        task_id: 1,
                        attempt_id: u64::from(i),
                        passed: i < *s,
                    })
                    .collect(),
            })
            .collect();
        let dispatch = entries_harnesses(&entries);
        crate::priors::PriorsOracle::new(
            &crate::ports::PriorsSnapshot {
                sequence: 1,
                priors_sha256: "0".repeat(64),
                entries,
            },
            &dispatch,
        )
    }

    fn entries_harnesses(
        entries: &[meshloop_domain::advisory::PriorEntry],
    ) -> HashMap<String, meshloop_domain::advisory::DispatchMode> {
        entries
            .iter()
            .map(|e| {
                (
                    e.key.harness.clone(),
                    meshloop_domain::advisory::DispatchMode::Subprocess,
                )
            })
            .collect()
    }

    /// LP test 2: for any π and n, the fused mean stays within 0.25 of the local mean.
    #[test]
    fn fused_mean_never_moves_more_than_the_bound() {
        for n in 0u64..=40 {
            for s in 0..=n {
                let mu_local = if n == 0 { 0.5 } else { s as f64 / n as f64 };
                for step in 0..=100 {
                    let pi = f64::from(step) / 100.0;
                    let fused = fused_historical_mean(mu_local, s as f64, n, pi);
                    assert!(
                        (fused - mu_local).abs() <= 0.25 + 1e-12,
                        "n={n} s={s} pi={pi} fused={fused}"
                    );
                    assert!((0.0..=1.0).contains(&fused));
                }
            }
        }
    }

    /// LP test 2, "no doubling": with priors on, a candidate's whole routing score moves
    /// by at most 0.25, because only the historical-success signal reads the oracle and
    /// it clips once.
    #[test]
    fn priors_move_the_total_score_by_at_most_the_bound() {
        let router = Router::default();
        for (n, s) in [(0u32, 0u32), (1, 0), (3, 3), (10, 2), (40, 39)] {
            let mut feedback = crate::ports::InMemoryFeedbackStore::default();
            let key = FeedbackKey {
                harness: "codex".into(),
                model_ref: "m".into(),
                tier: TierKey::Tier2,
            };
            for i in 0..n {
                feedback.record_outcome(&key, i < s).unwrap();
            }
            for (pn, ps) in [(5u32, 0u32), (5, 5), (4096, 4096), (4096, 0), (20, 10)] {
                let oracle = priors_oracle(&[("codex", "m", Tier::Tier2, pn, ps)]);
                let headroom = HashMap::new();
                let off = ctx(Tier::Tier2, &headroom, &feedback);
                let mut on = ctx(Tier::Tier2, &headroom, &feedback);
                on.oracle = Some(&oracle);
                let total = |c: &RoutingContext| -> f64 {
                    router
                        .signals
                        .iter()
                        .map(|sig| sig.score(&codex(), c).0)
                        .sum()
                };
                let moved = (total(&on) - total(&off)).abs();
                assert!(
                    moved <= 0.25 + 1e-12,
                    "n={n} s={s} prior={ps}/{pn} moved {moved}"
                );
            }
        }
    }

    /// LP test 1: priors never add or remove a candidate, whatever they say, including
    /// priors for candidates the filters remove and for arms nobody configured.
    #[test]
    fn priors_never_change_the_candidate_set() {
        let configured = vec![
            Candidate {
                harness: "codex".into(),
                model_ref: "top".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "codex".into(),
                model_ref: "mid".into(),
                model_tier: ModelCapabilityTier::MidTier,
            },
            Candidate {
                harness: "cooled".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "unprobed".into(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        profiles.insert("cooled".into(), profile(true, true));
        let mut quotas = HashMap::new();
        let mut q = QuotaState::default();
        q.on_capacity_exhausted(SystemTime::now(), std::time::Duration::from_secs(3600));
        quotas.insert("cooled".into(), q);
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let headroom = HashMap::new();
        let now = SystemTime::now();
        for tier in [Tier::Tier1, Tier::Tier2, Tier::Tier3] {
            let off: std::collections::BTreeSet<_> = Router::default()
                .select(
                    &configured,
                    &profiles,
                    &quotas,
                    now,
                    &ctx(tier, &headroom, &feedback),
                )
                .into_iter()
                .map(|c| (c.harness.clone(), c.model_ref.clone()))
                .collect();
            for (pn, ps) in [(5u32, 0u32), (5, 5), (4096, 4096)] {
                let oracle = priors_oracle(&[
                    ("codex", "top", tier, pn, ps),
                    ("codex", "mid", tier, pn, pn - ps),
                    ("cooled", "m", tier, pn, ps),
                    ("unprobed", "m", tier, pn, ps),
                    ("ghost", "m", tier, pn, ps),
                ]);
                let mut routing = ctx(tier, &headroom, &feedback);
                routing.oracle = Some(&oracle);
                let on: std::collections::BTreeSet<_> = Router::default()
                    .select(&configured, &profiles, &quotas, now, &routing)
                    .into_iter()
                    .map(|c| (c.harness.clone(), c.model_ref.clone()))
                    .collect();
                assert_eq!(on, off, "tier {tier:?} prior {ps}/{pn}");
                assert!(on.iter().all(|(h, _)| h == "codex"));
            }
        }
    }

    /// A strong prior reorders two otherwise equal fresh arms, within the bound.
    #[test]
    fn a_prior_reorders_equal_fresh_arms() {
        let configured = vec![
            Candidate {
                harness: "codex".into(),
                model_ref: "weak".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
            Candidate {
                harness: "codex".into(),
                model_ref: "strong".into(),
                model_tier: ModelCapabilityTier::TopTier,
            },
        ];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let feedback = crate::ports::InMemoryFeedbackStore::default();
        let headroom = HashMap::new();
        let off = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &ctx(Tier::Tier2, &headroom, &feedback),
        );
        assert_eq!(off[0].model_ref, "weak");
        let oracle = priors_oracle(&[
            ("codex", "weak", Tier::Tier2, 8, 1),
            ("codex", "strong", Tier::Tier2, 8, 8),
        ]);
        let mut routing = ctx(Tier::Tier2, &headroom, &feedback);
        routing.oracle = Some(&oracle);
        let on = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &routing,
        );
        assert_eq!(on[0].model_ref, "strong");
        // A prior keyed on another task tier does not apply.
        let other_tier = priors_oracle(&[("codex", "strong", Tier::Tier3, 8, 8)]);
        let mut routing = ctx(Tier::Tier2, &headroom, &feedback);
        routing.oracle = Some(&other_tier);
        let on = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &routing,
        );
        assert_eq!(on[0].model_ref, "weak");
    }

    #[test]
    fn predict_error_after_charge_keeps_local_mean_and_the_candidate() {
        let mut feedback = crate::ports::InMemoryFeedbackStore::default();
        let key = FeedbackKey {
            harness: "codex".into(),
            model_ref: "m".into(),
            tier: TierKey::Tier1,
        };
        feedback.record_outcome(&key, true).unwrap();
        feedback.record_outcome(&key, true).unwrap();
        let headroom = HashMap::new();
        let ledger = crate::ports::SessionSpendLedger::new(1, 1);
        let port = crate::ports::ScriptedDecisionPort::default();
        let candidate = codex();
        let mut routing = ctx(Tier::Tier1, &headroom, &feedback);
        routing.oracle = Some(&port);
        routing.spend = Some(&ledger);
        assert_eq!(
            HistoricalSuccessSignal.score(&candidate, &routing),
            SignalScore(1.0)
        );
        assert_eq!(ledger.remaining(), 0);

        let configured = vec![candidate];
        let mut profiles = HashMap::new();
        profiles.insert("codex".into(), profile(true, true));
        let selected = Router::default().select(
            &configured,
            &profiles,
            &HashMap::new(),
            SystemTime::now(),
            &routing,
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].harness, "codex");
        assert_eq!(ledger.remaining(), 0);
    }
}
