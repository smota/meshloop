//! Hardened host-side git (issue #180). Every git process Meshloop starts is built
//! here, so the worker's output stays data (AGENTS.md): host-side git never runs a
//! program the worker chose, and never follows a worktree `.git` link the worker
//! rewrote.
//!
//! What is neutralised, and why, is documented in
//! `docs/engineering/harnesses.md#host-side-git`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::process::OwnedSpawnSpec;

/// Inherited variables that would point git at another repository, index or config,
/// or name a program to run. The operator's environment is not the threat, but these
/// would bypass the verified worktree link and the overrides below.
const REMOVED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_SYSTEM",
    "GIT_ATTR_SOURCE",
    "GIT_EXTERNAL_DIFF",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_PROXY_COMMAND",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_PAGER",
];

/// The null device for config files. Git for Windows maps `/dev/null` to `NUL`.
const NULL_CONFIG: &str = "/dev/null";

/// Executable keys and program-running behaviour that repository config could
/// enable, overridden on the command line (the highest-precedence scope).
const OVERRIDES: &[&str] = &[
    // Hooks: fsmonitor and every hook (`core.hooksPath` is added separately).
    "core.fsmonitor=false",
    // Editors: never opened (`:` is git's no-op editor). Meshloop passes messages.
    "core.editor=:",
    "sequence.editor=:",
    // Signing and signature checks run `gpg.program` / `gpg.<format>.program`.
    "commit.gpgSign=false",
    "tag.gpgSign=false",
    "merge.verifySignatures=false",
    "log.showSignature=false",
    // Transports run ssh, proxies, `uploadpack`, credential helpers and askpass.
    // Meshloop never fetches or pushes; lazy fetch in a partial clone is refused.
    "protocol.allow=never",
    "credential.helper=",
    // No submodule recursion; no background gc/maintenance (and its hooks).
    "submodule.recurse=false",
    "gc.auto=0",
    "maintenance.auto=false",
];

/// Data-only keys imported from the operator's system and global config, which the
/// hardened environment otherwise drops. None of them names a program.
const IMPORTED_KEYS: &str =
    r"^(core\.(autocrlf|eol|safecrlf|longpaths|symlinks|fscache)|safe\.directory)$";

/// `core.hooksPath` for hooks that cannot exist: a path below a non-directory.
fn null_hooks_path() -> OsString {
    #[cfg(windows)]
    {
        // `<meshloop.exe>\<hook>` cannot exist while the executable is a regular file.
        // `NUL\<hook>` is avoided: Win32 may read it as a relative directory.
        if let Ok(exe) = std::env::current_exe() {
            let text = exe.to_string_lossy().into_owned();
            let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
            return OsString::from(text);
        }
        OsString::from(r"C:\Windows\System32\NUL")
    }
    #[cfg(not(windows))]
    {
        OsString::from("/dev/null")
    }
}

/// The `-c` arguments every host-side git gets, computed once per process.
fn config_args() -> &'static [OsString] {
    static ARGS: OnceLock<Vec<OsString>> = OnceLock::new();
    ARGS.get_or_init(|| {
        let mut args = Vec::new();
        let mut hooks = OsString::from("core.hooksPath=");
        hooks.push(null_hooks_path());
        args.push(OsString::from("-c"));
        args.push(hooks);
        for item in OVERRIDES {
            args.push(OsString::from("-c"));
            args.push(OsString::from(item));
        }
        for (key, value) in operator_settings() {
            args.push(OsString::from("-c"));
            args.push(OsString::from(format!("{key}={value}")));
        }
        args
    })
}

/// Reads the allowlisted data-only keys from the operator's system, then global,
/// config (later wins, as in git). `git config --get-regexp` runs no program. A read
/// failure imports nothing: the hardened defaults then apply, which is the safe side.
fn operator_settings() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for scope in ["--system", "--global"] {
        let mut cmd = std::process::Command::new("git");
        for key in REMOVED_ENV {
            cmd.env_remove(key);
        }
        cmd.current_dir(std::env::temp_dir())
            .args(["config", scope, "--includes", "--null", "--get-regexp"])
            .arg(IMPORTED_KEYS)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        crate::process::hide_console(&mut cmd);
        let Ok(output) = crate::process::spawn_command(&mut cmd).and_then(|c| c.wait_with_output())
        else {
            continue;
        };
        // Exit 1 is "no such key"; anything else unreadable is skipped.
        if !output.status.success() {
            continue;
        }
        found.extend(parse_settings(&output.stdout));
    }
    found
}

/// `--null --get-regexp` output: `key\nvalue\0` records. Values are kept only when
/// they are one plain line; core values must also be short words.
fn parse_settings(bytes: &[u8]) -> Vec<(String, String)> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    text.split('\0')
        .filter_map(|record| {
            let (key, value) = record.split_once('\n')?;
            let key = key.to_ascii_lowercase();
            if value.is_empty() || value.contains(['\n', '\r', '\0']) {
                return None;
            }
            let plain_word = value.len() <= 32
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            let allowed = match key.as_str() {
                "safe.directory" => true,
                "core.autocrlf" | "core.eol" | "core.safecrlf" | "core.longpaths"
                | "core.symlinks" | "core.fscache" => plain_word,
                _ => false,
            };
            allowed.then(|| (key, value.to_string()))
        })
        .collect()
}

/// The hardened environment: (key, Some(value)) sets, (key, None) removes.
fn environment() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env: Vec<(&'static str, Option<&'static str>)> =
        REMOVED_ENV.iter().map(|key| (*key, None)).collect();
    env.extend([
        ("GIT_CONFIG_NOSYSTEM", Some("1")),
        ("GIT_CONFIG_GLOBAL", Some(NULL_CONFIG)),
        ("GIT_ATTR_NOSYSTEM", Some("1")),
        ("GIT_TERMINAL_PROMPT", Some("0")),
        ("GIT_NO_LAZY_FETCH", Some("1")),
        ("GIT_EDITOR", Some(":")),
        ("GIT_SEQUENCE_EDITOR", Some(":")),
    ]);
    env
}

/// A hardened `std::process::Command` for `git`. Callers append the subcommand.
pub fn command(git: impl AsRef<OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(git);
    for (key, value) in environment() {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
    cmd.arg("--no-pager").args(config_args());
    cmd
}

/// A hardened [`OwnedSpawnSpec`] for `git`. Callers append the subcommand.
pub fn spec(git: impl AsRef<OsStr>) -> OwnedSpawnSpec {
    let mut spec = OwnedSpawnSpec::new(git);
    for (key, value) in environment() {
        match value {
            Some(value) => spec.env(key, value),
            None => spec.env_remove(key),
        };
    }
    spec.arg("--no-pager").args(config_args());
    spec
}

/// Global options that pin git to a verified worktree: the admin dir from
/// [`verify_worktree_link`] and the working directory (`.`, the command's cwd), so
/// git never reads the worktree's `.git` link itself.
pub fn pinned(admin: &Path) -> [OsString; 2] {
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(admin.as_os_str());
    [git_dir, OsString::from("--work-tree=.")]
}

/// A worktree whose `.git` link no longer points at its own admin dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLinkError {
    /// The worktree's `.git` path.
    pub link: PathBuf,
    pub reason: &'static str,
}

impl std::fmt::Display for GitLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refusing to run git: {} was modified ({})",
            self.link.display(),
            self.reason
        )
    }
}

const MAX_LINK_BYTES: u64 = 4096;

/// Checks, without running git, that `worktree/.git` is a regular file naming an
/// admin dir directly under `<common>/worktrees/`, whose `gitdir` file points back at
/// this `.git` and whose `commondir` is `common`. Returns the admin dir as written in
/// the link (absolute), for [`pinned`]. Never follows anything the checks reject.
pub fn verify_worktree_link(worktree: &Path, common: &Path) -> Result<PathBuf, GitLinkError> {
    let link = worktree.join(".git");
    let refuse = |reason| GitLinkError {
        link: link.clone(),
        reason,
    };
    let meta = std::fs::symlink_metadata(&link).map_err(|_| refuse("missing"))?;
    if !meta.file_type().is_file() {
        return Err(refuse("not a regular file"));
    }
    if meta.len() > MAX_LINK_BYTES {
        return Err(refuse("too large"));
    }
    let text = std::fs::read_to_string(&link).map_err(|_| refuse("unreadable"))?;
    let target = text
        .strip_prefix("gitdir: ")
        .map(|rest| rest.trim_end_matches(['\r', '\n']))
        .filter(|rest| !rest.is_empty() && !rest.contains(['\r', '\n']))
        .ok_or_else(|| refuse("not a gitdir link"))?;
    let admin = absolute_from(worktree, Path::new(target));
    let canonical = |path: &Path, reason| std::fs::canonicalize(path).map_err(|_| refuse(reason));
    let admin_canon = canonical(&admin, "admin dir missing")?;
    let common_canon = canonical(common, "repository common dir missing")?;
    if admin_canon.parent() != Some(common_canon.join("worktrees").as_path()) {
        return Err(refuse("points outside the repository's worktrees"));
    }
    let back = read_small(&admin_canon.join("gitdir")).ok_or_else(|| refuse("no back link"))?;
    let back = absolute_from(&admin_canon, Path::new(&back));
    if canonical(&back, "back link missing")? != canonical(&link, "link missing")? {
        return Err(refuse("admin dir belongs to another worktree"));
    }
    let common_file =
        read_small(&admin_canon.join("commondir")).ok_or_else(|| refuse("no commondir"))?;
    let named_common = absolute_from(&admin_canon, Path::new(&common_file));
    if canonical(&named_common, "commondir missing")? != common_canon {
        return Err(refuse("admin dir names another repository"));
    }
    Ok(admin)
}

fn absolute_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// One line from a small regular file (git's admin `gitdir` / `commondir`).
fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_LINK_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.trim_end_matches(['\r', '\n']);
    (!line.is_empty() && !line.contains(['\r', '\n'])).then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_settings_are_allowlisted_data_only() {
        let raw = b"core.autocrlf\ntrue\0core.fsmonitor\n/bin/evil\0safe.directory\n/srv/repo one\0core.eol\nlf; touch x\0filter.lfs.clean\ngit-lfs clean\0";
        assert_eq!(
            parse_settings(raw),
            vec![
                ("core.autocrlf".to_string(), "true".to_string()),
                ("safe.directory".to_string(), "/srv/repo one".to_string()),
            ]
        );
    }

    #[test]
    fn every_command_carries_the_overrides_and_environment() {
        let cmd = command("git");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "--no-pager");
        for item in OVERRIDES {
            assert!(args.iter().any(|a| a == item), "{item} missing in {args:?}");
        }
        assert!(args.iter().any(|a| a.starts_with("core.hooksPath=")));
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("GIT_CONFIG_NOSYSTEM".into(), Some("1".into()))));
        assert!(envs.contains(&("GIT_CONFIG_GLOBAL".into(), Some(NULL_CONFIG.into()))));
        assert!(envs.contains(&("GIT_DIR".into(), None)));
    }
}
