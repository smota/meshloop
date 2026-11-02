//! `workspace inventory` and `workspace preview` print JSON and do not write.
//! `preview --cleanup` is a read-only verdict and does not remove artifacts.
//! `apply --cleanup --digest` removes only eligible registered worktrees.

use std::path::PathBuf;
use std::process::ExitCode;

use meshloop_adapters::workspace::{InventoryScope, discover_inventory, observe_cleanup};
use meshloop_engine::origin::Origin;
use meshloop_engine::workspace::cleanup::cleanup_preview;
use meshloop_engine::workspace::inventory::{adoption_preview, classify};

use crate::args::WorkspaceReport;
use crate::compose::{default_db, default_worktree_base};
use crate::json_out;

pub fn cmd_workspace(
    report: WorkspaceReport,
    cleanup: bool,
    digest: Option<String>,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    origin: Origin,
) -> ExitCode {
    let command = match report {
        WorkspaceReport::Inventory => "meshloop:workspace-inventory",
        WorkspaceReport::Preview => "meshloop:workspace-preview",
        WorkspaceReport::Apply => "meshloop:workspace-apply",
    };
    if let Some(path) = &config
        && let Err(error) = crate::config::load(path)
    {
        println!("{}", json_out::err(command, origin, format!("{error:?}")));
        return ExitCode::from(2);
    }
    let checkout = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            println!(
                "{}",
                json_out::err(command, origin, format!("current directory: {error}"))
            );
            return ExitCode::from(2);
        }
    };
    let ledger = db.unwrap_or_else(|| default_db(&checkout));
    let configured_root = worktree_base.unwrap_or_else(|| default_worktree_base(&checkout));
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
            match meshloop_adapters::workspace::apply_cleanup(
                InventoryScope {
                    checkout: &checkout,
                    ledger: &ledger,
                    configured_root: &configured_root,
                },
                digest.as_deref().unwrap_or(""),
                None,
            ) {
                Ok(result) => serde_json::json!({"cleanup": result}),
                Err(error) => {
                    println!("{}", json_out::err(command, origin, format!("{error:?}")));
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
    };
    println!("{}", json_out::ok(command, origin, data));
    ExitCode::SUCCESS
}
