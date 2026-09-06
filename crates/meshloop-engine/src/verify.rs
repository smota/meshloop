//! Git-diff verification helpers. Harness exit code is never the gate.

use meshloop_domain::evidence::{Evidence, RequiredEvidence, all_deterministic_passed, satisfies};

use crate::ports::DiffSummary;

pub fn git_diff_exit_code(diff: &DiffSummary, empty_diff_ok: bool) -> i32 {
    if diff.files.is_empty() && !empty_diff_ok {
        1
    } else {
        0
    }
}

/// Paths from `git diff --name-only`, `/` separators. A rule ending in `/` is a prefix.
pub fn path_allowed(changed: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let changed = changed.replace('\\', "/");
    allowed.iter().any(|rule| {
        let rule = rule.replace('\\', "/");
        if rule.ends_with('/') {
            changed.starts_with(&rule)
        } else {
            changed == rule
        }
    })
}

pub fn all_paths_allowed(files: &[String], allowed: &[String]) -> bool {
    allowed.is_empty() || files.iter().all(|f| path_allowed(f, allowed))
}

pub fn verification_passed(rows: &[Evidence]) -> bool {
    all_deterministic_passed(rows) && satisfies(RequiredEvidence::DeterministicOnly, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diff_fails_unless_flagged() {
        let diff = DiffSummary {
            base: "a".into(),
            head: "a".into(),
            files: vec![],
            stat_redacted: String::new(),
        };
        assert_eq!(git_diff_exit_code(&diff, false), 1);
        assert_eq!(git_diff_exit_code(&diff, true), 0);
    }

    #[test]
    fn allowed_paths_prefix_and_exact() {
        assert!(path_allowed("src/lib.rs", &["src/".into()]));
        assert!(!path_allowed("README.md", &["src/".into()]));
        assert!(path_allowed("README.md", &["README.md".into()]));
    }
}
