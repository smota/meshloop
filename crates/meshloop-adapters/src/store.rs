//! Repository pattern (design-patterns.md): SQLite is the only place SQL and the schema
//! live (ADR 0007 / runtime-design.md §4). Engine code depends only on the port traits.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use meshloop_domain::capability::{Breaker, QuotaState};
use meshloop_domain::evidence::{AttemptId, CandidateRef, Evidence};
use meshloop_domain::state::{Event, PlanState, TaskState};
use meshloop_domain::task_graph::TaskId;
use meshloop_engine::ports::{
    AttemptRow, EventLog, EvidenceStore, FeedbackCounters, FeedbackKey, QuotaStore,
    RoutingFeedbackStore, RunRow, RunStore, StoreError, TransitionRecord,
};

const SCHEMA_VERSION: i64 = 4;

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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Io(e.to_string()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| StoreError::Io(e.to_string()))?;
        conn.busy_timeout(Duration::from_millis(5000))
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

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
             );
             CREATE TABLE IF NOT EXISTS events (
                 event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 graph_id TEXT NOT NULL,
                 task_id INTEGER NOT NULL,
                 attempt_id INTEGER,
                 from_state TEXT NOT NULL,
                 to_state TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 reason TEXT,
                 executor TEXT NOT NULL,
                 evidence_ref TEXT,
                 occurred_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS runs (
                 graph_id TEXT PRIMARY KEY,
                 plan_state TEXT NOT NULL,
                 run_base TEXT NOT NULL,
                 integrate_ref TEXT NOT NULL,
                 plan_json TEXT NOT NULL,
                 plan_sha256 TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 review_note TEXT
             );
             CREATE TABLE IF NOT EXISTS attempts (
                 attempt_id INTEGER PRIMARY KEY,
                 graph_id TEXT NOT NULL,
                 task_id INTEGER NOT NULL,
                 harness TEXT,
                 model_ref TEXT,
                 worktree_path TEXT,
                 pid INTEGER,
                 image_name TEXT,
                 started_at TEXT,
                 ended_at TEXT,
                 outcome TEXT,
                 pane_id TEXT
             );
             CREATE TABLE IF NOT EXISTS quota_state (
                 harness TEXT PRIMARY KEY,
                 breaker TEXT NOT NULL,
                 opened_at TEXT,
                 cooldown_ms INTEGER NOT NULL
             );",
        )
        .map_err(|e| StoreError::Io(e.to_string()))?;

        if !column_exists(conn, "attempts", "pane_id")? {
            conn.execute("ALTER TABLE attempts ADD COLUMN pane_id TEXT", [])
                .map_err(|e| StoreError::Io(e.to_string()))?;
        }
        if !column_exists(conn, "runs", "review_note")? {
            conn.execute("ALTER TABLE runs ADD COLUMN review_note TEXT", [])
                .map_err(|e| StoreError::Io(e.to_string()))?;
        }

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
            Some(v) if v < SCHEMA_VERSION => {
                conn.execute(
                    "UPDATE schema_meta SET version = ?1",
                    params![SCHEMA_VERSION],
                )
                .map_err(|e| StoreError::Io(e.to_string()))?;
            }
            Some(_) => {}
        }
        Ok(())
    }
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, StoreError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| StoreError::Io(e.to_string()))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| StoreError::Io(e.to_string()))?;
    for name in names {
        if name.map_err(|e| StoreError::Io(e.to_string()))? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn parse_state(s: &str) -> Result<TaskState, StoreError> {
    match s {
        "Pending" => Ok(TaskState::Pending),
        "Ready" => Ok(TaskState::Ready),
        "Running" => Ok(TaskState::Running),
        "Verifying" => Ok(TaskState::Verifying),
        "AwaitingReview" => Ok(TaskState::AwaitingReview),
        "Accepted" => Ok(TaskState::Accepted),
        "Integrated" => Ok(TaskState::Integrated),
        "Failed" => Ok(TaskState::Failed),
        "Cancelled" => Ok(TaskState::Cancelled),
        "Blocked" => Ok(TaskState::Blocked),
        other => Err(StoreError::Corrupt(format!("unknown state {other}"))),
    }
}

fn parse_event(s: &str) -> Result<Event, StoreError> {
    match s {
        "DependencySatisfied" => Ok(Event::DependencySatisfied),
        "DependencyFailedOrScopeRevoked" => Ok(Event::DependencyFailedOrScopeRevoked),
        "AttemptStarted" => Ok(Event::AttemptStarted),
        "HarnessExited" => Ok(Event::HarnessExited),
        "HarnessCrashedOrTimeout" => Ok(Event::HarnessCrashedOrTimeout),
        "Cancel" => Ok(Event::Cancel),
        "DeterministicChecksPassed" => Ok(Event::DeterministicChecksPassed),
        "DeterministicChecksFailed" => Ok(Event::DeterministicChecksFailed),
        "ModelReviewPassed" => Ok(Event::ModelReviewPassed),
        "HumanAcceptanceRecorded" => Ok(Event::HumanAcceptanceRecorded),
        "ReviewRejected" => Ok(Event::ReviewRejected),
        "IntegrationOwnerMerge" => Ok(Event::IntegrationOwnerMerge),
        "StaleBaseDetected" => Ok(Event::StaleBaseDetected),
        "RetryAuthorized" => Ok(Event::RetryAuthorized),
        other => Err(StoreError::Corrupt(format!("unknown event {other}"))),
    }
}

fn parse_plan_state(s: &str) -> Result<PlanState, StoreError> {
    match s {
        "AwaitingPlanReview" => Ok(PlanState::AwaitingPlanReview),
        "PlanAccepted" => Ok(PlanState::PlanAccepted),
        "PlanDeclined" => Ok(PlanState::PlanDeclined),
        other => Err(StoreError::Corrupt(format!("unknown plan state {other}"))),
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

    fn evidence_for(&self, candidate: &CandidateRef) -> Result<Vec<Evidence>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload_json FROM evidence WHERE task_id = ?1 AND attempt_id = ?2 AND revision = ?3",
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![
                    candidate.task_id.0,
                    candidate.attempt_id.0,
                    candidate.revision
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            let json = row.map_err(|e| StoreError::Io(e.to_string()))?;
            let ev: Evidence =
                serde_json::from_str(&json).map_err(|e| StoreError::Corrupt(e.to_string()))?;
            out.push(ev);
        }
        Ok(out)
    }
}

impl RoutingFeedbackStore for SqliteStore {
    fn record_outcome(&mut self, key: &FeedbackKey, success: bool) -> Result<(), StoreError> {
        let tier = format!("{:?}", key.tier);
        let (succ_inc, fail_inc) = if success { (1, 0) } else { (0, 1) };
        self.conn
            .execute(
                "INSERT INTO routing_feedback (harness, model_ref, tier, success_count, failure_count)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(harness, model_ref, tier) DO UPDATE SET
                 success_count = success_count + ?4, failure_count = failure_count + ?5",
                params![key.harness, key.model_ref, tier, succ_inc, fail_inc],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn counters(&self, key: &FeedbackKey) -> Result<FeedbackCounters, StoreError> {
        let tier = format!("{:?}", key.tier);
        Ok(self
            .conn
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
            .optional()
            .map_err(|e| StoreError::Io(e.to_string()))?
            .unwrap_or_default())
    }
}

impl EventLog for SqliteStore {
    fn append(&mut self, record: TransitionRecord) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO events (graph_id, task_id, attempt_id, from_state, to_state, event_type, reason, executor, occurred_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.graph_id,
                    record.task_id.0,
                    record.attempt_id.map(|a| a.0),
                    format!("{:?}", record.from),
                    format!("{:?}", record.to),
                    format!("{:?}", record.event),
                    record.reason,
                    record.executor,
                    record.occurred_at,
                ],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn records_for_graph(&self, graph_id: &str) -> Result<Vec<TransitionRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT graph_id, task_id, attempt_id, from_state, to_state, event_type, reason, executor, occurred_at
                 FROM events WHERE graph_id = ?1 ORDER BY event_id ASC",
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let rows = stmt
            .query_map(params![graph_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, Option<u32>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            let (gid, tid, aid, from, to, ev, reason, executor, occurred) =
                row.map_err(|e| StoreError::Io(e.to_string()))?;
            out.push(TransitionRecord {
                graph_id: gid,
                task_id: TaskId(tid),
                attempt_id: aid.map(AttemptId),
                from: parse_state(&from)?,
                to: parse_state(&to)?,
                event: parse_event(&ev)?,
                reason,
                executor,
                occurred_at: occurred,
            });
        }
        Ok(out)
    }
}

impl QuotaStore for SqliteStore {
    fn load_quota(&self, harness: &str) -> Result<QuotaState, StoreError> {
        let row: Option<(String, Option<String>, i64)> = self
            .conn
            .query_row(
                "SELECT breaker, opened_at, cooldown_ms FROM quota_state WHERE harness = ?1",
                params![harness],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| StoreError::Io(e.to_string()))?;
        match row {
            None => Ok(QuotaState::default()),
            Some((breaker, opened, cooldown_ms)) => {
                let b = match breaker.as_str() {
                    "Open" => Breaker::Open,
                    "HalfOpen" => Breaker::HalfOpen,
                    _ => Breaker::Closed,
                };
                let opened_at = opened
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
                Ok(QuotaState::from_parts(
                    b,
                    opened_at,
                    Duration::from_millis(cooldown_ms as u64),
                ))
            }
        }
    }

    fn save_quota(&mut self, harness: &str, state: &QuotaState) -> Result<(), StoreError> {
        let opened = state
            .opened_at()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        let opened_s = opened.map(|d| d.as_secs().to_string());
        self.conn
            .execute(
                "INSERT INTO quota_state (harness, breaker, opened_at, cooldown_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(harness) DO UPDATE SET breaker=?2, opened_at=?3, cooldown_ms=?4",
                params![
                    harness,
                    format!("{:?}", state.breaker()),
                    opened_s,
                    state.cooldown().as_millis() as i64,
                ],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }
}

impl RunStore for SqliteStore {
    fn save_run(&mut self, row: &RunRow) -> Result<(), StoreError> {
        let plan_state = match row.plan_state {
            PlanState::AwaitingPlanReview => "AwaitingPlanReview",
            PlanState::PlanAccepted => "PlanAccepted",
            PlanState::PlanDeclined => "PlanDeclined",
        };
        self.conn
            .execute(
                "INSERT INTO runs (graph_id, plan_state, run_base, integrate_ref, plan_json, plan_sha256, created_at, review_note)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(graph_id) DO UPDATE SET
                    plan_state=?2, run_base=?3, integrate_ref=?4, plan_json=?5, plan_sha256=?6, review_note=?8",
                params![
                    row.graph_id,
                    plan_state,
                    row.run_base,
                    row.integrate_ref,
                    row.plan_json,
                    row.plan_sha256,
                    row.created_at,
                    row.review_note,
                ],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn load_run(&self, graph_id: &str) -> Result<Option<RunRow>, StoreError> {
        self.conn
            .query_row(
                "SELECT graph_id, plan_state, run_base, integrate_ref, plan_json, plan_sha256, created_at, review_note
                 FROM runs WHERE graph_id = ?1",
                params![graph_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| StoreError::Io(e.to_string()))?
            .map(|(gid, ps, rb, ir, pj, ph, ca, note)| {
                Ok(RunRow {
                    graph_id: gid,
                    plan_state: parse_plan_state(&ps)?,
                    run_base: rb,
                    integrate_ref: ir,
                    plan_json: pj,
                    plan_sha256: ph,
                    created_at: ca,
                    review_note: note,
                })
            })
            .transpose()
    }

    fn latest_run(&self) -> Result<Option<RunRow>, StoreError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT graph_id FROM runs ORDER BY created_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| StoreError::Io(e.to_string()))?;
        match id {
            Some(g) => self.load_run(&g),
            None => Ok(None),
        }
    }

    fn list_runs(&self) -> Result<Vec<RunRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT graph_id FROM runs ORDER BY created_at DESC")
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let ids: Result<Vec<String>, _> = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| StoreError::Io(e.to_string()))?
            .collect();
        let ids = ids.map_err(|e| StoreError::Io(e.to_string()))?;
        let mut out = Vec::new();
        for id in ids {
            if let Some(row) = self.load_run(&id)? {
                out.push(row);
            }
        }
        Ok(out)
    }

    fn next_attempt_id(&self) -> Result<AttemptId, StoreError> {
        let max: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(attempt_id), 0) FROM attempts",
                [],
                |r| r.get(0),
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(AttemptId((max as u32) + 1))
    }

    fn save_attempt(&mut self, row: &AttemptRow) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO attempts (attempt_id, graph_id, task_id, harness, model_ref, worktree_path, pid, image_name, pane_id, started_at, ended_at, outcome)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(attempt_id) DO UPDATE SET
                    harness=?4, model_ref=?5, worktree_path=?6, pid=?7, image_name=?8, pane_id=?9, started_at=?10, ended_at=?11, outcome=?12",
                params![
                    row.attempt_id.0,
                    row.graph_id,
                    row.task_id.0,
                    row.harness,
                    row.model_ref,
                    row.worktree_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                    row.pid.map(|p| p as i64),
                    row.image_name,
                    row.pane_id,
                    row.started_at,
                    row.ended_at,
                    row.outcome,
                ],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn load_attempt(&self, attempt_id: AttemptId) -> Result<Option<AttemptRow>, StoreError> {
        self.conn
            .query_row(
                "SELECT attempt_id, graph_id, task_id, harness, model_ref, worktree_path, pid, image_name, pane_id, started_at, ended_at, outcome
                 FROM attempts WHERE attempt_id = ?1",
                params![attempt_id.0],
                |r| {
                    Ok(AttemptRow {
                        attempt_id: AttemptId(r.get(0)?),
                        graph_id: r.get(1)?,
                        task_id: TaskId(r.get(2)?),
                        harness: r.get(3)?,
                        model_ref: r.get(4)?,
                        worktree_path: r.get::<_, Option<String>>(5)?.map(PathBuf::from),
                        pid: r.get::<_, Option<i64>>(6)?.map(|p| p as u32),
                        image_name: r.get(7)?,
                        pane_id: r.get(8)?,
                        started_at: r.get(9)?,
                        ended_at: r.get(10)?,
                        outcome: r.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(|e| StoreError::Io(e.to_string()))
    }

    fn attempts_for_task(
        &self,
        graph_id: &str,
        task_id: TaskId,
    ) -> Result<Vec<AttemptRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT attempt_id FROM attempts WHERE graph_id = ?1 AND task_id = ?2 ORDER BY attempt_id",
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let ids: Result<Vec<u32>, _> = stmt
            .query_map(params![graph_id, task_id.0], |r| r.get(0))
            .map_err(|e| StoreError::Io(e.to_string()))?
            .collect();
        let ids = ids.map_err(|e| StoreError::Io(e.to_string()))?;
        let mut out = Vec::new();
        for id in ids {
            if let Some(row) = self.load_attempt(AttemptId(id))? {
                out.push(row);
            }
        }
        Ok(out)
    }

    fn latest_attempt_for_task(
        &self,
        graph_id: &str,
        task_id: TaskId,
    ) -> Result<Option<AttemptRow>, StoreError> {
        Ok(self
            .attempts_for_task(graph_id, task_id)?
            .into_iter()
            .last())
    }

    fn update_attempt_pid(
        &mut self,
        attempt_id: AttemptId,
        pid: Option<u32>,
        image_name: Option<&str>,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE attempts SET pid = ?1, image_name = ?2 WHERE attempt_id = ?3",
                params![pid.map(|p| p as i64), image_name, attempt_id.0],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn update_attempt_pane(
        &mut self,
        attempt_id: AttemptId,
        pane_id: Option<&str>,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE attempts SET pane_id = ?1 WHERE attempt_id = ?2",
                params![pane_id, attempt_id.0],
            )
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }
}

#[allow(dead_code)]
pub fn now_stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

pub fn plan_digest(json: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    json.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::evidence::DeterministicEvidence;
    use meshloop_engine::ports::{AttemptRow, RunStore, TierKey};

    fn candidate() -> CandidateRef {
        CandidateRef {
            task_id: TaskId(1),
            attempt_id: AttemptId(1),
            revision: "deadbeef".into(),
        }
    }

    #[test]
    fn attempt_pane_id_round_trips() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let row = AttemptRow {
            attempt_id: AttemptId(1),
            graph_id: "g".into(),
            task_id: TaskId(1),
            harness: Some("grok".into()),
            model_ref: Some("grok".into()),
            worktree_path: None,
            pid: None,
            image_name: None,
            pane_id: None,
            started_at: None,
            ended_at: None,
            outcome: None,
        };
        store.save_attempt(&row).unwrap();
        store
            .update_attempt_pane(AttemptId(1), Some("w3:p2"))
            .unwrap();
        let loaded = store.load_attempt(AttemptId(1)).unwrap().unwrap();
        assert_eq!(loaded.pane_id.as_deref(), Some("w3:p2"));
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
        let fetched = store.evidence_for(&candidate()).unwrap();
        assert_eq!(fetched, vec![evidence]);
    }

    #[test]
    fn evidence_for_an_unrelated_candidate_is_empty() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(store.evidence_for(&candidate()).unwrap().is_empty());
    }

    #[test]
    fn routing_feedback_accumulates_across_calls() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let key = FeedbackKey {
            harness: "codex".into(),
            model_ref: "m".into(),
            tier: TierKey::Tier2,
        };
        store.record_outcome(&key, true).unwrap();
        store.record_outcome(&key, true).unwrap();
        store.record_outcome(&key, false).unwrap();
        let counters = store.counters(&key).unwrap();
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

    #[test]
    fn event_log_round_trips_nullable_attempt() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let rec = TransitionRecord {
            graph_id: "g".into(),
            task_id: TaskId(1),
            attempt_id: None,
            from: TaskState::Pending,
            to: TaskState::Ready,
            event: Event::DependencySatisfied,
            reason: None,
            executor: "meshloop".into(),
            occurred_at: "1".into(),
        };
        store.append(rec.clone()).unwrap();
        let got = store.records_for_graph("g").unwrap();
        assert_eq!(got, vec![rec]);
    }
}
