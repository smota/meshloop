//! Explicit local identity/fence adapter. Absolute locations stay in its protected registry.
use meshloop_domain::workspace::*;
use meshloop_engine::ports::StoreError;
use meshloop_engine::workspace::{RepositoryFence, RepositoryIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod ledger;
#[cfg(test)]
mod tests;

const REGISTRY: &str = "meshloop-workspace-registry.json";
fn refused() -> StoreError {
    StoreError::Corrupt(
        "workspace identity or authority is unavailable, conflicting, or unsupported".into(),
    )
}
fn io_refused(_: impl std::fmt::Display) -> StoreError {
    StoreError::Io("workspace local operation failed".into())
}

pub(crate) fn native_identity(path: &Path) -> Result<NativeIdentity, StoreError> {
    let id = file_id::get_file_id(path).map_err(|_| refused())?;
    let encoded = serde_json::to_vec(&id).map_err(|_| refused())?;
    NativeIdentity::new(format!("{:x}", Sha256::digest(encoded))).map_err(|_| refused())
}
fn regular_entry(path: &Path, directory: bool) -> Result<(), StoreError> {
    let meta = std::fs::symlink_metadata(path).map_err(io_refused)?;
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(refused());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(refused());
        }
    }
    Ok(())
}
fn lock(file: &File) -> Result<(), StoreError> {
    file.try_lock().map_err(|_| refused())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalRoot {
    canonical_path: PathBuf,
    native_identity: NativeIdentity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    pending_roots: BTreeMap<RootAlias, PendingRoot>,
    version: u32,
    repository: RepositoryRecord,
    common: LocalRoot,
    ledger: LocalRoot,
    roots: BTreeMap<RootAlias, LocalRoot>,
}

/// Explicit initializer; existing run-loop producers do not call this API.
pub struct GitRepositoryIdentity {
    git: PathBuf,
}
impl GitRepositoryIdentity {
    pub fn new(git: impl Into<PathBuf>) -> Self {
        Self { git: git.into() }
    }
    fn acquire_with(
        &self,
        checkout: &Path,
        ledger: &Path,
        identify: &dyn Fn(&Path) -> Result<NativeIdentity, StoreError>,
    ) -> Result<RepositoryLease, StoreError> {
        self.acquire_with_hooks(checkout, ledger, identify, &|_| Ok(()), &lock)
    }
    fn acquire_with_hooks(
        &self,
        checkout: &Path,
        ledger: &Path,
        identify: &dyn Fn(&Path) -> Result<NativeIdentity, StoreError>,
        after_open: &dyn Fn(&Path) -> Result<(), StoreError>,
        acquire_lock: &dyn Fn(&File) -> Result<(), StoreError>,
    ) -> Result<RepositoryLease, StoreError> {
        let mut command = Command::new(&self.git);
        command.arg("-C").arg(checkout).args([
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ]);
        let output = bounded_stdout(command, Duration::from_secs(10))?;
        let common = PathBuf::from(std::str::from_utf8(&output).map_err(|_| refused())?.trim());
        let common = std::fs::canonicalize(common).map_err(io_refused)?;
        regular_entry(&common, true)?;
        // Both native probes precede any registry creation or writable opening.
        let common_id = identify(&common)?;
        let ledger = std::fs::canonicalize(ledger).map_err(io_refused)?;
        regular_entry(&ledger, false)?;
        let ledger_id = identify(&ledger)?;
        let registry_path = common.join(REGISTRY);
        let (mut file, created) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&registry_path)
        {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                regular_entry(&registry_path, false)?;
                (
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&registry_path)
                        .map_err(io_refused)?,
                    false,
                )
            }
            Err(error) => return Err(io_refused(error)),
        };
        after_open(&registry_path)?;
        acquire_lock(&file)?;
        regular_entry(&registry_path, false)?;
        let registry_handle = same_file::Handle::from_file(file.try_clone().map_err(io_refused)?)
            .map_err(io_refused)?;
        verify_handle(&registry_handle, &registry_path)?;
        let registry_identity = native_identity(&registry_path)?;
        let manifest = if created {
            let record = RepositoryRecord {
                id: LifecycleId::new(uuid::Uuid::new_v4().to_string()).map_err(|_| refused())?,
                common_dir_identity: common_id.clone(),
            };
            let manifest = Manifest {
                version: 1,
                pending_roots: BTreeMap::new(),
                repository: record,
                common: LocalRoot {
                    canonical_path: common,
                    native_identity: common_id,
                },
                ledger: LocalRoot {
                    canonical_path: ledger,
                    native_identity: ledger_id,
                },
                roots: BTreeMap::new(),
            };
            write_manifest(&mut file, &manifest)?;
            manifest
        } else {
            let manifest = read_manifest(&mut file)?;
            if manifest.version != 1
                || manifest.repository.common_dir_identity != common_id
                || manifest.common.native_identity != common_id
                || manifest.ledger.native_identity != ledger_id
            {
                return Err(refused());
            }
            // Canonical paths locate operational objects; native identity establishes equality.
            if native_identity(&manifest.common.canonical_path)? != common_id
                || native_identity(&manifest.ledger.canonical_path)? != ledger_id
            {
                return Err(refused());
            }
            manifest
        };
        let lease = RepositoryLease {
            file,
            registry_path,
            registry_identity,
            registry_handle,
            manifest,
        };
        lease.validate()?;
        Ok(lease)
    }
}
impl RepositoryIdentity for GitRepositoryIdentity {
    type Fence = RepositoryLease;
    fn acquire(&self, checkout: &Path, ledger: &Path) -> Result<Self::Fence, StoreError> {
        self.acquire_with(checkout, ledger, &native_identity)
    }
}
fn read_manifest(file: &mut File) -> Result<Manifest, StoreError> {
    file.seek(SeekFrom::Start(0)).map_err(io_refused)?;
    let mut bytes = Vec::new();
    // Bound malformed local state before parsing; registry roots are deliberately small.
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_refused)?;
    if bytes.len() > 1024 * 1024 {
        return Err(refused());
    }
    serde_json::from_slice(&bytes).map_err(|_| refused())
}
fn write_manifest(file: &mut File, manifest: &Manifest) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| refused())?;
    if bytes.len() > 1024 * 1024 {
        return Err(refused());
    }
    file.seek(SeekFrom::Start(0)).map_err(io_refused)?;
    file.write_all(&bytes).map_err(io_refused)?;
    file.set_len(bytes.len() as u64).map_err(io_refused)?;
    file.sync_all().map_err(io_refused)
}

/// RAII exclusive fence, retained outside removable artifacts until dropped.
pub struct RepositoryLease {
    file: File,
    registry_path: PathBuf,
    registry_identity: NativeIdentity,
    registry_handle: same_file::Handle,
    manifest: Manifest,
}
impl RepositoryLease {
    pub fn register_root(
        &mut self,
        alias: RootAlias,
        path: &Path,
    ) -> Result<RootRecord, StoreError> {
        if self.manifest.pending_roots.contains_key(&alias)
            && !self.manifest.roots.contains_key(&alias)
        {
            return Err(refused());
        }
        self.register_created_root(alias, path)
    }
    fn register_created_root(
        &mut self,
        alias: RootAlias,
        path: &Path,
    ) -> Result<RootRecord, StoreError> {
        self.validate()?;
        for component in path.ancestors() {
            if !component.as_os_str().is_empty() {
                regular_entry(component, true)?;
            }
        }
        let path = std::fs::canonicalize(path).map_err(io_refused)?;
        regular_entry(&path, true)?;
        let identity = native_identity(&path)?;
        if let Some(existing) = self.manifest.roots.get(&alias) {
            if existing.native_identity != identity {
                return Err(refused());
            }
        } else {
            let mut updated = self.manifest.clone();
            updated.roots.insert(
                alias.clone(),
                LocalRoot {
                    canonical_path: path,
                    native_identity: identity,
                },
            );
            write_manifest(&mut self.file, &updated)?;
            self.manifest = updated;
        }
        self.root(&alias)
    }
}
impl RepositoryFence for RepositoryLease {
    fn repository(&self) -> &RepositoryRecord {
        &self.manifest.repository
    }
    fn ledger_identity(&self) -> &NativeIdentity {
        &self.manifest.ledger.native_identity
    }
    fn validate(&self) -> Result<(), StoreError> {
        regular_entry(&self.registry_path, false)?;
        verify_handle(&self.registry_handle, &self.registry_path)?;
        if native_identity(&self.registry_path)? != self.registry_identity
            || native_identity(&self.manifest.common.canonical_path)?
                != self.manifest.common.native_identity
            || native_identity(&self.manifest.ledger.canonical_path)?
                != self.manifest.ledger.native_identity
        {
            return Err(refused());
        }
        // Clone shares the seek position, but no other code reads this handle concurrently.
        let mut file = self.file.try_clone().map_err(io_refused)?;
        if read_manifest(&mut file)? != self.manifest {
            return Err(refused());
        }
        Ok(())
    }
    fn root(&self, alias: &RootAlias) -> Result<RootRecord, StoreError> {
        self.validate()?;
        let root = self.manifest.roots.get(alias).ok_or_else(refused)?;
        regular_entry(&root.canonical_path, true)?;
        if native_identity(&root.canonical_path)? != root.native_identity {
            return Err(refused());
        }
        Ok(RootRecord {
            repository_id: self.repository().id.clone(),
            alias: alias.clone(),
            native_identity: root.native_identity.clone(),
        })
    }
}
pub(crate) mod schema;

pub(crate) fn verify_handle(handle: &same_file::Handle, path: &Path) -> Result<(), StoreError> {
    let actual = same_file::Handle::from_path(path).map_err(io_refused)?;
    if *handle != actual {
        return Err(refused());
    }
    Ok(())
}

fn bounded_stdout(mut command: Command, timeout: Duration) -> Result<Vec<u8>, StoreError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = crate::process::spawn_owned(command).map_err(io_refused)?;
    let pipe = child.stdout.take().ok_or_else(refused)?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = send.send(result);
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                child.kill_tree();
                return Err(refused());
            }
        }
    };
    // Kill any descendant still holding stdout before waiting for the bounded reader.
    drop(child);
    let bytes = receive
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| refused())?
        .map_err(io_refused)?;
    if !status.success() || bytes.len() > 16 * 1024 {
        return Err(refused());
    }
    Ok(bytes)
}

pub(crate) fn pin_database(path: &Path) -> Result<(same_file::Handle, NativeIdentity), StoreError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(io_refused)?;
    let handle = same_file::Handle::from_file(file).map_err(io_refused)?;
    verify_handle(&handle, path)?;
    let identity = native_identity(path)?;
    Ok((handle, identity))
}
mod runtime;
pub use runtime::CancellationClient;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingRoot {
    canonical_parent: PathBuf,
    parent_identity: NativeIdentity,
    child: RelativePath,
}
impl meshloop_engine::workspace::LifecycleAccess for RepositoryLease {
    fn new_id(&mut self) -> Result<LifecycleId, StoreError> {
        self.validate()?;
        LifecycleId::new(uuid::Uuid::new_v4().to_string()).map_err(|_| refused())
    }
    fn fence(&self) -> Result<&dyn RepositoryFence, StoreError> {
        self.validate()?;
        Ok(self)
    }
    fn resolve(&self, location: &PortableLocation) -> Result<PathBuf, StoreError> {
        self.root(&location.root_alias)?;
        let root = self
            .manifest
            .roots
            .get(&location.root_alias)
            .ok_or_else(refused)?;
        let path = root.canonical_path.join(location.relative_path.as_str());
        // Reject any existing reparse/symlink component; a missing suffix is not adopted.
        let mut cursor = root.canonical_path.clone();
        for component in location.relative_path.as_str().split('/') {
            cursor.push(component);
            match std::fs::symlink_metadata(&cursor) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(refused());
                    }
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if meta.file_attributes() & 0x400 != 0 {
                            return Err(refused());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => return Err(refused()),
            }
        }
        // Git for Windows rejects verbatim canonical paths (`//?/C:/...`).
        // Preserve registry identity; return the equivalent native absolute path.
        #[cfg(windows)]
        {
            use std::path::{Component, Prefix};
            if let Some(Component::Prefix(prefix)) = path.components().next() {
                match prefix.kind() {
                    Prefix::VerbatimDisk(drive) => {
                        let mut normal = PathBuf::from(format!("{}:\\", char::from(drive)));
                        normal.extend(path.components().skip(2));
                        return Ok(normal);
                    }
                    Prefix::VerbatimUNC(server, share) => {
                        let mut normal = PathBuf::from("\\\\");
                        normal.push(server);
                        normal.push(share);
                        normal.extend(path.components().skip(2));
                        return Ok(normal);
                    }
                    _ => {}
                }
            }
        }
        Ok(path)
    }
    fn bootstrap_root(&mut self, alias: RootAlias, path: &Path) -> Result<RootRecord, StoreError> {
        self.validate()?;
        if self.manifest.roots.contains_key(&alias) {
            return self.register_root(alias, path);
        }
        // Resolve only the nearest existing ancestor. Persist the entire missing
        // suffix before creating its first component; never create parents first.
        let mut ancestor = path.to_path_buf();
        let mut missing = Vec::new();
        loop {
            match std::fs::symlink_metadata(&ancestor) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    missing.push(
                        ancestor
                            .file_name()
                            .ok_or_else(refused)?
                            .to_str()
                            .ok_or_else(refused)?
                            .to_string(),
                    );
                    ancestor = ancestor.parent().ok_or_else(refused)?.to_path_buf();
                }
                Err(e) => return Err(io_refused(e)),
            }
        }
        for component in ancestor.ancestors() {
            if !component.as_os_str().is_empty() {
                regular_entry(component, true)?;
            }
        }
        let parent = std::fs::canonicalize(&ancestor).map_err(io_refused)?;
        missing.reverse();
        let child = RelativePath::new(missing.join("/")).map_err(|_| refused())?;
        let target = parent.join(child.as_str());
        let intent = PendingRoot {
            canonical_parent: parent.clone(),
            parent_identity: native_identity(&parent)?,
            child,
        };
        if let Some(old) = self.manifest.pending_roots.get(&alias) {
            if old != &intent {
                return Err(refused());
            }
        } else {
            if target.exists() {
                return Err(refused());
            }
            let mut updated = self.manifest.clone();
            updated.pending_roots.insert(alias.clone(), intent.clone());
            write_manifest(&mut self.file, &updated)?;
            self.manifest = updated;
        }
        if native_identity(&parent)? != intent.parent_identity {
            return Err(refused());
        }
        let mut cursor = parent.clone();
        for component in intent.child.as_str().split('/') {
            // Recheck the pinned ancestor and every created component before each
            // effect. Preexisting partial trees are ambiguous and are not adopted.
            if native_identity(&parent)? != intent.parent_identity {
                return Err(refused());
            }
            for ancestor in cursor.ancestors() {
                if !ancestor.as_os_str().is_empty() {
                    regular_entry(ancestor, true)?;
                }
            }
            cursor.push(component);
            std::fs::create_dir(&cursor).map_err(io_refused)?;
            regular_entry(&cursor, true)?;
        }
        // Keep the original pending intent as retained provenance after registration.
        self.register_created_root(alias, &target)
    }
}
