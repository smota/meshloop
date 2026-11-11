//! `meshloop priors accept|revoke|show|check` (ADR 0038, amendment 2026-11-10 for #139).
//!
//! `show` and `check` only read (ADR 0034): they open an existing ledger read-only and
//! never create one. Every loader diagnostic is listed and none changes the exit code.
//! `accept` reads the file once within a bounded size, refuses a file-level fault,
//! ignores entries whose resolved references contradict them, and appends one ledger
//! row; `revoke` appends a revoke row. Neither is reachable from MCP or the cell.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

use meshloop_adapters::advisory::{
    IgnoredEntry, LoadedPriors, PriorsAccept, PromotionRow, ignored_entries, load_priors,
    read_proposal,
};
use meshloop_adapters::store::SqliteStore;
use meshloop_domain::advisory::{
    EntryLabel, PRIOR_EVIDENCE_FLOOR, PriorEntry, PriorsCode, PriorsDiagnostic, display_safe,
    tier_name,
};
use meshloop_domain::digest::sha256_hex;
use meshloop_domain::policy::tier_fits;
use meshloop_engine::origin::Origin;
use meshloop_engine::router::Candidate;

use crate::args::PriorsAction;
use crate::json_out;

pub struct PriorsRequest {
    pub action: PriorsAction,
    pub file: Option<PathBuf>,
    pub identity: Option<String>,
    pub config: Option<PathBuf>,
    pub db: Option<PathBuf>,
    pub json: bool,
    pub origin: Origin,
}

fn command(action: PriorsAction) -> String {
    format!("meshloop:priors-{}", action.as_str())
}

pub fn cmd_priors(req: PriorsRequest) -> ExitCode {
    let name = command(req.action);
    let candidates = match configured_candidates(req.config.as_deref()) {
        Ok(c) => c,
        Err(e) => return fail(&name, &req, 2, e, Value::Null),
    };
    let db = req
        .db
        .clone()
        .unwrap_or_else(|| crate::compose::default_db(&crate::repo_root()));
    match req.action {
        PriorsAction::Show => show(&name, &req, &db, candidates.as_deref()),
        PriorsAction::Check => check(&name, &req, &db, candidates.as_deref()),
        PriorsAction::Accept => accept(&name, &req, &db, candidates.as_deref()),
        PriorsAction::Revoke => revoke(&name, &req, &db),
    }
}

/// The configured routing candidates, for `not_a_candidate`. Without a discoverable
/// config the report is skipped; an explicit `--config` that fails is an error.
fn configured_candidates(explicit: Option<&Path>) -> Result<Option<Vec<Candidate>>, String> {
    let path = match crate::config::discover(explicit) {
        Ok(p) => p,
        Err(_) if explicit.is_none() => return Ok(None),
        Err(_) => return Err("no config found".into()),
    };
    let cfg = crate::config::load(&path).map_err(|e| format!("failed to load config: {e:?}"))?;
    let mut out = Vec::new();
    for name in &cfg.selected_harnesses {
        if let Some(hc) = cfg.harnesses.get(name) {
            out.extend(hc.model_entries().into_iter().map(|m| Candidate {
                harness: name.clone(),
                model_tier: crate::compose::parse_model_tier(&m.model_tier),
                model_ref: m.model_ref,
            }));
        }
    }
    Ok(Some(out))
}

fn fail(name: &str, req: &PriorsRequest, code: u8, error: String, data: Value) -> ExitCode {
    if req.json {
        println!(
            "{}",
            json_out::report(name, req.origin.clone(), Some(error), data)
        );
    } else {
        eprintln!("{name}: {error}");
    }
    ExitCode::from(code)
}

fn is_candidate(entry: &PriorEntry, candidates: &[Candidate]) -> bool {
    candidates.iter().any(|c| {
        c.harness == entry.key.harness
            && c.model_ref == entry.key.model_ref
            && tier_fits(entry.key.tier, c.model_tier)
    })
}

/// `not_a_candidate` for each entry no configured candidate matches (report only).
fn candidate_report(
    entries: &[PriorEntry],
    candidates: Option<&[Candidate]>,
) -> Vec<PriorsDiagnostic> {
    let Some(candidates) = candidates else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|e| !is_candidate(e, candidates))
        .map(|e| PriorsDiagnostic {
            code: PriorsCode::NotACandidate,
            entry: Some(e.index),
            label: label(e),
            detail: "no configured candidate matches this entry; it is inert".into(),
        })
        .collect()
}

fn label(e: &PriorEntry) -> EntryLabel {
    EntryLabel {
        harness: Some(e.key.harness.clone()),
        model_ref: Some(e.key.model_ref.clone()),
        tier: Some(tier_name(e.key.tier).into()),
        dispatch_mode: Some(e.key.mode.as_str().into()),
    }
}

fn opt_safe(s: &Option<String>) -> Value {
    s.as_deref()
        .map(display_safe)
        .map_or(Value::Null, Value::from)
}

fn diagnostic_json(d: &PriorsDiagnostic) -> Value {
    json!({
        "code": d.code.as_str(),
        "level": d.code.level().as_str(),
        "entry": d.entry,
        "harness": opt_safe(&d.label.harness),
        "model_ref": opt_safe(&d.label.model_ref),
        "tier": opt_safe(&d.label.tier),
        "dispatch_mode": opt_safe(&d.label.dispatch_mode),
        "detail": display_safe(&d.detail),
    })
}

fn entry_json(e: &PriorEntry, candidates: Option<&[Candidate]>) -> Value {
    json!({
        "index": e.index,
        "harness": display_safe(&e.key.harness),
        "model_ref": display_safe(&e.key.model_ref),
        "tier": tier_name(e.key.tier),
        "dispatch_mode": e.key.mode.as_str(),
        "attempts": e.attempts,
        "successes": e.successes,
        "pi": e.pi(),
        "candidate": candidates.map(|c| is_candidate(e, c)),
    })
}

fn ignored_json(items: &[IgnoredEntry]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| {
                json!({
                    "index": i.index,
                    "code": i.code.as_str(),
                    "harness": opt_safe(&i.label.harness),
                    "model_ref": opt_safe(&i.label.model_ref),
                    "tier": opt_safe(&i.label.tier),
                    "dispatch_mode": opt_safe(&i.label.dispatch_mode),
                })
            })
            .collect(),
    )
}

/// A ledger row for display. An unreadable ignored-entries JSON is shown as
/// `ignored: null` with `ignored_error`; `priors_state` already reports it as
/// `ledger_corrupt`, so nothing is in effect.
fn row_json(row: &PromotionRow) -> Value {
    let (ignored, ignored_error) = match serde_json::from_str::<Value>(&row.ignored_json) {
        Ok(v) => (sanitize_value(v), Value::Null),
        Err(e) => (Value::Null, Value::from(format!("unreadable: {e}"))),
    };
    json!({
        "seq": row.seq,
        "action": row.action.as_str(),
        "priors_sha256": row.priors_sha256,
        "identity": display_safe(&row.identity),
        "recorded_at": row.recorded_at,
        "source_path": opt_safe(&row.source_path),
        "resolved_refs": row.resolved_refs,
        "unresolved_refs": row.unresolved_refs,
        "ignored": ignored,
        "ignored_error": ignored_error,
    })
}

/// Strips control and format characters from every string of a ledger JSON value.
fn sanitize_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(display_safe(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(sanitize_value).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (display_safe(&k), sanitize_value(v)))
                .collect(),
        ),
        other => other,
    }
}

fn print_diagnostics(diagnostics: &[PriorsDiagnostic]) {
    for d in diagnostics {
        let entry = d.entry.map(|i| format!(" entry {i}")).unwrap_or_default();
        let key = match (&d.label.harness, &d.label.model_ref, &d.label.tier) {
            (Some(h), Some(m), Some(t)) => format!(
                " ({}/{}/{})",
                display_safe(h),
                display_safe(m),
                display_safe(t)
            ),
            _ => String::new(),
        };
        println!(
            "  {} [{}]{entry}{key}: {}",
            d.code.as_str(),
            d.code.level().as_str(),
            display_safe(&d.detail)
        );
    }
}

/// `priors show`: the priors in effect and why, from one read of the latest row.
fn show(name: &str, req: &PriorsRequest, db: &Path, candidates: Option<&[Candidate]>) -> ExitCode {
    let store = match SqliteStore::open_read_only(db) {
        Ok(store) => store,
        Err(e) => return fail(name, req, 2, format!("{e:?}"), Value::Null),
    };
    let state = match &store {
        Some(store) => match store.priors_state() {
            Ok(state) => state,
            Err(e) => return fail(name, req, 2, format!("{e:?}"), Value::Null),
        },
        None => meshloop_adapters::advisory::PriorsState {
            fault: Some(PriorsDiagnostic::file(
                PriorsCode::NotPromoted,
                "no ledger exists; no priors file has been accepted",
            )),
            ..Default::default()
        },
    };
    let entries: Vec<PriorEntry> = state
        .snapshot
        .as_ref()
        .map(|s| s.entries.clone())
        .unwrap_or_default();
    let mut diagnostics = state.diagnostics();
    diagnostics.extend(candidate_report(&entries, candidates));
    let digest = state.snapshot.as_ref().map(|s| s.priors_sha256.clone());
    let routing = match (&store, &digest) {
        (Some(store), Some(d)) => match store.advisory_routing_counts(d) {
            Ok(counts) => counts,
            Err(e) => return fail(name, req, 2, format!("{e:?}"), Value::Null),
        },
        _ => (0, 0),
    };
    if req.json {
        let data = json!({
            "in_effect": state.in_effect(),
            "priors_sha256": digest,
            "evidence_floor": PRIOR_EVIDENCE_FLOOR,
            "row": state.row.as_ref().map(row_json),
            "priors": entries.iter().map(|e| entry_json(e, candidates)).collect::<Vec<_>>(),
            "routing": { "attempts": routing.0, "changed_top": routing.1 },
            "diagnostics": diagnostics.iter().map(diagnostic_json).collect::<Vec<_>>(),
        });
        println!("{}", json_out::ok(name, req.origin.clone(), data));
    } else {
        match (&state.row, &digest) {
            (Some(row), Some(d)) => println!(
                "priors in effect: {d} (ledger row {}, accepted by {})",
                row.seq,
                display_safe(&row.identity)
            ),
            _ => println!("no priors in effect"),
        }
        for e in &entries {
            println!(
                "  {}/{}/{} {}: {}/{} passed, pi={:.3}",
                display_safe(&e.key.harness),
                display_safe(&e.key.model_ref),
                tier_name(e.key.tier),
                e.key.mode.as_str(),
                e.successes,
                e.attempts,
                e.pi()
            );
        }
        if digest.is_some() {
            println!(
                "  routed attempts: {} ({} changed the top candidate)",
                routing.0, routing.1
            );
        }
        print_diagnostics(&diagnostics);
    }
    ExitCode::SUCCESS
}

/// The steps `check` and `accept` share: load the buffer, resolve provenance against
/// the local ledger, and ignore every contradicted entry.
struct Assessed {
    loaded: LoadedPriors,
    resolved: u64,
    unresolved: u64,
    report: Vec<PriorsDiagnostic>,
}

fn assess(
    bytes: &[u8],
    store: Option<&SqliteStore>,
    candidates: Option<&[Candidate]>,
) -> Result<Assessed, String> {
    let mut loaded = load_priors(bytes);
    let mut report = Vec::new();
    let (mut resolved, mut unresolved) = (0, 0);
    if loaded.file_fault.is_none() {
        let entries = loaded.validated.entries.clone();
        let provenance = match store {
            Some(store) => {
                let mut result = None;
                meshloop_engine::workspace::RuntimeLedger::with_read_snapshot(store, &mut || {
                    result = Some(meshloop_engine::priors::resolve_provenance(store, &entries));
                })
                .map_err(|e| format!("{e:?}"))?;
                result
                    .ok_or("read snapshot did not run")?
                    .map_err(|e| format!("{e:?}"))?
            }
            // No ledger: every reference is unresolved.
            None => meshloop_engine::priors::ProvenanceReport {
                unresolved: entries.iter().map(|e| e.provenance.len() as u64).sum(),
                unresolved_by_entry: entries
                    .iter()
                    .filter(|e| !e.provenance.is_empty())
                    .map(|e| (e.index, e.provenance.len() as u64))
                    .collect(),
                ..Default::default()
            },
        };
        resolved = provenance.resolved;
        unresolved = provenance.unresolved;
        for (index, why) in &provenance.contradicted {
            loaded
                .validated
                .ignore(*index, PriorsCode::ProvenanceContradicts, why.clone());
        }
        for (index, count) in &provenance.unresolved_by_entry {
            if let Some(e) = entries.iter().find(|e| e.index == *index) {
                report.push(PriorsDiagnostic {
                    code: PriorsCode::ProvenanceUnresolved,
                    entry: Some(*index),
                    label: label(e),
                    detail: format!("{count} reference(s) are not in the local ledger"),
                });
            }
        }
        report.extend(candidate_report(&loaded.validated.entries, candidates));
    }
    Ok(Assessed {
        loaded,
        resolved,
        unresolved,
        report,
    })
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    read_proposal(path).map_err(|e| {
        format!(
            "cannot read {}: {e}",
            display_safe(&path.display().to_string())
        )
    })
}

/// `priors check --file F`: what `accept` would record, without writing anything.
fn check(name: &str, req: &PriorsRequest, db: &Path, candidates: Option<&[Candidate]>) -> ExitCode {
    let Some(path) = req.file.as_deref() else {
        return fail(name, req, 2, "requires --file <path>".into(), Value::Null);
    };
    let bytes = match read_file(path) {
        Ok(b) => b,
        Err(e) => return fail(name, req, 2, e, Value::Null),
    };
    let store = match SqliteStore::open_read_only(db) {
        Ok(store) => store,
        Err(e) => return fail(name, req, 2, format!("{e:?}"), Value::Null),
    };
    let a = match assess(&bytes, store.as_ref(), candidates) {
        Ok(a) => a,
        Err(e) => return fail(name, req, 2, e, Value::Null),
    };
    let mut diagnostics = a.loaded.diagnostics();
    diagnostics.extend(a.report.iter().cloned());
    let ignored = ignored_entries(&a.loaded.validated);
    let file_fault = a.loaded.file_fault.as_ref().map(|f| f.code.as_str());
    let digest = sha256_hex(&bytes);
    if req.json {
        let data = json!({
            "file": display_safe(&path.display().to_string()),
            "bytes": bytes.len(),
            "priors_sha256": digest,
            "file_fault": file_fault,
            "evidence_floor": PRIOR_EVIDENCE_FLOOR,
            "resolved_refs": a.resolved,
            "unresolved_refs": a.unresolved,
            "usable": a.loaded.validated.entries.iter().map(|e| entry_json(e, candidates)).collect::<Vec<_>>(),
            "ignored": ignored_json(&ignored),
            "diagnostics": diagnostics.iter().map(diagnostic_json).collect::<Vec<_>>(),
        });
        println!("{}", json_out::ok(name, req.origin.clone(), data));
    } else {
        match file_fault {
            Some(code) => println!("{digest}: not acceptable ({code}); accept would refuse it"),
            None => println!(
                "{digest}: {} usable, {} ignored; references {} resolved, {} unresolved",
                a.loaded.validated.entries.len(),
                ignored.len(),
                a.resolved,
                a.unresolved
            ),
        }
        print_diagnostics(&diagnostics);
    }
    ExitCode::SUCCESS
}

/// `priors accept --file F --as ID`: one bounded read, then one appended row.
fn accept(
    name: &str,
    req: &PriorsRequest,
    db: &Path,
    candidates: Option<&[Candidate]>,
) -> ExitCode {
    let (Some(path), Some(identity)) = (req.file.as_deref(), req.identity.as_deref()) else {
        return fail(
            name,
            req,
            2,
            "requires --file <path> and --as <identity>".into(),
            Value::Null,
        );
    };
    // The digest, the parse and the stored BLOB all come from this one buffer.
    let bytes = match read_file(path) {
        Ok(b) => b,
        Err(e) => return fail(name, req, 2, e, Value::Null),
    };
    // A file-level fault is refused before the store is opened, so a refused accept
    // never creates, migrates or touches a ledger.
    let early = load_priors(&bytes);
    if let Some(fault) = &early.file_fault {
        let diagnostics = early.diagnostics();
        if !req.json {
            print_diagnostics(&diagnostics);
        }
        return fail(
            name,
            req,
            1,
            format!(
                "refused: {} ({}); nothing was recorded",
                fault.code.as_str(),
                display_safe(&fault.detail)
            ),
            json!({
                "file_fault": fault.code.as_str(),
                "diagnostics": diagnostics.iter().map(diagnostic_json).collect::<Vec<_>>(),
            }),
        );
    }
    let mut store = match SqliteStore::open(db) {
        Ok(store) => store,
        Err(e) => return fail(name, req, 2, format!("{e:?}"), Value::Null),
    };
    crate::gitignore::ensure_store_ignored(db);
    let a = match assess(&bytes, Some(&store), candidates) {
        Ok(a) => a,
        Err(e) => return fail(name, req, 2, e, Value::Null),
    };
    let mut diagnostics = a.loaded.diagnostics();
    diagnostics.extend(a.report.iter().cloned());
    let diagnostics_json: Vec<Value> = diagnostics.iter().map(diagnostic_json).collect();
    if a.loaded.file_fault.is_some() {
        // Unreachable: the same buffer passed the file-level check above.
        return fail(
            name,
            req,
            1,
            "refused: file-level fault".into(),
            Value::Null,
        );
    }
    let ignored = ignored_entries(&a.loaded.validated);
    let source = path.display().to_string();
    let seq = match store.append_priors_accept(&PriorsAccept {
        bytes: &bytes,
        identity,
        source_path: &source,
        resolved_refs: a.resolved,
        unresolved_refs: a.unresolved,
        ignored: &ignored,
    }) {
        Ok(seq) => seq,
        Err(e) => return fail(name, req, 1, format!("{e:?}"), Value::Null),
    };
    let digest = sha256_hex(&bytes);
    if req.json {
        let data = json!({
            "seq": seq,
            "priors_sha256": digest,
            "identity": display_safe(identity),
            "source_path": display_safe(&source),
            "bytes": bytes.len(),
            "resolved_refs": a.resolved,
            "unresolved_refs": a.unresolved,
            "usable": a.loaded.validated.entries.iter().map(|e| entry_json(e, candidates)).collect::<Vec<_>>(),
            "ignored": ignored_json(&ignored),
            "diagnostics": diagnostics_json,
        });
        println!("{}", json_out::ok(name, req.origin.clone(), data));
    } else {
        println!(
            "accepted {digest} as ledger row {seq} by {}: {} usable, {} ignored; references {} resolved, {} unresolved",
            display_safe(identity),
            a.loaded.validated.entries.len(),
            ignored.len(),
            a.resolved,
            a.unresolved
        );
        print_diagnostics(&diagnostics);
    }
    ExitCode::SUCCESS
}

/// `priors revoke --as ID`: appends a revoke row; routing stops using priors at once.
fn revoke(name: &str, req: &PriorsRequest, db: &Path) -> ExitCode {
    let Some(identity) = req.identity.as_deref() else {
        return fail(name, req, 2, "requires --as <identity>".into(), Value::Null);
    };
    let mut store = match SqliteStore::open(db) {
        Ok(store) => store,
        Err(e) => return fail(name, req, 2, format!("{e:?}"), Value::Null),
    };
    crate::gitignore::ensure_store_ignored(db);
    let (seq, revoked) = match store.append_priors_revoke(identity) {
        Ok(v) => v,
        Err(e) => return fail(name, req, 1, format!("{e:?}"), Value::Null),
    };
    if req.json {
        let data = json!({
            "seq": seq,
            "identity": display_safe(identity),
            "revoked_sha256": revoked,
        });
        println!("{}", json_out::ok(name, req.origin.clone(), data));
    } else {
        match revoked {
            Some(d) => println!("revoked {d} as ledger row {seq}; no priors are in effect"),
            None => println!("ledger row {seq}: revoke recorded; no priors were in effect"),
        }
    }
    ExitCode::SUCCESS
}

/// What `doctor` says about the priors ledger. Read-only; it never creates a ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorPriors {
    /// No ledger, no row, or priors in effect: nothing to say.
    Quiet,
    /// A ledger row exists but no priors are in effect.
    NotInEffect,
    /// The ledger or its priors could not be read.
    Unreadable,
}

pub fn doctor_priors(db: &Path) -> DoctorPriors {
    match SqliteStore::open_read_only(db) {
        Ok(None) => DoctorPriors::Quiet,
        Ok(Some(store)) => match store.priors_state() {
            Ok(state) if state.row.is_some() && !state.in_effect() => DoctorPriors::NotInEffect,
            Ok(_) => DoctorPriors::Quiet,
            Err(_) => DoctorPriors::Unreadable,
        },
        Err(_) => DoctorPriors::Unreadable,
    }
}
