//! Keeps the local `.meshloop/` store out of consuming repositories' Git status.
//! Probes use `git check-ignore` with structured arguments; nothing here edits a root
//! `.gitignore` except the explicit, opt-in `bundle --gitignore` block.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const STORE_DIR: &str = ".meshloop";
pub const STORE_PROBE: &str = ".meshloop/state.sqlite";
pub const IGNORE_LINE: &str = "/.meshloop/";
const BLOCK_BEGIN: &str = "# >>> meshloop >>>";
const BLOCK_END: &str = "# <<< meshloop <<<";

/// Result of asking Git whether a path is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    Yes,
    No,
    /// Not inside a Git work tree (or Git is unavailable): nothing to flag.
    NotApplicable,
}

fn git_ok(cwd: &Path, args: &[&str]) -> Option<(bool, i32)> {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    Some((status.success(), status.code().unwrap_or(-1)))
}

/// `git check-ignore -q <rel>` run in `cwd`: exit 0 ignored, 1 not ignored, else n/a.
pub fn check_ignored(cwd: &Path, rel: &str) -> Ignored {
    match git_ok(cwd, &["check-ignore", "-q", "--", rel]) {
        Some((true, _)) => Ignored::Yes,
        Some((false, 1)) => Ignored::No,
        _ => Ignored::NotApplicable,
    }
}

fn in_work_tree(dir: &Path) -> bool {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    matches!(out, Ok(o) if o.status.success() && o.stdout.starts_with(b"true"))
}

/// Ignore status of a store file. Paths outside any work tree are `NotApplicable`.
pub fn store_status(db: &Path) -> Ignored {
    let Ok(db) = std::path::absolute(db) else {
        return Ignored::NotApplicable;
    };
    let db = db.as_path();
    let Some(parent) = db.parent() else {
        return Ignored::NotApplicable;
    };
    let Some(base) = parent.ancestors().find(|a| a.is_dir()) else {
        return Ignored::NotApplicable;
    };
    if !in_work_tree(base) {
        return Ignored::NotApplicable;
    }
    match db.strip_prefix(base) {
        Ok(rel) if !rel.as_os_str().is_empty() => check_ignored(base, &rel.to_string_lossy()),
        _ => Ignored::NotApplicable,
    }
}

/// `store_ignored` for `doctor`: true when ignored or not applicable.
pub fn store_ignored(db: &Path) -> bool {
    store_status(db) != Ignored::No
}

/// After the store is created: if it sits in `.meshloop/` inside a work tree and is not
/// ignored, write `.meshloop/.gitignore` (`*`) and notify once on stderr. Never touches
/// the root `.gitignore`.
pub fn ensure_store_ignored(db: &Path) {
    let Some(dir) = db.parent() else { return };
    if dir.file_name().is_none_or(|n| n != STORE_DIR) || store_status(db) != Ignored::No {
        return;
    }
    let marker = dir.join(".gitignore");
    if marker.exists() {
        return;
    }
    match fs::write(&marker, "*\n") {
        Ok(()) => eprintln!(
            "meshloop: wrote {} so the local store is not tracked by Git (one-time notice)",
            marker.display()
        ),
        Err(e) => eprintln!("warning: could not write {}: {e}", marker.display()),
    }
}

/// Outcome of the `bundle` ignore check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleIgnore {
    Present,
    Missing,
    Added,
    NotApplicable,
}

impl BundleIgnore {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Added => "added",
            Self::NotApplicable => "not-applicable",
        }
    }
}

fn marked_block() -> String {
    format!("{BLOCK_BEGIN}\n{IGNORE_LINE}\n{BLOCK_END}\n")
}

/// Checks `repo` and, only when `apply` is set, appends the marked block (idempotent).
pub fn bundle_check(repo: &Path, apply: bool) -> Result<BundleIgnore, String> {
    match check_ignored(repo, STORE_PROBE) {
        Ignored::Yes => return Ok(BundleIgnore::Present),
        Ignored::NotApplicable => return Ok(BundleIgnore::NotApplicable),
        Ignored::No => {}
    }
    if !apply {
        return Ok(BundleIgnore::Missing);
    }
    let path: PathBuf = repo.join(".gitignore");
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    if existing.contains(BLOCK_BEGIN) {
        return Ok(BundleIgnore::Present);
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&marked_block());
    fs::write(&path, next).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(BundleIgnore::Added)
}
