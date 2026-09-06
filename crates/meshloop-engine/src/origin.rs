//! Originating supervisor session. Never a worker on the same pane/session.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Origin {
    pub harness: Option<String>,
    pub session: Option<String>,
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

impl Origin {
    pub fn from_flags(harness: Option<String>, session: Option<String>) -> Self {
        Self { harness, session }
    }

    /// Flags win; otherwise `MESHLOOP_ORIGIN_HARNESS` / `MESHLOOP_ORIGIN_SESSION`.
    pub fn from_flags_or_env(harness: Option<String>, session: Option<String>) -> Self {
        Self {
            harness: harness.or_else(|| env_nonempty("MESHLOOP_ORIGIN_HARNESS")),
            session: session.or_else(|| env_nonempty("MESHLOOP_ORIGIN_SESSION")),
        }
    }

    pub fn is_set(&self) -> bool {
        self.harness.is_some() || self.session.is_some()
    }

    /// Supervisor-only: refuse to dispatch a worker into the origin session.
    pub fn blocks_worker(&self, worker_harness: &str, worker_session: Option<&str>) -> bool {
        match (&self.harness, &self.session, worker_session) {
            (Some(origin_h), Some(origin_s), Some(ws)) => {
                origin_h == worker_harness && origin_s == ws
            }
            (Some(origin_h), None, None) => origin_h == worker_harness,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_harness_and_session_is_blocked() {
        let o = Origin::from_flags(Some("grok".into()), Some("sess-1".into()));
        assert!(o.blocks_worker("grok", Some("sess-1")));
        assert!(!o.blocks_worker("claude", Some("sess-1")));
        assert!(!o.blocks_worker("grok", Some("other")));
    }
}
