//! Digest-bound legacy adoption. Verified generations are pinned in place.
//! Unknown files, missing paths, and name-only runs are reported and left untouched.

use std::collections::BTreeSet;

use meshloop_domain::digest::sha256_hex;
use meshloop_domain::workspace::{GenerationRecord, RelativePath, RootAlias, RunBinding};
use meshloop_engine::ports::StoreError;
use meshloop_engine::workspace::RepositoryIdentity;
use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;

use super::cleanup::{PathProbe, common_dir, join_relative, probe_path};
use super::native_identity;
use super::{GitRepositoryIdentity, InventoryScope, REGISTRY};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionReport {
    pub apply: bool,
    pub digest: String,
    pub status: String,
    pub adopted: Vec<AdoptionPin>,
    pub retained: Vec<AdoptionHold>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionPin {
    pub graph_id: String,
    pub generation_id: String,
    pub root_alias: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionHold {
    pub graph_id: String,
    pub reason: String,
}

#[derive(Clone)]
struct Proven {
    graph_id: String,
    generation_id: String,
    repository_id: String,
    root_alias: RootAlias,
    relative_path: RelativePath,
    layout_version: u32,
}

pub fn apply_adoption(
    scope: InventoryScope<'_>,
    digest: Option<&str>,
    apply: bool,
) -> Result<AdoptionReport, StoreError> {
    if apply && !digest.is_some_and(|value| value.len() == 64 && value.bytes().all(hex_digit)) {
        return Err(StoreError::Io("invalid digest".into()));
    }
    let Some(common) = common_dir(scope.checkout) else {
        return Ok(empty("git-unavailable"));
    };
    let registry_path = common.join(REGISTRY);
    if !registry_path.is_file() {
        return Ok(empty("fence-missing"));
    }
    if !scope.ledger.is_file() {
        return Ok(empty("ledger-missing"));
    }
    let conn = Connection::open_with_flags(scope.ledger, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(failed)?;
    if !apply {
        let registry = super::read_registry(&registry_path)?.ok_or_else(refused)?;
        return Ok(decide(&conn, &registry, &scope));
    }
    drop(conn);
    let fence = GitRepositoryIdentity::new("git").acquire(scope.checkout, scope.ledger)?;
    let registry = fence.registry_view();
    let conn = Connection::open_with_flags(scope.ledger, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(failed)?;
    let report = decide(&conn, &registry, &scope);
    drop(conn);
    if digest != Some(report.digest.as_str()) || report.adopted.is_empty() {
        let mut stale = report;
        stale.apply = false;
        if digest != Some(stale.digest.as_str()) {
            stale.status = "stale".to_string();
        }
        return Ok(stale);
    }
    let mut conn = Connection::open(scope.ledger).map_err(failed)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(failed)?;
    for pin in &report.adopted {
        let proven = load_proven(&tx, &registry, scope.checkout, pin)?;
        let Some(proven) = proven else {
            return Ok(AdoptionReport {
                apply: false,
                digest: report.digest.clone(),
                status: "stale".into(),
                adopted: report.adopted.clone(),
                retained: report.retained.clone(),
            });
        };
        let removed = tx
            .execute(
                "DELETE FROM workspace_legacy_unbound WHERE source_table='runs' AND source_id=?1",
                [proven.graph_id.as_str()],
            )
            .map_err(failed)?;
        if removed != 1 {
            return Ok(AdoptionReport {
                apply: false,
                digest: report.digest.clone(),
                status: "stale".into(),
                adopted: report.adopted.clone(),
                retained: report.retained.clone(),
            });
        }
        let binding = RunBinding {
            graph_id: meshloop_domain::workspace::GraphId::new(proven.graph_id.clone())
                .map_err(failed)?,
            repository_id: meshloop_domain::workspace::LifecycleId::new(
                proven.repository_id.clone(),
            )
            .map_err(failed)?,
            location: meshloop_domain::workspace::PortableLocation {
                root_alias: proven.root_alias.clone(),
                relative_path: proven.relative_path.clone(),
            },
            layout_version: proven.layout_version,
            active_generation: None,
            current_revision: None,
        };
        let payload = serde_json::to_string(&binding).map_err(failed)?;
        tx.execute(
            "INSERT INTO workspace_run_bindings VALUES(?1,?2,?3,NULL,NULL,?4)",
            params![
                binding.graph_id.as_str(),
                binding.repository_id.as_str(),
                binding.location.root_alias.as_str(),
                payload
            ],
        )
        .map_err(failed)?;
    }
    tx.commit().map_err(failed)?;
    Ok(AdoptionReport {
        apply: true,
        status: "adopted".into(),
        ..report
    })
}

fn decide(
    conn: &Connection,
    registry: &super::RegistryView,
    scope: &InventoryScope<'_>,
) -> AdoptionReport {
    let marked = strings(
        conn,
        "SELECT source_id FROM workspace_legacy_unbound WHERE source_table='runs' ORDER BY source_id",
    );
    let runs = strings(conn, "SELECT graph_id FROM runs ORDER BY graph_id");
    let generations = load_generations(conn);
    let mut adopted = Vec::new();
    let mut retained = Vec::new();
    let mut seen = BTreeSet::new();
    for graph in runs.iter().chain(marked.iter()) {
        if !seen.insert(graph.clone()) {
            continue;
        }
        let mine: Vec<_> = generations
            .iter()
            .filter(|generation| generation.graph_id.as_str() == graph)
            .collect();
        if !marked.iter().any(|id| id == graph) {
            if mine.is_empty() {
                retained.push(hold(graph, "unverified-location"));
            }
            continue;
        }
        if mine.is_empty() {
            retained.push(hold(graph, "unverified-location"));
            continue;
        }
        match prove_graph(registry, scope.checkout, &mine) {
            Ok(proven) => adopted.push(AdoptionPin {
                graph_id: proven.graph_id,
                generation_id: proven.generation_id,
                root_alias: proven.root_alias.as_str().to_string(),
                relative_path: proven.relative_path.as_str().to_string(),
            }),
            Err(reason) => retained.push(hold(graph, reason)),
        }
    }
    adopted.sort_by(|left, right| left.graph_id.cmp(&right.graph_id));
    retained.sort_by(|left, right| left.graph_id.cmp(&right.graph_id));
    let digest = digest_of(registry.repository.id.as_str(), &adopted, &retained);
    AdoptionReport {
        apply: false,
        digest,
        status: "preview".into(),
        adopted,
        retained,
    }
}

fn prove_graph(
    registry: &super::RegistryView,
    checkout: &std::path::Path,
    generations: &[&GenerationRecord],
) -> Result<Proven, &'static str> {
    let mut proven = Vec::new();
    for generation in generations {
        if generation.repository_id != registry.repository.id {
            return Err("identity-unverified");
        }
        let Some(root) = registry.roots.get(&generation.root_alias) else {
            return Err("unverified-location");
        };
        if native_identity(&root.path).ok().as_ref() != Some(&root.identity) {
            return Err("identity-unverified");
        }
        let path = join_relative(&root.path, &generation.relative_path);
        if path == checkout || checkout.starts_with(&path) {
            return Err("unsafe-path");
        }
        match probe_path(&path) {
            PathProbe::Present => {}
            PathProbe::Missing => return Err("path-missing"),
            PathProbe::Reparse { .. } => return Err("unsafe-path"),
            PathProbe::Inaccessible => return Err("path-inaccessible"),
        }
        let canonical = std::fs::canonicalize(&path).map_err(|_| "path-inaccessible")?;
        let root_canonical =
            std::fs::canonicalize(&root.path).map_err(|_| "identity-unverified")?;
        if !canonical.starts_with(&root_canonical) {
            return Err("unsafe-path");
        }
        proven.push(Proven {
            graph_id: generation.graph_id.as_str().to_string(),
            generation_id: generation.id.as_str().to_string(),
            repository_id: generation.repository_id.as_str().to_string(),
            root_alias: generation.root_alias.clone(),
            relative_path: generation.relative_path.clone(),
            layout_version: generation.layout_version,
        });
    }
    proven.sort_by_key(|row| row.generation_id.clone());
    let first = proven.first().ok_or("unverified-location")?;
    if proven.iter().any(|row| row.root_alias != first.root_alias) {
        return Err("ambiguous-root");
    }
    Ok(first.clone())
}

fn load_proven(
    conn: &Connection,
    registry: &super::RegistryView,
    checkout: &std::path::Path,
    pin: &AdoptionPin,
) -> Result<Option<Proven>, StoreError> {
    let generations = load_generations(conn);
    let mine: Vec<_> = generations
        .iter()
        .filter(|generation| generation.graph_id.as_str() == pin.graph_id)
        .collect();
    match prove_graph(registry, checkout, &mine) {
        Ok(proven)
            if proven.generation_id == pin.generation_id
                && proven.root_alias.as_str() == pin.root_alias
                && proven.relative_path.as_str() == pin.relative_path =>
        {
            Ok(Some(proven))
        }
        _ => Ok(None),
    }
}

fn load_generations(conn: &Connection) -> Vec<GenerationRecord> {
    let Ok(mut statement) =
        conn.prepare("SELECT payload_json FROM workspace_generations ORDER BY id")
    else {
        return Vec::new();
    };
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|payload| serde_json::from_str(&payload).ok())
        .collect()
}

fn strings(conn: &Connection, sql: &str) -> Vec<String> {
    let Ok(mut statement) = conn.prepare(sql) else {
        return Vec::new();
    };
    statement
        .query_map([], |row| row.get(0))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .collect()
}

fn digest_of(repository: &str, adopted: &[AdoptionPin], retained: &[AdoptionHold]) -> String {
    let mut bytes = format!("repository\t{repository}\n");
    for pin in adopted {
        bytes.push_str(&format!(
            "adopt\t{}\t{}\t{}\t{}\n",
            pin.graph_id, pin.generation_id, pin.root_alias, pin.relative_path
        ));
    }
    for hold in retained {
        bytes.push_str(&format!("retain\t{}\t{}\n", hold.graph_id, hold.reason));
    }
    sha256_hex(bytes.as_bytes())
}

fn hold(graph: &str, reason: &str) -> AdoptionHold {
    AdoptionHold {
        graph_id: graph.to_string(),
        reason: reason.to_string(),
    }
}

fn empty(status: &str) -> AdoptionReport {
    AdoptionReport {
        apply: false,
        digest: sha256_hex(format!("repository\t\nstatus\t{status}\n").as_bytes()),
        status: status.to_string(),
        adopted: Vec::new(),
        retained: Vec::new(),
    }
}

fn hex_digit(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn failed(error: impl std::fmt::Display) -> StoreError {
    let _ = error;
    StoreError::Io("adoption operation failed".into())
}

fn refused() -> StoreError {
    StoreError::Io("adoption operation failed".into())
}
