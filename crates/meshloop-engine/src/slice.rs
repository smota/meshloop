//! Syntactic impact slicing: compare public signatures before a full compile.
//!
//! If a round only rewrote function bodies, the skeleton hash is unchanged and
//! dependents do not need a full type-check. Signature changes return the files
//! whose public surface moved — those, plus diagnostic-ranked neighbors, are the
//! slice the inner loop should re-check.

use std::collections::{BTreeMap, BTreeSet};

use meshloop_context::quant::{SignatureIndex, signature_fingerprint};
use meshloop_context::{BitWidth, Language, extract_skeleton};
use meshloop_domain::diagnostic::DiagnosticLattice;
use meshloop_domain::digest::fnv1a64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureSnapshot {
    /// path → FNV-1a of that file's sorted public signatures.
    pub files: BTreeMap<String, u64>,
}

impl SignatureSnapshot {
    pub fn from_sources<'a, I>(sources: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut files = BTreeMap::new();
        for (path, source) in sources {
            files.insert(path.to_string(), signature_fingerprint(path, source));
        }
        Self { files }
    }

    pub fn fingerprint(&self) -> u64 {
        let mut acc = fnv1a64(b"meshloop.slice.snapshot.v1");
        for (path, hash) in &self.files {
            acc ^= fnv1a64(path.as_bytes());
            acc = acc.wrapping_mul(0x100000001b3);
            acc ^= *hash;
            acc = acc.wrapping_mul(0x100000001b3);
        }
        acc
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Impact {
    /// Every file's public signature blob is unchanged. Bodies may have moved.
    BodyOnly,
    /// Named files changed their public surface (and possibly bodies).
    SignatureChanged { files: Vec<String> },
    /// Snapshot sets differ in membership (files added or removed).
    MembershipChanged {
        added: Vec<String>,
        removed: Vec<String>,
    },
}

pub fn impact(before: &SignatureSnapshot, after: &SignatureSnapshot) -> Impact {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for path in after.files.keys() {
        if !before.files.contains_key(path) {
            added.push(path.clone());
        }
    }
    for path in before.files.keys() {
        if !after.files.contains_key(path) {
            removed.push(path.clone());
        }
    }
    if !added.is_empty() || !removed.is_empty() {
        return Impact::MembershipChanged { added, removed };
    }
    let mut changed = Vec::new();
    for (path, hash) in &after.files {
        if before.files.get(path) != Some(hash) {
            changed.push(path.clone());
        }
    }
    if changed.is_empty() {
        Impact::BodyOnly
    } else {
        Impact::SignatureChanged { files: changed }
    }
}

/// Files the next check must cover: signature-changed files, plus files the
/// diagnostic lattice already names, plus nearest neighbors in the index.
pub fn check_slice(
    impact: &Impact,
    lattice: &DiagnosticLattice,
    index: &SignatureIndex,
    query: &str,
    neighbor_k: usize,
) -> Vec<String> {
    let mut set = BTreeSet::new();
    match impact {
        Impact::BodyOnly => {}
        Impact::SignatureChanged { files } => {
            set.extend(files.iter().cloned());
        }
        Impact::MembershipChanged { added, removed } => {
            set.extend(added.iter().cloned());
            set.extend(removed.iter().cloned());
        }
    }
    for atom in lattice.atoms() {
        if !atom.path.is_empty() {
            set.insert(atom.path.clone());
        }
    }
    for (path, _) in index.rank_files(query, neighbor_k) {
        set.insert(path);
    }
    set.into_iter().collect()
}

/// True when a full `verify_command` (cargo test / tsc --noEmit / ...) can be
/// skipped in favor of a targeted check: signatures did not move and there is
/// no syntax diagnostic. Callers still run git-diff; this only gates the
/// expensive compile.
pub fn skip_full_compile(impact: &Impact, lattice: &DiagnosticLattice) -> bool {
    matches!(impact, Impact::BodyOnly)
        && lattice.count(meshloop_domain::diagnostic::DiagnosticSeverity::Syntax) == 0
        && lattice.count(meshloop_domain::diagnostic::DiagnosticSeverity::Type) == 0
}

/// Convenience: skeleton-hash a source so tests don't depend on extract internals.
pub fn skeleton_hash(path: &str, source: &str) -> u64 {
    let language = Language::from_path(std::path::Path::new(path));
    let skel = extract_skeleton(source, language);
    fnv1a64(skel.skeleton.as_bytes())
}

pub fn empty_index() -> SignatureIndex {
    SignatureIndex::new(BitWidth::Two)
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::diagnostic::parse_diagnostics;

    const AUTH_BEFORE: &str = "pub trait Auth { fn verify(&self); }\npub fn login() { let x = 1; }";
    const AUTH_BODY_ONLY: &str =
        "pub trait Auth { fn verify(&self); }\npub fn login() { let x = 2; }";
    const AUTH_SIG_CHANGE: &str =
        "pub trait Auth { fn verify(&self, token: &str); }\npub fn login() { let x = 1; }";

    #[test]
    fn body_only_edit_is_not_a_signature_change() {
        let before = SignatureSnapshot::from_sources([("src/auth.rs", AUTH_BEFORE)]);
        let after = SignatureSnapshot::from_sources([("src/auth.rs", AUTH_BODY_ONLY)]);
        assert_eq!(impact(&before, &after), Impact::BodyOnly);
        assert!(skip_full_compile(
            &Impact::BodyOnly,
            &DiagnosticLattice::empty()
        ));
    }

    #[test]
    fn signature_edit_names_the_file() {
        let before = SignatureSnapshot::from_sources([("src/auth.rs", AUTH_BEFORE)]);
        let after = SignatureSnapshot::from_sources([("src/auth.rs", AUTH_SIG_CHANGE)]);
        match impact(&before, &after) {
            Impact::SignatureChanged { files } => {
                assert_eq!(files, vec!["src/auth.rs".to_string()])
            }
            other => panic!("expected SignatureChanged, got {other:?}"),
        }
        assert!(!skip_full_compile(
            &Impact::SignatureChanged {
                files: vec!["src/auth.rs".into()]
            },
            &DiagnosticLattice::empty()
        ));
    }

    #[test]
    fn added_file_is_membership_change() {
        let before = SignatureSnapshot::from_sources([("a.rs", "pub fn a() {}")]);
        let after =
            SignatureSnapshot::from_sources([("a.rs", "pub fn a() {}"), ("b.rs", "pub fn b() {}")]);
        match impact(&before, &after) {
            Impact::MembershipChanged { added, removed } => {
                assert_eq!(added, vec!["b.rs".to_string()]);
                assert!(removed.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn check_slice_unions_diagnostics_and_neighbors() {
        let mut index = SignatureIndex::new(BitWidth::Two);
        index.add_skeleton("src/auth.rs", AUTH_BEFORE);
        index.add_skeleton("src/db.rs", "pub struct Pool;");
        let lattice = parse_diagnostics("error[E0308]: mismatched types\n --> src/token.rs:2:5\n");
        let slice = check_slice(
            &Impact::SignatureChanged {
                files: vec!["src/auth.rs".into()],
            },
            &lattice,
            &index,
            "Auth verify",
            1,
        );
        assert!(slice.iter().any(|p| p == "src/auth.rs"));
        assert!(slice.iter().any(|p| p.contains("token") || p == "token.rs"));
    }
}
