//! SQL-backed explicit lifecycle ledger. All writes require the live repository fence.
use super::{native_identity, refused};
use crate::store::SqliteStore;
use meshloop_domain::workspace::*;
use meshloop_engine::ports::StoreError;
use meshloop_engine::workspace::{RepositoryFence, WorkspaceLedger};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;

fn sql_error(_: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt("workspace ledger operation or constraint failed".into())
}
fn encode<T: Serialize>(record: &T) -> Result<String, StoreError> {
    serde_json::to_string(record).map_err(sql_error)
}
fn decode<T: DeserializeOwned>(json: String) -> Result<T, StoreError> {
    serde_json::from_str(&json).map_err(sql_error)
}
fn one<T: DeserializeOwned>(
    conn: &Connection,
    table: &str,
    id_column: &str,
    id: &str,
) -> Result<Option<T>, StoreError> {
    conn.query_row(
        &format!("SELECT payload_json FROM {table} WHERE {id_column}=?1"),
        params![id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(sql_error)?
    .map(decode)
    .transpose()
}
fn many<T: DeserializeOwned>(
    conn: &Connection,
    query: &str,
    id: &str,
) -> Result<Vec<T>, StoreError> {
    let mut stmt = conn.prepare(query).map_err(sql_error)?;
    stmt.query_map(params![id], |r| r.get::<_, String>(0))
        .map_err(sql_error)?
        .map(|result| decode(result.map_err(sql_error)?))
        .collect()
}
impl SqliteStore {
    pub(super) fn validate_fence(
        &self,
        fence: &dyn RepositoryFence,
        bound: bool,
    ) -> Result<(), StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }
        fence.validate()?;
        let path = self
            .conn
            .path()
            .filter(|path| !path.is_empty())
            .ok_or_else(refused)?;
        super::verify_handle(
            self.database_handle.as_ref().ok_or_else(refused)?,
            Path::new(path),
        )?;
        if self.database_identity.as_ref() != Some(fence.ledger_identity())
            || native_identity(Path::new(path))? != *fence.ledger_identity()
        {
            return Err(refused());
        }
        if bound {
            let record: RepositoryRecord = one(
                &self.conn,
                "workspace_repositories",
                "repo_id",
                fence.repository().id.as_str(),
            )?
            .ok_or_else(refused)?;
            if record != *fence.repository() {
                return Err(refused());
            }
            let ledger: String = self
                .conn
                .query_row(
                    "SELECT ledger_native_id FROM workspace_repositories WHERE repo_id=?1",
                    params![record.id.as_str()],
                    |r| r.get(0),
                )
                .map_err(sql_error)?;
            if ledger != fence.ledger_identity().as_str() {
                return Err(refused());
            }
        }
        Ok(())
    }
    fn owned_artifact(
        &self,
        fence: &dyn RepositoryFence,
        id: &LifecycleId,
    ) -> Result<(), StoreError> {
        let owned: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspace_artifacts WHERE id=?1 AND repo_id=?2)",
                params![id.as_str(), fence.repository().id.as_str()],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        if !owned {
            return Err(refused());
        }
        Ok(())
    }
}
impl WorkspaceLedger for SqliteStore {
    fn bind_repository(&mut self, fence: &dyn RepositoryFence) -> Result<(), StoreError> {
        self.validate_fence(fence, false)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        // Legacy graph IDs are global: this database may bind exactly one repository.
        let existing: Option<(String,String,String)> = tx.query_row("SELECT repo_id,common_native_id,ledger_native_id FROM workspace_repositories LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql_error)?;
        let record = fence.repository();
        if let Some((repo, native, ledger)) = existing {
            if repo != record.id.as_str()
                || native != record.common_dir_identity.as_str()
                || ledger != fence.ledger_identity().as_str()
            {
                return Err(refused());
            }
        } else {
            tx.execute(
                "INSERT INTO workspace_repositories VALUES(?1,?2,?3,?4)",
                params![
                    record.id.as_str(),
                    record.common_dir_identity.as_str(),
                    fence.ledger_identity().as_str(),
                    encode(record)?
                ],
            )
            .map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)
    }
    fn repository(&self, id: &LifecycleId) -> Result<Option<RepositoryRecord>, StoreError> {
        one(&self.conn, "workspace_repositories", "repo_id", id.as_str())
    }
    fn register_root(
        &mut self,
        fence: &dyn RepositoryFence,
        alias: &RootAlias,
    ) -> Result<(), StoreError> {
        self.validate_fence(fence, true)?;
        let record = fence.root(alias)?;
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT payload_json FROM workspace_roots WHERE repo_id=?1 AND alias=?2",
                params![record.repository_id.as_str(), alias.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some(json) = existing {
            if decode::<RootRecord>(json)? != record {
                return Err(refused());
            }
        } else {
            self.conn
                .execute(
                    "INSERT INTO workspace_roots VALUES(?1,?2,?3)",
                    params![
                        record.repository_id.as_str(),
                        alias.as_str(),
                        encode(&record)?
                    ],
                )
                .map_err(sql_error)?;
        }
        Ok(())
    }
    fn insert_generation(
        &mut self,
        fence: &dyn RepositoryFence,
        record: &GenerationRecord,
    ) -> Result<(), StoreError> {
        self.validate_fence(fence, true)?;
        if record.repository_id != fence.repository().id
            || record.ordinal == 0
            || record.layout_version == 0
        {
            return Err(refused());
        }
        fence.root(&record.root_alias)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let accepted: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE graph_id=?1 AND plan_state='PlanAccepted' AND plan_sha256=?2)",params![record.graph_id.as_str(),record.accepted_plan_hash.as_str()],|r|r.get(0)).map_err(sql_error)?;
        if !accepted {
            return Err(refused());
        }
        tx.execute(
            "INSERT INTO workspace_generations VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                record.id.as_str(),
                record.repository_id.as_str(),
                record.graph_id.as_str(),
                record.ordinal,
                record.root_alias.as_str(),
                record.accepted_plan_hash.as_str(),
                encode(record)?
            ],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }
    fn generation(&self, id: &LifecycleId) -> Result<Option<GenerationRecord>, StoreError> {
        one(&self.conn, "workspace_generations", "id", id.as_str())
    }
    fn generations(
        &self,
        repository: &LifecycleId,
        graph: &GraphId,
    ) -> Result<Vec<GenerationRecord>, StoreError> {
        let mut stmt=self.conn.prepare("SELECT payload_json FROM workspace_generations WHERE repo_id=?1 AND graph_id=?2 ORDER BY ordinal").map_err(sql_error)?;
        stmt.query_map(params![repository.as_str(), graph.as_str()], |r| {
            r.get::<_, String>(0)
        })
        .map_err(sql_error)?
        .map(|result| decode(result.map_err(sql_error)?))
        .collect()
    }
    fn insert_artifacts(
        &mut self,
        fence: &dyn RepositoryFence,
        records: &[ArtifactRecord],
        dependencies: &[ArtifactDependency],
    ) -> Result<(), StoreError> {
        self.validate_fence(fence, true)?;
        // Resolve/validate portable locations before the first write.
        for record in records {
            fence.root(&record.root_alias)?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        for record in records {
            let generation: GenerationRecord = one(
                &tx,
                "workspace_generations",
                "id",
                record.generation_id.as_str(),
            )?
            .ok_or_else(refused)?;
            if generation.repository_id != fence.repository().id
                || record.root_alias != generation.root_alias
                || (record.kind == ArtifactKind::Attempt) != record.attempt_id.is_some()
            {
                return Err(refused());
            }
            if let Some(attempt) = record.attempt_id {
                let valid: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM attempts WHERE attempt_id=?1 AND graph_id=?2)",
                        params![attempt.0, generation.graph_id.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(sql_error)?;
                if !valid {
                    return Err(refused());
                }
            }
            tx.execute(
                "INSERT INTO workspace_artifacts VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    record.id.as_str(),
                    record.generation_id.as_str(),
                    generation.repository_id.as_str(),
                    record.root_alias.as_str(),
                    record.attempt_id.map(|id| id.0),
                    encode(record)?
                ],
            )
            .map_err(sql_error)?;
        }
        for dependency in dependencies {
            let owned: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM workspace_artifacts WHERE id IN (?1,?2) AND repo_id=?3",
                    params![
                        dependency.artifact_id.as_str(),
                        dependency.requires_artifact_id.as_str(),
                        fence.repository().id.as_str()
                    ],
                    |r| r.get(0),
                )
                .map_err(sql_error)?;
            if owned != 2 {
                return Err(refused());
            }
            tx.execute(
                "INSERT INTO workspace_dependencies VALUES(?1,?2)",
                params![
                    dependency.artifact_id.as_str(),
                    dependency.requires_artifact_id.as_str()
                ],
            )
            .map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)
    }
    fn artifact(&self, id: &LifecycleId) -> Result<Option<ArtifactRecord>, StoreError> {
        one(&self.conn, "workspace_artifacts", "id", id.as_str())
    }
    fn dependencies(&self, artifact: &LifecycleId) -> Result<Vec<ArtifactDependency>, StoreError> {
        let mut stmt=self.conn.prepare("SELECT requires_id FROM workspace_dependencies WHERE artifact_id=?1 ORDER BY requires_id").map_err(sql_error)?;
        stmt.query_map(params![artifact.as_str()], |r| r.get::<_, String>(0))
            .map_err(sql_error)?
            .map(|result| {
                Ok(ArtifactDependency {
                    artifact_id: artifact.clone(),
                    requires_artifact_id: LifecycleId::new(result.map_err(sql_error)?)
                        .map_err(sql_error)?,
                })
            })
            .collect()
    }
    fn append_fact(
        &mut self,
        fence: &dyn RepositoryFence,
        fact: &LifecycleFact,
    ) -> Result<(), StoreError> {
        self.validate_fence(fence, true)?;
        self.owned_artifact(fence, &fact.artifact_id)?;
        self.conn
            .execute(
                "INSERT INTO workspace_facts VALUES(?1,?2,?3)",
                params![fact.id.as_str(), fact.artifact_id.as_str(), encode(fact)?],
            )
            .map_err(sql_error)?;
        Ok(())
    }
    fn facts(&self, artifact: &LifecycleId) -> Result<Vec<LifecycleFact>, StoreError> {
        many(
            &self.conn,
            "SELECT payload_json FROM workspace_facts WHERE artifact_id=?1 ORDER BY rowid",
            artifact.as_str(),
        )
    }
    fn append_journal(
        &mut self,
        fence: &dyn RepositoryFence,
        entry: &JournalEntry,
    ) -> Result<(), StoreError> {
        self.validate_fence(fence, true)?;
        self.owned_artifact(fence, &entry.artifact_id)?;
        self.conn
            .execute(
                "INSERT INTO workspace_journal VALUES(?1,?2,?3)",
                params![
                    entry.id.as_str(),
                    entry.artifact_id.as_str(),
                    encode(entry)?
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }
    fn journal(&self, artifact: &LifecycleId) -> Result<Vec<JournalEntry>, StoreError> {
        many(
            &self.conn,
            "SELECT payload_json FROM workspace_journal WHERE artifact_id=?1 ORDER BY rowid",
            artifact.as_str(),
        )
    }
}
