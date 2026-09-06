//! HarnessProfile, the ADR 0003 error taxonomy, and the circuit-breaker QuotaState from
//! runtime-design.md §2. Pure data — no probing or subprocess logic lives here.

use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Compatible,
    Degraded,
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct HarnessProfile {
    pub harness: String,
    pub version: String,
    pub compatibility: Compatibility,
    pub supports_noninteractive: bool,
    pub supports_structured_output: bool,
    pub supports_cancellation: bool,
}

impl HarnessProfile {
    /// A harness must be Compatible/Degraded, non-interactive, and cancellable to ever be
    /// a routing candidate — an unconfirmed capability is never assumed present.
    pub fn is_dispatchable(&self) -> bool {
        self.compatibility != Compatibility::Unsupported
            && self.supports_noninteractive
            && self.supports_cancellation
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HarnessError {
    CapacityExhausted { retry_after: Duration },
    Unsupported,
    Timeout,
    ProcessFault { detail: String },
}

/// Circuit breaker per runtime-design.md §2: Closed (available) -> Open (cooldown) ->
/// HalfOpen (first probe after cooldown) -> Closed or back to Open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breaker {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct QuotaState {
    state: Breaker,
    opened_at: Option<SystemTime>,
    cooldown: Duration,
}

impl Default for QuotaState {
    fn default() -> Self {
        Self {
            state: Breaker::Closed,
            opened_at: None,
            cooldown: Duration::ZERO,
        }
    }
}

impl QuotaState {
    pub fn from_parts(breaker: Breaker, opened_at: Option<SystemTime>, cooldown: Duration) -> Self {
        Self {
            state: breaker,
            opened_at,
            cooldown,
        }
    }

    pub fn breaker(&self) -> Breaker {
        self.state
    }

    pub fn opened_at(&self) -> Option<SystemTime> {
        self.opened_at
    }

    pub fn cooldown(&self) -> Duration {
        self.cooldown
    }

    /// A candidate with no observed exhaustion is assumed available, never assumed to
    /// have unlimited headroom — this is the reactive tracking runtime-design.md §5 requires.
    pub fn is_available(&self, now: SystemTime) -> bool {
        match self.state {
            Breaker::Closed => true,
            Breaker::HalfOpen => true,
            Breaker::Open => match self.opened_at {
                Some(opened) => {
                    now.duration_since(opened).unwrap_or(Duration::ZERO) >= self.cooldown
                }
                None => true,
            },
        }
    }

    pub fn on_capacity_exhausted(&mut self, now: SystemTime, retry_after: Duration) {
        self.state = Breaker::Open;
        self.opened_at = Some(now);
        self.cooldown = retry_after;
    }

    pub fn on_success(&mut self) {
        self.state = Breaker::Closed;
        self.opened_at = None;
    }

    /// Call before dispatch: if cooldown has elapsed, the breaker moves to HalfOpen so the
    /// next attempt is a probe, not an assumption of full health.
    pub fn refresh(&mut self, now: SystemTime) {
        if self.state == Breaker::Open
            && let Some(opened) = self.opened_at
            && now.duration_since(opened).unwrap_or(Duration::ZERO) >= self.cooldown
        {
            self.state = Breaker::HalfOpen;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_breaker_is_available() {
        let q = QuotaState::default();
        assert!(q.is_available(SystemTime::now()));
        assert_eq!(q.breaker(), Breaker::Closed);
    }

    #[test]
    fn open_breaker_blocks_until_cooldown_elapses() {
        let mut q = QuotaState::default();
        let t0 = SystemTime::now();
        q.on_capacity_exhausted(t0, Duration::from_secs(60));
        assert!(!q.is_available(t0 + Duration::from_secs(30)));
        assert!(q.is_available(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn refresh_moves_open_to_half_open_after_cooldown() {
        let mut q = QuotaState::default();
        let t0 = SystemTime::now();
        q.on_capacity_exhausted(t0, Duration::from_secs(10));
        q.refresh(t0 + Duration::from_secs(11));
        assert_eq!(q.breaker(), Breaker::HalfOpen);
    }

    #[test]
    fn success_closes_the_breaker() {
        let mut q = QuotaState::default();
        let t0 = SystemTime::now();
        q.on_capacity_exhausted(t0, Duration::from_secs(10));
        q.on_success();
        assert_eq!(q.breaker(), Breaker::Closed);
        assert!(q.is_available(t0));
    }

    #[test]
    fn unsupported_or_non_cancellable_harness_is_never_dispatchable() {
        let base = HarnessProfile {
            harness: "test".into(),
            version: "1.0".into(),
            compatibility: Compatibility::Compatible,
            supports_noninteractive: true,
            supports_structured_output: false,
            supports_cancellation: true,
        };
        assert!(base.is_dispatchable());

        let mut unsupported = base.clone();
        unsupported.compatibility = Compatibility::Unsupported;
        assert!(!unsupported.is_dispatchable());

        let mut no_cancel = base;
        no_cancel.supports_cancellation = false;
        assert!(!no_cancel.is_dispatchable());
    }
}
