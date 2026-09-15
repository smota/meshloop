//! Git-diff verification helpers. Harness exit code is never the gate.

use meshloop_domain::diagnostic::{DiagnosticLattice, parse_diagnostics};
use meshloop_domain::evidence::{Evidence, RequiredEvidence, all_deterministic_passed, satisfies};

use crate::ports::DiffSummary;

/// Prefix a check's redacted output with a lattice fingerprint so the event log
/// carries a deterministic diagnostic identity without storing raw stderr.
pub fn annotate_with_lattice(output_redacted: &str) -> (DiagnosticLattice, String) {
    let lattice = parse_diagnostics(output_redacted);
    let annotated = format!(
        "lattice={:016x} syntax={} type={} test={} blocking={}\n{output_redacted}",
        lattice.fingerprint(),
        lattice.progress().syntax,
        lattice.progress().type_errors,
        lattice.progress().tests,
        lattice.blocking(),
    );
    (lattice, annotated)
}

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

    #[test]
    fn lattice_annotation_is_stable_across_worktree_paths() {
        let a = "error[E0308]: mismatched types\n --> src/main.rs:2:5\n";
        let b = "error[E0308]: mismatched types\n --> C:\\wt\\task-1\\src\\main.rs:99:1\n";
        let (la, sa) = annotate_with_lattice(a);
        let (lb, sb) = annotate_with_lattice(b);
        assert_eq!(la.fingerprint(), lb.fingerprint());
        assert!(sa.starts_with("lattice="));
        assert!(sb.starts_with("lattice="));
        assert_eq!(&sa[..23], &sb[..23]);
    }
}
