//! Cleanup facts for inventory entries, plus digest-bound apply.
//! Observation stats only those entries' joined paths. It does not scan a home
//! directory, create a ledger, or remove anything. Apply removes a registered
//! worktree only after a fresh eligible verdict, and it holds the repository fence.

use std::path::{Path, PathBuf};
use std::time::Duration;

use meshloop_domain::workspace::{
    GitRef, InventoryEntry, LifecycleId, LocationScope, ObjectId, RelativePath,
};
use meshloop_engine::workspace::RepositoryIdentity;
use meshloop_engine::workspace::cleanup::{CleanupObservation, ObservedCleanup};

use super::{InventoryScope, RegistryView};

/// Observe `inventory` entries. Paths outside the checkout are flagged and not walked.
pub fn observe_cleanup(scope: InventoryScope<'_>, entries: &[InventoryEntry]) -> ObservedCleanup {
    observe_cleanup_with(scope, entries, None)
}

fn observe_cleanup_with(
    scope: InventoryScope<'_>,
    entries: &[InventoryEntry],
    held: Option<&RegistryView>,
) -> ObservedCleanup {
    let common = common_dir(scope.checkout);
    let owned = if let Some(view) = held {
        Some(view.clone())
    } else {
        common.as_ref().and_then(|dir| {
            super::read_registry(&dir.join("meshloop-workspace-registry.json")).unwrap_or_default()
        })
    };
    let registry = owned.as_ref();
    let repository_id = registry
        .map(|view| view.repository.id.clone())
        .unwrap_or_else(unbound_repository);
    let retained_ref = read_retained_ref(scope.checkout);
    let retained_commit = read_retained_commit(scope.checkout);
    let checkout_canon = canonical_directory(scope.checkout);
    let observations = entries
        .iter()
        .map(|entry| {
            observe_entry(
                entry,
                scope.checkout,
                scope.configured_root,
                registry,
                checkout_canon.as_deref(),
                retained_commit.as_ref(),
            )
        })
        .collect();
    ObservedCleanup {
        repository_id,
        retained_ref,
        retained_commit,
        observations,
    }
}

fn observe_entry(
    entry: &InventoryEntry,
    checkout: &Path,
    configured_root: &Path,
    registry: Option<&RegistryView>,
    checkout_canon: Option<&Path>,
    retained_commit: Option<&ObjectId>,
) -> CleanupObservation {
    if entry.withheld {
        return flagged(true, false, false, false, false);
    }
    let Some(location) = &entry.location else {
        return CleanupObservation::default();
    };
    let Some(root) = location_root(location, checkout, configured_root, registry) else {
        return flagged(true, true, false, false, false);
    };
    let path = join_relative(&root, &location.relative_path);
    match probe_path(&path) {
        PathProbe::Reparse { symlink } => flagged(false, false, true, symlink, false),
        PathProbe::Missing => flagged(false, true, false, false, false),
        PathProbe::Inaccessible => flagged(true, false, false, false, true),
        PathProbe::Present => {
            contain_and_read(&path, &root, checkout, checkout_canon, retained_commit)
        }
    }
}

fn flagged(
    outside: bool,
    missing: bool,
    reparse: bool,
    symlink: bool,
    escape: bool,
) -> CleanupObservation {
    CleanupObservation {
        path_outside_repository: outside,
        missing_dependency: missing,
        path_reparse: reparse,
        path_symlink: symlink,
        path_escape: escape,
        ..CleanupObservation::default()
    }
}

fn contain_and_read(
    path: &Path,
    root: &Path,
    checkout: &Path,
    checkout_canon: Option<&Path>,
    retained_commit: Option<&ObjectId>,
) -> CleanupObservation {
    let Some(canonical) = canonical_directory_or_file(path) else {
        return flagged(false, false, false, false, true);
    };
    let Some(root_canon) = canonical_directory(root) else {
        return flagged(false, false, false, false, true);
    };
    if !canonical.starts_with(&root_canon) {
        return flagged(false, false, false, false, true);
    }
    let inside = checkout_canon.is_some_and(|checkout| canonical.starts_with(checkout));
    if !inside {
        return flagged(true, false, false, false, false);
    }
    git_facts(path, checkout, retained_commit)
}

fn git_facts(path: &Path, checkout: &Path, retained: Option<&ObjectId>) -> CleanupObservation {
    let mut observation = CleanupObservation::default();
    match git_ok(
        path,
        &[
            "status",
            "--porcelain",
            "--ignored",
            "--untracked-files=all",
            "-z",
        ],
    ) {
        Ok(bytes) => {
            if classify_porcelain(&bytes, &mut observation).is_err() {
                content_unknown(&mut observation);
            }
        }
        Err(()) => content_unknown(&mut observation),
    }
    match git_ok(path, &["rev-parse", "--verify", "--end-of-options", "HEAD"]) {
        Ok(bytes) => match parse_oid(&bytes) {
            Some(commit) => {
                observation.commit_reachable = retained.is_some_and(|retained| {
                    git_ok(
                        checkout,
                        &[
                            "merge-base",
                            "--is-ancestor",
                            commit.as_str(),
                            retained.as_str(),
                        ],
                    )
                    .is_ok()
                });
                observation.commit = Some(commit);
            }
            None => observation.commit_unproven = true,
        },
        Err(()) => observation.commit_unproven = true,
    }
    observation
}

fn content_unknown(observation: &mut CleanupObservation) {
    observation.dirty_tracked = true;
    observation.dirty_untracked = true;
    observation.unknown_ignored = true;
}

fn classify_porcelain(bytes: &[u8], observation: &mut CleanupObservation) -> Result<(), ()> {
    let parts: Vec<&[u8]> = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .collect();
    let mut index = 0;
    while index < parts.len() {
        let record = parts[index];
        if record.len() < 4 || record[2] != b' ' {
            return Err(());
        }
        let xy = [record[0], record[1]];
        match &xy {
            b"??" => observation.dirty_untracked = true,
            b"!!" => observation.unknown_ignored = true,
            _ => observation.dirty_tracked = true,
        }
        index += 1;
        if xy[0] == b'R' || xy[0] == b'C' || xy[1] == b'R' || xy[1] == b'C' {
            if index >= parts.len() {
                return Err(());
            }
            index += 1;
        }
    }
    Ok(())
}

fn location_root(
    location: &meshloop_domain::workspace::InventoryLocation,
    checkout: &Path,
    configured_root: &Path,
    registry: Option<&RegistryView>,
) -> Option<PathBuf> {
    match location.scope {
        LocationScope::Repository => Some(checkout.to_path_buf()),
        LocationScope::ConfiguredRoot => Some(configured_root.to_path_buf()),
        LocationScope::LedgerRoot => {
            let alias = location.root_alias.as_ref()?;
            let registry = registry?;
            let root = registry.roots.get(alias)?;
            if super::native_identity(&root.path).ok().as_ref() != Some(&root.identity) {
                return None;
            }
            Some(root.path.clone())
        }
    }
}

fn join_relative(root: &Path, relative: &RelativePath) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in relative.as_str().split('/') {
        path.push(part);
    }
    path
}

enum PathProbe {
    Reparse { symlink: bool },
    Missing,
    Inaccessible,
    Present,
}

fn probe_path(path: &Path) -> PathProbe {
    if path.as_os_str().is_empty() {
        return PathProbe::Inaccessible;
    }
    let mut missing = false;
    let mut chain: Vec<&Path> = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect();
    chain.reverse();
    for ancestor in chain {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if is_reparse(&meta) => {
                return PathProbe::Reparse {
                    symlink: meta.file_type().is_symlink(),
                };
            }
            Ok(_) if missing => return PathProbe::Missing,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(_) => return PathProbe::Inaccessible,
        }
    }
    if missing {
        PathProbe::Missing
    } else {
        PathProbe::Present
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

fn canonical_directory(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if is_reparse(&meta) || !meta.is_dir() {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

fn canonical_directory_or_file(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if is_reparse(&meta) {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

fn common_dir(checkout: &Path) -> Option<PathBuf> {
    let bytes = git_ok(
        checkout,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let path = PathBuf::from(text);
    let path = if path.is_absolute() {
        path
    } else {
        checkout.join(path)
    };
    canonical_directory(&path)
}

fn read_retained_ref(checkout: &Path) -> GitRef {
    if let Ok(bytes) = git_ok(checkout, &["symbolic-ref", "--quiet", "HEAD"])
        && let Ok(text) = String::from_utf8(bytes)
        && let Ok(git_ref) = GitRef::new(text.trim())
    {
        return git_ref;
    }
    match GitRef::new("HEAD") {
        Ok(git_ref) => git_ref,
        Err(_) => unreachable!("HEAD is a legal git ref"),
    }
}

fn read_retained_commit(checkout: &Path) -> Option<ObjectId> {
    let bytes = git_ok(
        checkout,
        &["rev-parse", "--verify", "--end-of-options", "HEAD"],
    )
    .ok()?;
    parse_oid(&bytes)
}

fn parse_oid(bytes: &[u8]) -> Option<ObjectId> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    ObjectId::new(text).ok()
}

fn unbound_repository() -> LifecycleId {
    match LifecycleId::new("unbound") {
        Ok(id) => id,
        Err(_) => unreachable!("unbound is a legal lifecycle id"),
    }
}

fn git_ok(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, ()> {
    let mut command = crate::process::OwnedSpawnSpec::new("git");
    command.arg("--no-optional-locks");
    command.arg("-C").arg(cwd);
    command.args(args.iter().copied());
    command.env("GIT_OPTIONAL_LOCKS", "0");
    super::bounded_stdout(command, Duration::from_secs(10)).map_err(|_| ())
}

/// Deterministic crash injection for library callers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupFault {
    AfterIntent,
    AfterRemove,
}

#[derive(Debug, serde::Serialize)]
pub struct CleanupApply {
    pub apply: bool,
    pub digest: String,
    pub candidates: Vec<meshloop_domain::cleanup::CleanupCandidatePreview>,
    pub status: &'static str,
    pub reasons: Vec<&'static str>,
}

type Before = (InventoryEntry, CleanupObservation);
#[derive(serde::Serialize, serde::Deserialize)]
struct BeforeToken {
    candidate: Before,
    // Inventory may represent the same worktree both as ledger-owned and git-registered.
    aliases: Vec<Before>,
}

fn cleanup_error(error: impl std::fmt::Display) -> meshloop_engine::ports::StoreError {
    // Do not expose native locations or git stderr.
    let _ = error;
    meshloop_engine::ports::StoreError::Io("cleanup operation failed".into())
}

fn copy_scope<'a>(scope: &InventoryScope<'a>) -> InventoryScope<'a> {
    InventoryScope {
        checkout: scope.checkout,
        ledger: scope.ledger,
        configured_root: scope.configured_root,
    }
}

fn preview(scope: &InventoryScope<'_>) -> (Vec<InventoryEntry>, ObservedCleanup) {
    preview_with(scope, None)
}

fn preview_with(
    scope: &InventoryScope<'_>,
    held: Option<&RegistryView>,
) -> (Vec<InventoryEntry>, ObservedCleanup) {
    let inventory = meshloop_engine::workspace::inventory::classify(
        &super::inventory::discover_inventory_with(copy_scope(scope), held),
    );
    let observed = observe_cleanup_with(copy_scope(scope), &inventory.entries, held);
    (inventory.entries, observed)
}

fn verdict(
    entries: &[InventoryEntry],
    observed: &ObservedCleanup,
) -> meshloop_domain::cleanup::CleanupPreview {
    meshloop_engine::workspace::cleanup::cleanup_preview(
        observed.repository_id.clone(),
        observed.retained_ref.clone(),
        observed.retained_commit.clone(),
        entries,
        &observed.observations,
        None,
    )
}

fn candidate_path(
    scope: &InventoryScope<'_>,
    entry: &InventoryEntry,
    held: Option<&RegistryView>,
) -> Option<PathBuf> {
    let owned = if let Some(view) = held {
        Some(view.clone())
    } else {
        let common = common_dir(scope.checkout)?;
        super::read_registry(&common.join("meshloop-workspace-registry.json")).ok()?
    };
    let registry = owned.as_ref()?;
    let location = entry.location.as_ref()?;
    let root = location_root(
        location,
        scope.checkout,
        scope.configured_root,
        Some(registry),
    )?;
    Some(join_relative(&root, &location.relative_path))
}

fn registered(
    scope: &InventoryScope<'_>,
    path: &Path,
) -> Result<bool, meshloop_engine::ports::StoreError> {
    let mut command = crate::process::OwnedSpawnSpec::new("git");
    command
        .arg("-C")
        .arg(scope.checkout)
        .args(["worktree", "list", "--porcelain", "-z"]);
    let bytes = super::bounded_stdout(command, Duration::from_secs(10))?;
    Ok(bytes.split(|b| *b == 0).any(|record| {
        record
            .strip_prefix(b"worktree ")
            .is_some_and(|listed| same_worktree(listed, path))
    }))
}

/// Git for Windows prints worktree paths with `/`. Compare the filesystem
/// identity when both paths exist, and the separator-normalized text otherwise.
fn same_worktree(listed: &[u8], path: &Path) -> bool {
    let Ok(text) = std::str::from_utf8(listed) else {
        return false;
    };
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() {
        return false;
    }
    let listed = PathBuf::from(text);
    if same_path_text(&listed, path) {
        return true;
    }
    match (std::fs::canonicalize(&listed), std::fs::canonicalize(path)) {
        (Ok(left), Ok(right)) => same_path_text(&left, &right),
        _ => false,
    }
}

fn same_path_text(left: &Path, right: &Path) -> bool {
    fn fold(path: &Path) -> String {
        #[cfg(windows)]
        {
            let mut text = path.to_string_lossy().replace('/', "\\");
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                text = format!(r"\\{rest}");
            } else if let Some(rest) = text.strip_prefix(r"\\?\") {
                text = rest.to_string();
            }
            text.make_ascii_lowercase();
            text
        }
        #[cfg(not(windows))]
        {
            path.to_string_lossy().into_owned()
        }
    }
    fold(left) == fold(right)
}

/// Git for Windows rejects `\\?\` paths in `worktree remove`. The registry
/// stores canonical paths, which carry that prefix on Windows.
fn git_worktree_arg(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    PathBuf::from(text.as_ref())
}

/// Apply only a digest-bound preview, journaling each non-forced git removal.
pub fn apply_cleanup(
    scope: InventoryScope<'_>,
    digest: &str,
    fault: Option<CleanupFault>,
) -> Result<CleanupApply, meshloop_engine::ports::StoreError> {
    use meshloop_domain::cleanup::CleanupEligibility;
    use rusqlite::{Connection, params};
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(cleanup_error("invalid digest"));
    }
    let (mut entries, mut observed) = preview(&scope);
    let fresh = verdict(&entries, &observed);
    let mut report = CleanupApply {
        apply: false,
        digest: fresh.digest.clone(),
        candidates: fresh.candidates,
        status: "blocked",
        reasons: vec![],
    };
    // Read existing recovery evidence without creating or migrating a ledger.
    let mut journal: Vec<(String, String, String)> = Vec::new();
    if scope.ledger.exists() {
        let conn =
            Connection::open_with_flags(scope.ledger, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(cleanup_error)?;
        match super::schema::schema_version(&conn)? {
            Some(8) => {
                super::schema::validate8(&conn)?;
                let mut stmt = conn.prepare("SELECT candidate_key,CASE WHEN MAX(step='blocked') THEN 'blocked' WHEN MAX(step='completed') THEN 'completed' ELSE 'intent' END,expected_before_token FROM cleanup_journal WHERE repo_id=?1 AND digest=?2 GROUP BY candidate_key,expected_before_token ORDER BY candidate_key").map_err(cleanup_error)?;
                journal = stmt
                    .query_map(params![observed.repository_id.as_str(), digest], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .map_err(cleanup_error)?
                    .collect::<Result<_, _>>()
                    .map_err(cleanup_error)?;
            }
            Some(7) => super::schema::validate7(&conn)?,
            _ => return Err(cleanup_error("unsupported ledger")),
        }
    }
    // Restore only proven removed candidates into the digest input. Every other
    // current fact remains fresh, so unrelated drift still invalidates recovery.
    let mut reconciled = Vec::new();
    for (key, step, token) in &journal {
        let saved: BeforeToken = serde_json::from_str(token).map_err(cleanup_error)?;
        let before = &saved.candidate;
        let Some(path) = candidate_path(&scope, &before.0, None) else {
            report.status = "recovery-required";
            return Ok(report);
        };
        let present = match std::fs::symlink_metadata(&path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(cleanup_error(error)),
        };
        let listed = registered(&scope, &path)?;
        if step == "blocked" || present != listed || (step == "completed" && present) {
            report.status = "recovery-required";
            return Ok(report);
        }
        if !present {
            for row in std::iter::once(before).chain(saved.aliases.iter()) {
                if let Some(index) = entries.iter().position(|e| {
                    e.lifecycle_id == row.0.lifecycle_id
                        && e.source == row.0.source
                        && e.location == row.0.location
                }) {
                    let mut current = entries[index].clone();
                    // Absence changes only the inventory's missing classification.
                    current.class = row.0.class;
                    current
                        .blockers
                        .retain(|b| *b != meshloop_domain::workspace::BlockerCode::PathMissing);
                    if current != row.0 {
                        report.status = "recovery-required";
                        return Ok(report);
                    }
                    entries[index] = row.0.clone();
                    observed.observations[index] = row.1.clone();
                } else {
                    if row.0.lifecycle_id.is_some() {
                        report.status = "recovery-required";
                        return Ok(report);
                    }
                    entries.push(row.0.clone());
                    observed.observations.push(row.1.clone());
                }
            }
            reconciled.push(key.clone());
        } else {
            let current = entries.iter().position(|e| e == &before.0);
            if !current.is_some_and(|i| observed.observations[i] == before.1) {
                report.status = "recovery-required";
                return Ok(report);
            }
        }
    }
    let bound = verdict(&entries, &observed);

    if bound.digest != digest {
        report.status = if journal.is_empty() {
            "blocked"
        } else {
            "recovery-required"
        };
        report.reasons.push("stale-facts");
        for c in &mut report.candidates {
            c.status = CleanupEligibility::Blocked;
            c.reasons
                .push(meshloop_domain::cleanup::CleanupReason::StaleFacts);
        }
        return Ok(report);
    }
    let eligible: Vec<Before> = entries
        .iter()
        .zip(&observed.observations)
        .filter(|(e, _)| {
            bound.candidates.iter().any(|c| {
                c.lifecycle_id == e.lifecycle_id
                    && c.source == e.source
                    && c.location == e.location
                    && c.status == CleanupEligibility::Eligible
            })
        })
        .map(|(e, o)| (e.clone(), o.clone()))
        .collect();
    if eligible.is_empty() {
        return Ok(report);
    }
    let Some(common) = common_dir(scope.checkout) else {
        report.status = "recovery-required";
        return Ok(report);
    };
    if !common.join(super::REGISTRY).is_file() {
        report.status = "recovery-required";
        return Ok(report);
    }
    let fence = super::GitRepositoryIdentity::new("git").acquire(scope.checkout, scope.ledger)?;
    let held = fence.registry_view();
    let conn = Connection::open(scope.ledger).map_err(cleanup_error)?;
    super::schema::migrate(&conn)?;
    let write = |key: &str,
                 step: &str,
                 token: &str|
     -> Result<(), meshloop_engine::ports::StoreError> {
        conn.execute("INSERT INTO cleanup_journal(repo_id,digest,candidate_key,step,expected_before_token,expected_after_token) VALUES(?1,?2,?3,?4,?5,'absent-unregistered') ON CONFLICT(repo_id,digest,candidate_key,step) DO NOTHING", params![observed.repository_id.as_str(),digest,key,step,token]).map_err(cleanup_error)?;
        Ok(())
    };
    for before in eligible {
        let path = candidate_path(&scope, &before.0, Some(&held));
        let aliases = entries
            .iter()
            .zip(&observed.observations)
            .filter(|(entry, _)| **entry != before.0 && candidate_path(&scope, entry, Some(&held)) == path)
            .map(|(entry, observation)| (entry.clone(), observation.clone()))
            .collect();
        let token = serde_json::to_string(&BeforeToken {
            candidate: before.clone(),
            aliases,
        })
        .map_err(cleanup_error)?;
        let key = meshloop_domain::digest::sha256_hex(token.as_bytes());
        if reconciled.contains(&key) {
            write(&key, "completed", &token)?;
            continue;
        }
        let (now_entries, now_observed) = preview_with(&scope, Some(&held));
        let now = verdict(&now_entries, &now_observed);
        let valid = now_entries
            .iter()
            .position(|e| e == &before.0)
            .is_some_and(|i| now_observed.observations[i] == before.1)
            && now.candidates.iter().any(|c| {
                c.lifecycle_id == before.0.lifecycle_id
                    && c.source == before.0.source
                    && c.location == before.0.location
                    && c.status == CleanupEligibility::Eligible
            });
        let Some(path) = candidate_path(&scope, &before.0, Some(&held)) else {
            report.reasons.push("candidate-path");
            report.status = "recovery-required";
            return Ok(report);
        };
        let safe = path != scope.checkout
            && !scope.ledger.starts_with(&path)
            && common_dir(scope.checkout).is_some_and(|p| !p.starts_with(&path));
        let listed = registered(&scope, &path)?;
        if !valid || !safe || !listed {
            if !valid {
                report.reasons.push("revalidation");
            }
            if !safe {
                report.reasons.push("unsafe-path");
            }
            if !listed {
                report.reasons.push("unregistered");
            }
            write(&key, "blocked", &token)?;
            report.status = "recovery-required";
            return Ok(report);
        }
        write(&key, "intent", &token)?;
        if fault == Some(CleanupFault::AfterIntent) {
            report.status = "interrupted";
            return Ok(report);
        }
        let mut command = crate::process::OwnedSpawnSpec::new("git");
        command
            .current_dir(scope.checkout)
            .args(["worktree", "remove"])
            .arg(git_worktree_arg(&path));
        if let Err(error) = super::bounded_stdout(command, Duration::from_secs(10)) {
            write(&key, "blocked", &token)?;
            return Err(error);
        }
        if fault == Some(CleanupFault::AfterRemove) {
            report.status = "interrupted";
            return Ok(report);
        }
        write(&key, "completed", &token)?;
    }
    report.apply = true;
    report.status = "completed";
    Ok(report)
}
