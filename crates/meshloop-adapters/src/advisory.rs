//! Advisory routing priors: the file loader and the append-only promotion ledger
//! (ADR 0038, amendment 2026-11-10 for #139; learning-plane.md "Advisory input").
//!
//! The loader, not `schemas/advisory-priors.v1.schema.json`, is the authority. Every
//! fault is a diagnostic, never an error: a file-level fault means no priors, an
//! entry-level fault ignores that entry, and report codes change nothing.

use std::io::Read;
use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value};

use meshloop_domain::advisory::{
    AttemptRef, EntryLabel, FaultyPrior, ParsedPrior, PriorsCode, PriorsDiagnostic, RawPrior,
    ValidatedPriors, check_identity, parse_tier, validate_priors,
};
use meshloop_domain::digest::sha256_hex;
use meshloop_engine::ports::{AdvisoryPriorsSource, PriorsSnapshot, StoreError};

use crate::store::SqliteStore;

/// Largest priors file, in bytes (1 MiB). The stored BLOB has the same bound.
pub const MAX_PRIORS_BYTES: usize = 1_048_576;
/// The `schema` value of a v1 file.
pub const PRIORS_SCHEMA: &str = "meshloop/advisory-priors";
const MAX_ENTRIES: usize = 1024;
const MAX_COUNT: u64 = 4096;
const MAX_HARNESS_BYTES: usize = 64;
const MAX_GRAPH_ID_BYTES: usize = 128;
const TOP_FIELDS: [&str; 6] = [
    "schema",
    "version",
    "generated_at",
    "generator",
    "source",
    "priors",
];
const ENTRY_FIELDS: [&str; 7] = [
    "harness",
    "model_ref",
    "tier",
    "dispatch_mode",
    "attempts",
    "successes",
    "provenance",
];
const REF_FIELDS: [&str; 4] = ["graph_id", "task_id", "attempt_id", "passed"];

/// Reads a proposal once, at most [`MAX_PRIORS_BYTES`] + 1 bytes. The digest, the parse
/// and any stored BLOB all come from this one buffer; a longer file is reported
/// `too_large` by [`load_priors`] without reading the rest.
pub fn read_proposal(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_PRIORS_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// One file as the loader read it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoadedPriors {
    /// A file-level fault: no priors are applied.
    pub file_fault: Option<PriorsDiagnostic>,
    /// Entries with effect and entry-level diagnostics. Empty after a file fault.
    pub validated: ValidatedPriors,
    /// Report-only diagnostics (`unknown_field`).
    pub report: Vec<PriorsDiagnostic>,
}

impl LoadedPriors {
    fn fault(code: PriorsCode, detail: impl Into<String>) -> Self {
        Self {
            file_fault: Some(PriorsDiagnostic::file(code, detail)),
            ..Self::default()
        }
    }

    /// Every diagnostic: the file fault, then entry-level, then report-only.
    pub fn diagnostics(&self) -> Vec<PriorsDiagnostic> {
        self.file_fault
            .iter()
            .chain(&self.validated.diagnostics)
            .chain(&self.report)
            .cloned()
            .collect()
    }
}

/// Parses and validates one priors file. Never fails: see the module docs.
pub fn load_priors(bytes: &[u8]) -> LoadedPriors {
    if bytes.len() > MAX_PRIORS_BYTES {
        return LoadedPriors::fault(
            PriorsCode::TooLarge,
            format!("the file is over {MAX_PRIORS_BYTES} bytes"),
        );
    }
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            return LoadedPriors::fault(
                PriorsCode::NotJson,
                format!("not valid JSON at line {} column {}", e.line(), e.column()),
            );
        }
    };
    let Some(top) = value.as_object() else {
        return LoadedPriors::fault(PriorsCode::WrongSchema, "the file is not a JSON object");
    };
    if top.get("schema").and_then(Value::as_str) != Some(PRIORS_SCHEMA) {
        return LoadedPriors::fault(
            PriorsCode::WrongSchema,
            format!("schema is not {PRIORS_SCHEMA}"),
        );
    }
    if top.get("version").and_then(Value::as_u64) != Some(1) {
        return LoadedPriors::fault(PriorsCode::WrongVersion, "version is not 1");
    }
    if !top.get("generated_at").is_some_and(Value::is_string) {
        return LoadedPriors::fault(PriorsCode::WrongSchema, "generated_at is not a string");
    }
    let generator_ok = top
        .get("generator")
        .and_then(Value::as_object)
        .is_some_and(|g| {
            ["name", "version"].iter().all(|k| {
                g.get(*k)
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
            })
        });
    if !generator_ok {
        return LoadedPriors::fault(
            PriorsCode::WrongSchema,
            "generator must name its name and version",
        );
    }
    let Some(priors) = top.get("priors").and_then(Value::as_array) else {
        return LoadedPriors::fault(PriorsCode::WrongSchema, "priors is not an array");
    };
    if priors.len() > MAX_ENTRIES {
        return LoadedPriors::fault(
            PriorsCode::WrongSchema,
            format!("priors has more than {MAX_ENTRIES} entries"),
        );
    }
    let report = top
        .keys()
        .filter(|k| !TOP_FIELDS.contains(&k.as_str()))
        .map(|k| {
            PriorsDiagnostic::file(
                PriorsCode::UnknownField,
                format!("top-level field `{k}` is not defined in v1 and is ignored"),
            )
        })
        .collect();
    let parsed = priors
        .iter()
        .enumerate()
        .map(|(index, v)| parse_entry(index, v))
        .collect();
    LoadedPriors {
        file_fault: None,
        validated: validate_priors(parsed),
        report,
    }
}

fn label_of(obj: &Map<String, Value>) -> EntryLabel {
    let text = |k: &str| obj.get(k).and_then(Value::as_str).map(String::from);
    EntryLabel {
        harness: text("harness"),
        model_ref: text("model_ref"),
        tier: text("tier"),
        dispatch_mode: text("dispatch_mode"),
    }
}

fn parse_entry(index: usize, value: &Value) -> ParsedPrior {
    let Some(obj) = value.as_object() else {
        return ParsedPrior::Faulty(FaultyPrior {
            index,
            label: EntryLabel::default(),
            faults: vec![(
                PriorsCode::MalformedEntry,
                "the entry is not an object".into(),
            )],
        });
    };
    let mut faults: Vec<(PriorsCode, String)> = Vec::new();
    for key in obj.keys().filter(|k| !ENTRY_FIELDS.contains(&k.as_str())) {
        faults.push((
            PriorsCode::UnknownEntryField,
            format!("entry field `{key}` is not defined in v1"),
        ));
    }
    let mut malformed = |field: &str, why: &str| {
        faults.push((PriorsCode::MalformedEntry, format!("{field} {why}")));
    };
    let harness = obj.get("harness").and_then(Value::as_str);
    if !harness.is_some_and(|h| !h.is_empty() && h.len() <= MAX_HARNESS_BYTES) {
        malformed("harness", "must be a string of 1 to 64 bytes");
    }
    let model_ref = obj.get("model_ref").and_then(Value::as_str);
    if model_ref.is_none() {
        malformed("model_ref", "must be a string");
    }
    let tier = obj.get("tier").and_then(Value::as_str).and_then(parse_tier);
    if tier.is_none() {
        malformed("tier", "must be Tier1, Tier2 or Tier3");
    }
    let dispatch_mode = obj.get("dispatch_mode").and_then(Value::as_str);
    if dispatch_mode.is_none() {
        malformed("dispatch_mode", "must be a string");
    }
    let count = |k: &str| {
        obj.get(k)
            .and_then(Value::as_u64)
            .filter(|n| *n <= MAX_COUNT)
    };
    let attempts = count("attempts");
    if attempts.is_none() {
        malformed("attempts", "must be an integer from 0 to 4096");
    }
    let successes = count("successes");
    if successes.is_none() {
        malformed("successes", "must be an integer from 0 to 4096");
    }
    let mut provenance = Vec::new();
    match obj.get("provenance").and_then(Value::as_array) {
        Some(refs) if refs.len() as u64 <= MAX_COUNT => {
            for (i, r) in refs.iter().enumerate() {
                match parse_ref(r) {
                    Ok(reference) => provenance.push(reference),
                    Err((code, why)) => faults.push((code, format!("provenance[{i}] {why}"))),
                }
            }
        }
        _ => malformed("provenance", "must be an array of at most 4096 references"),
    }
    match (harness, model_ref, tier, dispatch_mode, attempts, successes) {
        (
            Some(harness),
            Some(model_ref),
            Some(tier),
            Some(mode),
            Some(attempts),
            Some(successes),
        ) if faults.is_empty() => ParsedPrior::Parsed(RawPrior {
            index,
            harness: harness.into(),
            model_ref: model_ref.into(),
            tier,
            dispatch_mode: mode.into(),
            attempts,
            successes,
            provenance,
        }),
        _ => ParsedPrior::Faulty(FaultyPrior {
            index,
            label: label_of(obj),
            faults,
        }),
    }
}

fn parse_ref(value: &Value) -> Result<AttemptRef, (PriorsCode, String)> {
    let malformed = |why: &str| (PriorsCode::MalformedEntry, why.to_string());
    let obj = value
        .as_object()
        .ok_or_else(|| malformed("is not an object"))?;
    if let Some(key) = obj.keys().find(|k| !REF_FIELDS.contains(&k.as_str())) {
        return Err((
            PriorsCode::UnknownEntryField,
            format!("field `{key}` is not defined in v1"),
        ));
    }
    let graph_id = obj
        .get("graph_id")
        .and_then(Value::as_str)
        .filter(|g| !g.is_empty() && g.len() <= MAX_GRAPH_ID_BYTES)
        .ok_or_else(|| malformed("graph_id must be a string of 1 to 128 bytes"))?;
    let task_id = obj
        .get("task_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed("task_id must be a non-negative integer"))?;
    let attempt_id = obj
        .get("attempt_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed("attempt_id must be a non-negative integer"))?;
    let passed = obj
        .get("passed")
        .and_then(Value::as_bool)
        .ok_or_else(|| malformed("passed must be a boolean"))?;
    Ok(AttemptRef {
        graph_id: graph_id.into(),
        task_id,
        attempt_id,
        passed,
    })
}

/// One ignored entry as the ledger row records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredEntry {
    pub index: usize,
    pub code: PriorsCode,
    pub label: EntryLabel,
}

impl IgnoredEntry {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "index": self.index,
            "code": self.code.as_str(),
            "harness": self.label.harness,
            "model_ref": self.label.model_ref,
            "tier": self.label.tier,
            "dispatch_mode": self.label.dispatch_mode,
        })
    }
}

/// The ignored entries of a validated file, one per (entry, code), for the ledger row.
pub fn ignored_entries(validated: &ValidatedPriors) -> Vec<IgnoredEntry> {
    validated
        .ignored()
        .into_iter()
        .filter_map(|d| {
            Some(IgnoredEntry {
                index: d.entry?,
                code: d.code,
                label: d.label.clone(),
            })
        })
        .collect()
}

fn ignored_json(entries: &[IgnoredEntry]) -> String {
    Value::Array(entries.iter().map(IgnoredEntry::to_json).collect()).to_string()
}

fn parse_ignored(raw: &str) -> Option<Vec<IgnoredEntry>> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    serde_json::from_str::<Value>(raw)
        .ok()?
        .as_array()?
        .iter()
        .map(|v| {
            Some(IgnoredEntry {
                index: usize::try_from(v.get("index")?.as_u64()?).ok()?,
                code: PriorsCode::parse(v.get("code")?.as_str()?)?,
                label: EntryLabel {
                    harness: text(v, "harness"),
                    model_ref: text(v, "model_ref"),
                    tier: text(v, "tier"),
                    dispatch_mode: text(v, "dispatch_mode"),
                },
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionAction {
    Accept,
    Revoke,
}

impl PromotionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Revoke => "revoke",
        }
    }
}

/// One ledger row. `bytes` is set on `accept` rows only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionRow {
    pub seq: i64,
    pub action: PromotionAction,
    /// The accepted file's SHA-256; on a revoke row, the digest it revoked, if any.
    pub priors_sha256: Option<String>,
    pub bytes: Option<Vec<u8>>,
    pub identity: String,
    pub recorded_at: String,
    pub source_path: Option<String>,
    pub resolved_refs: u64,
    pub unresolved_refs: u64,
    pub ignored_json: String,
}

/// What one `accept` appends. `bytes` is the single buffer the file was read into.
pub struct PriorsAccept<'a> {
    pub bytes: &'a [u8],
    pub identity: &'a str,
    pub source_path: &'a str,
    pub resolved_refs: u64,
    pub unresolved_refs: u64,
    pub ignored: &'a [IgnoredEntry],
}

/// The priors in effect, read from one ledger row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PriorsState {
    pub row: Option<PromotionRow>,
    /// The stored file, loaded, with the row's ignored entries applied. `None` when no
    /// file is stored or it could not be read back.
    pub loaded: Option<LoadedPriors>,
    /// Why no priors are in effect, when none are.
    pub fault: Option<PriorsDiagnostic>,
    pub snapshot: Option<PriorsSnapshot>,
}

impl PriorsState {
    pub fn in_effect(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Every diagnostic: why nothing is in effect, then the stored file's.
    pub fn diagnostics(&self) -> Vec<PriorsDiagnostic> {
        let mut out: Vec<PriorsDiagnostic> = self.fault.iter().cloned().collect();
        if let Some(loaded) = &self.loaded {
            for d in loaded.diagnostics() {
                if self.fault.as_ref() != Some(&d) {
                    out.push(d);
                }
            }
        }
        out
    }
}

fn io(e: impl std::fmt::Display) -> StoreError {
    StoreError::Io(e.to_string())
}

const ROW_COLUMNS: &str = "seq, action, priors_sha256, bytes, identity, recorded_at, source_path, resolved_refs, unresolved_refs, ignored_json";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PromotionRow> {
    let action: String = r.get(1)?;
    Ok(PromotionRow {
        seq: r.get(0)?,
        action: if action == "accept" {
            PromotionAction::Accept
        } else {
            PromotionAction::Revoke
        },
        priors_sha256: r.get(2)?,
        bytes: r.get(3)?,
        identity: r.get(4)?,
        recorded_at: r.get(5)?,
        source_path: r.get(6)?,
        resolved_refs: r.get::<_, i64>(7)?.max(0) as u64,
        unresolved_refs: r.get::<_, i64>(8)?.max(0) as u64,
        ignored_json: r.get(9)?,
    })
}

impl SqliteStore {
    fn writable_ledger(&self, identity: &str) -> Result<(), StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }
        if !self.advisory_schema {
            return Err(StoreError::SchemaUpgradeRequired);
        }
        check_identity(identity).map_err(|f| StoreError::Corrupt(f.message().into()))
    }

    /// Appends one `accept` row in one transaction. Returns its sequence number.
    pub fn append_priors_accept(&mut self, accept: &PriorsAccept<'_>) -> Result<i64, StoreError> {
        self.writable_ledger(accept.identity)?;
        if accept.bytes.len() > MAX_PRIORS_BYTES {
            return Err(StoreError::Corrupt("priors file is over 1 MiB".into()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(io)?;
        tx.execute(
            "INSERT INTO advisory_promotions (action, priors_sha256, bytes, identity, recorded_at, source_path, resolved_refs, unresolved_refs, ignored_json)
             VALUES ('accept', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                sha256_hex(accept.bytes),
                accept.bytes,
                accept.identity,
                crate::store::now_stamp(),
                accept.source_path,
                i64::try_from(accept.resolved_refs).unwrap_or(i64::MAX),
                i64::try_from(accept.unresolved_refs).unwrap_or(i64::MAX),
                ignored_json(accept.ignored),
            ],
        )
        .map_err(io)?;
        let seq = tx.last_insert_rowid();
        tx.commit().map_err(io)?;
        Ok(seq)
    }

    /// Appends one `revoke` row. It records the digest it revoked, if an accept was in
    /// the ledger's latest row. Returns the row's sequence number and that digest.
    pub fn append_priors_revoke(
        &mut self,
        identity: &str,
    ) -> Result<(i64, Option<String>), StoreError> {
        self.writable_ledger(identity)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(io)?;
        let revoked: Option<String> = tx
            .query_row(
                "SELECT CASE WHEN action='accept' THEN priors_sha256 END FROM advisory_promotions ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(io)?
            .flatten();
        tx.execute(
            "INSERT INTO advisory_promotions (action, priors_sha256, bytes, identity, recorded_at, source_path, resolved_refs, unresolved_refs, ignored_json)
             VALUES ('revoke', ?1, NULL, ?2, ?3, NULL, 0, 0, '[]')",
            params![revoked, identity, crate::store::now_stamp()],
        )
        .map_err(io)?;
        let seq = tx.last_insert_rowid();
        tx.commit().map_err(io)?;
        Ok((seq, revoked))
    }

    /// The latest ledger row, in one read. `None` with no row or a pre-v9 read-only store.
    pub fn latest_promotion(&self) -> Result<Option<PromotionRow>, StoreError> {
        if !self.advisory_schema {
            return Ok(None);
        }
        self.conn
            .query_row(
                &format!("SELECT {ROW_COLUMNS} FROM advisory_promotions ORDER BY seq DESC LIMIT 1"),
                [],
                read_row,
            )
            .optional()
            .map_err(io)
    }

    /// Up to `limit` latest rows, newest first, without their bytes.
    pub fn promotion_history(&self, limit: u32) -> Result<Vec<PromotionRow>, StoreError> {
        if !self.advisory_schema {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT seq, action, priors_sha256, NULL, identity, recorded_at, source_path, resolved_refs, unresolved_refs, ignored_json
                 FROM advisory_promotions ORDER BY seq DESC LIMIT ?1",
            )
            .map_err(io)?;
        stmt.query_map([limit], read_row)
            .map_err(io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io)
    }

    /// `(attempts routed, attempts whose top candidate changed)` under one digest.
    pub fn advisory_routing_counts(&self, priors_sha256: &str) -> Result<(u64, u64), StoreError> {
        if !self.advisory_schema {
            return Ok((0, 0));
        }
        self.conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(changed_top), 0) FROM advisory_routing WHERE priors_sha256 = ?1",
                [priors_sha256],
                |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64)),
            )
            .map_err(io)
    }

    /// The `advisory_routing` row of one attempt: `(priors_sha256, changed_top)`.
    pub fn advisory_routing(
        &self,
        attempt_id: meshloop_domain::evidence::AttemptId,
    ) -> Result<Option<(String, bool)>, StoreError> {
        if !self.advisory_schema {
            return Ok(None);
        }
        self.conn
            .query_row(
                "SELECT priors_sha256, changed_top FROM advisory_routing WHERE attempt_id = ?1",
                [attempt_id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(io)
    }

    /// The priors in effect, from one read of the latest ledger row. Entries the row's
    /// `accept` ignored stay ignored; provenance is never resolved again here.
    pub fn priors_state(&self) -> Result<PriorsState, StoreError> {
        let Some(row) = self.latest_promotion()? else {
            return Ok(PriorsState {
                fault: Some(PriorsDiagnostic::file(
                    PriorsCode::NotPromoted,
                    "no priors file has been accepted",
                )),
                ..PriorsState::default()
            });
        };
        let mut state = PriorsState {
            row: Some(row.clone()),
            ..PriorsState::default()
        };
        if row.action == PromotionAction::Revoke {
            state.fault = Some(PriorsDiagnostic::file(
                PriorsCode::Revoked,
                format!("revoked in ledger row {}", row.seq),
            ));
            return Ok(state);
        }
        let (Some(bytes), Some(digest)) = (&row.bytes, &row.priors_sha256) else {
            state.fault = Some(PriorsDiagnostic::file(
                PriorsCode::DigestMismatch,
                "the accept row holds no file",
            ));
            return Ok(state);
        };
        if &sha256_hex(bytes) != digest {
            state.fault = Some(PriorsDiagnostic::file(
                PriorsCode::DigestMismatch,
                "the stored file does not hash to the row's digest",
            ));
            return Ok(state);
        }
        let Some(ignored) = parse_ignored(&row.ignored_json) else {
            state.fault = Some(PriorsDiagnostic::file(
                PriorsCode::LedgerCorrupt,
                "the row's ignored entries are unreadable",
            ));
            return Ok(state);
        };
        let mut loaded = load_priors(bytes);
        if let Some(fault) = &loaded.file_fault {
            state.fault = Some(fault.clone());
            state.loaded = Some(loaded);
            return Ok(state);
        }
        for item in &ignored {
            loaded.validated.ignore(
                item.index,
                item.code,
                format!("ignored at accept ({})", item.code.as_str()),
            );
        }
        state.snapshot = Some(PriorsSnapshot {
            sequence: row.seq,
            priors_sha256: digest.clone(),
            entries: loaded.validated.entries.clone(),
        });
        state.loaded = Some(loaded);
        Ok(state)
    }
}

impl AdvisoryPriorsSource for SqliteStore {
    fn snapshot(&self) -> Result<Option<PriorsSnapshot>, StoreError> {
        Ok(self.priors_state()?.snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshloop_domain::advisory::{DispatchMode, PriorsLevel};
    use meshloop_domain::task_graph::Tier;

    pub(crate) fn entry_json(harness: &str, model: &str, tier: &str, passed: &[bool]) -> Value {
        serde_json::json!({
            "harness": harness,
            "model_ref": model,
            "tier": tier,
            "dispatch_mode": "subprocess",
            "attempts": passed.len(),
            "successes": passed.iter().filter(|p| **p).count(),
            "provenance": passed.iter().enumerate().map(|(i, p)| serde_json::json!({
                "graph_id": "g",
                "task_id": 1,
                "attempt_id": i + 1,
                "passed": p,
            })).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn file_json(entries: Vec<Value>) -> Value {
        serde_json::json!({
            "schema": PRIORS_SCHEMA,
            "version": 1,
            "generated_at": "2026-11-11T00:00:00Z",
            "generator": {"name": "nodecode-meshloop", "version": "0.1.0"},
            "priors": entries,
        })
    }

    fn bytes(v: &Value) -> Vec<u8> {
        serde_json::to_vec(v).unwrap()
    }

    fn codes(loaded: &LoadedPriors) -> Vec<&'static str> {
        loaded
            .diagnostics()
            .iter()
            .map(|d| d.code.as_str())
            .collect()
    }

    #[test]
    fn a_valid_file_loads_every_entry() {
        let loaded = load_priors(&bytes(&file_json(vec![
            entry_json("codex", "gpt", "Tier2", &[true, true, true, false, true]),
            entry_json("claude", "opus", "Tier3", &[true; 6]),
        ])));
        assert_eq!(loaded.file_fault, None);
        assert!(
            loaded.diagnostics().is_empty(),
            "{:?}",
            loaded.diagnostics()
        );
        assert_eq!(loaded.validated.entries.len(), 2);
        let e = &loaded.validated.entries[0];
        assert_eq!(e.key.harness, "codex");
        assert_eq!(e.key.tier, Tier::Tier2);
        assert_eq!(e.key.mode, DispatchMode::Subprocess);
        assert_eq!((e.attempts, e.successes), (5, 4));
    }

    /// LP test 12: file-level faults are diagnostics, never errors.
    #[test]
    fn file_level_faults_apply_no_priors() {
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (b"{not json".to_vec(), "not_json"),
            (vec![0xff, 0xfe], "not_json"),
            (b"[]".to_vec(), "wrong_schema"),
            (
                bytes(&serde_json::json!({"schema": "other", "version": 1})),
                "wrong_schema",
            ),
            (
                {
                    let mut v = file_json(vec![]);
                    v["version"] = 2.into();
                    bytes(&v)
                },
                "wrong_version",
            ),
            (
                {
                    let mut v = file_json(vec![]);
                    v["priors"] = "x".into();
                    bytes(&v)
                },
                "wrong_schema",
            ),
            (vec![b' '; MAX_PRIORS_BYTES + 1], "too_large"),
        ];
        for (input, code) in cases {
            let loaded = load_priors(&input);
            let fault = loaded.file_fault.as_ref().expect(code);
            assert_eq!(fault.code.as_str(), code);
            assert_eq!(fault.code.level(), PriorsLevel::File);
            assert!(loaded.validated.entries.is_empty());
        }
    }

    #[test]
    fn unknown_top_level_fields_are_reported_and_ignored() {
        let mut v = file_json(vec![entry_json("codex", "gpt", "Tier2", &[true; 5])]);
        v["tier_hints"] = serde_json::json!([{"tier": "Tier3"}]);
        v["notes"] = "x".into();
        let loaded = load_priors(&bytes(&v));
        assert_eq!(loaded.file_fault, None);
        assert_eq!(loaded.validated.entries.len(), 1);
        let mut c = codes(&loaded);
        c.sort_unstable();
        assert_eq!(c, ["unknown_field", "unknown_field"]);
    }

    /// LP test 11 (unknown field part), in an entry and in a reference.
    #[test]
    fn an_unknown_field_in_an_entry_or_reference_ignores_the_entry() {
        let mut in_entry = entry_json("codex", "gpt", "Tier2", &[true; 5]);
        in_entry["pi"] = 0.9.into();
        let mut in_ref = entry_json("claude", "opus", "Tier2", &[true; 5]);
        in_ref["provenance"][0]["worker_output"] = "done!".into();
        let ok = entry_json("pi", "m", "Tier2", &[true; 5]);
        let loaded = load_priors(&bytes(&file_json(vec![in_entry, in_ref, ok])));
        assert_eq!(loaded.validated.entries.len(), 1);
        assert_eq!(loaded.validated.entries[0].key.harness, "pi");
        assert_eq!(
            codes(&loaded),
            ["unknown_entry_field", "unknown_entry_field"]
        );
    }

    #[test]
    fn a_malformed_entry_is_ignored_with_a_diagnostic() {
        let mut bad_tier = entry_json("codex", "gpt", "Tier4", &[true; 5]);
        bad_tier["attempts"] = 5000.into();
        let loaded = load_priors(&bytes(&file_json(vec![bad_tier, "x".into()])));
        assert!(loaded.validated.entries.is_empty());
        assert!(codes(&loaded).iter().all(|c| *c == "malformed_entry"));
        assert_eq!(codes(&loaded).len(), 3);
    }

    #[test]
    fn ignored_entries_round_trip_through_the_row_json() {
        let loaded = load_priors(&bytes(&file_json(vec![
            entry_json("codex", "gpt", "Tier2", &[true; 4]),
            entry_json("claude", "opus", "Tier2", &[true; 5]),
        ])));
        let ignored = ignored_entries(&loaded.validated);
        assert_eq!(ignored.len(), 1);
        assert_eq!(ignored[0].index, 0);
        assert_eq!(ignored[0].code, PriorsCode::BelowEvidenceFloor);
        assert_eq!(parse_ignored(&ignored_json(&ignored)), Some(ignored));
        assert_eq!(parse_ignored("not json"), None);
        assert_eq!(parse_ignored("[{\"index\":0,\"code\":\"nope\"}]"), None);
    }

    #[test]
    fn read_proposal_reads_at_most_one_byte_past_the_bound() {
        let dir = std::env::temp_dir().join(format!("meshloop-priors-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.json");
        std::fs::write(&path, vec![b' '; MAX_PRIORS_BYTES + 100]).unwrap();
        let read = read_proposal(&path).unwrap();
        assert_eq!(read.len(), MAX_PRIORS_BYTES + 1);
        assert_eq!(
            load_priors(&read).file_fault.map(|f| f.code),
            Some(PriorsCode::TooLarge)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn accept(store: &mut SqliteStore, raw: &[u8], ignored: &[IgnoredEntry]) -> i64 {
        store
            .append_priors_accept(&PriorsAccept {
                bytes: raw,
                identity: "sam",
                source_path: "proposal.json",
                resolved_refs: 0,
                unresolved_refs: 5,
                ignored,
            })
            .unwrap()
    }

    #[test]
    fn no_row_and_revoke_mean_no_snapshot() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let state = store.priors_state().unwrap();
        assert_eq!(state.fault.map(|f| f.code), Some(PriorsCode::NotPromoted));
        assert_eq!(store.snapshot().unwrap(), None);

        let raw = bytes(&file_json(vec![entry_json(
            "codex", "gpt", "Tier2", &[true; 5],
        )]));
        let seq = accept(&mut store, &raw, &[]);
        let snapshot = store.snapshot().unwrap().expect("in effect");
        assert_eq!(snapshot.sequence, seq);
        assert_eq!(snapshot.priors_sha256, sha256_hex(&raw));
        assert_eq!(snapshot.entries.len(), 1);

        // LP test: revoke takes effect immediately, on the next read.
        let (_, revoked) = store.append_priors_revoke("sam").unwrap();
        assert_eq!(revoked, Some(sha256_hex(&raw)));
        assert_eq!(store.snapshot().unwrap(), None);
        assert_eq!(
            store.priors_state().unwrap().fault.map(|f| f.code),
            Some(PriorsCode::Revoked)
        );
        // A later accept is in effect again.
        accept(&mut store, &raw, &[]);
        assert!(store.snapshot().unwrap().is_some());
    }

    /// LP test 16 (routing half): an entry ignored at accept is never applied.
    #[test]
    fn the_snapshot_drops_entries_the_accept_row_ignored() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let raw = bytes(&file_json(vec![
            entry_json("codex", "gpt", "Tier2", &[true; 5]),
            entry_json("claude", "opus", "Tier2", &[true; 5]),
        ]));
        let ignored = [IgnoredEntry {
            index: 1,
            code: PriorsCode::ProvenanceContradicts,
            label: EntryLabel::default(),
        }];
        accept(&mut store, &raw, &ignored);
        let state = store.priors_state().unwrap();
        let snapshot = state.snapshot.clone().unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].key.harness, "codex");
        assert!(
            state
                .diagnostics()
                .iter()
                .any(|d| d.code == PriorsCode::ProvenanceContradicts && d.entry == Some(1))
        );
    }

    /// LP tests 5 and 16 (loader side): a resolved reference whose attempt did not pass
    /// the model, ran another model (`unverified` included), has no check result, or
    /// disagrees with `passed` contradicts its entry. Unknown references are unresolved.
    #[test]
    fn provenance_contradictions_cover_each_exclusion() {
        use meshloop_domain::advisory::{AttemptRef, PriorEntry, PriorKey};
        use meshloop_domain::evidence::AttemptId;
        use meshloop_domain::state::PlanState;
        use meshloop_domain::task_graph::TaskId;
        use meshloop_engine::ports::{AttemptRow, RunRow, RunStore};

        let mut store = SqliteStore::open_in_memory().unwrap();
        let nodes: Vec<Value> = (1..=6)
            .map(|id| serde_json::json!({"id": id, "description": "x", "depends_on": [], "tier": "Tier2"}))
            .collect();
        store
            .save_run(&RunRow {
                graph_id: "g".into(),
                plan_state: PlanState::PlanAccepted,
                run_base: "abc".into(),
                integrate_ref: "meshloop/g/integrate".into(),
                plan_json: serde_json::json!({"graph_id": "g", "nodes": nodes}).to_string(),
                plan_id: "x".into(),
                created_at: "0".into(),
                review_note: None,
            })
            .unwrap();
        // (task, model, model_passed, final event)
        let cases: [(u32, &str, Option<bool>, &str); 6] = [
            (1, "gpt", Some(true), "DeterministicChecksPassed"),
            (2, "gpt", Some(false), "DeterministicChecksPassed"),
            (3, "gpt", None, "DeterministicChecksPassed"),
            (4, "unverified", Some(true), "DeterministicChecksPassed"),
            (5, "gpt", Some(true), "HarnessCrashedOrTimeout"),
            (6, "gpt", Some(true), "DeterministicChecksFailed"),
        ];
        for (task, model, model_passed, last) in cases {
            store
                .save_attempt(&AttemptRow {
                    attempt_id: AttemptId(task),
                    graph_id: "g".into(),
                    task_id: TaskId(task),
                    harness: Some("codex".into()),
                    model_ref: Some(model.into()),
                    worktree_path: None,
                    pid: None,
                    image_name: None,
                    pane_id: None,
                    started_at: Some("1".into()),
                    ended_at: Some("2".into()),
                    outcome: Some("base:abc".into()),
                    model_passed,
                })
                .unwrap();
            let (from, to) = match last {
                "HarnessCrashedOrTimeout" => ("Running", "Failed"),
                "DeterministicChecksFailed" => ("Verifying", "Failed"),
                _ => ("Verifying", "AwaitingReview"),
            };
            store
                .conn
                .execute(
                    "INSERT INTO events (graph_id, task_id, attempt_id, from_state, to_state, event_type, executor, occurred_at)
                     VALUES ('g', ?1, ?1, ?2, ?3, ?4, 't', '1')",
                    params![task, from, to, last],
                )
                .unwrap();
        }
        let entry = |index: usize, task: u64, passed: bool, model: &str| PriorEntry {
            index,
            key: PriorKey {
                harness: "codex".into(),
                model_ref: model.into(),
                tier: Tier::Tier2,
                mode: DispatchMode::Subprocess,
            },
            attempts: 1,
            successes: u32::from(passed),
            provenance: vec![AttemptRef {
                graph_id: "g".into(),
                task_id: task,
                attempt_id: task,
                passed,
            }],
        };
        let entries = vec![
            entry(0, 1, true, "gpt"),
            entry(1, 2, true, "gpt"),
            entry(2, 3, true, "gpt"),
            entry(3, 4, true, "gpt"),
            entry(4, 5, false, "gpt"),
            entry(5, 6, true, "gpt"),
            // Wrong graph for attempt 1, and an attempt that does not exist.
            PriorEntry {
                provenance: vec![
                    AttemptRef {
                        graph_id: "other".into(),
                        task_id: 1,
                        attempt_id: 1,
                        passed: true,
                    },
                    AttemptRef {
                        graph_id: "g".into(),
                        task_id: 1,
                        attempt_id: 99,
                        passed: true,
                    },
                ],
                ..entry(6, 1, true, "gpt")
            },
            // Right attempt, wrong tier.
            PriorEntry {
                key: PriorKey {
                    tier: Tier::Tier3,
                    ..entry(7, 1, true, "gpt").key
                },
                ..entry(7, 1, true, "gpt")
            },
        ];
        let report = meshloop_engine::priors::resolve_provenance(&store, &entries).unwrap();
        let why = |index: usize| {
            report
                .contradicted
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, w)| w.clone())
        };
        assert_eq!(why(0), None);
        assert!(why(1).unwrap().contains("model_passed"));
        assert!(why(2).unwrap().contains("model_passed"));
        assert!(why(3).unwrap().contains("another model"));
        assert!(why(4).unwrap().contains("check_result null"));
        assert!(why(5).unwrap().contains("check_result failed"));
        assert_eq!(why(6), None);
        assert!(why(7).unwrap().contains("another tier"));
        assert_eq!(report.resolved, 7);
        assert_eq!(report.unresolved, 2);
        assert_eq!(report.unresolved_by_entry, vec![(6, 2)]);
    }

    /// LP test 15: the ledger is append-only.
    #[test]
    fn update_and_delete_on_the_ledger_are_refused() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let raw = bytes(&file_json(vec![]));
        accept(&mut store, &raw, &[]);
        for sql in [
            "UPDATE advisory_promotions SET identity='mallory'",
            "UPDATE advisory_promotions SET bytes=X'00'",
            "DELETE FROM advisory_promotions",
        ] {
            let err = store.conn.execute(sql, []).unwrap_err().to_string();
            assert!(err.contains("append-only"), "{sql}: {err}");
        }
        assert_eq!(store.latest_promotion().unwrap().unwrap().identity, "sam");
    }

    #[test]
    fn a_tampered_blob_is_a_digest_mismatch() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let raw = bytes(&file_json(vec![entry_json(
            "codex", "gpt", "Tier2", &[true; 5],
        )]));
        accept(&mut store, &raw, &[]);
        // Simulate corruption under the triggers.
        store
            .conn
            .execute_batch(
                "DROP TRIGGER advisory_promotions_no_update;
                 UPDATE advisory_promotions SET bytes = X'7B7D';",
            )
            .unwrap();
        let state = store.priors_state().unwrap();
        assert_eq!(
            state.fault.map(|f| f.code),
            Some(PriorsCode::DigestMismatch)
        );
        assert!(state.snapshot.is_none());
    }

    #[test]
    fn the_ledger_refuses_bad_identities_and_read_only_writes() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        for bad in ["", "-x", "a\nb"] {
            assert!(store.append_priors_revoke(bad).is_err(), "{bad:?}");
        }
        assert!(store.latest_promotion().unwrap().is_none());
    }
}
