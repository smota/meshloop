//! Issue #142: runs created by this version own their worktrees and branches, and a
//! landed run is reclaimed by one digest-bound cleanup call. Fixture harness only.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GRAPH: &str = "probe-graph";

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    plan: PathBuf,
    config: PathBuf,
}

impl Fixture {
    /// Disposable repository. Plan and config live outside the checkout so the
    /// base branch stays clean for `integrate --into`.
    fn new(tag: &str, workspace_toml: &str, invoke_args: &str, plan_json: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "meshloop-run-ownership-{tag}-{}-{nonce}",
            std::process::id()
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        fs::write(repo.join("README.md"), "seed\n").unwrap();
        fs::write(repo.join("unrelated.txt"), "unrelated tracked bytes\n").unwrap();
        git(&repo, &["add", "README.md", "unrelated.txt"]);
        git(&repo, &["commit", "--quiet", "-m", "seed"]);
        let plan = base.join("plan.json");
        fs::write(&plan, plan_json).unwrap();
        let config = base.join("meshloop.toml");
        fs::write(
            &config,
            format!(
                r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 30

[verify]
verify_command = []

{workspace_toml}

[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = {invoke_args}
model_ref = "fixture-model"
model_tier = "top"
"#,
                fixture_path().replace('\\', "\\\\"),
            ),
        )
        .unwrap();
        Self {
            base,
            repo,
            plan,
            config,
        }
    }

    fn landing(tag: &str, workspace_toml: &str) -> Self {
        Self::new(
            tag,
            workspace_toml,
            r#"["--prompt-file", "{prompt_file}"]"#,
            &three_task_plan(GRAPH),
        )
    }

    fn meshloop(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_meshloop"));
        command
            .current_dir(&self.repo)
            .args(args)
            .arg("--config")
            .arg(&self.config);
        bounded_output(&mut command)
    }

    /// Runs one command and requires success.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.meshloop(args);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "meshloop {args:?} failed: {text}");
        text
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut full = args.to_vec();
        full.push("--json");
        let out = self.meshloop(&full);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success(),
            "meshloop {args:?} failed: {stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !stdout.contains(&self.base.display().to_string()),
            "workspace JSON leaked an absolute path: {stdout}"
        );
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"))
    }

    fn plan_arg(&self) -> String {
        self.plan.to_string_lossy().into_owned()
    }

    /// review-plan --accept, run, then accept/resume until every task is integrated.
    fn integrate_all(&self) {
        let plan = self.plan_arg();
        self.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
        self.ok(&["run", "--plan", &plan]);
        self.ok(&["accept", "--task", "1", "--as", "probe"]);
        self.ok(&["resume"]);
        self.ok(&["accept", "--task", "2", "--as", "probe"]);
        self.ok(&["accept", "--task", "3", "--as", "probe"]);
        let last = self.ok(&["resume"]);
        assert!(last.contains("Integrated"), "{last}");
    }

    fn worktrees(&self) -> Vec<String> {
        String::from_utf8(git(&self.repo, &["worktree", "list", "--porcelain"]))
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(str::to_string)
            .collect()
    }

    fn meshloop_worktrees(&self) -> Vec<String> {
        self.worktrees()
            .into_iter()
            .filter(|path| path.contains(".meshloop"))
            .collect()
    }

    fn branches(&self, prefix: &str) -> Vec<String> {
        String::from_utf8(git(
            &self.repo,
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                &format!("refs/heads/{prefix}"),
            ],
        ))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
    }

    fn head(&self, rev: &str) -> String {
        String::from_utf8(git(&self.repo, &["rev-parse", rev]))
            .unwrap()
            .trim()
            .to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.base.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn three_task_plan(graph: &str) -> String {
    format!(
        r#"{{"graph_id":"{graph}","nodes":[
{{"id":1,"description":"first","depends_on":[],"tier":null}},
{{"id":2,"description":"second","depends_on":[1],"tier":null}},
{{"id":3,"description":"third","depends_on":[],"tier":null}}]}}"#
    )
}

fn fixture_path() -> String {
    let exe = format!("fixture_harness{}", std::env::consts::EXE_SUFFIX);
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    dir.pop();
    let path = dir.join(exe);
    assert!(path.exists(), "build meshloop-adapters first: {path:?}");
    path.to_string_lossy().into_owned()
}

fn bounded_output(command: &mut Command) -> Output {
    let mut child = meshloop_adapters::process::spawn_command(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("subprocess exceeded its deadline: {command:?}: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = bounded_output(Command::new("git").current_dir(repo).args(args));
    assert!(out.status.success(), "git {args:?}: {out:?}");
    out.stdout
}

fn candidates(report: &serde_json::Value) -> Vec<serde_json::Value> {
    report["data"]["cleanup"]["candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn run_candidates(report: &serde_json::Value, graph: &str) -> Vec<serde_json::Value> {
    candidates(report)
        .into_iter()
        .filter(|item| {
            item["source"] == "artifact"
                && item["location"]["relative_path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with(&format!("{graph}/")))
        })
        .collect()
}

/// After a run is reclaimed, inventory, cleanup preview and maintenance carry no
/// entry for it: retired records are not reported and never crowd the window.
fn assert_run_gone(fixture: &Fixture, graph: &str) {
    let inventory = fixture.json(&["workspace", "inventory"]);
    let left: Vec<&serde_json::Value> = inventory["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry["graph_id"] == graph
                || entry["location"]["relative_path"]
                    .as_str()
                    .is_some_and(|path| path == graph || path.starts_with(&format!("{graph}/")))
        })
        .collect();
    assert!(left.is_empty(), "{inventory}");
    let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
    assert!(
        candidates(&preview).iter().all(|item| {
            !item["location"]["relative_path"]
                .as_str()
                .is_some_and(|path| path.starts_with(graph))
        }),
        "{preview}"
    );
    let maintain = fixture.json(&["workspace", "maintain"]);
    assert_eq!(
        maintain["data"]["selected"],
        serde_json::json!([]),
        "{maintain}"
    );
    assert_eq!(
        maintain["data"]["retained"],
        serde_json::json!([]),
        "{maintain}"
    );
}

fn digest(report: &serde_json::Value) -> String {
    report["data"]["cleanup"]["digest"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The reproduction from issue #142, ending with zero Meshloop worktrees and branches.
#[test]
fn fresh_run_is_owned_and_one_cleanup_reclaims_the_landed_run() {
    let fixture = Fixture::landing("regression", "");
    // Unrelated user state that cleanup must never touch.
    fs::write(fixture.repo.join("notes-untracked.txt"), "keep me\n").unwrap();
    let user_tree = fixture.base.join("user-feature");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "user/feature",
            user_tree.to_str().unwrap(),
            "HEAD",
        ],
    );
    fixture.integrate_all();

    // A fresh run is owned, never legacy-unbound, and carries lifecycle ids.
    let inventory = fixture.json(&["workspace", "inventory"]);
    let entries = inventory["data"]["entries"].as_array().unwrap();
    let mine: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|entry| entry["graph_id"] == GRAPH)
        .collect();
    assert!(!mine.is_empty(), "{inventory}");
    for entry in &mine {
        assert_eq!(entry["ownership"], "ledger-owned", "{entry}");
        assert!(entry["lifecycle_id"].is_string(), "{entry}");
    }
    assert!(
        !inventory.to_string().contains("legacy-unbound"),
        "{inventory}"
    );

    let base_before = fixture.head("main");
    let integrate_head = fixture.head(&format!("meshloop/{GRAPH}/integrate"));
    assert_eq!(
        fixture.meshloop_worktrees().len(),
        4,
        "{:?}",
        fixture.worktrees()
    );
    assert_eq!(fixture.branches(&format!("meshloop/{GRAPH}")).len(), 4);

    // Before delivery nothing of the run is eligible: its commits are not on main.
    let early = fixture.json(&["workspace", "preview", "--cleanup"]);
    let early_run = run_candidates(&early, GRAPH);
    assert_eq!(early_run.len(), 4, "{early}");
    assert!(
        early_run.iter().all(|item| item["status"] == "blocked"),
        "{early}"
    );

    // Deliver to the base branch.
    git(
        &fixture.repo,
        &[
            "merge",
            "--quiet",
            "--ff-only",
            &format!("meshloop/{GRAPH}/integrate"),
        ],
    );
    let base_landed = fixture.head("main");
    assert_eq!(base_landed, integrate_head);
    assert_ne!(base_landed, base_before);

    let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
    let landed = run_candidates(&preview, GRAPH);
    assert_eq!(landed.len(), 4, "{preview}");
    for item in &landed {
        assert_eq!(item["status"], "eligible", "{item}");
        assert_eq!(item["ownership"], "ledger-owned", "{item}");
        assert!(item["lifecycle_id"].is_string(), "{item}");
    }
    let maintain = fixture.json(&["workspace", "maintain"]);
    assert_eq!(
        maintain["data"]["selected"].as_array().map(Vec::len),
        Some(1),
        "{maintain}"
    );

    // A digest that does not match writes nothing.
    let stale = fixture.json(&[
        "workspace",
        "apply",
        "--cleanup",
        "--digest",
        &"0".repeat(64),
    ]);
    assert_eq!(stale["data"]["cleanup"]["apply"], false, "{stale}");
    assert_eq!(fixture.meshloop_worktrees().len(), 4);

    // One digest-bound call reclaims the whole landed run.
    let applied = fixture.json(&[
        "workspace",
        "apply",
        "--cleanup",
        "--digest",
        &digest(&preview),
    ]);
    assert_eq!(applied["data"]["cleanup"]["apply"], true, "{applied}");
    assert_eq!(
        applied["data"]["cleanup"]["status"], "completed",
        "{applied}"
    );
    assert!(
        fixture.meshloop_worktrees().is_empty(),
        "{:?}",
        fixture.worktrees()
    );
    assert!(
        fixture.branches(&format!("meshloop/{GRAPH}")).is_empty(),
        "{:?}",
        fixture.branches("meshloop")
    );
    assert!(
        !fixture
            .repo
            .join(".meshloop")
            .join("worktrees")
            .join(GRAPH)
            .exists()
    );

    // Nothing of the reclaimed run is left to report or select.
    assert_run_gone(&fixture, GRAPH);

    // Base branch and unrelated state are unchanged.
    assert_eq!(fixture.head("main"), base_landed);
    assert_eq!(
        fs::read(fixture.repo.join("unrelated.txt")).unwrap(),
        b"unrelated tracked bytes\n"
    );
    assert_eq!(
        fs::read(fixture.repo.join("notes-untracked.txt")).unwrap(),
        b"keep me\n"
    );
    assert!(user_tree.join("README.md").is_file());
    assert_eq!(fixture.branches("user/"), ["user/feature"]);
    assert!(
        fixture
            .repo
            .join(".meshloop")
            .join("state.sqlite")
            .is_file()
    );
}

fn has_reason(item: &serde_json::Value, reason: &str) -> bool {
    item["reasons"]
        .as_array()
        .is_some_and(|reasons| reasons.iter().any(|r| r == reason))
}

fn worktree_for(fixture: &Fixture, leaf_prefix: &str) -> PathBuf {
    let found = fixture
        .meshloop_worktrees()
        .into_iter()
        .find(|path| {
            Path::new(path)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(leaf_prefix))
        })
        .unwrap_or_else(|| panic!("no worktree {leaf_prefix}: {:?}", fixture.worktrees()));
    PathBuf::from(found)
}

/// Default `auto_reclaim = true`: `integrate --into` lands the run and reclaims it.
#[test]
fn auto_reclaim_reclaims_a_run_once_integrate_lands_it() {
    let fixture = Fixture::landing("auto", "");
    fixture.integrate_all();
    // Not on the base branch yet: the final resume keeps the run and says why.
    assert_eq!(fixture.meshloop_worktrees().len(), 4);
    let integrate_head = fixture.head(&format!("meshloop/{GRAPH}/integrate"));
    let out = fixture.ok(&[
        "integrate",
        "--graph",
        GRAPH,
        "--into",
        "main",
        "--accept-integrate",
    ]);
    assert!(out.contains(&format!("reclaimed run {GRAPH}")), "{out}");
    assert!(fixture.meshloop_worktrees().is_empty(), "{out}");
    assert!(fixture.branches(&format!("meshloop/{GRAPH}")).is_empty());
    assert_eq!(fixture.head("main"), integrate_head);
    assert_eq!(
        fs::read(fixture.repo.join("unrelated.txt")).unwrap(),
        b"unrelated tracked bytes\n"
    );
}

/// `auto_reclaim = false`: nothing is removed automatically; the printed command
/// is exact and reclaims the run when run as printed.
#[test]
fn auto_reclaim_off_prints_the_exact_cleanup_command() {
    let fixture = Fixture::landing("manual", "[workspace]\nauto_reclaim = false");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
    fixture.ok(&["resume"]);
    fixture.ok(&["accept", "--task", "2", "--as", "probe"]);
    fixture.ok(&["accept", "--task", "3", "--as", "probe"]);
    let last = fixture.ok(&["resume"]);
    assert!(
        last.contains(&format!(
            "meshloop workspace apply --cleanup --graph {GRAPH} --digest"
        )),
        "{last}"
    );
    let out = fixture.ok(&[
        "integrate",
        "--graph",
        GRAPH,
        "--into",
        "main",
        "--accept-integrate",
    ]);
    assert!(out.contains("auto_reclaim = false"), "{out}");
    assert_eq!(fixture.meshloop_worktrees().len(), 4, "{out}");
    assert_eq!(fixture.branches(&format!("meshloop/{GRAPH}")).len(), 4);
    let command = out
        .lines()
        .find_map(|line| line.split("reclaim it with: meshloop ").nth(1))
        .unwrap_or_else(|| panic!("no exact command in {out}"));
    let args: Vec<&str> = command.split_whitespace().collect();
    assert_eq!(args.last().map(|d| d.len()), Some(64), "{command}");
    let applied = fixture.json(&args);
    assert_eq!(
        applied["data"]["cleanup"]["status"], "completed",
        "{applied}"
    );
    assert!(fixture.meshloop_worktrees().is_empty());
    assert!(fixture.branches(&format!("meshloop/{GRAPH}")).is_empty());
}

/// Failed, cancelled and interrupted runs are owned but retained with reasons,
/// even where their worktrees are clean and reachable.
#[test]
fn failed_cancelled_and_interrupted_runs_are_retained_with_reasons() {
    // Failed: the fixture makes no change, so verification fails. The attempt
    // worktree stays clean at the base commit: only the run outcome retains it.
    let failed = Fixture::new(
        "failed",
        "",
        r#"["--noop"]"#,
        r#"{"graph_id":"failed-graph","nodes":[{"id":1,"description":"noop","depends_on":[],"tier":null}]}"#,
    );
    let plan = failed.plan_arg();
    let _ = failed.meshloop(&["run", "--plan", &plan, "--accept-plan"]);
    // Cancelled: the run pauses for review and is then cancelled.
    let cancelled = Fixture::landing("cancelled", "");
    let plan = cancelled.plan_arg();
    cancelled.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    cancelled.ok(&["run", "--plan", &plan]);
    cancelled.ok(&["cancel", "--graph", GRAPH]);
    // Interrupted: the run stops while task 1 awaits review.
    let interrupted = Fixture::landing("interrupted", "");
    let plan = interrupted.plan_arg();
    interrupted.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    interrupted.ok(&["run", "--plan", &plan]);

    for (fixture, graph, reason) in [
        (&failed, "failed-graph", "run-failed"),
        (&cancelled, GRAPH, "run-cancelled"),
        (&interrupted, GRAPH, "run-incomplete"),
    ] {
        let before = fixture.meshloop_worktrees();
        assert!(!before.is_empty(), "{reason}");
        let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
        let run = run_candidates(&preview, graph);
        assert_eq!(run.len(), before.len(), "{reason}: {preview}");
        for item in &run {
            assert_eq!(item["ownership"], "ledger-owned", "{item}");
            assert_eq!(item["status"], "blocked", "{item}");
            assert!(has_reason(item, reason), "{reason}: {item}");
        }
        if reason == "run-failed" {
            // Clean and reachable: the run outcome is the only reason.
            assert!(
                run.iter()
                    .any(|item| item["reasons"] == serde_json::json!(["run-failed"])),
                "{preview}"
            );
        }
        let applied = fixture.json(&[
            "workspace",
            "apply",
            "--cleanup",
            "--digest",
            &digest(&preview),
        ]);
        assert_ne!(applied["data"]["cleanup"]["apply"], true, "{applied}");
        assert_eq!(fixture.meshloop_worktrees(), before, "{reason}");
        assert_eq!(
            fixture.branches(&format!("meshloop/{graph}")).len(),
            before.len(),
            "{reason}"
        );
    }
}

/// Unknown, untracked and modified files block their worktree; cleanup never
/// deletes them, and automatic reclamation keeps the whole run.
#[test]
fn dirty_and_untracked_worktrees_are_never_reclaimed() {
    let fixture = Fixture::landing("dirty", "");
    fixture.integrate_all();
    let task1 = worktree_for(&fixture, "task-1-");
    let task3 = worktree_for(&fixture, "task-3-");
    fs::write(task1.join("README.md"), "modified tracked bytes\n").unwrap();
    fs::write(task3.join("scratch.txt"), "untracked user bytes\n").unwrap();
    let out = fixture.ok(&[
        "integrate",
        "--graph",
        GRAPH,
        "--into",
        "main",
        "--accept-integrate",
    ]);
    assert!(out.contains(&format!("run {GRAPH} kept")), "{out}");
    assert_eq!(fixture.meshloop_worktrees().len(), 4, "{out}");

    let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
    let run = run_candidates(&preview, GRAPH);
    let status = |leaf: &str| {
        run.iter()
            .find(|item| {
                item["location"]["relative_path"]
                    .as_str()
                    .is_some_and(|path| path.contains(leaf))
            })
            .cloned()
            .unwrap_or_else(|| panic!("{leaf}: {preview}"))
    };
    assert!(has_reason(&status("task-1-"), "dirty-tracked"), "{preview}");
    assert!(
        has_reason(&status("task-3-"), "dirty-untracked"),
        "{preview}"
    );
    assert_eq!(status("task-2-")["status"], "eligible", "{preview}");
    assert_eq!(status("/integrate")["status"], "eligible", "{preview}");
    let applied = fixture.json(&[
        "workspace",
        "apply",
        "--cleanup",
        "--digest",
        &digest(&preview),
    ]);
    assert_eq!(
        applied["data"]["cleanup"]["status"], "completed",
        "{applied}"
    );
    let left = fixture.meshloop_worktrees();
    assert_eq!(left.len(), 2, "{left:?}");
    assert_eq!(
        fs::read(task1.join("README.md")).unwrap(),
        b"modified tracked bytes\n"
    );
    assert_eq!(
        fs::read(task3.join("scratch.txt")).unwrap(),
        b"untracked user bytes\n"
    );
    let branches = fixture.branches(&format!("meshloop/{GRAPH}"));
    assert_eq!(branches.len(), 2, "{branches:?}");
    assert!(
        branches
            .iter()
            .all(|b| b.contains("/task-1/") || b.contains("/task-3/"))
    );
}

fn sql(fixture: &Fixture, statement: &str) {
    let conn =
        rusqlite::Connection::open(fixture.repo.join(".meshloop").join("state.sqlite")).unwrap();
    conn.execute_batch(statement).unwrap();
}

/// A run created before this version (no ownership records) keeps today's
/// behaviour: it is never bound later, inventory reports it legacy-unbound,
/// and cleanup does not select it. A migration-marked run stays refused until
/// it is adopted.
#[test]
fn pre_existing_legacy_runs_stay_unowned_until_adopted() {
    let fixture = Fixture::landing("legacy", "");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    // Reduce the ledger to the shape an earlier version left: no fence file, no
    // repository, root, generation or artifact records.
    sql(
        &fixture,
        "DELETE FROM workspace_artifacts; DELETE FROM workspace_generations; \
         DELETE FROM workspace_roots; DELETE FROM workspace_repositories;",
    );
    let registry = fixture
        .repo
        .join(".git")
        .join("meshloop-workspace-registry.json");
    fs::remove_file(&registry).unwrap();
    fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
    fixture.ok(&["resume"]);
    fixture.ok(&["accept", "--task", "2", "--as", "probe"]);
    fixture.ok(&["accept", "--task", "3", "--as", "probe"]);
    fixture.ok(&["resume"]);
    git(
        &fixture.repo,
        &[
            "merge",
            "--quiet",
            "--ff-only",
            &format!("meshloop/{GRAPH}/integrate"),
        ],
    );
    // Nothing this version ran afterwards created a fence for the legacy run.
    assert!(!registry.exists());
    let inventory = fixture.json(&["workspace", "inventory"]);
    let mine: Vec<&serde_json::Value> = inventory["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["graph_id"] == GRAPH)
        .collect();
    assert!(!mine.is_empty(), "{inventory}");
    assert!(
        mine.iter()
            .all(|entry| entry["ownership"] == "legacy-unbound" && entry["lifecycle_id"].is_null()),
        "{inventory}"
    );
    let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
    assert!(
        candidates(&preview)
            .iter()
            .all(|item| item["status"] == "blocked"),
        "{preview}"
    );
    let applied = fixture.json(&[
        "workspace",
        "apply",
        "--cleanup",
        "--digest",
        &digest(&preview),
    ]);
    assert_ne!(applied["data"]["cleanup"]["apply"], true, "{applied}");
    assert_eq!(fixture.meshloop_worktrees().len(), 4);

    // A migration-marked run is refused until explicit adoption.
    sql(
        &fixture,
        &format!("INSERT INTO workspace_legacy_unbound VALUES('runs','{GRAPH}','Unresolved');"),
    );
    let refused = fixture.meshloop(&["resume", "--graph", GRAPH]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("LegacyOwnershipUnresolved"),
        "{refused:?}"
    );
    // Adoption never creates the fence; it reports what is missing and writes nothing.
    let adopt = fixture.json(&["workspace", "adopt"]);
    assert_eq!(adopt["data"]["apply"], false, "{adopt}");
    assert_eq!(adopt["data"]["status"], "fence-missing", "{adopt}");
    assert!(!registry.exists());
    assert_eq!(fixture.meshloop_worktrees().len(), 4);
}

/// Ownership is written under the repository fence. When another holder keeps it,
/// the run stops before it records an attempt or adds a worktree, and continues
/// once the fence is free.
#[test]
fn busy_fence_stops_before_any_attempt_state_changes() {
    use meshloop_engine::workspace::RepositoryIdentity;
    let fixture = Fixture::landing("busy", "");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
    let ledger = fixture.repo.join(".meshloop").join("state.sqlite");
    let count = || -> i64 {
        rusqlite::Connection::open(&ledger)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM attempts", [], |r| r.get(0))
            .unwrap()
    };
    let attempts = count();
    let worktrees = fixture.meshloop_worktrees();
    let held = meshloop_adapters::workspace::GitRepositoryIdentity::new("git")
        .acquire(&fixture.repo, &ledger)
        .unwrap();
    let refused = fixture.meshloop(&["resume"]);
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stdout).contains("holds the repository fence"),
        "{refused:?}"
    );
    assert_eq!(count(), attempts);
    assert_eq!(fixture.meshloop_worktrees(), worktrees);
    drop(held);
    let resumed = fixture.ok(&["resume"]);
    assert!(count() > attempts, "{resumed}");
    let inventory = fixture.json(&["workspace", "inventory"]);
    assert!(
        !inventory.to_string().contains("legacy-unbound"),
        "{inventory}"
    );
}

/// A new run is pinned to the root it was bound under: resuming it with another
/// worktree base is refused before an attempt is recorded or a worktree added.
#[test]
fn owned_run_is_pinned_to_its_registered_root() {
    let fixture = Fixture::landing("pinned", "");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
    let ledger = fixture.repo.join(".meshloop").join("state.sqlite");
    let attempts = || -> i64 {
        rusqlite::Connection::open(&ledger)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM attempts", [], |r| r.get(0))
            .unwrap()
    };
    let before = attempts();
    let other = fixture.repo.join("other-root");
    let refused = fixture.meshloop(&["resume", "--worktree-base", other.to_str().unwrap()]);
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("pinned"),
        "{refused:?}"
    );
    assert_eq!(attempts(), before);
    assert!(!other.join(GRAPH).exists());
    fixture.ok(&["resume"]);
    assert!(attempts() > before);
}

/// A worktree of an owned run that has no artifact record (for example one made by
/// an embedder without ownership) is unresolved: cleanup never selects it, and the
/// run is not reclaimed automatically.
#[test]
fn unregistered_worktree_of_an_owned_run_is_never_selected() {
    let fixture = Fixture::landing("unregistered", "");
    fixture.integrate_all();
    sql(
        &fixture,
        &format!(
            "DELETE FROM workspace_artifacts WHERE json_extract(payload_json,'$.relative_path') LIKE '{GRAPH}/task-3-%';"
        ),
    );
    let out = fixture.ok(&[
        "integrate",
        "--graph",
        GRAPH,
        "--into",
        "main",
        "--accept-integrate",
    ]);
    assert!(out.contains(&format!("run {GRAPH} kept")), "{out}");
    assert_eq!(fixture.meshloop_worktrees().len(), 4);
    let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
    let unowned: Vec<serde_json::Value> = candidates(&preview)
        .into_iter()
        .filter(|item| {
            item["location"]["relative_path"]
                .as_str()
                .is_some_and(|path| path.contains("task-3-"))
        })
        .collect();
    assert!(!unowned.is_empty(), "{preview}");
    for item in &unowned {
        assert_eq!(item["status"], "blocked", "{item}");
        assert!(has_reason(item, "ambiguous-ownership"), "{item}");
    }
    fixture.json(&[
        "workspace",
        "apply",
        "--cleanup",
        "--digest",
        &digest(&preview),
    ]);
    let task3 = fixture.meshloop_worktrees();
    assert_eq!(task3.len(), 1, "{task3:?}");
    assert!(task3[0].contains("task-3-"), "{task3:?}");
}

/// Review round 1 blocker: landing is judged against the base branch recorded when
/// the run was bound, never against whatever HEAD the main checkout has now.
#[test]
fn detached_or_foreign_head_never_counts_as_landed() {
    for (tag, checkout) in [
        ("detached", vec!["checkout", "--quiet", "--detach"]),
        ("foreign", vec!["checkout", "--quiet", "-b", "user/other"]),
    ] {
        let fixture = Fixture::landing(tag, "");
        fixture.integrate_all();
        let integrate = format!("meshloop/{GRAPH}/integrate");
        let mut args = checkout.clone();
        args.push(&integrate);
        git(&fixture.repo, &args);
        // HEAD now contains every run commit, but main (the base) does not.
        let out = fixture.ok(&["resume"]);
        assert!(!out.contains("reclaimed run"), "{tag}: {out}");
        assert_eq!(fixture.meshloop_worktrees().len(), 4, "{tag}: {out}");
        assert_eq!(
            fixture.branches(&format!("meshloop/{GRAPH}")).len(),
            4,
            "{tag}"
        );
        let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
        let run = run_candidates(&preview, GRAPH);
        assert!(
            run.iter().all(|item| item["status"] == "blocked"),
            "{tag}: {preview}"
        );
        let applied = fixture.json(&[
            "workspace",
            "apply",
            "--cleanup",
            "--digest",
            &digest(&preview),
        ]);
        assert_ne!(
            applied["data"]["cleanup"]["apply"], true,
            "{tag}: {applied}"
        );
        assert_eq!(fixture.meshloop_worktrees().len(), 4, "{tag}");
        assert_eq!(
            fixture.branches(&format!("meshloop/{GRAPH}")).len(),
            4,
            "{tag}"
        );
    }
}

/// Switches the fixture from failing to working attempts and grants one more
/// attempt, so `resume --retry` can supersede the failed one.
fn rewrite_invoke_args(fixture: &Fixture, from: &str, to: &str) {
    let text = fs::read_to_string(&fixture.config).unwrap();
    assert!(text.contains(from), "{text}");
    let text = text
        .replace(from, to)
        .replace("max_retries = 1", "max_retries = 2");
    fs::write(&fixture.config, text).unwrap();
}

fn ledger_count(fixture: &Fixture, sql: &str) -> i64 {
    rusqlite::Connection::open(fixture.repo.join(".meshloop").join("state.sqlite"))
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

const NOOP: &str = r#"["--noop"]"#;
const WORK: &str = r#"["--prompt-file", "{prompt_file}"]"#;

/// Item 5: a failed attempt superseded by an integrated one is reclaimed when it
/// is clean with nothing off the base branch; with its own commit it is retained
/// as `attempt-failed`.
#[test]
fn superseded_failed_attempts_follow_the_attempt_failed_rule() {
    for keep_commit in [false, true] {
        let tag = if keep_commit {
            "failed-kept"
        } else {
            "failed-clean"
        };
        let fixture = Fixture::new(
            tag,
            "",
            NOOP,
            r#"{"graph_id":"probe-graph","nodes":[{"id":1,"description":"only","depends_on":[],"tier":null}]}"#,
        );
        let plan = fixture.plan_arg();
        // The first attempt makes no change and fails verification.
        let _ = fixture.meshloop(&["run", "--plan", &plan, "--accept-plan"]);
        let failed = worktree_for(&fixture, "task-1-attempt-1");
        if keep_commit {
            fs::write(failed.join("work.txt"), "unlanded work\n").unwrap();
            git(&failed, &["add", "work.txt"]);
            git(&failed, &["commit", "--quiet", "-m", "unlanded"]);
        }
        rewrite_invoke_args(&fixture, NOOP, WORK);
        fixture.ok(&["resume", "--retry"]);
        fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
        fixture.ok(&["resume"]);
        let out = fixture.ok(&[
            "integrate",
            "--graph",
            GRAPH,
            "--into",
            "main",
            "--accept-integrate",
        ]);
        if keep_commit {
            assert!(out.contains("attempt-failed"), "{out}");
            assert!(failed.join("work.txt").is_file());
            let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
            let item = run_candidates(&preview, GRAPH)
                .into_iter()
                .find(|item| {
                    item["location"]["relative_path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with("task-1-attempt-1"))
                })
                .unwrap_or_else(|| panic!("{preview}"));
            assert!(has_reason(&item, "attempt-failed"), "{item}");
            assert!(
                fixture
                    .branches(&format!("meshloop/{GRAPH}"))
                    .iter()
                    .any(|b| b.ends_with("attempt-1"))
            );
        } else {
            assert!(out.contains(&format!("reclaimed run {GRAPH}")), "{out}");
            assert!(fixture.meshloop_worktrees().is_empty());
            assert!(fixture.branches(&format!("meshloop/{GRAPH}")).is_empty());
            assert_run_gone(&fixture, GRAPH);
        }
    }
}

/// Item 9: `run --accept-plan` on a new graph refuses leftover integrate state
/// before it stages anything.
#[test]
fn run_refuses_leftover_integrate_branch_before_staging() {
    let fixture = Fixture::landing("leftover", "");
    git(
        &fixture.repo,
        &["branch", &format!("meshloop/{GRAPH}/integrate"), "main"],
    );
    let plan = fixture.plan_arg();
    let out = fixture.meshloop(&["run", "--plan", &plan, "--accept-plan"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("without a runs row"),
        "{out:?}"
    );
    assert_eq!(ledger_count(&fixture, "SELECT COUNT(*) FROM runs"), 0);
    assert_eq!(
        ledger_count(&fixture, "SELECT COUNT(*) FROM workspace_generations"),
        0
    );
}

/// #137 review: after `resume --restart` (and `run --reset`) of an owned run-loop
/// graph, `meshloop events` folds what status folds: the moved history is not a
/// transition of the current run, and `nodes` equals status's node states.
#[test]
fn events_after_a_run_loop_restart_fold_as_status_does() {
    let fixture = Fixture::landing("events-restart", "");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    // Read as the cell reads them (status and events carry worktree paths, so not
    // through `json`, which is for the workspace verbs).
    let read = |verb: &str| {
        let out = fixture.meshloop(&[verb, "--graph", GRAPH, "--json"]);
        assert!(out.status.success(), "{verb}: {out:?}");
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let events_of = || read("events");
    let before = events_of();
    fixture.ok(&["resume", "--restart"]);
    let _ = fixture.meshloop(&["run", "--plan", &plan, "--reset"]);
    let events = events_of();
    let status = read("status");
    let nodes = status["data"]["nodes"].as_array().unwrap();
    assert!(!nodes.is_empty());
    for node in nodes {
        // A node with no transition yet is Pending in status and absent from the fold.
        match events["data"]["nodes"].get(node["id"].to_string()) {
            Some(state) => assert_eq!(state, &node["state"], "{events:#}\n{status:#}"),
            None => assert_eq!(node["state"], "Pending", "{events:#}\n{status:#}"),
        }
    }
    // The restarts hid the earlier rows; every row shown is a transition of this run,
    // and its seqs come after the first run's.
    let rows = events["data"]["events"].as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|e| e["superseded"] == false), "{events:#}");
    let first_max = before["data"]["last_seq"].as_i64().unwrap();
    assert!(
        rows.iter().all(|e| e["seq"].as_i64().unwrap() > first_max),
        "{events:#}"
    );
}

/// Restart (`resume --restart`, `run --reset`) of an owned run keeps its records:
/// history moves into a generation, a new generation is pinned to the plan, and
/// only worktrees confirmed gone are retired. The restarted run still reclaims.
#[test]
fn restart_and_reset_keep_records_and_still_reclaim() {
    let fixture = Fixture::landing("restart", "");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    let artifacts = ledger_count(&fixture, "SELECT COUNT(*) FROM workspace_artifacts");
    fixture.ok(&["resume", "--restart"]);
    let _ = fixture.meshloop(&["run", "--plan", &plan, "--reset"]);
    // Nothing deleted; two restarts added two generations.
    assert!(ledger_count(&fixture, "SELECT COUNT(*) FROM workspace_artifacts") > artifacts);
    assert_eq!(
        ledger_count(
            &fixture,
            &format!("SELECT COUNT(*) FROM workspace_generations WHERE graph_id='{GRAPH}'")
        ),
        3
    );
    assert!(
        ledger_count(
            &fixture,
            "SELECT COUNT(*) FROM attempts WHERE generation_id IS NOT NULL"
        ) > 0
    );
    // Attempt ids are never reused after a restart.
    assert_eq!(
        ledger_count(
            &fixture,
            "SELECT COUNT(*) FROM (SELECT attempt_id FROM attempts GROUP BY attempt_id HAVING COUNT(*)>1)"
        ),
        0
    );
    fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
    fixture.ok(&["resume"]);
    fixture.ok(&["accept", "--task", "2", "--as", "probe"]);
    fixture.ok(&["accept", "--task", "3", "--as", "probe"]);
    let last = fixture.ok(&["resume"]);
    assert!(last.contains("Integrated"), "{last}");
    let out = fixture.ok(&[
        "integrate",
        "--graph",
        GRAPH,
        "--into",
        "main",
        "--accept-integrate",
    ]);
    assert!(out.contains(&format!("reclaimed run {GRAPH}")), "{out}");
    assert!(fixture.meshloop_worktrees().is_empty());
    assert!(fixture.branches(&format!("meshloop/{GRAPH}")).is_empty());
    assert_run_gone(&fixture, GRAPH);
}

/// A crash between registering an artifact and adding its worktree leaves a
/// record with nothing on disk. Recovery retires it, and the run still reclaims.
#[test]
fn record_without_worktree_is_retired_on_recovery() {
    let fixture = Fixture::new(
        "crash",
        "",
        NOOP,
        r#"{"graph_id":"probe-graph","nodes":[{"id":1,"description":"only","depends_on":[],"tier":null}]}"#,
    );
    let plan = fixture.plan_arg();
    let _ = fixture.meshloop(&["run", "--plan", &plan, "--accept-plan"]);
    // Emulate the crash: the attempt worktree and branch never existed.
    let failed = worktree_for(&fixture, "task-1-attempt-1");
    git(
        &fixture.repo,
        &["worktree", "remove", "--force", failed.to_str().unwrap()],
    );
    git(
        &fixture.repo,
        &[
            "branch",
            "-D",
            &format!("meshloop/{GRAPH}/task-1/attempt-1"),
        ],
    );
    rewrite_invoke_args(&fixture, NOOP, WORK);
    fixture.ok(&["resume", "--retry"]);
    assert_eq!(
        ledger_count(&fixture, "SELECT COUNT(*) FROM workspace_facts"),
        1
    );
    fixture.ok(&["accept", "--task", "1", "--as", "probe"]);
    fixture.ok(&["resume"]);
    let out = fixture.ok(&[
        "integrate",
        "--graph",
        GRAPH,
        "--into",
        "main",
        "--accept-integrate",
    ]);
    assert!(out.contains(&format!("reclaimed run {GRAPH}")), "{out}");
    assert!(fixture.meshloop_worktrees().is_empty());
    assert_run_gone(&fixture, GRAPH);
}

/// A run started on a detached HEAD records no base branch and is never judged
/// landed, even once its work is merged.
#[test]
fn run_without_base_branch_is_never_landed() {
    let fixture = Fixture::landing("no-base", "");
    git(&fixture.repo, &["checkout", "--quiet", "--detach", "main"]);
    fixture.integrate_all();
    git(&fixture.repo, &["checkout", "--quiet", "main"]);
    git(
        &fixture.repo,
        &[
            "merge",
            "--quiet",
            "--ff-only",
            &format!("meshloop/{GRAPH}/integrate"),
        ],
    );
    let preview = fixture.json(&["workspace", "preview", "--cleanup"]);
    let run = run_candidates(&preview, GRAPH);
    assert!(!run.is_empty(), "{preview}");
    for item in &run {
        assert!(has_reason(item, "base-ref-unknown"), "{item}");
    }
    fixture.json(&[
        "workspace",
        "apply",
        "--cleanup",
        "--digest",
        &digest(&preview),
    ]);
    assert_eq!(fixture.meshloop_worktrees().len(), 4);
}

/// A plan re-accepted after a mutation is pinned by a new generation before any
/// further worktree is registered.
#[test]
fn mutated_plan_gets_a_new_generation() {
    let fixture = Fixture::landing("mutated", "");
    let plan = fixture.plan_arg();
    fixture.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
    fixture.ok(&["run", "--plan", &plan]);
    let generations = || {
        ledger_count(
            &fixture,
            &format!(
                "SELECT COUNT(DISTINCT plan_hash) FROM workspace_generations WHERE graph_id='{GRAPH}'"
            ),
        )
    };
    assert_eq!(generations(), 1);
    fixture.ok(&[
        "mutate-plan",
        "--graph",
        GRAPH,
        "--mutation",
        r#"{"type":"append_followup","source_task":3,"new_tasks":[{"id":4,"description":"fourth","depends_on":[3],"tier":null}]}"#,
    ]);
    assert_eq!(generations(), 2);
}
