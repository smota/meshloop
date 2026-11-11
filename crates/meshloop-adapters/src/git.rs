//! Worktree-per-writer isolation (ADR 0005 / runtime-design.md §3). Real `git worktree`
//! subprocess calls with structured arguments — never shell-string interpolation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::host_git::GitLinkError;

use meshloop_engine::ports::{DiffSummary, IgnoredClean, WorkspaceError, WorkspacePort};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    CommandFailed {
        stderr: String,
    },
    Io(String),
    Conflict,
    Dirty,
    /// The worktree's `.git` link was modified; git was not run there (#180).
    LinkModified(GitLinkError),
    /// A nested repository risk in a worktree (#180): a `.git` entry below the top
    /// level, a gitlink not in the attempt base, or a walk that could not complete.
    /// `path` is worktree-relative.
    NestedGit {
        path: PathBuf,
        reason: &'static str,
    },
    /// Reading or staging the index failed, or the worktree walk failed, with no
    /// nested entry observed. Refuses the attempt, never the run (#180).
    StagingFailed {
        reason: &'static str,
    },
}

impl From<GitError> for WorkspaceError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::CommandFailed { stderr } => WorkspaceError::CommandFailed { stderr },
            GitError::Io(s) => WorkspaceError::Io(s),
            GitError::Conflict => WorkspaceError::Conflict,
            GitError::Dirty => WorkspaceError::Dirty,
            GitError::LinkModified(e) => WorkspaceError::GitLinkModified {
                path: e.link,
                reason: e.reason.to_string(),
            },
            GitError::NestedGit { path, reason } => WorkspaceError::NestedGit {
                path,
                reason: reason.to_string(),
            },
            GitError::StagingFailed { reason } => WorkspaceError::StagingFailed {
                reason: reason.to_string(),
            },
        }
    }
}

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static GIT_ADMIN_LOCK: Mutex<()> = Mutex::new(());
static LAYOUT_PROBES: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
const LAYOUT_HINT: &str = "set --worktree-base or [workspace] root";

pub struct GitWorktreeAdapter {
    repo_root: PathBuf,
    /// The repository's common dir, read once from `repo_root` (trusted).
    common: OnceLock<PathBuf>,
}

impl GitWorktreeAdapter {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            common: OnceLock::new(),
        }
    }

    fn common_dir(&self) -> Result<&Path, GitError> {
        if let Some(common) = self.common.get() {
            return Ok(common);
        }
        let output = self.spawn(
            &self.repo_root,
            None,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        if !output.status.success() {
            return Err(Self::failed(&["rev-parse", "--git-common-dir"], &output));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            return Err(GitError::Io("empty git common dir".into()));
        }
        Ok(self.common.get_or_init(|| PathBuf::from(text)))
    }

    fn is_repo_root(&self, cwd: &Path) -> bool {
        cwd == self.repo_root
            || matches!(
                (std::fs::canonicalize(cwd), std::fs::canonicalize(&self.repo_root)),
                (Ok(a), Ok(b)) if a == b
            )
    }

    /// The verified admin dir of a worktree other than the repository root. A link
    /// the worker modified is refused before any git runs there (#180).
    /// Gitlinks of the attempt base, as `(path, object)`. An unreadable base allows
    /// none, so every gitlink is refused.
    fn base_gitlinks(&self, worktree: &Path, base: &str) -> Vec<(String, String)> {
        if base.is_empty() || base.starts_with('-') {
            return Vec::new();
        }
        self.run_in(worktree, &["ls-tree", "-r", "-z", "--end-of-options", base])
            .map(|listing| gitlinks(&listing))
            .unwrap_or_default()
    }

    /// Reads the index (`ls-files --stage`, which spawns no helper) and refuses any
    /// gitlink not in `allowed` or whose `<path>/.git` exists on the filesystem.
    fn check_index_gitlinks(
        &self,
        worktree: &Path,
        allowed: &[(String, String)],
    ) -> Result<(), GitError> {
        let index = self.run_in(worktree, &["ls-files", "--stage", "-z"])?;
        for (path, object) in index_gitlinks(&index) {
            let rel = PathBuf::from(&path);
            if !allowed.contains(&(path.clone(), object)) {
                return Err(GitError::NestedGit {
                    path: rel,
                    reason: "gitlink not in the attempt base",
                });
            }
            if std::fs::symlink_metadata(worktree.join(&path).join(".git")).is_ok() {
                return Err(GitError::NestedGit {
                    path: rel.join(".git"),
                    reason: "populated gitlink",
                });
            }
            if !unpopulated(&worktree.join(&path)) {
                return Err(GitError::NestedGit {
                    path: rel,
                    reason: UNPOPULATED_CONTENT,
                });
            }
        }
        Ok(())
    }

    /// `add -A` of the worktree's content, excluding every gitlink in the index.
    /// Gitlinks carry no content, and `add` would otherwise inspect a populated one
    /// by running git inside it, under that repository's config (#180).
    fn stage_all(&self, worktree: &Path) -> Result<(), GitError> {
        let index = self.run_in(worktree, &["ls-files", "--stage", "-z"])?;
        let excludes: Vec<String> = index_gitlinks(&index)
            .into_iter()
            .map(|(path, _)| format!(":(exclude,top,literal){path}"))
            .collect();
        let mut args = vec!["add", "-A", "--", "."];
        args.extend(excludes.iter().map(String::as_str));
        self.run_in(worktree, &args).map(|_| ())
    }

    fn verified_admin(&self, worktree: &Path) -> Result<PathBuf, GitError> {
        let common = self.common_dir()?.to_path_buf();
        crate::host_git::verify_worktree_link(worktree, &common).map_err(GitError::LinkModified)
    }

    fn output_in(&self, cwd: &Path, args: &[&str]) -> Result<std::process::Output, GitError> {
        let admin = if self.is_repo_root(cwd) {
            None
        } else {
            Some(self.verified_admin(cwd)?)
        };
        self.spawn(cwd, admin.as_deref(), args)
    }

    /// Every git process this adapter starts: hardened, and pinned to `admin` when
    /// it runs in a worktree.
    fn spawn(
        &self,
        cwd: &Path,
        admin: Option<&Path>,
        args: &[&str],
    ) -> Result<std::process::Output, GitError> {
        let mut cmd = crate::host_git::command("git");
        if let Some(admin) = admin {
            cmd.args(crate::host_git::pinned(admin));
        }
        cmd.current_dir(cwd).args(args);
        cmd.env("GIT_AUTHOR_NAME", "meshloop");
        cmd.env("GIT_AUTHOR_EMAIL", "meshloop@localhost");
        cmd.env("GIT_COMMITTER_NAME", "meshloop");
        cmd.env("GIT_COMMITTER_EMAIL", "meshloop@localhost");
        crate::process::hide_console(&mut cmd);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::process::spawn_command(&mut cmd)
            .and_then(|child| child.wait_with_output())
            .map_err(|e| GitError::Io(e.to_string()))
    }

    fn failed(args: &[&str], output: &std::process::Output) -> GitError {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stderr = if stderr.trim().is_empty() {
            format!(
                "git {:?} failed (exit code {:?})",
                args,
                output.status.code()
            )
        } else {
            stderr
        };
        GitError::CommandFailed { stderr }
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Result<String, GitError> {
        let output = self.output_in(cwd, args)?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(Self::failed(args, &output))
        }
    }

    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        self.run_in(&self.repo_root, args)
    }

    /// Serializes administrative Git operations (worktree add/remove/prune) across threads
    /// and retries with exponential backoff if `.git/index.lock` is held by Windows indexers/antivirus.
    fn run_admin_with_retry(&self, args: &[&str]) -> Result<String, GitError> {
        let _guard = GIT_ADMIN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut backoff = Duration::from_millis(50);
        let max_wait = Duration::from_secs(10);
        let start = Instant::now();
        loop {
            match self.run_in(&self.repo_root, args) {
                Ok(out) => return Ok(out),
                Err(GitError::CommandFailed { ref stderr })
                    if (stderr.contains("index.lock")
                        || stderr.contains("Unable to create")
                        || stderr.contains("locked"))
                        && start.elapsed() < max_wait =>
                {
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_millis(500));
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn add_worktree(&self, path: &Path, branch: &str) -> Result<(), GitError> {
        self.run_admin_with_retry(&["worktree", "add", "-b", branch, &path.to_string_lossy()])
            .map(|_| ())
    }

    pub fn remove_worktree(&self, path: &Path) -> Result<(), GitError> {
        self.verified_admin(path)?;
        self.run_admin_with_retry(&["worktree", "remove", "--force", &path.to_string_lossy()])
            .map(|_| ())
    }

    /// Read-only commit resolution. Does not create, move, or delete refs.
    pub(crate) fn resolve_commit(&self, git_ref: &str) -> Result<String, GitError> {
        if meshloop_domain::workspace::GitRef::new(git_ref).is_err() {
            return Err(GitError::CommandFailed {
                stderr: "unreadable ref".into(),
            });
        }
        let spec = format!("{git_ref}^{{commit}}");
        self.run(&["rev-parse", "--verify", "--end-of-options", &spec])
    }

    /// `merge-base --is-ancestor`. Exit 1 is not-ancestor; other failures stay errors.
    pub(crate) fn commit_is_ancestor(
        &self,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, GitError> {
        if !oid_ok(ancestor) || !oid_ok(descendant) {
            return Err(GitError::CommandFailed {
                stderr: "unreadable ref".into(),
            });
        }
        let args = ["merge-base", "--is-ancestor", ancestor, descendant];
        let output = self.output_in(&self.repo_root, &args)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(Self::failed(&args, &output)),
        }
    }

    pub(crate) fn commit_parents(&self, commit: &str) -> Result<Vec<String>, GitError> {
        if !oid_ok(commit) {
            return Err(GitError::CommandFailed {
                stderr: "unreadable ref".into(),
            });
        }
        let args = ["--no-pager", "log", "-1", "--format=%P", commit];
        let output = self.output_in(&self.repo_root, &args)?;
        if !output.status.success() {
            return Err(Self::failed(&args, &output));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut parents = Vec::new();
        for parent in text.split_whitespace() {
            if !oid_ok(parent) {
                return Err(GitError::CommandFailed {
                    stderr: "unreadable ref".into(),
                });
            }
            parents.push(parent.to_string());
        }
        Ok(parents)
    }

    /// Non-pruning object walk. True when `tree` is reachable from `from`.
    pub(crate) fn tree_reachable_from(&self, tree: &str, from: &str) -> Result<bool, GitError> {
        if !oid_ok(tree) || !oid_ok(from) {
            return Err(GitError::CommandFailed {
                stderr: "unreadable ref".into(),
            });
        }
        let args = ["rev-list", "--objects", from];
        let output = self.output_in(&self.repo_root, &args)?;
        if !output.status.success() {
            return Err(Self::failed(&args, &output));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text
            .lines()
            .any(|line| line.split_whitespace().next() == Some(tree)))
    }

    pub(crate) fn object_exists(&self, object: &str) -> Result<bool, GitError> {
        if !oid_ok(object) {
            return Ok(false);
        }
        let args = ["cat-file", "-e", object];
        let output = self.output_in(&self.repo_root, &args)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(Self::failed(&args, &output)),
        }
    }

    pub(crate) fn planner_is_dirty(&self, worktree: &Path) -> Result<bool, GitError> {
        self.run_in(
            worktree,
            &["status", "--porcelain", "--ignore-submodules=all"],
        )
        .map(|porcelain| !porcelain.is_empty())
    }

    /// Fail closed before the real worktree is created. A successful probe is remembered
    /// for this process. An existing probe path is left untouched.
    fn prove_layout(&self, parent: &Path) -> Result<(), GitError> {
        let mut proven = LAYOUT_PROBES
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let key = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        if proven.contains(&key) {
            return Ok(());
        }
        if !parent.is_dir() {
            return Err(GitError::Io(format!(
                "layout probe parent is missing; {LAYOUT_HINT}"
            )));
        }
        let probe = parent.join(".meshloop-layout-probe");
        if std::fs::symlink_metadata(&probe).is_ok() {
            return Err(GitError::Io(format!(
                "layout probe path already exists; {LAYOUT_HINT}"
            )));
        }
        let probe_arg = git_worktree_arg(&probe);
        let probe_text = probe_arg.to_string_lossy().into_owned();
        self.run_admin_with_retry(&["worktree", "add", "--detach", &probe_text])
            .map_err(annotate_layout)?;
        self.run_admin_with_retry(&["worktree", "remove", &probe_text])
            .map_err(annotate_layout)?;
        if std::fs::symlink_metadata(&probe).is_ok() {
            return Err(GitError::Io(format!(
                "layout probe remained after removal; {LAYOUT_HINT}"
            )));
        }
        proven.insert(key);
        Ok(())
    }
}

fn git_worktree_arg(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

fn annotate_layout(error: GitError) -> GitError {
    match error {
        GitError::CommandFailed { stderr } => GitError::CommandFailed {
            stderr: format!("{stderr}\n{LAYOUT_HINT}"),
        },
        GitError::Io(message) => GitError::Io(format!("{message}; {LAYOUT_HINT}")),
        other => other,
    }
}

/// The first entry named `.git` (any case, or the NTFS short name `git~1`) below the
/// worktree's top level, worktree-relative, with the reason. The top-level link is
/// checked separately ([`crate::host_git::verify_worktree_link`]). Walks without
/// following symlinks or junctions. An entry that vanishes during the walk is
/// skipped; any other walk error is a refusal, never a run-aborting I/O error.
fn nested_git_entry(worktree: &Path) -> Option<(PathBuf, &'static str)> {
    let rel = |path: &Path| path.strip_prefix(worktree).unwrap_or(path).to_path_buf();
    let gone = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let mut stack = vec![(worktree.to_path_buf(), true)];
    while let Some((dir, top)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if gone(&e) && !top => continue,
            Err(_) => return Some((rel(&dir), "worktree walk failed")),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) if gone(&e) => continue,
                Err(_) => return Some((rel(&dir), "worktree walk failed")),
            };
            let name = entry.file_name();
            let lower = name.to_string_lossy().to_ascii_lowercase();
            if top && lower == ".git" {
                continue;
            }
            if lower == ".git" || lower == "git~1" {
                return Some((rel(&entry.path()), NESTED_ENTRY));
            }
            // `DirEntry::file_type` does not follow symlinks (or junctions on Windows).
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push((entry.path(), false)),
                Ok(_) => {}
                Err(e) if gone(&e) => {}
                Err(_) => return Some((rel(&entry.path()), "worktree walk failed")),
            }
        }
    }
    None
}

/// Reason of anything at an index gitlink's path other than an empty directory.
const UNPOPULATED_CONTENT: &str = "content in an unpopulated gitlink";

/// True when an index gitlink's path is absent or an empty real directory, as
/// `worktree add` leaves an uninitialised submodule. Status, `add` and `clean` ignore
/// what is inside a gitlink's directory, so content there (or a file or link in its
/// place) would reach a check without being reviewed. Unreadable counts as content.
fn unpopulated(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() && !reparse_point(&meta) => {
            std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
        }
        Ok(_) => false,
    }
}

/// A Windows reparse point (junction, symlink, mount point or other); never on Unix,
/// where `FileType::is_symlink` covers links.
fn reparse_point(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Reason of a `.git` entry found below the worktree's top level.
const NESTED_ENTRY: &str = "nested .git entry";

/// Arguments listing a worktree's ignored entries, NUL-separated, an ignored directory
/// once (`--directory`): the set `clean -d -X` removes.
const LIST_IGNORED: [&str; 6] = [
    "ls-files",
    "-z",
    "--others",
    "--ignored",
    "--exclude-standard",
    "--directory",
];

/// Bound of the walk over a worktree's ignored entries before `git clean`. Past it the
/// clean is refused (fail closed), never done on a tree that was not fully walked.
const IGNORED_WALK_ENTRIES: usize = 5_000_000;

/// True when `worktree` is `<attempt_root>/<graph>/task-<n>-attempt-<m>`, compared on
/// canonical paths, the run loop's attempt layout (#142). Anything unresolvable is not.
fn attempt_worktree(worktree: &Path, attempt_root: &Path) -> bool {
    let (Ok(worktree), Ok(root)) = (
        std::fs::canonicalize(worktree),
        std::fs::canonicalize(attempt_root),
    ) else {
        return false;
    };
    let leaf = worktree
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let numbered = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let attempt_leaf = leaf
        .strip_prefix("task-")
        .and_then(|rest| rest.split_once("-attempt-"))
        .is_some_and(|(task, attempt)| numbered(task) && numbered(attempt));
    attempt_leaf && worktree.parent().and_then(Path::parent) == Some(root.as_path())
}

/// Worktree-relative paths of `ls-files -z` output, trailing `/` dropped. `None` when
/// any record is absolute or climbs out with `..`: git never lists such a path, so
/// the listing is not trusted.
fn ignored_entries(listing: &[u8]) -> Option<Vec<PathBuf>> {
    use std::path::Component;
    let mut entries = Vec::new();
    for record in listing.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let text = String::from_utf8_lossy(record);
        let text = text.trim_end_matches('/');
        let path = PathBuf::from(text);
        if text.is_empty()
            || text.starts_with('/')
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
        {
            return None;
        }
        entries.push(path);
    }
    Some(entries)
}

/// State of [`unlink_and_measure`].
struct IgnoredWalk {
    bytes: u64,
    seen: usize,
    /// One open directory iterator per level: memory is bounded by depth, not by how
    /// many entries a directory holds.
    stack: Vec<std::fs::ReadDir>,
}

impl IgnoredWalk {
    fn visit(&mut self, path: &Path) -> Result<(), &'static str> {
        self.seen += 1;
        if self.seen > IGNORED_WALK_ENTRIES {
            return Err("too many ignored entries");
        }
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("ignored entry unreadable"),
        };
        self.bytes = self.bytes.saturating_add(meta.len());
        if meta.file_type().is_symlink() || reparse_point(&meta) {
            return remove_link(path, &meta).map_err(|_| "ignored link not removed");
        }
        if meta.is_dir() {
            let entries = std::fs::read_dir(path).map_err(|_| "ignored directory unreadable")?;
            self.stack.push(entries);
        }
        Ok(())
    }
}

/// Walks the ignored `entries` below `worktree` without following links, sums their
/// bytes (a link counts as itself), and removes every symlink, junction or other
/// reparse point among them: the link itself, never what it points at. `git clean`
/// then meets no link at all, so it cannot traverse one whatever its own rules are
/// (#196). Any entry that cannot be read or removed, or a walk past
/// [`IGNORED_WALK_ENTRIES`], refuses the clean. Entries that vanish are skipped.
fn unlink_and_measure(worktree: &Path, entries: &[PathBuf]) -> Result<u64, &'static str> {
    let mut walk = IgnoredWalk {
        bytes: 0,
        seen: 0,
        stack: Vec::new(),
    };
    for entry in entries {
        walk.visit(&worktree.join(entry))?;
        loop {
            let next = match walk.stack.last_mut() {
                None => break,
                Some(dir) => dir.next(),
            };
            match next {
                None => {
                    walk.stack.pop();
                }
                Some(Ok(child)) => walk.visit(&child.path())?,
                Some(Err(_)) => return Err("ignored directory unreadable"),
            }
        }
    }
    Ok(walk.bytes)
}

/// Removes the link or reparse point at `path` itself. A Windows directory link or
/// junction is removed with `remove_dir`, which does not touch its target.
fn remove_link(path: &Path, meta: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        if meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return std::fs::remove_dir(path);
        }
    }
    #[cfg(not(windows))]
    let _ = meta;
    std::fs::remove_file(path)
}

/// A git failure while reading or staging the index (for example `unable to stat`
/// when the worktree changes underneath) refuses the attempt; it never fails the run.
fn refuse_index_fault(error: GitError) -> GitError {
    match error {
        GitError::CommandFailed { .. } => GitError::StagingFailed {
            reason: "index read or staging failed",
        },
        other => other,
    }
}

/// Gitlink (mode 160000) entries of `ls-files --stage -z` output, as `(path, object)`.
fn index_gitlinks(listing: &str) -> Vec<(String, String)> {
    listing
        .split('\0')
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let mut parts = meta.split(' ');
            let (mode, object) = (parts.next()?, parts.next()?);
            (mode == "160000").then(|| (path.to_string(), object.to_string()))
        })
        .collect()
}

/// Gitlink (mode 160000) entries of `ls-tree -r -z` output, as `(path, object)`.
fn gitlinks(listing: &str) -> Vec<(String, String)> {
    listing
        .split('\0')
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let mut parts = meta.split(' ');
            let (mode, _kind, object) = (parts.next()?, parts.next()?, parts.next()?);
            (mode == "160000").then(|| (path.to_string(), object.to_string()))
        })
        .collect()
}

fn oid_ok(value: &str) -> bool {
    meshloop_domain::workspace::ObjectId::new(value).is_ok()
}

impl WorkspacePort for GitWorktreeAdapter {
    fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn add_worktree_from(
        &self,
        path: &Path,
        branch: &str,
        start_point: &str,
    ) -> Result<(), WorkspaceError> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        let Some(parent) = parent else {
            return Err(WorkspaceError::Io(format!(
                "layout probe parent is missing; {LAYOUT_HINT}"
            )));
        };
        self.prove_layout(parent)?;
        self.run_admin_with_retry(&[
            "worktree",
            "add",
            "-b",
            branch,
            &path.to_string_lossy(),
            start_point,
        ])
        .map(|_| ())
        .map_err(Into::into)
    }

    fn head(&self, worktree: &Path) -> Result<String, WorkspaceError> {
        self.run_in(worktree, &["rev-parse", "HEAD"])
            .map_err(Into::into)
    }

    fn diff_against(&self, worktree: &Path, base: &str) -> Result<DiffSummary, WorkspaceError> {
        let head = self.head(worktree)?;
        let files_raw = self
            .run_in(
                worktree,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--ignore-submodules=all",
                    "--name-only",
                    base,
                    "HEAD",
                ],
            )
            .or_else(|_| {
                self.run_in(
                    worktree,
                    &[
                        "diff",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--ignore-submodules=all",
                        "--name-only",
                        base,
                    ],
                )
            })
            .map_err(WorkspaceError::from)?;
        let files: Vec<String> = files_raw
            .lines()
            .map(|l| l.trim().replace('\\', "/"))
            .filter(|l| !l.is_empty())
            .collect();
        let stat = self
            .run_in(
                worktree,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--ignore-submodules=all",
                    "--stat",
                    base,
                    "HEAD",
                ],
            )
            .or_else(|_| {
                self.run_in(
                    worktree,
                    &[
                        "diff",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--ignore-submodules=all",
                        "--stat",
                        base,
                    ],
                )
            })
            .unwrap_or_default();
        // New or changed gitlinks are reported separately: the diffs above ignore
        // submodules, and the gate refuses gitlinks (#180).
        let head_links = gitlinks(&self.run_in(worktree, &["ls-tree", "-r", "-z", "HEAD"])?);
        let base_links = self
            .run_in(worktree, &["ls-tree", "-r", "-z", base])
            .map(|listing| gitlinks(&listing))
            .unwrap_or_default();
        let gitlinks = head_links
            .into_iter()
            .filter(|link| !base_links.contains(link))
            .map(|(path, _)| path)
            .collect();
        Ok(DiffSummary {
            base: base.into(),
            head,
            files,
            stat_redacted: crate::redact::redact(&stat),
            gitlinks,
        })
    }

    fn status_porcelain(&self, worktree: &Path) -> Result<String, WorkspaceError> {
        self.run_in(
            worktree,
            &["status", "--porcelain", "--ignore-submodules=all"],
        )
        .map_err(Into::into)
    }

    fn commit_all(
        &self,
        worktree: &Path,
        base: &str,
        message: &str,
    ) -> Result<String, WorkspaceError> {
        // A nested repository would be staged as a gitlink, and later status/diff
        // could recurse into it and run its config (#180). Refuse before staging.
        // The link is checked first, so a renamed top-level `.git` is reported as a
        // modified link rather than a nested entry.
        if !self.is_repo_root(worktree) {
            self.verified_admin(worktree)?;
        }
        match nested_git_entry(worktree) {
            Some((path, NESTED_ENTRY)) => {
                return Err(GitError::NestedGit {
                    path,
                    reason: NESTED_ENTRY,
                }
                .into());
            }
            Some((_, reason)) => return Err(GitError::StagingFailed { reason }.into()),
            None => {}
        }
        let allowed = self.base_gitlinks(worktree, base);
        self.check_index_gitlinks(worktree, &allowed)
            .map_err(refuse_index_fault)?;
        self.stage_all(worktree).map_err(refuse_index_fault)?;
        // Anything that appeared between the walk and `add -A` (a process outside the
        // worker's tree) and was staged as a gitlink is refused, terminally.
        self.check_index_gitlinks(worktree, &allowed)
            .map_err(refuse_index_fault)?;
        let porcelain = self.status_porcelain(worktree)?;
        if porcelain.trim().is_empty() {
            return self.head(worktree);
        }
        match self.run_in(worktree, &["commit", "-q", "-m", message]) {
            Ok(_) => self.head(worktree),
            Err(GitError::CommandFailed { ref stderr })
                if stderr.contains("nothing to commit") || stderr.contains("clean") =>
            {
                self.head(worktree)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn clean_ignored(
        &self,
        worktree: &Path,
        attempt_root: &Path,
    ) -> Result<IgnoredClean, WorkspaceError> {
        // Only a run-loop attempt worktree is cleaned: never the operator's checkout,
        // an integrate or a deliverable worktree. The link is verified next (as for
        // `commit_all`), then any nested `.git` below the top level refuses the clean
        // before git runs, so `clean` never meets a nested repository or a populated
        // gitlink (#180, #196).
        if self.is_repo_root(worktree) || !attempt_worktree(worktree, attempt_root) {
            return Err(WorkspaceError::Io(
                "refused: ignored files are removed only from an attempt worktree".into(),
            ));
        }
        self.verified_admin(worktree)?;
        match nested_git_entry(worktree) {
            Some((path, NESTED_ENTRY)) => {
                return Err(GitError::NestedGit {
                    path,
                    reason: NESTED_ENTRY,
                }
                .into());
            }
            Some((_, reason)) => return Err(GitError::StagingFailed { reason }.into()),
            None => {}
        }
        let list = |adapter: &Self| -> Result<Vec<PathBuf>, WorkspaceError> {
            let output = adapter.output_in(worktree, &LIST_IGNORED)?;
            if !output.status.success() {
                return Err(Self::failed(&LIST_IGNORED, &output).into());
            }
            ignored_entries(&output.stdout).ok_or_else(|| {
                WorkspaceError::Io(
                    "refused: git listed an ignored path outside the worktree".into(),
                )
            })
        };
        let entries = list(self)?;
        if entries.is_empty() {
            return Ok(IgnoredClean::default());
        }
        let bytes = unlink_and_measure(worktree, &entries)
            .map_err(|reason| WorkspaceError::Io(format!("refused: {reason}")))?;
        // `-X`: ignored entries only; `-d`: ignored directories too; a single `-f`,
        // so a nested repository would be skipped, never removed. No `-e`: under `-X`
        // an extra pattern would widen what is removed. Every link among the ignored
        // entries is already gone. The pathspec keeps git at this worktree's top level.
        self.run_in(worktree, &["clean", "-f", "-d", "-X", "-q", "--", "."])
            .map_err(WorkspaceError::from)?;
        let remaining = list(self)?.len() as u64;
        Ok(IgnoredClean {
            entries: (entries.len() as u64).saturating_sub(remaining),
            bytes,
            remaining,
        })
    }

    fn merge_in_worktree(&self, worktree: &Path, from_ref: &str) -> Result<(), WorkspaceError> {
        match self.run_in(worktree, &["merge", "--no-ff", "--no-edit", from_ref]) {
            Ok(_) => Ok(()),
            Err(e @ GitError::LinkModified(_)) => Err(e.into()),
            Err(_) => {
                let _ = self.run_in(worktree, &["merge", "--abort"]);
                Err(WorkspaceError::Conflict)
            }
        }
    }

    fn cherry_pick_range(
        &self,
        worktree: &Path,
        base: &str,
        head: &str,
    ) -> Result<(), WorkspaceError> {
        let range = format!("{base}..{head}");
        let count = self.run_in(worktree, &["rev-list", "--count", &range])?;
        if count.trim() == "0" {
            return Ok(());
        }
        match self.run_in(worktree, &["cherry-pick", &range]) {
            Ok(_) => Ok(()),
            Err(e @ GitError::LinkModified(_)) => Err(e.into()),
            Err(_) => {
                let _ = self.run_in(worktree, &["cherry-pick", "--abort"]);
                Err(WorkspaceError::Conflict)
            }
        }
    }

    fn branch_exists(&self, branch: &str) -> Result<bool, WorkspaceError> {
        match self.run(&["rev-parse", "--verify", branch]) {
            Ok(_) => Ok(true),
            Err(GitError::CommandFailed { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn worktree_exists(&self, path: &Path) -> bool {
        path.is_dir()
            && self
                .run_in(path, &["rev-parse", "--is-inside-work-tree"])
                .is_ok()
    }

    fn prune(&self) -> Result<(), WorkspaceError> {
        self.run_admin_with_retry(&["worktree", "prune"])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, WorkspaceError> {
        match self.run(&["merge-base", "--is-ancestor", ancestor, descendant]) {
            Ok(_) => Ok(true),
            Err(GitError::CommandFailed { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn checkout_ref(&self, git_ref: &str) -> Result<(), WorkspaceError> {
        let dirty = self.status_porcelain(&self.repo_root)?;
        if !dirty.trim().is_empty() {
            return Err(WorkspaceError::Dirty);
        }
        self.run(&["checkout", git_ref])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn merge_ff_only(&self, from: &str) -> Result<(), WorkspaceError> {
        self.run(&["merge", "--ff-only", from])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn merge_no_ff(&self, from: &str) -> Result<(), WorkspaceError> {
        match self.run(&["merge", "--no-ff", "--no-edit", from]) {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = self.run(&["merge", "--abort"]);
                Err(WorkspaceError::Conflict)
            }
        }
    }

    fn current_head(&self) -> Result<String, WorkspaceError> {
        self.head(&self.repo_root)
    }

    fn current_branch(&self) -> Result<Option<String>, WorkspaceError> {
        // `symbolic-ref --quiet` exits 1 on a detached HEAD: that is `None`.
        Ok(self
            .run_in(&self.repo_root, &["symbolic-ref", "--quiet", "HEAD"])
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()))
    }

    fn reset_hard(&self, worktree: &Path, rev: &str) -> Result<(), WorkspaceError> {
        self.run_in(worktree, &["reset", "--hard", rev])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn remove_file(&self, worktree: &Path, rel: &str) -> Result<(), WorkspaceError> {
        let path = worktree.join(rel);
        // `symlink_metadata`: a (dangling) symlink at `rel` is removed, never followed.
        if std::fs::symlink_metadata(&path).is_ok() {
            std::fs::remove_file(&path).map_err(|e| WorkspaceError::Io(e.to_string()))?;
        }
        Ok(())
    }

    fn unified_diff(&self, worktree: &Path, base: &str) -> Result<String, WorkspaceError> {
        self.run_in(
            worktree,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=all",
                "--no-color",
                base,
                "HEAD",
            ],
        )
        .or_else(|_| {
            self.run_in(
                worktree,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--ignore-submodules=all",
                    "--no-color",
                    base,
                ],
            )
        })
        .map_err(Into::into)
    }

    fn remove_worktree(&self, path: &Path) -> Result<(), WorkspaceError> {
        if !path.is_dir() {
            return Ok(());
        }
        // A worktree whose `.git` link was modified is refused, not skipped.
        self.verified_admin(path)?;
        if !self.worktree_exists(path) {
            return Ok(());
        }
        self.run_admin_with_retry(&["worktree", "remove", "--force", &path.to_string_lossy()])
            .map(|_| ())
            .map_err(Into::into)
    }

    fn delete_branch(&self, branch: &str) -> Result<(), WorkspaceError> {
        match self.run(&["branch", "-D", branch]) {
            Ok(_) => Ok(()),
            Err(GitError::CommandFailed { stderr })
                if stderr.contains("not found") || stderr.contains("not exist") =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::TestCommandExt;
    use std::fs;
    use std::process::Command as StdCommand;

    fn disposable_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meshloop-git-test-{}", uuid_ish()));
        fs::create_dir_all(&dir).unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["init", "-q"])
            .guarded_status()
            .unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["config", "user.email", "test@test"])
            .guarded_status()
            .unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["config", "user.name", "test"])
            .guarded_status()
            .unwrap();
        fs::write(dir.join("README.md"), "seed").unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["add", "."])
            .guarded_status()
            .unwrap();
        StdCommand::new("git")
            .current_dir(&dir)
            .args(["commit", "-q", "-m", "seed"])
            .guarded_status()
            .unwrap();
        dir
    }

    fn uuid_ish() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[test]
    fn add_and_remove_worktree_round_trips_on_a_disposable_repo() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let wt_path = repo.parent().unwrap().join(format!("wt-{}", uuid_ish()));

        adapter
            .add_worktree(&wt_path, "meshloop/test")
            .expect("add should succeed");
        assert!(wt_path.join(".git").exists());

        adapter
            .remove_worktree(&wt_path)
            .expect("remove should succeed");
        assert!(!wt_path.exists());

        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn removing_a_nonexistent_worktree_reports_a_git_error_not_a_panic() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let missing = repo.parent().unwrap().join("never-existed");
        assert!(adapter.remove_worktree(&missing).is_err());
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn add_worktree_from_fails_when_path_exists() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let wt_path = repo
            .parent()
            .unwrap()
            .join(format!("wt-exists-{}", uuid_ish()));
        fs::create_dir_all(&wt_path).unwrap();
        fs::write(wt_path.join("blocker.txt"), "nope").unwrap();
        let head = adapter.current_head().unwrap();
        assert!(
            adapter
                .add_worktree_from(&wt_path, "meshloop/exists", &head)
                .is_err()
        );
        fs::remove_dir_all(&wt_path).ok();
        fs::remove_dir_all(&repo).ok();
    }

    /// #180: a worktree whose `.git` link was rewritten (to another gitdir, to a sibling
    /// worktree's admin dir, or replaced by a directory) is refused before git runs.
    #[test]
    fn a_modified_git_link_is_refused_before_git_runs() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let head = adapter.current_head().unwrap();
        let parent = repo.parent().unwrap();
        let wt = parent.join(format!("wt-link-{}", uuid_ish()));
        let sibling = parent.join(format!("wt-sibling-{}", uuid_ish()));
        adapter
            .add_worktree_from(&wt, "meshloop/link", &head)
            .unwrap();
        adapter
            .add_worktree_from(&sibling, "meshloop/sibling", &head)
            .unwrap();
        assert!(adapter.worktree_exists(&wt));
        let original = fs::read_to_string(wt.join(".git")).unwrap();
        let sibling_link = fs::read_to_string(sibling.join(".git")).unwrap();
        let elsewhere = parent.join(format!("elsewhere-{}", uuid_ish()));
        fs::create_dir_all(&elsewhere).unwrap();
        let rewrites = [
            format!("gitdir: {}\n", elsewhere.display()),
            sibling_link,
            "not a link\n".to_string(),
        ];
        for text in rewrites {
            fs::write(wt.join(".git"), &text).unwrap();
            let err = adapter.commit_all(&wt, &head, "x").unwrap_err();
            assert!(
                matches!(&err, WorkspaceError::GitLinkModified { path, .. } if path == &wt.join(".git")),
                "{text}: {err:?}"
            );
            assert!(!adapter.worktree_exists(&wt));
            assert!(matches!(
                WorkspacePort::remove_worktree(&adapter, &wt),
                Err(WorkspaceError::GitLinkModified { .. })
            ));
        }
        fs::remove_file(wt.join(".git")).unwrap();
        fs::create_dir(wt.join(".git")).unwrap();
        assert!(matches!(
            adapter.status_porcelain(&wt),
            Err(WorkspaceError::GitLinkModified { .. })
        ));
        fs::remove_dir(wt.join(".git")).unwrap();
        fs::write(wt.join(".git"), original).unwrap();
        assert!(adapter.worktree_exists(&wt));
        WorkspacePort::remove_worktree(&adapter, &wt).unwrap();
        WorkspacePort::remove_worktree(&adapter, &sibling).unwrap();
        fs::remove_dir_all(&elsewhere).ok();
        fs::remove_dir_all(&repo).ok();
    }

    /// #180: any `.git`-named entry below the top level (any case, or `git~1`) is a
    /// nested repository or link; symlinked directories and junctions are not followed.
    #[test]
    fn nested_git_entries_are_found_without_following_links() {
        let dir = std::env::temp_dir().join(format!("meshloop-nested-{}", uuid_ish()));
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join(".git"), "gitdir: x\n").unwrap();
        assert_eq!(nested_git_entry(&dir), None);
        for name in [".git", ".GIT", "git~1", "GIT~1"] {
            let path = dir.join("a/b").join(name);
            fs::write(&path, "gitdir: elsewhere\n").unwrap();
            assert_eq!(
                nested_git_entry(&dir),
                Some((PathBuf::from("a").join("b").join(name), NESTED_ENTRY))
            );
            fs::remove_file(&path).unwrap();
        }
        // On NTFS a top-level `GIT~1` is the 8.3 short name of `.git` itself: writing it
        // rewrites the link, which the worktree-link check refuses instead. Only on
        // filesystems without short names is it a separate top-level entry.
        #[cfg(not(windows))]
        {
            fs::write(dir.join("GIT~1"), "x").unwrap();
            assert_eq!(
                nested_git_entry(&dir),
                Some((PathBuf::from("GIT~1"), NESTED_ENTRY))
            );
            fs::remove_file(dir.join("GIT~1")).unwrap();
        }
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("meshloop-outside-{}", uuid_ish()));
            fs::create_dir_all(outside.join(".git")).unwrap();
            std::os::unix::fs::symlink(&outside, dir.join("a/link")).unwrap();
            assert_eq!(nested_git_entry(&dir), None);
            fs::remove_dir_all(&outside).unwrap();
        }
        #[cfg(windows)]
        {
            // A junction to a directory holding `.git` is not followed.
            let outside = std::env::temp_dir().join(format!("meshloop-outside-{}", uuid_ish()));
            fs::create_dir_all(outside.join(".git")).unwrap();
            let junction = dir.join("a").join("junction");
            let made = StdCommand::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(&junction)
                .arg(&outside)
                .guarded_output()
                .unwrap();
            assert!(made.status.success(), "{made:?}");
            assert_eq!(nested_git_entry(&dir), None);
            fs::remove_dir(&junction).unwrap();
            fs::remove_dir_all(&outside).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// #180: a gitlink staged by the worker that is not in the attempt base, or whose
    /// directory holds a `.git`, is refused; a top-level `.git` rename is a link change.
    #[test]
    fn worker_gitlinks_and_a_renamed_link_are_refused() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let head = adapter.current_head().unwrap();
        let wt = repo
            .parent()
            .unwrap()
            .join(format!("wt-gitlink-{}", uuid_ish()));
        adapter
            .add_worktree_from(&wt, "meshloop/gitlink", &head)
            .unwrap();
        let staged = StdCommand::new("git")
            .current_dir(&wt)
            .args(["update-index", "--add", "--cacheinfo"])
            .arg(format!("160000,{head},sub"))
            .guarded_output()
            .unwrap();
        assert!(staged.status.success(), "{staged:?}");
        assert!(matches!(
            adapter.commit_all(&wt, &head, "x"),
            Err(WorkspaceError::NestedGit { path, reason })
                if path == Path::new("sub") && reason == "gitlink not in the attempt base"
        ));
        fs::rename(wt.join(".git"), wt.join(".GIT")).unwrap();
        let renamed = adapter.commit_all(&wt, &head, "x");
        if fs::symlink_metadata(wt.join(".git")).is_err() {
            // Case-sensitive filesystem: the link itself is gone.
            assert!(
                matches!(renamed, Err(WorkspaceError::GitLinkModified { .. })),
                "{renamed:?}"
            );
        }
        fs::rename(wt.join(".GIT"), wt.join(".git")).unwrap();
        WorkspacePort::remove_worktree(&adapter, &wt).unwrap();
        fs::remove_dir_all(&repo).ok();
    }

    /// #180 round 4: the base has a submodule gitlink and its directory holds a
    /// populated repository at the recorded commit (as a process outside the worker's
    /// tree could arrange after the checks). Staging must not look inside it, so the
    /// nested repository's own filter never runs.
    #[cfg(unix)]
    #[test]
    fn staging_never_looks_inside_a_base_gitlink() {
        use std::os::unix::fs::PermissionsExt;
        let repo = disposable_repo();
        let parent = repo.parent().unwrap().to_path_buf();
        let tag = uuid_ish();
        let git = |dir: &Path, args: &[&str]| {
            let out = StdCommand::new("git")
                .current_dir(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .guarded_output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let inner = parent.join(format!("inner-{tag}"));
        fs::create_dir_all(&inner).unwrap();
        git(&inner, &["init", "-q"]);
        fs::write(inner.join("f"), "one\n").unwrap();
        git(&inner, &["add", "f"]);
        git(&inner, &["commit", "-q", "-m", "inner"]);
        let inner_head = git(&inner, &["rev-parse", "HEAD"]);
        git(
            &repo,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{inner_head},sub"),
            ],
        );
        git(&repo, &["commit", "-q", "-m", "gitlink"]);
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let base = adapter.current_head().unwrap();
        let wt = parent.join(format!("wt-sub-{tag}"));
        adapter
            .add_worktree_from(&wt, "meshloop/sub", &base)
            .unwrap();
        // Populated at the recorded commit, with a filter its own config defines.
        fs::remove_dir_all(wt.join("sub")).ok();
        git(&wt, &["clone", "-q", inner.to_str().unwrap(), "sub"]);
        let marker = parent.join(format!("marker-{tag}"));
        let trigger = parent.join(format!("trigger-{tag}.sh"));
        fs::write(
            &trigger,
            format!("#!/bin/sh\n: > '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&trigger, fs::Permissions::from_mode(0o755)).unwrap();
        let sub = wt.join("sub");
        git(
            &sub,
            &["config", "filter.marker.clean", trigger.to_str().unwrap()],
        );
        fs::write(sub.join(".gitattributes"), "* filter=marker\n").unwrap();
        fs::write(sub.join("f"), "two\n").unwrap();
        fs::write(wt.join("work.txt"), "work\n").unwrap();

        adapter.stage_all(&wt).unwrap();
        assert!(
            !marker.exists(),
            "staging ran the nested repository's filter"
        );
        let staged = adapter
            .run_in(&wt, &["diff", "--cached", "--name-only"])
            .unwrap();
        assert!(staged.lines().any(|l| l == "work.txt"), "{staged}");

        fs::remove_dir_all(&sub).unwrap();
        WorkspacePort::remove_worktree(&adapter, &wt).unwrap();
        for path in [&inner, &repo] {
            fs::remove_dir_all(path).ok();
        }
        fs::remove_file(&trigger).ok();
        fs::remove_file(&marker).ok();
    }

    #[test]
    fn commit_all_creates_a_meshloop_owned_commit() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let wt_path = repo
            .parent()
            .unwrap()
            .join(format!("wt-commit-{}", uuid_ish()));
        let head = adapter.current_head().unwrap();
        adapter
            .add_worktree_from(&wt_path, "meshloop/commit-test", &head)
            .unwrap();
        fs::write(wt_path.join("new.txt"), "hello").unwrap();
        let new_head = adapter
            .commit_all(&wt_path, &head, "meshloop: attempt")
            .unwrap();
        assert_ne!(new_head, head);
        adapter.remove_worktree(&wt_path).ok();
        fs::remove_dir_all(&repo).ok();
    }

    /// An attempt worktree (`<root>/g/task-1-attempt-1`) of a fresh repository whose
    /// `.gitignore` ignores `target/`, `*.log` and `link`, committed on the base.
    /// Returns `(repo, worktree, adapter, base, attempt root)`.
    fn ignoring_worktree(tag: &str) -> (PathBuf, PathBuf, GitWorktreeAdapter, String, PathBuf) {
        let repo = disposable_repo();
        fs::write(repo.join(".gitignore"), "target/\n*.log\nlink\n").unwrap();
        for args in [
            vec!["add", ".gitignore"],
            vec!["commit", "-q", "-m", "ignore"],
        ] {
            StdCommand::new("git")
                .current_dir(&repo)
                .args(&args)
                .guarded_status()
                .unwrap();
        }
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let head = adapter.current_head().unwrap();
        let root = repo
            .parent()
            .unwrap()
            .join(format!("root-clean-{tag}-{}", uuid_ish()));
        fs::create_dir_all(root.join("g")).unwrap();
        let wt = root.join("g").join("task-1-attempt-1");
        adapter
            .add_worktree_from(&wt, &format!("meshloop/clean-{tag}"), &head)
            .unwrap();
        (repo, wt, adapter, head, root)
    }

    /// #196: only ignored entries go; tracked files (even ones an ignore rule matches),
    /// untracked-but-not-ignored files and everything behind a link stay. Counts and
    /// sizes are reported, and a second clean finds nothing.
    #[test]
    fn clean_ignored_removes_only_ignored_entries_and_never_follows_links() {
        let (repo, wt, adapter, head, root) = ignoring_worktree("only");
        // A tracked file an ignore rule matches (force-added): never removed.
        fs::write(wt.join("kept.log"), "tracked log").unwrap();
        StdCommand::new("git")
            .current_dir(&wt)
            .args(["add", "-f", "kept.log"])
            .guarded_status()
            .unwrap();
        adapter.commit_all(&wt, &head, "tracked log").unwrap();
        fs::create_dir_all(wt.join("target/debug")).unwrap();
        fs::write(wt.join("target/debug/out.bin"), vec![0u8; 1000]).unwrap();
        fs::write(wt.join("build.log"), "12345").unwrap();
        fs::write(wt.join("notes.txt"), "untracked, not ignored").unwrap();
        // Outside data reachable only through links inside ignored entries.
        let outside = repo
            .parent()
            .unwrap()
            .join(format!("outside-{}", uuid_ish()));
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("precious.txt"), "operator bytes").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, wt.join("target/escape")).unwrap();
            std::os::unix::fs::symlink(&outside, wt.join("link")).unwrap();
        }
        #[cfg(windows)]
        {
            // A junction is a directory reparse point: removed, never followed.
            // Native separators: `mklink` reads a `/` inside the path as a switch.
            for junction in [wt.join("target").join("escape"), wt.join("link")] {
                let made = StdCommand::new("cmd")
                    .arg("/C")
                    .arg("mklink")
                    .arg("/J")
                    .arg(&junction)
                    .arg(&outside)
                    .guarded_output()
                    .unwrap();
                assert!(made.status.success(), "{made:?}");
            }
        }

        let report = adapter.clean_ignored(&wt, &root).unwrap();
        assert_eq!(report.remaining, 0, "{report:?}");
        assert!(report.entries >= 2, "{report:?}");
        assert!(report.bytes >= 1005, "{report:?}");
        assert!(!wt.join("target").exists());
        assert!(!wt.join("build.log").exists());
        assert!(fs::symlink_metadata(wt.join("link")).is_err());
        assert_eq!(
            fs::read_to_string(wt.join("kept.log")).unwrap(),
            "tracked log"
        );
        assert_eq!(
            fs::read_to_string(wt.join("notes.txt")).unwrap(),
            "untracked, not ignored"
        );
        assert_eq!(fs::read_to_string(wt.join("README.md")).unwrap(), "seed");
        assert_eq!(
            fs::read_to_string(outside.join("precious.txt")).unwrap(),
            "operator bytes"
        );
        assert_eq!(
            adapter.clean_ignored(&wt, &root).unwrap(),
            IgnoredClean::default()
        );

        WorkspacePort::remove_worktree(&adapter, &wt).unwrap();
        fs::remove_dir_all(&outside).ok();
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&repo).ok();
    }

    /// #196: the operator's checkout is never cleaned, nor a worktree whose `.git`
    /// link was modified, nor one holding a nested repository: nothing is removed.
    #[test]
    fn clean_ignored_refuses_the_checkout_a_modified_link_and_a_nested_repository() {
        let (repo, wt, adapter, head, root) = ignoring_worktree("refuse");
        fs::create_dir_all(repo.join("target")).unwrap();
        fs::write(repo.join("target/operator-cache"), "cache").unwrap();
        assert!(adapter.clean_ignored(&repo, &root).is_err());
        assert!(repo.join("target/operator-cache").is_file());
        // Integrate and deliverable worktrees of the same run, and an attempt worktree
        // under another root, are not attempt worktrees of this root.
        for (leaf, branch) in [
            ("integrate", "meshloop/g/integrate"),
            ("integrate-v1", "meshloop/g/deliver"),
        ] {
            let other = root.join("g").join(leaf);
            adapter.add_worktree_from(&other, branch, &head).unwrap();
            fs::create_dir_all(other.join("target")).unwrap();
            fs::write(other.join("target/out.bin"), "built").unwrap();
            assert!(adapter.clean_ignored(&other, &root).is_err(), "{leaf}");
            assert!(other.join("target/out.bin").is_file(), "{leaf}");
            WorkspacePort::remove_worktree(&adapter, &other).ok();
            fs::remove_dir_all(&other).ok();
        }
        assert!(adapter.clean_ignored(&wt, &root.join("g")).is_err());
        assert!(adapter.clean_ignored(&wt, &repo).is_err());

        fs::create_dir_all(wt.join("target/inner/.git")).unwrap();
        fs::write(wt.join("target/inner/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(wt.join("target/out.bin"), "built").unwrap();
        let err = adapter.clean_ignored(&wt, &root).unwrap_err();
        assert!(matches!(err, WorkspaceError::NestedGit { .. }), "{err:?}");
        assert!(wt.join("target/out.bin").is_file());
        fs::remove_dir_all(wt.join("target/inner")).unwrap();

        let original = fs::read_to_string(wt.join(".git")).unwrap();
        fs::write(wt.join(".git"), "not a link\n").unwrap();
        let err = adapter.clean_ignored(&wt, &root).unwrap_err();
        assert!(
            matches!(err, WorkspaceError::GitLinkModified { .. }),
            "{err:?}"
        );
        assert!(wt.join("target/out.bin").is_file());
        fs::write(wt.join(".git"), original).unwrap();

        assert_eq!(adapter.clean_ignored(&wt, &root).unwrap().remaining, 0);
        assert!(!wt.join("target").exists());
        WorkspacePort::remove_worktree(&adapter, &wt).unwrap();
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&repo).ok();
    }

    /// Review of #196: a base gitlink's directory is empty after `worktree add`. Content
    /// written there is invisible to status, `add` and `clean`, so it would reach the
    /// check unreviewed; it refuses the attempt before staging, as does a file in its
    /// place.
    #[test]
    fn content_in_an_unpopulated_gitlink_is_refused() {
        let repo = disposable_repo();
        let adapter = GitWorktreeAdapter::new(repo.clone());
        let seed = adapter.current_head().unwrap();
        for args in [
            vec![
                "update-index".to_string(),
                "--add".into(),
                "--cacheinfo".into(),
                format!("160000,{seed},sub"),
            ],
            vec!["commit".into(), "-q".into(), "-m".into(), "gitlink".into()],
        ] {
            let out = StdCommand::new("git")
                .current_dir(&repo)
                .args(&args)
                .guarded_output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        }
        let base = adapter.current_head().unwrap();
        let wt = repo
            .parent()
            .unwrap()
            .join(format!("wt-unpopulated-{}", uuid_ish()));
        adapter
            .add_worktree_from(&wt, "meshloop/unpopulated", &base)
            .unwrap();
        assert!(wt.join("sub").is_dir());
        fs::write(wt.join("work.txt"), "work\n").unwrap();
        // An empty gitlink directory, as `worktree add` leaves it, is fine.
        let first = adapter.commit_all(&wt, &base, "work").unwrap();
        assert_ne!(first, base);

        fs::write(wt.join("sub").join("payload.sh"), "exit 0\n").unwrap();
        let refused = adapter.commit_all(&wt, &base, "payload");
        assert!(
            matches!(&refused, Err(WorkspaceError::NestedGit { path, reason })
                if path == Path::new("sub") && reason == UNPOPULATED_CONTENT),
            "{refused:?}"
        );
        fs::remove_dir_all(wt.join("sub")).unwrap();
        fs::write(wt.join("sub"), "a file in its place\n").unwrap();
        assert!(matches!(
            adapter.commit_all(&wt, &base, "file"),
            Err(WorkspaceError::NestedGit { .. })
        ));
        fs::remove_file(wt.join("sub")).unwrap();
        assert!(adapter.commit_all(&wt, &base, "absent").is_ok());
        WorkspacePort::remove_worktree(&adapter, &wt).unwrap();
        fs::remove_dir_all(&repo).ok();
    }

    /// Review of #196: every link among the ignored entries is removed as a link before
    /// `git clean` runs, at any depth; the bytes count it as itself.
    #[cfg(unix)]
    #[test]
    fn links_among_ignored_entries_are_removed_without_being_followed() {
        let dir = std::env::temp_dir().join(format!("meshloop-unlink-{}", uuid_ish()));
        let outside = std::env::temp_dir().join(format!("meshloop-unlink-out-{}", uuid_ish()));
        fs::create_dir_all(dir.join("target/a/b")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("precious.txt"), "operator bytes").unwrap();
        fs::write(dir.join("target/a/b/out.bin"), vec![0u8; 100]).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("target/a/b/deep")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("top")).unwrap();
        let bytes =
            unlink_and_measure(&dir, &[PathBuf::from("target"), PathBuf::from("top")]).unwrap();
        assert!(bytes >= 100, "{bytes}");
        assert!(fs::symlink_metadata(dir.join("target/a/b/deep")).is_err());
        assert!(fs::symlink_metadata(dir.join("top")).is_err());
        assert!(dir.join("target/a/b/out.bin").is_file());
        assert_eq!(
            fs::read_to_string(outside.join("precious.txt")).unwrap(),
            "operator bytes"
        );
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn ignored_listing_parses_nul_records_and_refuses_unsafe_paths() {
        assert_eq!(
            ignored_entries(b"target/\0a b.log\0").unwrap(),
            vec![PathBuf::from("target"), PathBuf::from("a b.log")]
        );
        assert!(ignored_entries(b"").unwrap().is_empty());
        assert!(ignored_entries(b"../up\0").is_none());
        assert!(ignored_entries(b"/abs\0").is_none());
        assert!(ignored_entries(b"a/../../b\0").is_none());
    }
}
