//! Read-only discovery for workspace inventory.
//! Never creates a ledger, fence, registry, or root.

use super::{RegistryView, bounded_stdout, native_identity, read_registry};
use crate::process::OwnedSpawnSpec;
use crate::store::SqliteStore;
use meshloop_domain::workspace::*;
use meshloop_engine::ports::StoreError;
use meshloop_engine::workspace::inventory::{InventoryFacts, Observation, Presence};
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_CHILDREN: usize = 256;

pub struct InventoryScope<'a> {
    pub checkout: &'a Path,
    pub ledger: &'a Path,
    pub configured_root: &'a Path,
}

/// Read-only task outcome for one graph. Age is not read here.
#[derive(Debug, Clone, Default)]
pub struct GraphOutcome {
    pub known: bool,
    pub failed: bool,
    pub cancelled: bool,
    pub landed: bool,
}

/// Read task states without creating or migrating a ledger.
/// A missing file or unreadable table returns an empty map.
pub fn graph_outcomes(ledger: &Path) -> BTreeMap<String, GraphOutcome> {
    let mut outcomes = BTreeMap::new();
    if !ledger.is_file() {
        return outcomes;
    }
    let Ok(connection) =
        Connection::open_with_flags(ledger, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return outcomes;
    };
    let Ok(mut statement) =
        connection.prepare("SELECT graph_id, to_state FROM events ORDER BY graph_id, event_id")
    else {
        return outcomes;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return outcomes;
    };
    for row in rows.flatten() {
        let outcome = outcomes.entry(row.0).or_default();
        outcome.known = true;
        outcome.failed |= row.1 == "Failed";
        outcome.cancelled |= row.1 == "Cancelled";
        outcome.landed |= row.1 == "Integrated" || row.1 == "Accepted";
    }
    outcomes
}

/// A run the run loop owns (issue #142), with its run directory generation and
/// the current outcome of its tasks.
#[derive(Debug, Clone)]
pub(crate) struct RunLoopRun {
    pub generation: GenerationRecord,
    pub outcome: meshloop_domain::cleanup::RunOutcome,
}

/// Read run-loop-owned runs without creating or migrating a ledger. A run whose
/// tasks cannot be replayed is reported `Incomplete`, so cleanup retains it.
pub(crate) fn run_loop_runs(ledger: &Path) -> BTreeMap<String, RunLoopRun> {
    use meshloop_domain::cleanup::RunOutcome;
    use meshloop_domain::state::TaskState;
    use meshloop_engine::ports::{EventLog, RunStore};
    use meshloop_engine::workspace::RuntimeLedger;
    let mut runs = BTreeMap::new();
    if !ledger.is_file() {
        return runs;
    }
    let Ok(Some(store)) = SqliteStore::open_read_only(ledger) else {
        return runs;
    };
    let Ok(generations) = payloads::<GenerationRecord>(
        &store.conn,
        "SELECT payload_json FROM workspace_generations WHERE ordinal=1 ORDER BY id",
    ) else {
        return runs;
    };
    for first in generations
        .into_iter()
        .filter(GenerationRecord::is_run_loop_binding)
    {
        // Same rule as the store: unbound, unmarked, bound by this generation.
        let Some(generation) = store
            .run_loop_generation(&first.graph_id)
            .ok()
            .flatten()
            .filter(|binding| *binding == first)
        else {
            continue;
        };
        let graph = generation.graph_id.as_str().to_string();
        let graph = graph.as_str();
        let states = store
            .records_for_graph(graph)
            .ok()
            .and_then(|records| meshloop_engine::recovery::replay_tasks(&records).ok());
        let plan = store.graph_from_run(graph).ok();
        let outcome = match (plan, states) {
            (Some(plan), Some(states)) => {
                let finals: Vec<TaskState> = plan
                    .nodes
                    .iter()
                    .map(|node| states.get(&node.id).copied().unwrap_or(TaskState::Pending))
                    .collect();
                if finals
                    .iter()
                    .any(|state| matches!(state, TaskState::Failed | TaskState::Blocked))
                {
                    RunOutcome::Failed
                } else if finals.contains(&TaskState::Cancelled) {
                    RunOutcome::Cancelled
                } else if !finals.is_empty()
                    && finals.iter().all(|state| *state == TaskState::Integrated)
                {
                    RunOutcome::Landed
                } else {
                    RunOutcome::Incomplete
                }
            }
            _ => RunOutcome::Incomplete,
        };
        runs.insert(
            graph.to_string(),
            RunLoopRun {
                generation,
                outcome,
            },
        );
    }
    runs
}

struct AttemptSeen {
    id: String,
    graph_id: String,
    path: Option<String>,
}

struct LedgerView {
    repos: Vec<RepositoryRecord>,
    generations: Vec<GenerationRecord>,
    artifacts: Vec<ArtifactRecord>,
    provisionals: Vec<ProvisionalOwner>,
    drivers: Vec<DriverRecord>,
    runs: Vec<String>,
    bindings: BTreeSet<String>,
    legacy_runs: BTreeSet<String>,
    legacy_attempts: BTreeSet<String>,
    attempts: Vec<AttemptSeen>,
}

pub fn discover_inventory(scope: InventoryScope<'_>) -> InventoryFacts {
    discover_inventory_with(scope, None)
}

pub(crate) fn discover_inventory_with(
    scope: InventoryScope<'_>,
    held: Option<&RegistryView>,
) -> InventoryFacts {
    let mut blockers = Vec::new();
    let mut observations = Vec::new();
    if probe(scope.checkout) != Presence::Present {
        push(&mut blockers, BlockerCode::GitUnavailable);
        return finish(blockers, observations);
    }
    let common = match common_dir(scope.checkout) {
        CommonDir::Ready(path) => path,
        CommonDir::Unsafe => {
            push(&mut blockers, BlockerCode::FenceInaccessible);
            push(&mut blockers, BlockerCode::GitUnavailable);
            return finish(blockers, observations);
        }
        CommonDir::Unavailable => {
            push(&mut blockers, BlockerCode::GitUnavailable);
            return finish(blockers, observations);
        }
    };
    // `held` is the manifest already loaded by the fence. Do not open the
    // registry file again: Windows denies that read while the lock is held.
    let owned = if let Some(view) = held {
        Some(view.clone())
    } else {
        match read_registry(&common.join("meshloop-workspace-registry.json")) {
            Ok(None) => {
                push(&mut blockers, BlockerCode::FenceMissing);
                None
            }
            Ok(Some(view)) => Some(view),
            Err(_) => {
                push(&mut blockers, BlockerCode::FenceInaccessible);
                None
            }
        }
    };
    let registry = owned.as_ref();
    let mut conflict = registry_conflicts(&common, registry);
    let ledger = open_ledger(scope.ledger, &mut blockers);
    let identity_confirmed = confirm_identity(
        scope.ledger,
        registry,
        ledger.as_ref(),
        &mut conflict,
        &mut blockers,
    );
    if conflict {
        push(&mut blockers, BlockerCode::IdentityConflict);
    }
    let confirmed = identity_confirmed && !conflict;
    let owned = Owned {
        registry,
        keys: if confirmed {
            ledger.as_ref().map(owned_keys).unwrap_or_default()
        } else {
            BTreeSet::new()
        },
    };
    if let Some(view) = ledger.as_ref() {
        push_ledger_observations(
            view,
            registry,
            confirmed,
            scope.checkout,
            scope.configured_root,
            &owned,
            &mut observations,
        );
    }
    push_git_worktrees(
        scope.checkout,
        scope.configured_root,
        registry,
        confirmed,
        &owned,
        &mut observations,
        &mut blockers,
    );
    push_configured_children(
        scope.checkout,
        scope.configured_root,
        registry,
        confirmed,
        &owned,
        &mut observations,
        &mut blockers,
    );
    finish(blockers, observations)
}

/// Ledger record locations, keyed by root alias and relative path, used to report
/// a path the ledger owns under its ledger location from every discovery source.
/// Without this, one worktree would appear once as an owned artifact and again as
/// an unowned Git worktree or configured-root child.
struct Owned<'a> {
    registry: Option<&'a RegistryView>,
    keys: BTreeSet<(RootAlias, RelativePath)>,
}

impl Owned<'_> {
    fn location(&self, path: &Path) -> Option<InventoryLocation> {
        if self.keys.is_empty() {
            return None;
        }
        let registry = self.registry?;
        for (alias, root) in &registry.roots {
            if native_identity(&root.path).ok().as_ref() != Some(&root.identity) {
                continue;
            }
            if let Some(relative_path) = contained_relative(&root.path, path)
                && self.keys.contains(&(alias.clone(), relative_path.clone()))
            {
                return Some(InventoryLocation {
                    scope: LocationScope::LedgerRoot,
                    root_alias: Some(alias.clone()),
                    relative_path,
                });
            }
        }
        None
    }
}

fn owned_keys(view: &LedgerView) -> BTreeSet<(RootAlias, RelativePath)> {
    view.generations
        .iter()
        .map(|g| (g.root_alias.clone(), g.relative_path.clone()))
        .chain(
            view.artifacts
                .iter()
                .map(|a| (a.root_alias.clone(), a.relative_path.clone())),
        )
        .chain(view.provisionals.iter().map(|p| {
            (
                p.location.root_alias.clone(),
                p.location.relative_path.clone(),
            )
        }))
        .collect()
}

fn finish(blockers: Vec<BlockerCode>, observations: Vec<Observation>) -> InventoryFacts {
    InventoryFacts {
        command_blockers: blockers,
        observations,
    }
}

enum CommonDir {
    Ready(PathBuf),
    Unsafe,
    Unavailable,
}

fn common_dir(checkout: &Path) -> CommonDir {
    let Ok(bytes) = git_bytes(
        checkout,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) else {
        return CommonDir::Unavailable;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return CommonDir::Unavailable;
    };
    let text = text.trim();
    if text.is_empty() {
        return CommonDir::Unavailable;
    }
    let path = PathBuf::from(text);
    let path = if path.is_absolute() {
        path
    } else {
        checkout.join(path)
    };
    match probe(&path) {
        Presence::Present => std::fs::canonicalize(&path)
            .map(CommonDir::Ready)
            .unwrap_or(CommonDir::Unavailable),
        Presence::Reparse => CommonDir::Unsafe,
        _ => CommonDir::Unavailable,
    }
}

fn registry_conflicts(common: &Path, registry: Option<&RegistryView>) -> bool {
    let Some(view) = registry else {
        return false;
    };
    let Ok(live) = native_identity(common) else {
        return true;
    };
    view.common_identity != live || view.repository.common_dir_identity != live
}

fn confirm_identity(
    ledger_path: &Path,
    registry: Option<&RegistryView>,
    ledger: Option<&LedgerView>,
    conflict: &mut bool,
    blockers: &mut Vec<BlockerCode>,
) -> bool {
    let Some(view) = ledger else {
        return false;
    };
    if view.repos.len() > 1 {
        *conflict = true;
        return false;
    }
    let Some(repo) = view.repos.first() else {
        if registry.is_some() && !*conflict {
            push(blockers, BlockerCode::RepositoryUnbound);
        }
        return false;
    };
    let Some(registry) = registry else {
        return false;
    };
    if registry.repository.id != repo.id
        || registry.common_identity != repo.common_dir_identity
        || registry.repository.common_dir_identity != repo.common_dir_identity
    {
        *conflict = true;
        return false;
    }
    match native_identity(ledger_path) {
        Ok(identity) if identity == registry.ledger_identity => !*conflict,
        _ => {
            *conflict = true;
            false
        }
    }
}

fn open_ledger(path: &Path, blockers: &mut Vec<BlockerCode>) -> Option<LedgerView> {
    match probe(path) {
        Presence::Missing => {
            push(blockers, BlockerCode::LedgerMissing);
            return None;
        }
        Presence::Reparse | Presence::Inaccessible | Presence::NotApplicable => {
            push(blockers, BlockerCode::LedgerInaccessible);
            return None;
        }
        Presence::Present => {}
    }
    match SqliteStore::open_read_only(path) {
        Ok(None) => {
            push(blockers, BlockerCode::LedgerMissing);
            None
        }
        Ok(Some(store)) => match load_ledger(&store.conn) {
            Ok(view) => Some(view),
            Err(()) => {
                push(blockers, BlockerCode::LedgerInaccessible);
                None
            }
        },
        Err(StoreError::SchemaUpgradeRequired) => {
            push(blockers, BlockerCode::SchemaUpgradeRequired);
            None
        }
        Err(_) => {
            push(blockers, BlockerCode::LedgerInaccessible);
            None
        }
    }
}

fn load_ledger(conn: &Connection) -> Result<LedgerView, ()> {
    let runs = column_strings(conn, "SELECT graph_id FROM runs ORDER BY graph_id")?;
    let bindings = column_strings(
        conn,
        "SELECT graph_id FROM workspace_run_bindings ORDER BY graph_id",
    )?
    .into_iter()
    .collect();
    let legacy_runs = column_strings(
        conn,
        "SELECT source_id FROM workspace_legacy_unbound WHERE source_table='runs' ORDER BY source_id",
    )?
    .into_iter()
    .collect();
    let legacy_attempts = column_strings(
        conn,
        "SELECT source_id FROM workspace_legacy_unbound WHERE source_table='attempts' ORDER BY source_id",
    )?
    .into_iter()
    .collect();
    Ok(LedgerView {
        repos: payloads(
            conn,
            "SELECT payload_json FROM workspace_repositories ORDER BY repo_id",
        )?,
        generations: payloads(
            conn,
            "SELECT payload_json FROM workspace_generations ORDER BY ordinal, id",
        )?,
        artifacts: payloads(
            conn,
            "SELECT payload_json FROM workspace_artifacts ORDER BY id",
        )?,
        provisionals: payloads(
            conn,
            "SELECT payload_json FROM workspace_provisional_owners ORDER BY id",
        )?,
        drivers: payloads(
            conn,
            "SELECT payload_json FROM workspace_drivers ORDER BY id",
        )?,
        runs,
        bindings,
        legacy_runs,
        legacy_attempts,
        attempts: load_attempts(conn)?,
    })
}

fn column_strings(conn: &Connection, sql: &str) -> Result<Vec<String>, ()> {
    let mut stmt = conn.prepare(sql).map_err(|_| ())?;
    let rows = stmt.query_map([], |row| row.get(0)).map_err(|_| ())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|_| ())
}

fn payloads<T: DeserializeOwned>(conn: &Connection, sql: &str) -> Result<Vec<T>, ()> {
    column_strings(conn, sql)?
        .into_iter()
        .map(|json| serde_json::from_str(&json).map_err(|_| ()))
        .collect()
}

fn load_attempts(conn: &Connection) -> Result<Vec<AttemptSeen>, ()> {
    let mut stmt = conn
        .prepare(
            "SELECT CAST(attempt_id AS TEXT), graph_id, worktree_path FROM attempts ORDER BY attempt_id",
        )
        .map_err(|_| ())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AttemptSeen {
                id: row.get(0)?,
                graph_id: row.get(1)?,
                path: row.get(2)?,
            })
        })
        .map_err(|_| ())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|_| ())
}

fn push_ledger_observations(
    view: &LedgerView,
    registry: Option<&RegistryView>,
    identity_confirmed: bool,
    checkout: &Path,
    configured_root: &Path,
    owned: &Owned<'_>,
    observations: &mut Vec<Observation>,
) {
    let generation_by_id: BTreeMap<&str, &GenerationRecord> = view
        .generations
        .iter()
        .map(|generation| (generation.id.as_str(), generation))
        .collect();
    for generation in &view.generations {
        let (location, presence, withheld) = resolve_record(
            &generation.root_alias,
            &generation.relative_path,
            registry,
            identity_confirmed,
        );
        observations.push(ledger_observation(
            identity_confirmed,
            LedgerObservation {
                source: EntrySource::Generation,
                graph_id: Some(generation.graph_id.clone()),
                lifecycle_id: Some(generation.id.clone()),
                location,
                presence,
                withheld,
                legacy_marker: view.legacy_runs.contains(generation.graph_id.as_str()),
                driver_active: false,
            },
        ));
    }
    for artifact in &view.artifacts {
        let legacy = generation_by_id
            .get(artifact.generation_id.as_str())
            .is_some_and(|generation| view.legacy_runs.contains(generation.graph_id.as_str()));
        let (location, presence, withheld) = resolve_record(
            &artifact.root_alias,
            &artifact.relative_path,
            registry,
            identity_confirmed,
        );
        observations.push(ledger_observation(
            identity_confirmed,
            LedgerObservation {
                source: EntrySource::Artifact,
                graph_id: generation_by_id
                    .get(artifact.generation_id.as_str())
                    .map(|generation| generation.graph_id.clone()),
                lifecycle_id: Some(artifact.id.clone()),
                location,
                presence,
                withheld,
                legacy_marker: legacy,
                driver_active: false,
            },
        ));
    }
    for provisional in &view.provisionals {
        let legacy = provisional
            .graph_id
            .as_ref()
            .is_some_and(|graph| view.legacy_runs.contains(graph.as_str()));
        let (location, presence, withheld) = resolve_record(
            &provisional.location.root_alias,
            &provisional.location.relative_path,
            registry,
            identity_confirmed,
        );
        observations.push(ledger_observation(
            identity_confirmed,
            LedgerObservation {
                source: EntrySource::Provisional,
                graph_id: provisional.graph_id.clone(),
                lifecycle_id: Some(provisional.id.clone()),
                location,
                presence,
                withheld,
                legacy_marker: legacy,
                driver_active: false,
            },
        ));
    }
    for graph in &view.runs {
        if !legacy_run(view, graph) {
            continue;
        }
        let Ok(graph_id) = GraphId::new(graph.clone()) else {
            continue;
        };
        observations.push(ledger_observation(
            identity_confirmed,
            LedgerObservation {
                source: EntrySource::LegacyRun,
                graph_id: Some(graph_id),
                lifecycle_id: None,
                location: None,
                presence: Presence::NotApplicable,
                withheld: false,
                legacy_marker: true,
                driver_active: false,
            },
        ));
    }
    for attempt in &view.attempts {
        let marked = view.legacy_attempts.contains(&attempt.id)
            || view.legacy_runs.contains(&attempt.graph_id)
            || legacy_run(view, &attempt.graph_id);
        if attempt.path.is_none() && (legacy_run(view, &attempt.graph_id) || !marked) {
            continue;
        }
        let Ok(graph_id) = GraphId::new(attempt.graph_id.clone()) else {
            continue;
        };
        let owned_location = attempt
            .path
            .as_deref()
            .and_then(|path| owned.location(Path::new(path)));
        let artifact_backed = owned_location.is_some();
        let (location, presence, withheld) = match (owned_location, attempt.path.as_deref()) {
            (Some(location), Some(path)) => (Some(location), probe(Path::new(path)), false),
            (_, Some(path)) => observe_persisted_path(
                Path::new(path),
                checkout,
                configured_root,
                registry,
                identity_confirmed,
            ),
            (_, None) => (None, Presence::NotApplicable, false),
        };
        let mut observation = ledger_observation(
            identity_confirmed,
            LedgerObservation {
                source: if marked {
                    EntrySource::LegacyAttempt
                } else {
                    EntrySource::Artifact
                },
                graph_id: Some(graph_id),
                lifecycle_id: None,
                location,
                presence,
                withheld,
                legacy_marker: marked,
                driver_active: false,
            },
        );
        // An unmarked attempt path that no artifact record owns proves nothing about
        // ownership (issue #142): it stays unresolved and cleanup never selects it.
        if !marked && !artifact_backed {
            observation.ledger_record = false;
            observation.identity_confirmed = false;
        }
        observations.push(observation);
    }
    for driver in &view.drivers {
        if !driver_is_active(driver.phase) {
            continue;
        }
        let graph_id = match &driver.target {
            DriverTarget::Execution(scope) => Some(scope.graph_id.clone()),
            DriverTarget::Provisional { .. } => None,
        };
        let (location, presence, withheld) = driver
            .diagnostic
            .as_ref()
            .map(|diagnostic| {
                resolve_record(
                    &diagnostic.root_alias,
                    &diagnostic.relative_path,
                    registry,
                    identity_confirmed,
                )
            })
            .unwrap_or((None, Presence::NotApplicable, false));
        observations.push(ledger_observation(
            identity_confirmed,
            LedgerObservation {
                source: EntrySource::Driver,
                graph_id,
                lifecycle_id: Some(driver.id.clone()),
                location,
                presence,
                withheld,
                legacy_marker: false,
                driver_active: true,
            },
        ));
    }
}

struct LedgerObservation {
    source: EntrySource,
    graph_id: Option<GraphId>,
    lifecycle_id: Option<LifecycleId>,
    location: Option<InventoryLocation>,
    presence: Presence,
    withheld: bool,
    legacy_marker: bool,
    driver_active: bool,
}

fn legacy_run(view: &LedgerView, graph: &str) -> bool {
    if view.bindings.contains(graph) {
        return false;
    }
    if view
        .generations
        .iter()
        .any(|generation| generation.graph_id.as_str() == graph)
    {
        return false;
    }
    if view.provisionals.iter().any(|provisional| {
        provisional
            .graph_id
            .as_ref()
            .is_some_and(|id| id.as_str() == graph)
    }) {
        return false;
    }
    true
}

fn ledger_observation(identity_confirmed: bool, record: LedgerObservation) -> Observation {
    Observation {
        source: record.source,
        graph_id: record.graph_id,
        lifecycle_id: record.lifecycle_id,
        location: record.location,
        withheld: record.withheld,
        presence: record.presence,
        legacy_marker: record.legacy_marker,
        ledger_record: true,
        identity_confirmed,
        git_registered: false,
        driver_active: record.driver_active,
        configured_entry: false,
    }
}

fn resolve_record(
    alias: &RootAlias,
    relative: &RelativePath,
    registry: Option<&RegistryView>,
    identity_confirmed: bool,
) -> (Option<InventoryLocation>, Presence, bool) {
    let location = Some(InventoryLocation {
        scope: LocationScope::LedgerRoot,
        root_alias: Some(alias.clone()),
        relative_path: relative.clone(),
    });
    if !identity_confirmed {
        return (location, Presence::NotApplicable, false);
    }
    let Some(root) = registry.and_then(|view| view.roots.get(alias)) else {
        return (location, Presence::NotApplicable, false);
    };
    if native_identity(&root.path).ok().as_ref() != Some(&root.identity) {
        return (location, Presence::Inaccessible, false);
    }
    if probe(&root.path) != Presence::Present {
        return (location, probe(&root.path), false);
    }
    (location, probe(&join_relative(&root.path, relative)), false)
}

fn push_git_worktrees(
    checkout: &Path,
    configured_root: &Path,
    registry: Option<&RegistryView>,
    identity_confirmed: bool,
    owned: &Owned<'_>,
    observations: &mut Vec<Observation>,
    blockers: &mut Vec<BlockerCode>,
) {
    let bytes = match git_bytes(checkout, &["worktree", "list", "--porcelain", "-z"]) {
        Ok(bytes) => bytes,
        Err(()) => {
            push(blockers, BlockerCode::GitUnavailable);
            return;
        }
    };
    let (paths, invalid) = parse_worktrees(&bytes);
    if invalid {
        push(blockers, BlockerCode::PathInaccessible);
    }
    for path in paths {
        if same_dir(&path, checkout) {
            continue;
        }
        let presence = probe(&path);
        let (location, withheld) = match owned.location(&path).or_else(|| {
            relative_location(
                &path,
                checkout,
                configured_root,
                registry,
                identity_confirmed,
            )
        }) {
            Some(location) => (Some(location), false),
            None => (None, true),
        };
        observations.push(Observation {
            source: EntrySource::GitWorktree,
            graph_id: None,
            lifecycle_id: None,
            location,
            withheld,
            presence,
            legacy_marker: false,
            ledger_record: false,
            identity_confirmed: false,
            git_registered: true,
            driver_active: false,
            configured_entry: false,
        });
    }
}

fn push_configured_children(
    checkout: &Path,
    configured_root: &Path,
    registry: Option<&RegistryView>,
    identity_confirmed: bool,
    owned: &Owned<'_>,
    observations: &mut Vec<Observation>,
    blockers: &mut Vec<BlockerCode>,
) {
    list_children(
        configured_root,
        LocationScope::ConfiguredRoot,
        None,
        None,
        checkout,
        owned,
        observations,
        blockers,
    );
    let nested = checkout.join(".meshloop").join("worktrees");
    if !same_dir(&nested, configured_root) {
        list_children(
            &nested,
            LocationScope::Repository,
            None,
            Some(Path::new(".meshloop/worktrees")),
            checkout,
            owned,
            observations,
            blockers,
        );
    }
    if identity_confirmed && let Some(registry) = registry {
        for (alias, root) in &registry.roots {
            if same_dir(&root.path, checkout)
                || same_dir(&root.path, configured_root)
                || same_dir(&root.path, &nested)
            {
                continue;
            }
            list_children(
                &root.path,
                LocationScope::LedgerRoot,
                Some(alias),
                None,
                checkout,
                owned,
                observations,
                blockers,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn list_children(
    root: &Path,
    scope: LocationScope,
    alias: Option<&RootAlias>,
    prefix: Option<&Path>,
    checkout: &Path,
    owned: &Owned<'_>,
    observations: &mut Vec<Observation>,
    blockers: &mut Vec<BlockerCode>,
) {
    if same_dir(root, checkout) {
        return;
    }
    match probe(root) {
        Presence::Missing | Presence::NotApplicable => return,
        presence @ (Presence::Reparse | Presence::Inaccessible) => {
            observations.push(configured_observation(None, presence, true));
            return;
        }
        Presence::Present => {}
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        push(blockers, BlockerCode::PathInaccessible);
        return;
    };
    for (index, entry) in entries.enumerate() {
        if index >= MAX_CHILDREN {
            push(blockers, BlockerCode::InventoryTruncated);
            break;
        }
        let Ok(entry) = entry else {
            push(blockers, BlockerCode::PathInaccessible);
            continue;
        };
        let presence = probe(&entry.path());
        if let Some(location) = owned.location(&entry.path()) {
            observations.push(configured_observation(Some(location), presence, false));
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            observations.push(configured_observation(None, presence, true));
            continue;
        };
        let relative = match prefix {
            Some(prefix) => prefix.join(name).to_string_lossy().replace('\\', "/"),
            None => name.to_string(),
        };
        match RelativePath::new(relative) {
            Ok(relative_path) => observations.push(configured_observation(
                Some(InventoryLocation {
                    scope,
                    root_alias: alias.cloned(),
                    relative_path,
                }),
                presence,
                false,
            )),
            Err(_) => observations.push(configured_observation(None, presence, true)),
        }
    }
}

fn configured_observation(
    location: Option<InventoryLocation>,
    presence: Presence,
    withheld: bool,
) -> Observation {
    Observation {
        source: EntrySource::ConfiguredEntry,
        graph_id: None,
        lifecycle_id: None,
        location,
        withheld,
        presence,
        legacy_marker: false,
        ledger_record: false,
        identity_confirmed: false,
        git_registered: false,
        driver_active: false,
        configured_entry: true,
    }
}

fn observe_persisted_path(
    path: &Path,
    checkout: &Path,
    configured_root: &Path,
    registry: Option<&RegistryView>,
    identity_confirmed: bool,
) -> (Option<InventoryLocation>, Presence, bool) {
    let presence = probe(path);
    match relative_location(
        path,
        checkout,
        configured_root,
        registry,
        identity_confirmed,
    ) {
        Some(location) => (Some(location), presence, false),
        None => (None, presence, true),
    }
}

fn relative_location(
    path: &Path,
    checkout: &Path,
    configured_root: &Path,
    registry: Option<&RegistryView>,
    identity_confirmed: bool,
) -> Option<InventoryLocation> {
    if let Some(relative_path) = contained_relative(checkout, path) {
        return Some(InventoryLocation {
            scope: LocationScope::Repository,
            root_alias: None,
            relative_path,
        });
    }
    if let Some(relative_path) = contained_relative(configured_root, path) {
        return Some(InventoryLocation {
            scope: LocationScope::ConfiguredRoot,
            root_alias: None,
            relative_path,
        });
    }
    if !identity_confirmed {
        return None;
    }
    let registry = registry?;
    for (alias, root) in &registry.roots {
        if native_identity(&root.path).ok().as_ref() != Some(&root.identity) {
            continue;
        }
        if let Some(relative_path) = contained_relative(&root.path, path) {
            return Some(InventoryLocation {
                scope: LocationScope::LedgerRoot,
                root_alias: Some(alias.clone()),
                relative_path,
            });
        }
    }
    None
}

fn contained_relative(root: &Path, target: &Path) -> Option<RelativePath> {
    if probe(root) != Presence::Present || same_dir(root, target) {
        return None;
    }
    if matches!(probe(target), Presence::Reparse | Presence::Inaccessible) {
        return None;
    }
    let root = std::fs::canonicalize(root).ok()?;
    let relative = if probe(target) == Presence::Present {
        std::fs::canonicalize(target)
            .ok()?
            .strip_prefix(&root)
            .ok()?
            .to_path_buf()
    } else {
        let target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            root.join(target)
        };
        target.strip_prefix(&root).ok()?.to_path_buf()
    };
    if relative.as_os_str().is_empty() {
        return None;
    }
    RelativePath::new(relative.to_string_lossy().replace('\\', "/")).ok()
}

fn join_relative(root: &Path, relative: &RelativePath) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in relative.as_str().split('/') {
        path.push(part);
    }
    path
}

fn parse_worktrees(bytes: &[u8]) -> (Vec<PathBuf>, bool) {
    let mut paths = Vec::new();
    let mut invalid = false;
    let parts: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut index = 0;
    while index < parts.len() {
        if parts[index].is_empty() {
            index += 1;
            continue;
        }
        let mut path = None;
        while index < parts.len() && !parts[index].is_empty() {
            if let Some(rest) = parts[index].strip_prefix(b"worktree ") {
                match std::str::from_utf8(rest) {
                    Ok(text) if !text.is_empty() => path = Some(PathBuf::from(text)),
                    _ => invalid = true,
                }
            }
            index += 1;
        }
        if let Some(path) = path {
            paths.push(path);
        }
    }
    (paths, invalid)
}

fn driver_is_active(phase: DriverPhase) -> bool {
    matches!(
        phase,
        DriverPhase::HandoffPending | DriverPhase::Starting | DriverPhase::Running
    )
}

fn git_bytes(checkout: &Path, args: &[&str]) -> Result<Vec<u8>, ()> {
    let mut command = OwnedSpawnSpec::new("git");
    command.arg("-C").arg(checkout);
    command.args(args.iter().copied());
    bounded_stdout(command, Duration::from_secs(10)).map_err(|_| ())
}

fn same_dir(left: &Path, right: &Path) -> bool {
    if probe(left) != Presence::Present || probe(right) != Presence::Present {
        return false;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn probe(path: &Path) -> Presence {
    if path.as_os_str().is_empty() {
        return Presence::Inaccessible;
    }
    let mut saw_missing = false;
    let mut chain: Vec<&Path> = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect();
    chain.reverse();
    for ancestor in chain {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if is_reparse(&meta) => return Presence::Reparse,
            Ok(_) if saw_missing => return Presence::Missing,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => saw_missing = true,
            Err(_) => return Presence::Inaccessible,
        }
    }
    if saw_missing {
        Presence::Missing
    } else {
        Presence::Present
    }
}

fn is_reparse(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn push(blockers: &mut Vec<BlockerCode>, code: BlockerCode) {
    if !blockers.contains(&code) {
        blockers.push(code);
    }
}
