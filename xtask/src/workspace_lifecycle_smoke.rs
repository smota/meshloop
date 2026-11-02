//! Disposable workspace lifecycle. This scenario is not part of `smoke.wall_clock_s`.

use std::fs;
use std::path::Path;
use std::process::Command;

use meshloop_adapters::store::SqliteStore;
use meshloop_adapters::workspace::GitRepositoryIdentity;
use meshloop_domain::state::PlanState;
use meshloop_domain::workspace::{
    GenerationRecord, GraphId, LifecycleId, PlanHash, RelativePath, RootAlias,
};
use meshloop_engine::ports::{RunRow, RunStore};
use meshloop_engine::workspace::{
    LifecycleAccess, RepositoryFence, RepositoryIdentity, WorkspaceLedger,
};

pub fn smoke_workspace_lifecycle(bin: &Path) -> Result<(), String> {
    let help = Command::new(bin)
        .arg("--help")
        .output()
        .map_err(|error| format!("help spawn failed: {error}"))?;
    let help_text = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    if !help.status.success() || !help_text.contains("matching 64-character digest") {
        return Err(format!(
            "workspace help missing digest contract:\n{help_text}"
        ));
    }

    let base =
        std::env::temp_dir().join(format!("meshloop-smoke-workspace-{}", std::process::id()));
    let _guard = TempDirGuard(&base);
    if base.exists() {
        remove_smoke_dir(&base)?;
    }
    let repo = base.join("repo");
    fs::create_dir_all(&repo).map_err(|error| format!("{error:?}"))?;
    git(&repo, &["init", "--quiet"])?;
    fs::write(repo.join("tracked.txt"), "committed seed\n")
        .map_err(|error| format!("{error:?}"))?;
    git(&repo, &["add", "tracked.txt"])?;
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "seed",
        ],
    )?;
    let config = repo.join("meshloop.toml");
    fs::write(
        &config,
        "selected_harnesses = []\n[limits]\nmax_concurrent_workers = 1\nmax_retries = 1\ntask_timeout_seconds = 10\n",
    )
    .map_err(|error| format!("{error:?}"))?;
    let db = repo.join(".meshloop").join("state.sqlite");
    seed(&repo, &db)?;
    let root = repo.join("legacy-pin");
    fs::write(repo.join("unknown-artifact.txt"), "leave me\n")
        .map_err(|error| format!("{error:?}"))?;
    fs::write(repo.join("protected-data.txt"), "protected\n")
        .map_err(|error| format!("{error:?}"))?;

    let inventory = meshloop(bin, &repo, &config, &db, &["workspace", "inventory"])?;
    require_apply_false(&inventory, "inventory")?;
    let cleanup = meshloop(
        bin,
        &repo,
        &config,
        &db,
        &["workspace", "preview", "--cleanup"],
    )?;
    require_apply_false(&cleanup, "cleanup preview")?;
    let preview = meshloop(bin, &repo, &config, &db, &["workspace", "adopt"])?;
    require_apply_false(&preview, "adopt preview")?;
    if preview["data"]["status"] != "preview" {
        return Err(format!("adopt preview status: {preview}"));
    }
    let digest = preview["data"]["digest"]
        .as_str()
        .filter(|value| value.len() == 64)
        .ok_or_else(|| format!("adopt preview digest: {preview}"))?
        .to_string();
    require_pin(&preview, "kept-run", "kept.txt")?;
    require_hold(&preview, "missing-run", "path-missing")?;
    require_hold(&preview, "name-only", "unverified-location")?;
    protected(&repo, &root)?;

    let stale = meshloop(
        bin,
        &repo,
        &config,
        &db,
        &["workspace", "adopt", "--apply", "--digest", &"0".repeat(64)],
    )?;
    require_apply_false(&stale, "stale adopt")?;
    if stale["data"]["status"] != "stale" {
        return Err(format!("stale adopt status: {stale}"));
    }
    protected(&repo, &root)?;

    let blocked = meshloop(
        bin,
        &repo,
        &config,
        &db,
        &[
            "workspace",
            "apply",
            "--cleanup",
            "--digest",
            &"0".repeat(64),
        ],
    )?;
    if blocked["data"]["cleanup"]["apply"] != false
        || blocked["data"]["cleanup"]["status"] != "blocked"
    {
        return Err(format!("stale cleanup apply: {blocked}"));
    }
    protected(&repo, &root)?;

    let applied = meshloop(
        bin,
        &repo,
        &config,
        &db,
        &["workspace", "adopt", "--apply", "--digest", &digest],
    )?;
    if applied["ok"] != true
        || applied["data"]["apply"] != true
        || applied["data"]["status"] != "adopted"
    {
        return Err(format!("adopt apply: {applied}"));
    }
    assert_binding(&db)?;
    protected(&repo, &root)?;

    let maintain = meshloop(bin, &repo, &config, &db, &["workspace", "maintain"])?;
    require_apply_false(&maintain, "maintain")?;
    let maintain_stale = meshloop(
        bin,
        &repo,
        &config,
        &db,
        &[
            "workspace",
            "maintain",
            "--apply",
            "--digest",
            &"0".repeat(64),
        ],
    )?;
    require_apply_false(&maintain_stale, "stale maintain")?;
    protected(&repo, &root)?;
    if repo.join(".meshloop").join("worktrees").exists()
        || base.join(".meshloop-worktrees").exists()
    {
        return Err("workspace commands created a worktree root".into());
    }
    println!("smoke: workspace lifecycle ok");
    Ok(())
}

fn seed(repo: &Path, db: &Path) -> Result<(), String> {
    let plan_hash = "a".repeat(64);
    let mut store = SqliteStore::open(db).map_err(|error| format!("{error:?}"))?;
    for graph_id in ["kept-run", "missing-run"] {
        store
            .save_run(&RunRow {
                graph_id: graph_id.into(),
                plan_state: PlanState::PlanAccepted,
                run_base: "base".into(),
                integrate_ref: "integrate".into(),
                plan_json: "{}".into(),
                plan_id: plan_hash.clone(),
                created_at: "1".into(),
                review_note: None,
            })
            .map_err(|error| format!("{error:?}"))?;
    }
    store
        .save_run(&RunRow {
            graph_id: "name-only".into(),
            plan_state: PlanState::AwaitingPlanReview,
            run_base: "base".into(),
            integrate_ref: "integrate".into(),
            plan_json: "{}".into(),
            plan_id: plan_hash.clone(),
            created_at: "1".into(),
            review_note: None,
        })
        .map_err(|error| format!("{error:?}"))?;
    let mut lease = GitRepositoryIdentity::new("git")
        .acquire(repo, db)
        .map_err(|error| format!("{error:?}"))?;
    store
        .bind_repository(&lease)
        .map_err(|error| format!("{error:?}"))?;
    let alias = RootAlias::new("legacy-pin").map_err(|error| format!("{error:?}"))?;
    let root = repo.join("legacy-pin");
    lease
        .bootstrap_root(alias.clone(), &root)
        .map_err(|error| format!("{error:?}"))?;
    store
        .register_root(&lease, &alias)
        .map_err(|error| format!("{error:?}"))?;
    fs::write(root.join("kept.txt"), "kept\n").map_err(|error| format!("{error:?}"))?;
    let accepted = PlanHash::new(plan_hash).map_err(|error| format!("{error:?}"))?;
    let kept = GenerationRecord {
        id: LifecycleId::new("kept-generation").map_err(|error| format!("{error:?}"))?,
        repository_id: lease.repository().id.clone(),
        graph_id: GraphId::new("kept-run").map_err(|error| format!("{error:?}"))?,
        ordinal: 1,
        accepted_plan_hash: accepted.clone(),
        root_alias: alias.clone(),
        relative_path: RelativePath::new("kept.txt").map_err(|error| format!("{error:?}"))?,
        layout_version: 1,
    };
    let missing = GenerationRecord {
        id: LifecycleId::new("missing-generation").map_err(|error| format!("{error:?}"))?,
        repository_id: lease.repository().id.clone(),
        graph_id: GraphId::new("missing-run").map_err(|error| format!("{error:?}"))?,
        ordinal: 1,
        accepted_plan_hash: accepted,
        root_alias: alias,
        relative_path: RelativePath::new("missing.txt").map_err(|error| format!("{error:?}"))?,
        layout_version: 1,
    };
    store
        .insert_generation(&lease, &kept)
        .map_err(|error| format!("{error:?}"))?;
    store
        .insert_generation(&lease, &missing)
        .map_err(|error| format!("{error:?}"))?;
    drop(store);
    drop(lease);
    let connection = rusqlite::Connection::open(db).map_err(|error| format!("{error:?}"))?;
    connection
        .execute(
            "INSERT INTO workspace_legacy_unbound VALUES('runs','kept-run','Unresolved')",
            [],
        )
        .map_err(|error| format!("{error:?}"))?;
    connection
        .execute(
            "INSERT INTO workspace_legacy_unbound VALUES('runs','missing-run','Unresolved')",
            [],
        )
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

fn meshloop(
    bin: &Path,
    repo: &Path,
    config: &Path,
    db: &Path,
    args: &[&str],
) -> Result<serde_json::Value, String> {
    let output = Command::new(bin)
        .current_dir(repo)
        .args(args)
        .arg("--json")
        .arg("--config")
        .arg(config)
        .arg("--db")
        .arg(db)
        .output()
        .map_err(|error| format!("{args:?} spawn failed: {error}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        return Err(format!("{args:?} failed ({}):\n{text}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).map_err(|error| format!("{args:?} json {error}:\n{text}"))
}

fn require_apply_false(value: &serde_json::Value, step: &str) -> Result<(), String> {
    if value["ok"] == true && value["data"]["apply"] == false {
        Ok(())
    } else {
        Err(format!("{step}: {value}"))
    }
}

fn require_pin(value: &serde_json::Value, graph: &str, relative: &str) -> Result<(), String> {
    let found = value["data"]["adopted"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|pin| pin["graph_id"] == graph && pin["relative_path"] == relative)
    });
    if found {
        Ok(())
    } else {
        Err(format!("missing adopt pin {graph} {relative}: {value}"))
    }
}

fn require_hold(value: &serde_json::Value, graph: &str, reason: &str) -> Result<(), String> {
    let found = value["data"]["retained"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|hold| hold["graph_id"] == graph && hold["reason"] == reason)
    });
    if found {
        Ok(())
    } else {
        Err(format!("missing retain {graph} {reason}: {value}"))
    }
}

fn protected(repo: &Path, root: &Path) -> Result<(), String> {
    let protected =
        fs::read(repo.join("protected-data.txt")).map_err(|error| format!("{error:?}"))?;
    let unknown =
        fs::read(repo.join("unknown-artifact.txt")).map_err(|error| format!("{error:?}"))?;
    let kept = fs::read(root.join("kept.txt")).map_err(|error| format!("{error:?}"))?;
    if protected != b"protected\n" || unknown != b"leave me\n" || kept != b"kept\n" {
        return Err("protected, unknown, or kept bytes changed".into());
    }
    if root.join("missing.txt").exists() {
        return Err("missing generation path was created".into());
    }
    Ok(())
}

fn assert_binding(db: &Path) -> Result<(), String> {
    let connection = rusqlite::Connection::open(db).map_err(|error| format!("{error:?}"))?;
    let kept_marker: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM workspace_legacy_unbound WHERE source_table='runs' AND source_id='kept-run'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("{error:?}"))?;
    let missing_marker: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM workspace_legacy_unbound WHERE source_table='runs' AND source_id='missing-run'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("{error:?}"))?;
    let binding: String = connection
        .query_row(
            "SELECT root_alias FROM workspace_run_bindings WHERE graph_id='kept-run'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("{error:?}"))?;
    let name_only: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM workspace_run_bindings WHERE graph_id='name-only'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("{error:?}"))?;
    if kept_marker != 0 || missing_marker != 1 || binding != "legacy-pin" || name_only != 0 {
        return Err(format!(
            "binding kept={kept_marker} missing={missing_marker} alias={binding} name_only={name_only}"
        ));
    }
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|error| format!("git {args:?} spawn failed: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {args:?} failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn remove_smoke_dir(path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if path.starts_with(std::env::temp_dir()) && name.starts_with("meshloop-smoke-workspace-") {
        fs::remove_dir_all(path).map_err(|error| format!("{error:?}"))
    } else {
        Err(format!("refusing to remove {}", path.display()))
    }
}

struct TempDirGuard<'a>(&'a Path);

impl Drop for TempDirGuard<'_> {
    fn drop(&mut self) {
        let _ = remove_smoke_dir(self.0);
    }
}
