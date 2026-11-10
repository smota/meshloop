//! `workspace inventory` and `workspace preview` print JSON and do not write.
//! `preview --cleanup` is a read-only verdict and does not remove artifacts.
//! `apply --cleanup --digest` removes only eligible registered worktrees, plus the
//! merged `meshloop/<graph>/` branch of each removed run-loop worktree.
//! `adopt` pins verified legacy generations. `maintain` selects at most one
//! opt-in cleanup candidate and does not remove anything unless `--apply` matches.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use meshloop_adapters::workspace::{
    InventoryScope, discover_inventory, graph_outcomes, observe_cleanup,
};
use meshloop_domain::cleanup::CleanupEligibility;
use meshloop_domain::workspace::InventoryEntry;
use meshloop_engine::origin::Origin;
use meshloop_engine::ports::StoreError;
use meshloop_engine::workspace::cleanup::cleanup_preview;
use meshloop_engine::workspace::inventory::{adoption_preview, classify};
use meshloop_engine::workspace::maintenance::{
    MaintenanceBudget, MaintenanceCandidate, maintenance_plan,
};

use crate::args::WorkspaceReport;
use crate::compose::{default_db, resolve_worktree_base};
use crate::json_out;

pub struct WorkspaceInvocation {
    pub report: WorkspaceReport,
    pub cleanup: bool,
    pub apply: bool,
    pub digest: Option<String>,
    /// `apply --cleanup` only: remove just this graph's eligible entries.
    pub graph: Option<String>,
    pub config: Option<PathBuf>,
    pub db: Option<PathBuf>,
    pub worktree_base: Option<PathBuf>,
    pub origin: Origin,
}

pub fn cmd_workspace(invocation: WorkspaceInvocation) -> ExitCode {
    let WorkspaceInvocation {
        report,
        cleanup,
        apply,
        digest,
        graph,
        config,
        db,
        worktree_base,
        origin,
    } = invocation;
    let command = match report {
        WorkspaceReport::Inventory => "meshloop:workspace-inventory",
        WorkspaceReport::Preview => "meshloop:workspace-preview",
        WorkspaceReport::Apply => "meshloop:workspace-apply",
        WorkspaceReport::Adopt => "meshloop:workspace-adopt",
        WorkspaceReport::Maintain => "meshloop:workspace-maintain",
    };
    let loaded = if let Some(path) = &config {
        match crate::config::load(path) {
            Ok(config) => Some(config),
            Err(error) => {
                println!(
                    "{}",
                    json_out::err(command, origin, public_config_error(&error))
                );
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };
    let checkout = match std::env::current_dir() {
        Ok(path) => path,
        Err(_error) => {
            println!("{}", json_out::err(command, origin, "current-directory"));
            return ExitCode::from(2);
        }
    };
    let ledger = db.unwrap_or_else(|| default_db(&checkout));
    let configured_root = resolve_worktree_base(
        &checkout,
        worktree_base.as_deref(),
        loaded
            .as_ref()
            .and_then(|config| config.workspace.root.as_deref()),
    );
    let scope = InventoryScope {
        checkout: &checkout,
        ledger: &ledger,
        configured_root: &configured_root,
    };
    let facts = discover_inventory(scope);
    let inventory = classify(&facts);
    let data = match report {
        WorkspaceReport::Inventory => {
            debug_assert!(!inventory.apply);
            serde_json::to_value(&inventory).unwrap_or(serde_json::Value::Null)
        }
        WorkspaceReport::Apply => {
            let scope = InventoryScope {
                checkout: &checkout,
                ledger: &ledger,
                configured_root: &configured_root,
            };
            let digest = digest.as_deref().unwrap_or("");
            let applied = match graph.as_deref() {
                Some(graph) => {
                    meshloop_adapters::workspace::apply_cleanup_for_graph(scope, digest, graph)
                }
                None => meshloop_adapters::workspace::apply_cleanup(scope, digest, None, None),
            };
            match applied {
                Ok(result) => {
                    if result.status == "git-remove-failed" {
                        println!(
                            "{}",
                            json_out::report(
                                command,
                                origin,
                                Some("git-remove-failed".into()),
                                serde_json::json!({"cleanup": result}),
                            )
                        );
                        return ExitCode::from(2);
                    }
                    serde_json::json!({"cleanup": result})
                }
                Err(error) => {
                    println!(
                        "{}",
                        json_out::err(command, origin, public_store_error(&error))
                    );
                    return ExitCode::from(2);
                }
            }
        }
        WorkspaceReport::Preview => {
            let preview = adoption_preview(&inventory);
            debug_assert!(!preview.apply);
            let mut data = serde_json::to_value(&preview).unwrap_or(serde_json::Value::Null);
            if cleanup {
                let observed = observe_cleanup(
                    InventoryScope {
                        checkout: &checkout,
                        ledger: &ledger,
                        configured_root: &configured_root,
                    },
                    &inventory.entries,
                );
                let cleanup_report = cleanup_preview(
                    observed.repository_id,
                    observed.retained_ref,
                    observed.retained_commit,
                    &inventory.entries,
                    &observed.observations,
                    digest.as_deref(),
                );
                debug_assert!(!cleanup_report.apply);
                if let Some(object) = data.as_object_mut() {
                    object.insert(
                        "cleanup".to_string(),
                        serde_json::to_value(&cleanup_report).unwrap_or(serde_json::Value::Null),
                    );
                }
            }
            data
        }
        WorkspaceReport::Adopt => {
            match meshloop_adapters::workspace::apply_adoption(
                InventoryScope {
                    checkout: &checkout,
                    ledger: &ledger,
                    configured_root: &configured_root,
                },
                digest.as_deref(),
                apply,
            ) {
                Ok(result) => serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                Err(error) => {
                    println!(
                        "{}",
                        json_out::err(command, origin, public_store_error(&error))
                    );
                    return ExitCode::from(2);
                }
            }
        }
        WorkspaceReport::Maintain => {
            match maintain_report(
                InventoryScope {
                    checkout: &checkout,
                    ledger: &ledger,
                    configured_root: &configured_root,
                },
                &inventory.entries,
                apply,
                digest.as_deref(),
                command,
                &origin,
            ) {
                Ok(value) => value,
                Err(code) => return code,
            }
        }
    };
    println!("{}", json_out::ok(command, origin, data));
    ExitCode::SUCCESS
}

fn maintain_report(
    scope: InventoryScope<'_>,
    entries: &[InventoryEntry],
    apply: bool,
    digest: Option<&str>,
    command: &str,
    origin: &Origin,
) -> Result<serde_json::Value, ExitCode> {
    let observed = observe_cleanup(
        InventoryScope {
            checkout: scope.checkout,
            ledger: scope.ledger,
            configured_root: scope.configured_root,
        },
        entries,
    );
    let cleanup = cleanup_preview(
        observed.repository_id,
        observed.retained_ref,
        observed.retained_commit,
        entries,
        &observed.observations,
        None,
    );
    let outcomes = graph_outcomes(scope.ledger);
    let mut candidates = Vec::new();
    for candidate in &cleanup.candidates {
        let Some(id) = candidate.lifecycle_id.as_ref() else {
            continue;
        };
        let matched = entries.iter().find(|entry| {
            entry.lifecycle_id.as_ref() == Some(id) && entry.location == candidate.location
        });
        let outcome = matched
            .and_then(|entry| entry.graph_id.as_ref())
            .and_then(|graph| outcomes.get(graph.as_str()))
            .cloned()
            .unwrap_or_default();
        candidates.push(MaintenanceCandidate {
            id: id.as_str().to_string(),
            eligible: candidate.status == CleanupEligibility::Eligible,
            // Integrated or Accepted is the success signal. Age is never a trigger.
            landed: outcome.landed,
            failed: outcome.failed,
            cancelled: outcome.cancelled,
            evidence: false,
            has_dependency: candidate
                .reasons
                .iter()
                .any(|reason| reason.as_str() == "missing-dependency"),
            outcome_known: outcome.known,
            age_days: None,
        });
    }
    let plan = maintenance_plan(
        &candidates,
        MaintenanceBudget::default(),
        Duration::ZERO,
        apply,
        digest,
    );
    let mut data = serde_json::to_value(&plan).unwrap_or(serde_json::Value::Null);
    if plan.apply {
        let Some(id) = plan.selected.first() else {
            return Ok(data);
        };
        match meshloop_adapters::workspace::apply_cleanup(scope, &cleanup.digest, None, Some(id)) {
            Ok(result) => {
                let failed = result.status == "git-remove-failed";
                if let Some(object) = data.as_object_mut() {
                    object.insert(
                        "cleanup".to_string(),
                        serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                    );
                }
                if failed {
                    println!(
                        "{}",
                        json_out::report(
                            command,
                            origin.clone(),
                            Some("git-remove-failed".into()),
                            data,
                        )
                    );
                    return Err(ExitCode::from(2));
                }
            }
            Err(error) => {
                println!(
                    "{}",
                    json_out::err(command, origin.clone(), public_store_error(&error))
                );
                return Err(ExitCode::from(2));
            }
        }
    }
    Ok(data)
}

fn public_store_error(error: &StoreError) -> &'static str {
    match error {
        StoreError::ReadOnly => "read-only",
        StoreError::SchemaUpgradeRequired => "schema-upgrade-required",
        StoreError::LegacyOwnershipUnresolved => "legacy-ownership-unresolved",
        StoreError::PlanRevisionMismatch => "plan-revision-mismatch",
        StoreError::LifecycleBusy => "lifecycle-busy",
        StoreError::FenceBusy => "fence-busy",
        StoreError::Io(_) => "io",
        StoreError::Corrupt(_) => "corrupt",
    }
}

fn public_config_error(error: &crate::config::ConfigError) -> &'static str {
    match error {
        crate::config::ConfigError::Io(_) => "config-io",
        crate::config::ConfigError::Parse(_) => "config-parse",
        crate::config::ConfigError::SelectedButNotConfigured(_) => "config-harness",
        crate::config::ConfigError::NotFound => "config-not-found",
    }
}
