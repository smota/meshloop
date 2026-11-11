//! Advisory routing priors (ADR 0038, amendment 2026-11-10 for #139). Value types and
//! pure predicates only: the file is read and the ledger kept by adapters, the snapshot
//! port and the priors oracle live in the engine, and the verbs in the CLI.
//!
//! The kernel derives π = (successes + 1) / (attempts + 2) from an entry's counts; the
//! file never states π. An entry has effect only with at least [`PRIOR_EVIDENCE_FLOOR`]
//! counted attempts, a compiled-in constant the file cannot lower.

use std::collections::{BTreeMap, BTreeSet};

use crate::state::Event;
use crate::task_graph::Tier;

/// How Meshloop dispatched an attempt. Every reader matches it exhaustively, so adding
/// `Native` (#147) fails to compile until each reader handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DispatchMode {
    Subprocess,
}

impl DispatchMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subprocess => "subprocess",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "subprocess" => Some(Self::Subprocess),
            _ => None,
        }
    }

    /// The mode an attempt row was dispatched with, from the row's recorded mode. The
    /// binary has one dispatch path and the store has no mode column, so every row
    /// carries [`RecordedDispatch::NoColumn`] and reads `Subprocess`. When #147 adds
    /// `Native`, both matches here stop compiling: add the attempts mode column, carry
    /// it in `RecordedDispatch`, and read it here. #147 must not merge without that.
    pub const fn of_recorded_attempt(recorded: RecordedDispatch) -> Self {
        match recorded {
            RecordedDispatch::NoColumn => {
                const ONLY_PATH: DispatchMode = DispatchMode::Subprocess;
                match ONLY_PATH {
                    DispatchMode::Subprocess => ONLY_PATH,
                }
            }
        }
    }

    /// Whether an attempt in this mode can be prior evidence, and whether a candidate
    /// dispatched in this mode may receive a prior (amendment rules 2 and 3).
    pub const fn admits_priors(self) -> bool {
        match self {
            Self::Subprocess => true,
        }
    }
}

/// The dispatch mode an attempt row records. A placeholder until the store has a mode
/// column (#147): every row today was dispatched by the binary's one subprocess path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecordedDispatch {
    /// The row predates any mode column.
    #[default]
    NoColumn,
}

/// The reason recorded on a `DeterministicChecksFailed` that Meshloop appends when its
/// own driver died during verification (`fail_orphaned_verification`). It is an
/// infrastructure fault, not the worker's result, so it gives no check result. The
/// text is the one rows have carried before #139, so existing ledgers read the same.
pub const VERIFICATION_INFRASTRUCTURE_FAULT: &str = "driver died during verification";

/// An attempt's deterministic check result: its final deterministic-result event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckResult {
    Passed,
    Failed,
}

impl CheckResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }

    /// `Some` for a deterministic-result event, holding what it says:
    /// `DeterministicChecksPassed` and `VerifyGatePassed` are a pass,
    /// `DeterministicChecksFailed` a failure, except the infrastructure fault
    /// ([`VERIFICATION_INFRASTRUCTURE_FAULT`]), which is no result. `None` for every
    /// other event.
    pub fn of_record(event: Event, reason: Option<&str>) -> Option<Option<Self>> {
        match event {
            Event::DeterministicChecksPassed | Event::VerifyGatePassed => Some(Some(Self::Passed)),
            Event::DeterministicChecksFailed
                if reason == Some(VERIFICATION_INFRASTRUCTURE_FAULT) =>
            {
                Some(None)
            }
            Event::DeterministicChecksFailed => Some(Some(Self::Failed)),
            _ => None,
        }
    }

    /// What the last deterministic-result event among one attempt's records, in append
    /// order, says. `None` when checks never recorded a result (a crash, a timeout, a
    /// cancellation before checks, a result whose append was refused (#167)) or when
    /// the last one is an infrastructure fault.
    pub fn final_of<'a>(
        records: impl IntoIterator<Item = (Event, Option<&'a str>)>,
    ) -> Option<Self> {
        records
            .into_iter()
            .filter_map(|(event, reason)| Self::of_record(event, reason))
            .last()
            .flatten()
    }
}

/// Upper bound on one `model_ref`, in bytes (#134).
pub const MAX_MODEL_REF_BYTES: usize = 256;
/// The model ADR 0039 records for a native attempt whose model is not known.
pub const UNVERIFIED_MODEL_REF: &str = "unverified";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRefFault {
    Empty,
    TooLong,
    LeadingDash,
    WhitespaceOrControl,
}

/// #134's `model_ref` predicate, moved from the CLI config loader: non-empty after
/// trimming, at most [`MAX_MODEL_REF_BYTES`] bytes, no leading `-` (the ref becomes
/// one argv element and must not read as a flag), no whitespace or control characters.
pub fn check_model_ref(model_ref: &str) -> Result<(), ModelRefFault> {
    if model_ref.trim().is_empty() {
        return Err(ModelRefFault::Empty);
    }
    if model_ref.len() > MAX_MODEL_REF_BYTES {
        return Err(ModelRefFault::TooLong);
    }
    if model_ref.starts_with('-') {
        return Err(ModelRefFault::LeadingDash);
    }
    if model_ref
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ModelRefFault::WhitespaceOrControl);
    }
    Ok(())
}

/// A model a prior may be keyed on: #134's predicate, and never `unverified`.
pub fn is_prior_model_ref(model_ref: &str) -> bool {
    check_model_ref(model_ref).is_ok() && model_ref != UNVERIFIED_MODEL_REF
}

/// Upper bound on a promotion identity (`--as`), in bytes.
pub const MAX_IDENTITY_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityFault {
    Empty,
    LeadingDash,
    ControlCharacter,
    TooLong,
}

impl IdentityFault {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "--as must not be empty",
            Self::LeadingDash => "--as must not start with '-'",
            Self::ControlCharacter => "--as must not contain control characters",
            Self::TooLong => "--as is longer than 128 bytes",
        }
    }
}

/// The `--as` identity of a promotion. It records a claim; it is not authenticated.
pub fn check_identity(identity: &str) -> Result<(), IdentityFault> {
    if identity.is_empty() {
        return Err(IdentityFault::Empty);
    }
    if identity.starts_with('-') {
        return Err(IdentityFault::LeadingDash);
    }
    if identity.chars().any(char::is_control) {
        return Err(IdentityFault::ControlCharacter);
    }
    if identity.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityFault::TooLong);
    }
    Ok(())
}

/// Counted attempts an entry needs before it has any effect. Compiled in: the file
/// cannot lower it. It is above the 4 pseudo-observations π gets in the fused mean.
pub const PRIOR_EVIDENCE_FLOOR: u32 = 5;

/// The key of one prior: (harness, model_ref, task tier, dispatch mode).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PriorKey {
    pub harness: String,
    pub model_ref: String,
    pub tier: Tier,
    pub mode: DispatchMode,
}

/// One counted attempt an entry stands for. Never worker text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttemptRef {
    pub graph_id: String,
    pub task_id: u64,
    pub attempt_id: u64,
    pub passed: bool,
}

/// An entry that passed every loader check.
#[derive(Debug, Clone, PartialEq)]
pub struct PriorEntry {
    /// Position in the file's `priors` array. The stored bytes never change, so this
    /// identifies the entry across `accept` and routing.
    pub index: usize,
    pub key: PriorKey,
    pub attempts: u32,
    pub successes: u32,
    pub provenance: Vec<AttemptRef>,
}

impl PriorEntry {
    /// π = (successes + 1) / (attempts + 2). Never 0 or 1.
    pub fn pi(&self) -> f64 {
        (f64::from(self.successes) + 1.0) / (f64::from(self.attempts) + 2.0)
    }
}

/// One entry as the adapter parsed it, before the kernel's rules run.
#[derive(Debug, Clone, PartialEq)]
pub struct RawPrior {
    pub index: usize,
    pub harness: String,
    pub model_ref: String,
    pub tier: Tier,
    /// The file's `dispatch_mode` string, kept raw so an unknown mode is reported.
    pub dispatch_mode: String,
    pub attempts: u64,
    pub successes: u64,
    pub provenance: Vec<AttemptRef>,
}

/// An entry the adapter could not parse into a [`RawPrior`].
#[derive(Debug, Clone, PartialEq)]
pub struct FaultyPrior {
    pub index: usize,
    /// Whatever key fields were readable, for the report.
    pub label: EntryLabel,
    pub faults: Vec<(PriorsCode, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedPrior {
    Parsed(RawPrior),
    Faulty(FaultyPrior),
}

/// The key fields of an entry as written, for diagnostics. Any may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryLabel {
    pub harness: Option<String>,
    pub model_ref: Option<String>,
    pub tier: Option<String>,
    pub dispatch_mode: Option<String>,
}

impl EntryLabel {
    fn of(raw: &RawPrior) -> Self {
        Self {
            harness: Some(raw.harness.clone()),
            model_ref: Some(raw.model_ref.clone()),
            tier: Some(tier_name(raw.tier).to_string()),
            dispatch_mode: Some(raw.dispatch_mode.clone()),
        }
    }
}

pub const fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Tier1 => "Tier1",
        Tier::Tier2 => "Tier2",
        Tier::Tier3 => "Tier3",
    }
}

pub fn parse_tier(raw: &str) -> Option<Tier> {
    match raw {
        "Tier1" => Some(Tier::Tier1),
        "Tier2" => Some(Tier::Tier2),
        "Tier3" => Some(Tier::Tier3),
        _ => None,
    }
}

/// Where a diagnostic applies, and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorsLevel {
    /// No priors are applied.
    File,
    /// The entry is ignored.
    Entry,
    /// Reported only.
    Report,
}

impl PriorsLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Entry => "entry",
            Self::Report => "report",
        }
    }
}

/// Loader diagnostic codes (learning-plane.md, "Loader diagnostics"). None is a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PriorsCode {
    NotPromoted,
    Revoked,
    DigestMismatch,
    /// The ledger row's ignored-entries JSON is unreadable: the store is corrupted.
    LedgerCorrupt,
    TooLarge,
    NotJson,
    WrongSchema,
    WrongVersion,
    BelowEvidenceFloor,
    DuplicateKey,
    DuplicateProvenance,
    CountMismatch,
    SuccessesMismatch,
    NotSubprocess,
    ModelInvalid,
    UnknownEntryField,
    /// A defined entry field has the wrong type or is out of its schema range.
    MalformedEntry,
    ProvenanceContradicts,
    NotACandidate,
    UnknownField,
    ProvenanceUnresolved,
}

impl PriorsCode {
    pub const ALL: [PriorsCode; 21] = [
        Self::NotPromoted,
        Self::Revoked,
        Self::DigestMismatch,
        Self::LedgerCorrupt,
        Self::TooLarge,
        Self::NotJson,
        Self::WrongSchema,
        Self::WrongVersion,
        Self::BelowEvidenceFloor,
        Self::DuplicateKey,
        Self::DuplicateProvenance,
        Self::CountMismatch,
        Self::SuccessesMismatch,
        Self::NotSubprocess,
        Self::ModelInvalid,
        Self::UnknownEntryField,
        Self::MalformedEntry,
        Self::ProvenanceContradicts,
        Self::NotACandidate,
        Self::UnknownField,
        Self::ProvenanceUnresolved,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotPromoted => "not_promoted",
            Self::Revoked => "revoked",
            Self::DigestMismatch => "digest_mismatch",
            Self::LedgerCorrupt => "ledger_corrupt",
            Self::TooLarge => "too_large",
            Self::NotJson => "not_json",
            Self::WrongSchema => "wrong_schema",
            Self::WrongVersion => "wrong_version",
            Self::BelowEvidenceFloor => "below_evidence_floor",
            Self::DuplicateKey => "duplicate_key",
            Self::DuplicateProvenance => "duplicate_provenance",
            Self::CountMismatch => "count_mismatch",
            Self::SuccessesMismatch => "successes_mismatch",
            Self::NotSubprocess => "not_subprocess",
            Self::ModelInvalid => "model_invalid",
            Self::UnknownEntryField => "unknown_entry_field",
            Self::MalformedEntry => "malformed_entry",
            Self::ProvenanceContradicts => "provenance_contradicts",
            Self::NotACandidate => "not_a_candidate",
            Self::UnknownField => "unknown_field",
            Self::ProvenanceUnresolved => "provenance_unresolved",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == raw)
    }

    pub const fn level(self) -> PriorsLevel {
        match self {
            Self::NotPromoted
            | Self::Revoked
            | Self::DigestMismatch
            | Self::LedgerCorrupt
            | Self::TooLarge
            | Self::NotJson
            | Self::WrongSchema
            | Self::WrongVersion => PriorsLevel::File,
            Self::BelowEvidenceFloor
            | Self::DuplicateKey
            | Self::DuplicateProvenance
            | Self::CountMismatch
            | Self::SuccessesMismatch
            | Self::NotSubprocess
            | Self::ModelInvalid
            | Self::UnknownEntryField
            | Self::MalformedEntry
            | Self::ProvenanceContradicts => PriorsLevel::Entry,
            Self::NotACandidate | Self::UnknownField | Self::ProvenanceUnresolved => {
                PriorsLevel::Report
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorsDiagnostic {
    pub code: PriorsCode,
    /// The entry's index in `priors`, for entry-level and per-entry report codes.
    pub entry: Option<usize>,
    pub label: EntryLabel,
    /// Plain text from the kernel. File strings in it are passed through
    /// [`display_safe`] by whoever renders it.
    pub detail: String,
}

impl PriorsDiagnostic {
    pub fn file(code: PriorsCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            entry: None,
            label: EntryLabel::default(),
            detail: detail.into(),
        }
    }
}

/// The outcome of the kernel's entry rules over one parsed file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValidatedPriors {
    /// Entries with effect, in file order.
    pub entries: Vec<PriorEntry>,
    /// Entry-level diagnostics, in file order.
    pub diagnostics: Vec<PriorsDiagnostic>,
}

impl ValidatedPriors {
    /// Ignores one surviving entry with `code` (e.g. `provenance_contradicts`, decided
    /// at `accept`). Unknown indices are a no-op.
    pub fn ignore(&mut self, index: usize, code: PriorsCode, detail: impl Into<String>) {
        let Some(pos) = self.entries.iter().position(|e| e.index == index) else {
            return;
        };
        let entry = self.entries.remove(pos);
        self.diagnostics.push(PriorsDiagnostic {
            code,
            entry: Some(index),
            label: EntryLabel {
                harness: Some(entry.key.harness),
                model_ref: Some(entry.key.model_ref),
                tier: Some(tier_name(entry.key.tier).to_string()),
                dispatch_mode: Some(entry.key.mode.as_str().to_string()),
            },
            detail: detail.into(),
        });
    }

    /// Every ignored entry with its codes, one row per (index, code), in file order.
    pub fn ignored(&self) -> Vec<&PriorsDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.code.level() == PriorsLevel::Entry)
            .collect()
    }
}

/// The kernel's entry rules (amendment rule 5 and learning-plane.md). Each fault yields
/// one diagnostic and ignores its entry; every copy of a duplicated key is ignored.
pub fn validate_priors(parsed: Vec<ParsedPrior>) -> ValidatedPriors {
    // Duplicate keys over every entry whose key fields were readable, faulty or not.
    let mut key_counts: BTreeMap<(String, String, Tier, String), usize> = BTreeMap::new();
    for p in &parsed {
        let key = match p {
            ParsedPrior::Parsed(raw) => Some((
                raw.harness.clone(),
                raw.model_ref.clone(),
                raw.tier,
                raw.dispatch_mode.clone(),
            )),
            ParsedPrior::Faulty(f) => match (
                &f.label.harness,
                &f.label.model_ref,
                f.label.tier.as_deref().and_then(parse_tier),
                &f.label.dispatch_mode,
            ) {
                (Some(h), Some(m), Some(t), Some(d)) => Some((h.clone(), m.clone(), t, d.clone())),
                _ => None,
            },
        };
        if let Some(key) = key {
            *key_counts.entry(key).or_default() += 1;
        }
    }
    let mut out = ValidatedPriors::default();
    for p in parsed {
        let raw = match p {
            ParsedPrior::Faulty(f) => {
                for (code, detail) in f.faults {
                    out.diagnostics.push(PriorsDiagnostic {
                        code,
                        entry: Some(f.index),
                        label: f.label.clone(),
                        detail,
                    });
                }
                continue;
            }
            ParsedPrior::Parsed(raw) => raw,
        };
        let mut faults: Vec<(PriorsCode, String)> = Vec::new();
        let duplicates = key_counts
            .get(&(
                raw.harness.clone(),
                raw.model_ref.clone(),
                raw.tier,
                raw.dispatch_mode.clone(),
            ))
            .copied()
            .unwrap_or(0);
        if duplicates > 1 {
            faults.push((
                PriorsCode::DuplicateKey,
                format!("the key appears {duplicates} times; every copy is ignored"),
            ));
        }
        let mode = DispatchMode::parse(&raw.dispatch_mode).filter(|m| m.admits_priors());
        if mode.is_none() {
            faults.push((
                PriorsCode::NotSubprocess,
                "dispatch_mode is not subprocess".into(),
            ));
        }
        if !is_prior_model_ref(&raw.model_ref) {
            faults.push((
                PriorsCode::ModelInvalid,
                "model_ref fails the model_ref predicate or is `unverified`".into(),
            ));
        }
        if raw.provenance.len() as u64 != raw.attempts {
            faults.push((
                PriorsCode::CountMismatch,
                format!(
                    "attempts is {} but provenance has {} references",
                    raw.attempts,
                    raw.provenance.len()
                ),
            ));
        }
        let passed = raw.provenance.iter().filter(|r| r.passed).count() as u64;
        if passed != raw.successes {
            faults.push((
                PriorsCode::SuccessesMismatch,
                format!(
                    "successes is {} but {passed} references passed",
                    raw.successes
                ),
            ));
        }
        let mut seen = BTreeSet::new();
        if raw
            .provenance
            .iter()
            .any(|r| !seen.insert((r.graph_id.as_str(), r.task_id, r.attempt_id)))
        {
            faults.push((
                PriorsCode::DuplicateProvenance,
                "two references name the same (graph_id, task_id, attempt_id)".into(),
            ));
        }
        if raw.attempts < u64::from(PRIOR_EVIDENCE_FLOOR) {
            faults.push((
                PriorsCode::BelowEvidenceFloor,
                format!(
                    "attempts is {}; the floor is {PRIOR_EVIDENCE_FLOOR}",
                    raw.attempts
                ),
            ));
        }
        let counts = (
            u32::try_from(raw.attempts).ok(),
            u32::try_from(raw.successes).ok(),
        );
        match (faults.is_empty(), mode, counts) {
            (true, Some(mode), (Some(attempts), Some(successes))) => {
                out.entries.push(PriorEntry {
                    index: raw.index,
                    key: PriorKey {
                        harness: raw.harness,
                        model_ref: raw.model_ref,
                        tier: raw.tier,
                        mode,
                    },
                    attempts,
                    successes,
                    provenance: raw.provenance,
                });
            }
            _ => {
                if faults.is_empty() {
                    faults.push((PriorsCode::MalformedEntry, "counts out of range".into()));
                }
                let label = EntryLabel::of(&raw);
                for (code, detail) in faults {
                    out.diagnostics.push(PriorsDiagnostic {
                        code,
                        entry: Some(raw.index),
                        label: label.clone(),
                        detail,
                    });
                }
            }
        }
    }
    out
}

/// A string from the file or the ledger, safe to display: control characters and
/// Unicode format (Cf) characters, bidi controls included, are removed.
pub fn display_safe(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .collect()
}

/// Unicode general category Cf (Unicode 15.1).
fn is_format_char(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(passed: &[bool]) -> Vec<AttemptRef> {
        passed
            .iter()
            .enumerate()
            .map(|(i, p)| AttemptRef {
                graph_id: "g".into(),
                task_id: 1,
                attempt_id: i as u64 + 1,
                passed: *p,
            })
            .collect()
    }

    fn raw(index: usize, harness: &str, passed: &[bool]) -> RawPrior {
        RawPrior {
            index,
            harness: harness.into(),
            model_ref: "m".into(),
            tier: Tier::Tier2,
            dispatch_mode: "subprocess".into(),
            attempts: passed.len() as u64,
            successes: passed.iter().filter(|p| **p).count() as u64,
            provenance: refs(passed),
        }
    }

    fn codes(v: &ValidatedPriors, index: usize) -> Vec<PriorsCode> {
        v.diagnostics
            .iter()
            .filter(|d| d.entry == Some(index))
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn dispatch_mode_reads_back_subprocess_for_every_recorded_attempt() {
        assert_eq!(
            DispatchMode::of_recorded_attempt(RecordedDispatch::NoColumn),
            DispatchMode::Subprocess
        );
        assert_eq!(DispatchMode::Subprocess.as_str(), "subprocess");
        assert_eq!(
            DispatchMode::parse("subprocess"),
            Some(DispatchMode::Subprocess)
        );
        assert_eq!(DispatchMode::parse("native"), None);
        assert_eq!(DispatchMode::parse(""), None);
    }

    fn plain<const N: usize>(events: [Event; N]) -> Vec<(Event, Option<&'static str>)> {
        events.into_iter().map(|e| (e, None)).collect()
    }

    /// Architect decision (#139 review): Meshloop's own verification fault is not the
    /// worker's failure; it is `no_check_result`.
    #[test]
    fn an_infrastructure_fault_gives_no_check_result() {
        use Event::*;
        assert_eq!(
            CheckResult::final_of([
                (AttemptStarted, None),
                (HarnessExited, None),
                (
                    DeterministicChecksFailed,
                    Some(VERIFICATION_INFRASTRUCTURE_FAULT)
                ),
            ]),
            None
        );
        assert_eq!(
            CheckResult::final_of([
                (HarnessExited, None),
                (DeterministicChecksFailed, Some("git diff failed")),
            ]),
            Some(CheckResult::Failed)
        );
        // A later real result after a retry is the attempt's own; a fault after a pass
        // cannot happen (checks run once per attempt), but the last one decides.
        assert_eq!(
            CheckResult::final_of([
                (
                    DeterministicChecksFailed,
                    Some(VERIFICATION_INFRASTRUCTURE_FAULT)
                ),
                (LiveWorkerSettled, None),
                (DeterministicChecksPassed, None),
            ]),
            Some(CheckResult::Passed)
        );
    }

    #[test]
    fn check_result_is_the_final_deterministic_result_event() {
        use Event::*;
        assert_eq!(
            CheckResult::final_of(plain([
                AttemptStarted,
                HarnessExited,
                DeterministicChecksPassed
            ])),
            Some(CheckResult::Passed)
        );
        assert_eq!(
            CheckResult::final_of(plain([
                AttemptStarted,
                HarnessExited,
                DeterministicChecksPassed,
                VerifyGatePassed,
                IntegrationOwnerMerge
            ])),
            Some(CheckResult::Passed)
        );
        assert_eq!(
            CheckResult::final_of(plain([
                AttemptStarted,
                HarnessExited,
                DeterministicChecksFailed
            ])),
            Some(CheckResult::Failed)
        );
        // Human acceptance after a pass keeps the pass; a rejection is a review, not a check.
        assert_eq!(
            CheckResult::final_of(plain([
                DeterministicChecksPassed,
                ReviewRejected,
                RetryAuthorized
            ])),
            Some(CheckResult::Passed)
        );
        // Crash, timeout, cancellation: no result.
        assert_eq!(
            CheckResult::final_of(plain([AttemptStarted, HarnessCrashedOrTimeout])),
            None
        );
        assert_eq!(
            CheckResult::final_of(plain([
                AttemptStarted,
                HarnessCrashedOrTimeout,
                HarnessExited
            ])),
            None
        );
        assert_eq!(CheckResult::final_of(plain([AttemptStarted, Cancel])), None);
        assert_eq!(
            CheckResult::final_of(plain([AttemptStarted, HarnessExited])),
            None
        );
        // A late settle after a premature failure reaches checks and records a result.
        assert_eq!(
            CheckResult::final_of(plain([
                AttemptStarted,
                HarnessCrashedOrTimeout,
                LiveWorkerSettled,
                DeterministicChecksFailed
            ])),
            Some(CheckResult::Failed)
        );
        assert_eq!(CheckResult::Passed.as_str(), "passed");
        assert_eq!(CheckResult::Failed.as_str(), "failed");
    }

    #[test]
    fn model_ref_predicate_matches_the_134_config_rules() {
        assert_eq!(check_model_ref("gpt-5"), Ok(()));
        assert_eq!(check_model_ref(""), Err(ModelRefFault::Empty));
        assert_eq!(check_model_ref("  "), Err(ModelRefFault::Empty));
        assert_eq!(check_model_ref(&"m".repeat(MAX_MODEL_REF_BYTES)), Ok(()));
        assert_eq!(
            check_model_ref(&"m".repeat(MAX_MODEL_REF_BYTES + 1)),
            Err(ModelRefFault::TooLong)
        );
        assert_eq!(check_model_ref("-x"), Err(ModelRefFault::LeadingDash));
        assert_eq!(
            check_model_ref("a b"),
            Err(ModelRefFault::WhitespaceOrControl)
        );
        assert_eq!(
            check_model_ref("a\u{7}b"),
            Err(ModelRefFault::WhitespaceOrControl)
        );
        assert!(is_prior_model_ref("gpt-5"));
        assert!(!is_prior_model_ref("unverified"));
        assert!(!is_prior_model_ref("-x"));
    }

    #[test]
    fn identity_validation_rejects_each_bad_form() {
        assert_eq!(check_identity("sam"), Ok(()));
        assert_eq!(check_identity("a-b@host"), Ok(()));
        assert_eq!(check_identity(""), Err(IdentityFault::Empty));
        assert_eq!(check_identity("-sam"), Err(IdentityFault::LeadingDash));
        assert_eq!(check_identity("--as"), Err(IdentityFault::LeadingDash));
        assert_eq!(
            check_identity("sam\nroot"),
            Err(IdentityFault::ControlCharacter)
        );
        assert_eq!(
            check_identity("sam\u{1b}[0m"),
            Err(IdentityFault::ControlCharacter)
        );
        assert_eq!(check_identity(&"a".repeat(128)), Ok(()));
        assert_eq!(
            check_identity(&"a".repeat(129)),
            Err(IdentityFault::TooLong)
        );
        // 128 bytes counts UTF-8 bytes, not characters.
        assert_eq!(check_identity(&"é".repeat(65)), Err(IdentityFault::TooLong));
    }

    #[test]
    fn pi_is_laplace_smoothed_and_never_zero_or_one() {
        let entry = |attempts, successes| PriorEntry {
            index: 0,
            key: PriorKey {
                harness: "h".into(),
                model_ref: "m".into(),
                tier: Tier::Tier1,
                mode: DispatchMode::Subprocess,
            },
            attempts,
            successes,
            provenance: vec![],
        };
        assert_eq!(entry(5, 5).pi(), 6.0 / 7.0);
        assert_eq!(entry(5, 0).pi(), 1.0 / 7.0);
        assert_eq!(entry(8, 4).pi(), 0.5);
        for n in [5u32, 100, 4096] {
            assert!(entry(n, n).pi() < 1.0);
            assert!(entry(n, 0).pi() > 0.0);
        }
    }

    #[test]
    fn a_valid_entry_survives() {
        let v = validate_priors(vec![ParsedPrior::Parsed(raw(
            0,
            "codex",
            &[true, true, true, false, true],
        ))]);
        assert_eq!(v.entries.len(), 1);
        assert!(v.diagnostics.is_empty());
        assert_eq!(v.entries[0].attempts, 5);
        assert_eq!(v.entries[0].successes, 4);
    }

    #[test]
    fn below_the_floor_is_ignored_and_reported() {
        let v = validate_priors(vec![ParsedPrior::Parsed(raw(
            0,
            "codex",
            &[true, true, true, true],
        ))]);
        assert!(v.entries.is_empty());
        assert_eq!(codes(&v, 0), [PriorsCode::BelowEvidenceFloor]);
    }

    #[test]
    fn duplicate_keys_ignore_every_copy() {
        let five = [true; 5];
        let v = validate_priors(vec![
            ParsedPrior::Parsed(raw(0, "codex", &five)),
            ParsedPrior::Parsed(raw(1, "claude", &five)),
            ParsedPrior::Parsed(raw(2, "codex", &five)),
        ]);
        assert_eq!(v.entries.len(), 1);
        assert_eq!(v.entries[0].key.harness, "claude");
        assert_eq!(codes(&v, 0), [PriorsCode::DuplicateKey]);
        assert_eq!(codes(&v, 2), [PriorsCode::DuplicateKey]);
    }

    #[test]
    fn duplicate_provenance_is_ignored_whatever_passed_says() {
        let mut r = raw(0, "codex", &[true, true, true, true, false]);
        r.provenance[4].attempt_id = r.provenance[0].attempt_id;
        r.provenance[4].passed = false;
        let v = validate_priors(vec![ParsedPrior::Parsed(r)]);
        assert!(v.entries.is_empty());
        assert_eq!(codes(&v, 0), [PriorsCode::DuplicateProvenance]);
    }

    #[test]
    fn successes_and_count_mismatches_are_ignored() {
        let mut s = raw(0, "codex", &[true; 6]);
        s.successes = 5;
        let mut c = raw(1, "claude", &[true; 6]);
        c.attempts = 7;
        let v = validate_priors(vec![ParsedPrior::Parsed(s), ParsedPrior::Parsed(c)]);
        assert!(v.entries.is_empty());
        assert_eq!(codes(&v, 0), [PriorsCode::SuccessesMismatch]);
        assert_eq!(codes(&v, 1), [PriorsCode::CountMismatch]);
    }

    #[test]
    fn mode_and_model_rules_ignore_the_entry() {
        let mut native = raw(0, "codex", &[true; 5]);
        native.dispatch_mode = "native".into();
        let mut unverified = raw(1, "claude", &[true; 5]);
        unverified.model_ref = "unverified".into();
        let mut flag = raw(2, "pi", &[true; 5]);
        flag.model_ref = "--yolo".into();
        let v = validate_priors(vec![
            ParsedPrior::Parsed(native),
            ParsedPrior::Parsed(unverified),
            ParsedPrior::Parsed(flag),
        ]);
        assert!(v.entries.is_empty());
        assert_eq!(codes(&v, 0), [PriorsCode::NotSubprocess]);
        assert_eq!(codes(&v, 1), [PriorsCode::ModelInvalid]);
        assert_eq!(codes(&v, 2), [PriorsCode::ModelInvalid]);
    }

    #[test]
    fn a_faulty_entry_is_reported_and_others_survive() {
        let v = validate_priors(vec![
            ParsedPrior::Faulty(FaultyPrior {
                index: 0,
                label: EntryLabel::default(),
                faults: vec![(PriorsCode::UnknownEntryField, "pi".into())],
            }),
            ParsedPrior::Parsed(raw(1, "codex", &[true; 5])),
        ]);
        assert_eq!(v.entries.len(), 1);
        assert_eq!(codes(&v, 0), [PriorsCode::UnknownEntryField]);
    }

    #[test]
    fn a_faulty_copy_still_makes_its_key_a_duplicate() {
        let v = validate_priors(vec![
            ParsedPrior::Faulty(FaultyPrior {
                index: 0,
                label: EntryLabel {
                    harness: Some("codex".into()),
                    model_ref: Some("m".into()),
                    tier: Some("Tier2".into()),
                    dispatch_mode: Some("subprocess".into()),
                },
                faults: vec![(PriorsCode::UnknownEntryField, "pi".into())],
            }),
            ParsedPrior::Parsed(raw(1, "codex", &[true; 5])),
        ]);
        assert!(v.entries.is_empty());
        assert_eq!(codes(&v, 1), [PriorsCode::DuplicateKey]);
    }

    #[test]
    fn ignore_moves_a_surviving_entry_to_the_diagnostics() {
        let mut v = validate_priors(vec![ParsedPrior::Parsed(raw(0, "codex", &[true; 5]))]);
        v.ignore(0, PriorsCode::ProvenanceContradicts, "harness differs");
        v.ignore(9, PriorsCode::ProvenanceContradicts, "no such entry");
        assert!(v.entries.is_empty());
        assert_eq!(codes(&v, 0), [PriorsCode::ProvenanceContradicts]);
        assert_eq!(v.ignored().len(), 1);
    }

    #[test]
    fn every_code_round_trips_and_has_a_level() {
        for code in PriorsCode::ALL {
            assert_eq!(PriorsCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(PriorsCode::Revoked.level(), PriorsLevel::File);
        assert_eq!(PriorsCode::DuplicateKey.level(), PriorsLevel::Entry);
        assert_eq!(PriorsCode::NotACandidate.level(), PriorsLevel::Report);
    }

    #[test]
    fn display_strips_controls_and_format_characters() {
        assert_eq!(display_safe("ok"), "ok");
        assert_eq!(display_safe("a\u{1b}[31mb\n"), "a[31mb");
        assert_eq!(display_safe("evil\u{202e}gpj.exe"), "evilgpj.exe");
        assert_eq!(display_safe("zero\u{200b}width\u{feff}"), "zerowidth");
        assert_eq!(display_safe("tag\u{e0041}"), "tag");
    }
}
