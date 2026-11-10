//! Run reclamation after a run lands (issue #142, ADR 0033 amendment of 2026-11-10).
//! With `[workspace] auto_reclaim` on (the default), a fully integrated run whose
//! result is reachable from the checked-out base branch is reclaimed through the
//! same digest-bound cleanup as `workspace apply --cleanup`. When it is off, or the
//! run is not reclaimable yet, the exact cleanup command is printed instead.

use std::path::PathBuf;

use meshloop_adapters::workspace::{InventoryScope, RunReclaim, reclaim_run};

/// Where the run loop's ledger and worktree root live for this invocation.
pub struct ReclaimContext {
    pub checkout: PathBuf,
    pub ledger: PathBuf,
    pub worktree_root: PathBuf,
    pub auto: bool,
}

/// Outcome to report. `None` when the run is not owned by the run loop (legacy).
pub struct ReclaimNote {
    pub report: RunReclaim,
    pub lines: Vec<String>,
}

impl ReclaimNote {
    pub fn json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(&self.report).unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.insert("next".into(), serde_json::json!(self.lines));
        }
        value
    }
}

/// Reclaims `graph` when allowed, or explains what keeps it and which command
/// reclaims it later. Errors are reported, never escalated: the run itself has
/// already succeeded and nothing is removed on an error.
pub fn after_run(ctx: &ReclaimContext, graph: &str) -> Option<ReclaimNote> {
    let scope = InventoryScope {
        checkout: &ctx.checkout,
        ledger: &ctx.ledger,
        configured_root: &ctx.worktree_root,
    };
    let report = match reclaim_run(scope, graph, ctx.auto) {
        Ok(report) => report,
        Err(_) => {
            return Some(ReclaimNote {
                report: RunReclaim {
                    graph_id: graph.to_string(),
                    status: "retained",
                    digest: String::new(),
                    reasons: vec!["reclaim-unavailable".into()],
                    cleanup: None,
                },
                lines: vec![format!(
                    "workspace: run {graph} kept; reclamation was unavailable. Inspect with: meshloop workspace preview --cleanup --json"
                )],
            });
        }
    };
    if report.reasons.iter().any(|r| r == "not-run-loop-owned") {
        return None;
    }
    let preview = "meshloop workspace preview --cleanup --json".to_string();
    let apply = format!("meshloop workspace apply --cleanup --graph {graph} --digest");
    let lines = match report.status {
        "reclaimed" => vec![format!(
            "workspace: reclaimed run {graph} (worktrees and meshloop/{graph}/ branches removed)"
        )],
        "retained" if report.reasons.is_empty() => vec![
            format!(
                "workspace: automatic reclamation is off ([workspace] auto_reclaim = false); run {graph} is landed and kept."
            ),
            format!("  reclaim it with: {apply} {}", report.digest),
        ],
        "retained" => vec![
            format!(
                "workspace: run {graph} kept ({}).",
                report.reasons.join(", ")
            ),
            // Only work that is not on the base branch yet is resolved by landing it.
            if ctx.auto && report.reasons.iter().all(|r| r == "unique-commit") {
                format!(
                    "  It is reclaimed automatically by `meshloop integrate --graph {graph} --into <base> --accept-integrate`. After landing it any other way: {preview}, then {apply} <digest>"
                )
            } else if report.reasons.iter().all(|r| r == "unique-commit") {
                format!("  After landing it on the base branch: {preview}, then {apply} <digest>")
            } else {
                format!("  Resolve the reasons above, then: {preview}, then {apply} <digest>")
            },
        ],
        "partial" => vec![format!(
            "workspace: run {graph} partly reclaimed ({}); see {preview}",
            report
                .cleanup
                .as_ref()
                .map(|c| c.reasons.join(", "))
                .unwrap_or_default()
        )],
        other => vec![format!(
            "workspace: reclamation of run {graph} stopped ({other}); inspect with: {preview}"
        )],
    };
    Some(ReclaimNote { report, lines })
}
