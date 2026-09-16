//! Deterministic lattice over compiler and test diagnostics.
//!
//! Atoms are hashed after path/line/ANSI stripping so two worktrees that emit the
//! same logical errors compare equal. The lattice is a set: inclusion is the
//! partial order, and a lexicographic progress vector is the ranking used by the
//! repair loop to forbid oscillation.

use std::collections::BTreeSet;

use crate::digest::fnv1a64;

/// Coarser than rustc's full taxonomy: enough to decide rollback vs continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    /// Parse/syntax failure. A new syntax error is always a regression.
    Syntax = 0,
    /// Type, borrow, unresolved-name, and equivalent static errors.
    Type = 1,
    /// Failed unit/integration tests.
    Test = 2,
    /// Other non-zero diagnostics (linker, I/O from the tool).
    Error = 3,
    Warning = 4,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticAtom {
    pub severity: DiagnosticSeverity,
    /// rustc `E0308`, tsc `TS2322`, or a stable fallback such as `syntax`.
    pub code: String,
    /// Basename only. Worktree prefixes are stripped so hashes survive reset.
    pub path: String,
    /// FNV-1a of the normalized message template (no lines, no columns, no ANSI).
    pub template: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLattice {
    atoms: BTreeSet<DiagnosticAtom>,
}

impl DiagnosticLattice {
    pub fn new(atoms: impl IntoIterator<Item = DiagnosticAtom>) -> Self {
        Self {
            atoms: atoms.into_iter().collect(),
        }
    }

    pub fn empty() -> Self {
        Self {
            atoms: BTreeSet::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    pub fn atoms(&self) -> impl Iterator<Item = &DiagnosticAtom> {
        self.atoms.iter()
    }

    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    pub fn count(&self, severity: DiagnosticSeverity) -> usize {
        self.atoms.iter().filter(|a| a.severity == severity).count()
    }

    /// Blocking errors: anything worse than a warning. Warnings may remain at Accept.
    pub fn blocking(&self) -> usize {
        self.atoms
            .iter()
            .filter(|a| a.severity < DiagnosticSeverity::Warning)
            .count()
    }

    /// Lexicographic progress: smaller is better. Syntax first so a type-error
    /// fix that introduces a parse error cannot look like progress.
    pub fn progress(&self) -> Progress {
        Progress {
            syntax: self.count(DiagnosticSeverity::Syntax) as u32,
            type_errors: self.count(DiagnosticSeverity::Type) as u32,
            tests: self.count(DiagnosticSeverity::Test) as u32,
            errors: self.count(DiagnosticSeverity::Error) as u32,
            atoms: self.blocking() as u32,
        }
    }

    /// Byte-stable fingerprint of the set. Equal lattices ⇒ equal fingerprints.
    pub fn fingerprint(&self) -> u64 {
        let mut acc = fnv1a64(b"meshloop.diagnostic.lattice.v1");
        for atom in &self.atoms {
            acc ^= fnv1a64(atom.severity_tag().as_bytes());
            acc = acc.wrapping_mul(0x100000001b3);
            acc ^= fnv1a64(atom.code.as_bytes());
            acc = acc.wrapping_mul(0x100000001b3);
            acc ^= fnv1a64(atom.path.as_bytes());
            acc = acc.wrapping_mul(0x100000001b3);
            acc ^= atom.template;
            acc = acc.wrapping_mul(0x100000001b3);
        }
        acc
    }

    pub fn contains_all(&self, other: &Self) -> bool {
        other.atoms.is_subset(&self.atoms)
    }

    /// Codes that appeared in `self` but not in `baseline`, for negative constraints.
    pub fn introduced_since(&self, baseline: &Self) -> Vec<DiagnosticAtom> {
        self.atoms.difference(&baseline.atoms).cloned().collect()
    }
}

impl DiagnosticSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticSeverity::Syntax => "syntax",
            DiagnosticSeverity::Type => "type",
            DiagnosticSeverity::Test => "test",
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warning",
        }
    }
}

impl DiagnosticAtom {
    fn severity_tag(&self) -> &'static str {
        self.severity.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Progress {
    pub syntax: u32,
    pub type_errors: u32,
    pub tests: u32,
    pub errors: u32,
    pub atoms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatticeOrder {
    Equal,
    Improves,
    Regresses,
}

pub fn compare(previous: &DiagnosticLattice, next: &DiagnosticLattice) -> LatticeOrder {
    if previous.fingerprint() == next.fingerprint() {
        return LatticeOrder::Equal;
    }
    if next.progress() < previous.progress() {
        LatticeOrder::Improves
    } else {
        LatticeOrder::Regresses
    }
}

pub fn syntax_regressed(previous: &DiagnosticLattice, next: &DiagnosticLattice) -> bool {
    next.progress().syntax > previous.progress().syntax
}

/// Parse rustc / cargo / tsc / python / go / generic compiler stderr into a lattice.
pub fn parse_diagnostics(stderr: &str) -> DiagnosticLattice {
    let stripped = strip_ansi(stderr);
    let mut atoms = BTreeSet::new();
    let lines: Vec<&str> = stripped.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.is_empty() {
            i += 1;
            continue;
        }
        if let Some(atom) = parse_rustc_error(line, &lines, i) {
            atoms.insert(atom);
            i += 1;
            continue;
        }
        if let Some(atom) = parse_tsc_error(line) {
            atoms.insert(atom);
            i += 1;
            continue;
        }
        if let Some(atom) = parse_python_syntax(line, &lines, i) {
            atoms.insert(atom);
            i += 1;
            continue;
        }
        if let Some(atom) = parse_go_error(line) {
            atoms.insert(atom);
            i += 1;
            continue;
        }
        if let Some(atom) = parse_cargo_test_fail(line) {
            atoms.insert(atom);
            i += 1;
            continue;
        }
        if let Some(atom) = parse_generic_error(line) {
            atoms.insert(atom);
        }
        i += 1;
    }
    DiagnosticLattice { atoms }
}

fn parse_rustc_error(line: &str, lines: &[&str], i: usize) -> Option<DiagnosticAtom> {
    let (severity, code, message) = if let Some(rest) = line.strip_prefix("error[") {
        let (code, rest) = rest.split_once(']')?;
        let message = rest.strip_prefix(':').unwrap_or(rest).trim();
        let sev = if is_syntax_code(code) || is_syntax_message(message) {
            DiagnosticSeverity::Syntax
        } else {
            DiagnosticSeverity::Type
        };
        (sev, code.to_string(), message)
    } else if let Some(rest) = line.strip_prefix("error:") {
        let message = rest.trim();
        if is_rustc_trailer(message) {
            return None;
        }
        let sev = if is_syntax_message(message) {
            DiagnosticSeverity::Syntax
        } else {
            DiagnosticSeverity::Error
        };
        (sev, syntax_or_error_code(sev), message)
    } else if let Some(rest) = line.strip_prefix("warning[") {
        let (code, rest) = rest.split_once(']')?;
        let message = rest.strip_prefix(':').unwrap_or(rest).trim();
        (DiagnosticSeverity::Warning, code.to_string(), message)
    } else {
        let rest = line.strip_prefix("warning:")?;
        (
            DiagnosticSeverity::Warning,
            "warning".to_string(),
            rest.trim(),
        )
    };
    let path = find_rustc_path(lines, i).unwrap_or_default();
    Some(DiagnosticAtom {
        severity,
        code,
        path,
        template: template_hash(message),
    })
}

fn parse_tsc_error(line: &str) -> Option<DiagnosticAtom> {
    // file.ts(10,5): error TS2322: Type 'X' is not assignable...
    let lower = line.to_ascii_lowercase();
    if !lower.contains("error ts") && !lower.contains(": error ts") {
        return None;
    }
    let code = line
        .split_whitespace()
        .find(|t| t.starts_with("TS") && t.chars().nth(2).is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or("TS")
        .trim_end_matches(':')
        .to_string();
    let path = line.split('(').next().map(basename).unwrap_or_default();
    let message = line.split(": ").nth(2).unwrap_or(line);
    let severity = if code == "TS1005" || is_syntax_message(message) {
        DiagnosticSeverity::Syntax
    } else {
        DiagnosticSeverity::Type
    };
    Some(DiagnosticAtom {
        severity,
        code,
        path,
        template: template_hash(message),
    })
}

fn parse_python_syntax(line: &str, lines: &[&str], i: usize) -> Option<DiagnosticAtom> {
    if !line.contains("SyntaxError") && !line.contains("IndentationError") {
        return None;
    }
    let mut path = String::new();
    for j in (0..i).rev().take(4) {
        let l = lines[j].trim();
        if let Some(rest) = l.strip_prefix("File \"") {
            path = basename(rest.split('"').next().unwrap_or(rest));
            break;
        }
    }
    Some(DiagnosticAtom {
        severity: DiagnosticSeverity::Syntax,
        code: "SyntaxError".into(),
        path,
        template: template_hash(line),
    })
}

fn parse_go_error(line: &str) -> Option<DiagnosticAtom> {
    // ./main.go:5:2: undefined: Foo
    let trimmed = line.trim();
    if !trimmed.contains(".go:") {
        return None;
    }
    let (path_part, message) = trimmed.split_once(": ")?;
    let path = basename(path_part.split(':').next().unwrap_or(path_part));
    let severity = if is_syntax_message(message) {
        DiagnosticSeverity::Syntax
    } else {
        DiagnosticSeverity::Type
    };
    Some(DiagnosticAtom {
        severity,
        code: if severity == DiagnosticSeverity::Syntax {
            "syntax".into()
        } else {
            "go".into()
        },
        path,
        template: template_hash(message),
    })
}

fn parse_cargo_test_fail(line: &str) -> Option<DiagnosticAtom> {
    let trimmed = line.trim();
    if !(trimmed.starts_with("test ") && trimmed.ends_with("FAILED")) {
        return None;
    }
    let name = trimmed
        .strip_prefix("test ")?
        .trim_end_matches("FAILED")
        .trim()
        .trim_end_matches('.')
        .trim();
    Some(DiagnosticAtom {
        severity: DiagnosticSeverity::Test,
        code: "test".into(),
        path: String::new(),
        template: template_hash(name),
    })
}

fn is_rustc_trailer(message: &str) -> bool {
    let m = message.trim().to_ascii_lowercase();
    m.contains("aborting due to")
        || m.contains("detailed explanations")
        || m.contains("for more information about an error")
}

fn parse_generic_error(line: &str) -> Option<DiagnosticAtom> {
    let lower = line.to_ascii_lowercase();
    if !(lower.contains("error:") || lower.contains("error ")) {
        return None;
    }
    if lower.contains("warning") {
        return None;
    }
    if is_rustc_trailer(line) {
        return None;
    }
    Some(DiagnosticAtom {
        severity: DiagnosticSeverity::Error,
        code: "error".into(),
        path: String::new(),
        template: template_hash(line),
    })
}

fn find_rustc_path(lines: &[&str], i: usize) -> Option<String> {
    for line in lines.iter().skip(i).take(6) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("-->") {
            return Some(path_from_rustc_loc(rest.trim()));
        }
    }
    None
}

/// rustc writes `path:line:col`. On Windows `path` itself contains `:`.
fn path_from_rustc_loc(loc: &str) -> String {
    let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    let path = if parts.len() == 3
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].chars().all(|c| c.is_ascii_digit())
    {
        parts[2]
    } else {
        loc
    };
    basename(path)
}

fn is_syntax_code(code: &str) -> bool {
    matches!(
        code,
        "E0426" | "E0428" | "E0753" | "E0761" | "E0110" | "E0758"
    ) || code.starts_with('S')
}

fn is_syntax_message(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    (m.contains("expected") && (m.contains("found") || m.contains("one of")))
        || m.contains("unexpected token")
        || m.contains("invalid syntax")
        || m.contains("unterminated")
        || m.contains("expected item")
        || m.contains("this file contains an unclosed delimiter")
}

fn syntax_or_error_code(sev: DiagnosticSeverity) -> String {
    match sev {
        DiagnosticSeverity::Syntax => "syntax".into(),
        _ => "error".into(),
    }
}

fn template_hash(message: &str) -> u64 {
    fnv1a64(normalize_template(message).as_bytes())
}

/// Drop locations, quoted idents (keep the quotes as holes), numbers, and ANSI.
pub fn normalize_template(message: &str) -> String {
    let stripped = strip_ansi(message);
    let mut out = String::with_capacity(stripped.len());
    let mut chars = stripped.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            out.push('`');
            out.push('_');
            out.push('`');
            while let Some(&n) = chars.peek() {
                chars.next();
                if n == '`' {
                    break;
                }
            }
            continue;
        }
        if c.is_ascii_digit() {
            if out.ends_with('#') {
                continue;
            }
            out.push('#');
            while chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                chars.next();
            }
            continue;
        }
        if c.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            continue;
        }
        out.push(c.to_ascii_lowercase());
    }
    out.trim().to_string()
}

pub fn basename(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .rsplit('/')
        .next()
        .unwrap_or(&normalized)
        .to_string()
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUSTC: &str = "\
error[E0308]: mismatched types
 --> src/main.rs:2:5
  |
2 |     1
  |     ^ expected `&str`, found integer
";

    const RUSTC_MOVED: &str = "\
error[E0308]: mismatched types
 --> C:\\tmp\\.meshloop-worktrees\\repo\\task-1-attempt-2\\src\\main.rs:40:12
  |
40 |     foo(1)
   |            ^ expected `&str`, found integer
";

    #[test]
    fn rustc_code_and_severity() {
        let lat = parse_diagnostics(RUSTC);
        assert_eq!(lat.len(), 1);
        let atom = lat.atoms().next().unwrap();
        assert_eq!(atom.code, "E0308");
        assert_eq!(atom.severity, DiagnosticSeverity::Type);
        assert_eq!(atom.path, "main.rs");
    }

    #[test]
    fn worktree_path_and_line_do_not_change_fingerprint() {
        let a = parse_diagnostics(RUSTC);
        let b = parse_diagnostics(RUSTC_MOVED);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(compare(&a, &b), LatticeOrder::Equal);
    }

    #[test]
    fn quoted_idents_are_holes_in_the_template() {
        let a = normalize_template("cannot find value `foo` in this scope");
        let b = normalize_template("cannot find value `bar` in this scope");
        assert_eq!(a, b);
        assert!(a.contains("`_`"));
    }

    #[test]
    fn syntax_error_is_worse_than_type_error() {
        let typed = parse_diagnostics(RUSTC);
        let syntax = parse_diagnostics(
            "error: this file contains an unclosed delimiter\n --> src/lib.rs:1:1\n",
        );
        assert!(syntax_regressed(&typed, &syntax));
        assert_eq!(compare(&typed, &syntax), LatticeOrder::Regresses);
        assert_eq!(compare(&syntax, &typed), LatticeOrder::Improves);
    }

    #[test]
    fn empty_lattice_is_the_minimum() {
        let empty = DiagnosticLattice::empty();
        assert_eq!(
            empty.progress(),
            Progress {
                syntax: 0,
                type_errors: 0,
                tests: 0,
                errors: 0,
                atoms: 0,
            }
        );
        assert_eq!(empty.blocking(), 0);
    }

    #[test]
    fn rustc_aborting_trailer_is_not_an_atom() {
        let lat = parse_diagnostics(
            "error[E0308]: mismatched types\n --> src/main.rs:2:5\n\
             error: aborting due to 1 previous error\n\
             Some errors have detailed explanations: E0308.\n\
             For more information about an error, try `rustc --explain E0308`.\n",
        );
        assert_eq!(lat.len(), 1);
        assert_eq!(lat.atoms().next().unwrap().code, "E0308");
        assert_eq!(lat.blocking(), 1);
    }

    #[test]
    fn cargo_test_failure_is_a_test_atom() {
        let lat = parse_diagnostics("test meshloop_domain::diagnostic::tests::foo ... FAILED");
        assert_eq!(lat.count(DiagnosticSeverity::Test), 1);
    }

    #[test]
    fn ansi_and_line_numbers_are_stripped() {
        let raw = "\u{1b}[31merror[E0425]\u{1b}[0m: cannot find value `x` in this scope";
        let lat = parse_diagnostics(raw);
        assert_eq!(lat.len(), 1);
        assert_eq!(lat.atoms().next().unwrap().code, "E0425");
    }

    #[test]
    fn introduced_since_lists_new_codes() {
        let prev = parse_diagnostics(RUSTC);
        let next = parse_diagnostics(&format!(
            "{RUSTC}error[E0425]: cannot find value `x` in this scope\n --> src/lib.rs:1:1\n"
        ));
        let introduced = next.introduced_since(&prev);
        assert_eq!(introduced.len(), 1);
        assert_eq!(introduced[0].code, "E0425");
    }
}
