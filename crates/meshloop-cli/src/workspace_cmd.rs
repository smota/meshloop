//! `workspace inventory` and `workspace preview`. Both print JSON and do not write.

use std::path::PathBuf;
use std::process::ExitCode;

use meshloop_adapters::workspace::{InventoryScope, discover_inventory};
use meshloop_engine::origin::Origin;
use meshloop_engine::workspace::inventory::{adoption_preview, classify};

use crate::args::WorkspaceReport;
use crate::compose::{default_db, default_worktree_base};
use crate::json_out;

pub fn cmd_workspace(
    report: WorkspaceReport,
    config: Option<PathBuf>,
    db: Option<PathBuf>,
    worktree_base: Option<PathBuf>,
    origin: Origin,
) -> ExitCode {
    let command = match report {
        WorkspaceReport::Inventory => "meshloop:workspace-inventory",
        WorkspaceReport::Preview => "meshloop:workspace-preview",
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
    let facts = discover_inventory(InventoryScope {
        checkout: &checkout,
        ledger: &ledger,
        configured_root: &configured_root,
    });
    let inventory = classify(&facts);
    let body = match report {
        WorkspaceReport::Inventory => inventory,
        WorkspaceReport::Preview => adoption_preview(&inventory),
    };
    debug_assert!(!body.apply);
    let data = serde_json::to_value(&body).unwrap_or(serde_json::Value::Null);
    println!("{}", json_out::ok(command, origin, data));
    ExitCode::SUCCESS
}
