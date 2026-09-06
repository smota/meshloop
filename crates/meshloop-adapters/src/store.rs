//! Repository pattern (design-patterns.md): SQLite is the only place SQL and the schema
//! live (ADR 0007 / runtime-design.md §4). Engine code depends only on the port traits.

use rusqlite::{Connection, OptionalExtension, params};

use meshloop_domain::evidence::{CandidateRef, Evidence};
use meshloop_engine::ports::{
    EvidenceStore, FeedbackCounters, FeedbackKey, RoutingFeedbackStore, StoreError,
};

const SCHEMA_VERSION: i64 = 1;

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(|e| StoreError::Io(e.to_string()))?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(|e| StoreError::Io(e.to_string()))?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Forward-only: refuses to run against a newer on-disk schema than it knows, rather
    /// than guessing (ADR 0007 / runtime-design.md §4).
    fn migrate(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS evidence (
                 task_id INTEGER NOT NULL,
                 attempt_id INTEGER NOT NULL,
                 revision TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 payload_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS routing_feedback (
                 harness TEXT NOT NULL,
                 model_ref TEXT NOT NULL,
                 tier TEXT NOT NULL,
                 success_count INTEGER NOT NULL DEFAULT 0,
                 failure_count INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (harness, model_ref, tier)
             );",
        )
        .map_err(|e| StoreError::Io(e.to_string()))?;

        let existing: Option<i64> = conn
            .query_row("SELECT version FROM schema_meta LIMIT 1", [], |r| r.get(0))
            .optional()
            .map_err(|e| StoreError::Io(e.to_string()))?;

        match existing {
            None => {
                conn.execute(
                    "INSERT INTO schema_meta (version) VALUES (?1)",
                    params![SCHEMA_VERSION],
                )
                .map_err(|e| StoreError::Io(e.to_string()))?;
            }
            Some(v) if v > SCHEMA_VERSION => {
                return Err(StoreError::Corrupt(format!(
                    "on-disk schema version {v} is newer than this build supports ({SCHEMA_VERSION})"
                )));
            }
            Some(_) => {}
        }
        Ok(())
    }
}

impl EvidenceStore for SqliteStore {
    fn record(&mut self, evidence: Evidence) -> Result<(), StoreError> {
        let candidate = evidence.candidate().clone();
        let kind = match &evidence {
            Evidence::Deterministic(_) => "deterministic",
            Evidence::ModelReview(_) => "model_review",
            Evidence::HumanAcceptance(_) => "human_acceptance",
        };
        let payload =
            serde_json::to_string(&evidence).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO evidence (task_id, attempt_id, revision, kind, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    candidate.task_id.0,
                    candidate.attempt_id.0,
                    candidate.revision,
                    kind,
                    payload
                ],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn evidence_for(&self, candidate: &CandidateRef) -> Vec<Evidence> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT payload_json FROM evidence WHERE task_id = ?1 AND attempt_id = ?2 AND revision = ?3",
        ) else {
            return vec![];
        };
        let Ok(rows) = stmt.query_map(
            params![
                candidate.task_id.0,
                candidate.attempt_id.0,
                candidate.revision
            ],
            |r| r.get::<_, String>(0),
        ) else {
            return vec![];
        };
        rows.filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect()
    }
}

impl RoutingFeedbackStore for SqliteStore {
    fn record_outcome(&mut self, key: &FeedbackKey, success: bool) {
        let tier = format!("{:?}", key.tier);
        let (succ_inc, fail_inc) = if success { (1, 0) } else { (0, 1) };
        let _ = self.conn.execute(
            "INSERT INTO routing_feedback (harness, model_ref, tier, success_count, failure_count)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(harness, model_ref, tier) DO UPDATE SET
                 success_count = success_count + ?4, failure_count = failure_count + ?5",
            params![key.harness, key.model_ref, tier, succ_inc, fail_inc],
        );
    }

    fn counters(&self, key: &FeedbackKey) -> FeedbackCounters {
        let tier = format!("{:?}", key.tier);
        self.conn
            .query_row(
                "SELECT success_count, failure_count FROM routing_feedback
                 WHERE harness = ?1 AND model_ref = ?2 AND tier = ?3",
                params![key.harness, key.model_ref, tier],
                |r| {
                    Ok(FeedbackCounters {
                        success: r.get(0)?,
                        failure: r.get(1)?,
                    })
                },
            )
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::evidence::{AttemptId, DeterministicEvidence};
    use meshloop_domain::task_graph::TaskId;
    use meshloop_engine::ports::TierKey;

    fn candidate() -> CandidateRef {
        CandidateRef {
            task_id: TaskId(1),
            attempt_id: AttemptId(1),
            revision: "deadbeef".into(),
        }
    }

    #[test]
    fn recorded_evidence_round_trips_exactly() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let evidence = Evidence::Deterministic(DeterministicEvidence {
            candidate: candidate(),
            tool: "cargo test".into(),
            tool_version: "1.98.0".into(),
            exit_code: 0,
            output_redacted: "ok".into(),
        });
        store.record(evidence.clone()).unwrap();
        let fetched = store.evidence_for(&candidate());
        assert_eq!(fetched, vec![evidence]);
    }

    #[test]
    fn evidence_for_an_unrelated_candidate_is_empty() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(store.evidence_for(&candidate()).is_empty());
    }

    #[test]
    fn routing_feedback_accumulates_across_calls() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let key = FeedbackKey {
            harness: "codex".into(),
            model_ref: "m".into(),
            tier: TierKey::Tier2,
        };
        store.record_outcome(&key, true);
        store.record_outcome(&key, true);
        store.record_outcome(&key, false);
        let counters = store.counters(&key);
        assert_eq!(counters.success, 2);
        assert_eq!(counters.failure, 1);
    }

    #[test]
    fn newer_on_disk_schema_version_is_refused_not_guessed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_meta (version INTEGER NOT NULL);")
            .unwrap();
        conn.execute(
            "INSERT INTO schema_meta (version) VALUES (?1)",
            params![9999],
        )
        .unwrap();
        let result = SqliteStore::migrate(&conn);
        assert!(matches!(result, Err(StoreError::Corrupt(_))));
    }
}
