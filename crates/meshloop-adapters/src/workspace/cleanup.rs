//! Read-only cleanup facts for inventory entries.
//! Stats only those entries' joined paths. Does not scan a home directory,
//! create a ledger, or remove anything.

use std::path::{Path, PathBuf};
use std::time::Duration;

use meshloop_domain::workspace::{
    GitRef, InventoryEntry, LifecycleId, LocationScope, ObjectId, RelativePath,
};
use meshloop_engine::workspace::cleanup::{CleanupObservation, ObservedCleanup};

use super::{InventoryScope, RegistryView};

/// Observe `inventory` entries. Paths outside the checkout are flagged and not walked.
pub fn observe_cleanup(scope: InventoryScope<'_>, entries: &[InventoryEntry]) -> ObservedCleanup {
    let common = common_dir(scope.checkout);
    let registry = common.as_ref().and_then(|dir| {
        super::read_registry(&dir.join("meshloop-workspace-registry.json")).unwrap_or_default()
    });
    let repository_id = registry
        .as_ref()
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
                registry.as_ref(),
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
