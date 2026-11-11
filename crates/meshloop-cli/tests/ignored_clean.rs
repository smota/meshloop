//! Issue #196: ignored files are not part of a candidate. Meshloop removes them from
//! the attempt worktree before every check and after verification, so a check cannot
//! pass on build output the worker left behind, and the check's own ignored output
//! does not keep a landed run from being reclaimed.
//!
//! Unix only: the worker and the check are `/bin/sh` scripts. Offline: git, `/bin/sh`
//! and this crate's binary only. Marker files stand in for build output (no cargo).
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GRAPH: &str = "ignored";

/// A check that passes only on prebuilt output, as a build tool that finds its
/// artifacts up to date would.
const REUSING_CHECK: &str = "if [ -f target/prebuilt ]; then echo reused-prebuilt-output; exit 0; fi\n\
                             echo built-from-a-clean-tree; exit 1\n";

/// A check that builds into the ignored `target/`, as `cargo test` does.
const BUILDING_CHECK: &str = "mkdir -p target/debug && echo built > target/debug/out && echo ok\n";

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    plan: PathBuf,
    config: PathBuf,
}

impl Fixture {
    /// `work` is the worker script body, run in the attempt worktree with `$BASE` =
    /// the fixture directory. `check` is the tracked `check.sh` run as the verify step.
    fn new(tag: &str, work: &str, check: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "meshloop-ignored-clean-{tag}-{}-{nonce}",
            std::process::id()
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let worker = base.join("worker.sh");
        fs::write(
            &worker,
            format!(
                "#!/bin/sh\nset -e\nBASE='{}'\necho work > work.txt\n{work}\n",
                base.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();

        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        fs::write(repo.join("README.md"), "seed\n").unwrap();
        fs::write(repo.join(".gitignore"), "target/\n").unwrap();
        fs::write(repo.join("check.sh"), check).unwrap();
        git(&repo, &["add", "README.md", ".gitignore", "check.sh"]);
        git(&repo, &["commit", "--quiet", "-m", "seed"]);
        let plan = base.join("plan.json");
        fs::write(
            &plan,
            format!(
                r#"{{"graph_id":"{GRAPH}","nodes":[{{"id":1,"description":"one","depends_on":[],"tier":null}}]}}"#
            ),
        )
        .unwrap();
        let config = base.join("meshloop.toml");
        fs::write(
            &config,
            format!(
                r#"
selected_harnesses = ["worker"]

[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 30

[verify]
verify_command = ["sh", "check.sh"]

[harnesses.worker]
executable = "/bin/sh"
version_args = ["-c", "echo worker-sh 1.0"]
invoke_args_template = ["{}", "{{prompt_file}}"]
model_ref = "fixture-model"
model_tier = "top"
"#,
                worker.display()
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

    fn meshloop(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_meshloop"));
        command
            .current_dir(&self.repo)
            .args(args)
            .arg("--config")
            .arg(&self.config);
        bounded_output(&mut command)
    }

    fn text(out: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.meshloop(args);
        let text = Self::text(&out);
        assert!(out.status.success(), "meshloop {args:?} failed: {text}");
        text
    }

    fn plan_arg(&self) -> String {
        self.plan.to_string_lossy().into_owned()
    }

    fn run(&self) -> String {
        let plan = self.plan_arg();
        self.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
        Self::text(&self.meshloop(&["run", "--plan", &plan]))
    }

    fn node_state(&self) -> String {
        let out = self.meshloop(&["status", "--graph", GRAPH, "--json"]);
        let status: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", Self::text(&out)));
        status["data"]["nodes"][0]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Every recorded evidence payload.
    fn evidence(&self) -> String {
        let db = self.repo.join(".meshloop").join("state.sqlite");
        let conn =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let mut stmt = conn.prepare("SELECT payload_json FROM evidence").unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.join("\n")
    }

    fn attempt_worktrees(&self) -> Vec<PathBuf> {
        let out = bounded_output(Command::new("git").current_dir(&self.repo).args([
            "worktree",
            "list",
            "--porcelain",
        ]));
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("task-1-attempt-"))
            })
            .collect()
    }

    fn meshloop_worktrees(&self) -> usize {
        let out = bounded_output(Command::new("git").current_dir(&self.repo).args([
            "worktree",
            "list",
            "--porcelain",
        ]));
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.starts_with("worktree ") && line.contains(".meshloop"))
            .count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.base.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.base);
    }
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

/// Fixture setup git: plain git, no operator global config.
fn git(repo: &Path, args: &[&str]) {
    let out = bounded_output(
        Command::new("git")
            .current_dir(repo)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null"),
    );
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// (i) The worker leaves prebuilt ignored output that would make the check pass. It
/// is not in the reviewed diff, so the check runs on a tree without it and fails.
#[test]
fn prebuilt_ignored_output_left_by_the_worker_cannot_make_the_check_pass() {
    let fixture = Fixture::new(
        "prebuilt",
        "mkdir -p target && echo stale > target/prebuilt",
        REUSING_CHECK,
    );
    let out = fixture.run();
    assert_eq!(fixture.node_state(), "Failed", "{out}");
    let evidence = fixture.evidence();
    assert!(
        evidence.contains("before check: removed 1 ignored entries"),
        "{evidence}"
    );
    assert!(evidence.contains("built-from-a-clean-tree"), "{evidence}");
    assert!(!evidence.contains("reused-prebuilt-output"), "{evidence}");
    // Counts only: no ignored name reaches the evidence note.
    assert!(!evidence.contains("target/prebuilt"), "{evidence}");
}

/// (ii) The check builds into an ignored directory. That output is removed after
/// verification, so the landed run is reclaimed automatically. A link the worker
/// planted in the ignored directory is removed without being followed, and the
/// operator's own ignored files in the checkout are never touched.
#[test]
fn ignored_output_of_the_check_does_not_block_reclaiming_a_landed_run() {
    let fixture = Fixture::new(
        "reclaim",
        "mkdir -p target && ln -s \"$BASE/outside\" target/escape",
        BUILDING_CHECK,
    );
    let outside = fixture.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("precious.txt"), "operator bytes\n").unwrap();
    fs::create_dir_all(fixture.repo.join("target")).unwrap();
    fs::write(fixture.repo.join("target/operator-cache"), "cache\n").unwrap();

    let out = fixture.run();
    assert_eq!(fixture.node_state(), "AwaitingReview", "{out}");
    let evidence = fixture.evidence();
    assert!(
        evidence.contains("before check: removed 1 ignored entries"),
        "{evidence}"
    );
    assert!(
        evidence.contains("after check: removed 1 ignored entries"),
        "{evidence}"
    );
    for worktree in fixture.attempt_worktrees() {
        assert!(!worktree.join("target").exists(), "{}", worktree.display());
    }
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
    assert_eq!(fixture.meshloop_worktrees(), 0, "{out}");
    assert_eq!(
        fs::read_to_string(outside.join("precious.txt")).unwrap(),
        "operator bytes\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.repo.join("target/operator-cache")).unwrap(),
        "cache\n"
    );
}

/// (v) A nested repository inside an ignored directory refuses the attempt before
/// anything is cleaned: the worktree keeps every ignored entry for inspection.
#[test]
fn a_nested_repository_is_refused_before_any_ignored_file_is_removed() {
    let fixture = Fixture::new(
        "nested",
        "mkdir -p target/inner && git -C target/inner init -q && echo keep > target/keep",
        BUILDING_CHECK,
    );
    let out = fixture.run();
    assert_eq!(fixture.node_state(), "Failed", "{out}");
    let evidence = fixture.evidence();
    assert!(evidence.contains("nested-git"), "{evidence}");
    assert!(!evidence.contains("clean-ignored"), "{evidence}");
    let attempts = fixture.attempt_worktrees();
    assert!(!attempts.is_empty(), "{out}");
    for worktree in attempts {
        assert!(
            worktree.join("target/keep").is_file(),
            "{}",
            worktree.display()
        );
        assert!(!worktree.join("target/debug").exists());
    }
}

/// (iii) A worktree whose `git status --ignored --untracked-files=all` lists far more
/// than 16 KiB of ignored entries is read in full: it is blocked as `unknown-ignored`
/// only, never as dirty tracked or untracked content.
#[test]
fn a_long_ignored_listing_is_not_reported_as_dirty_tracked() {
    let fixture = Fixture::new("long", "", BUILDING_CHECK);
    let out = fixture.run();
    assert_eq!(fixture.node_state(), "AwaitingReview", "{out}");
    let attempts = fixture.attempt_worktrees();
    assert_eq!(attempts.len(), 1, "{out}");
    let many = attempts[0].join("target").join("many");
    fs::create_dir_all(&many).unwrap();
    let name = "x".repeat(60);
    for index in 0..1000 {
        fs::write(many.join(format!("{name}-{index}")), b"").unwrap();
    }

    let out = fixture.meshloop(&["workspace", "preview", "--cleanup", "--json"]);
    let preview: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", Fixture::text(&out)));
    let candidates = preview["data"]["cleanup"]["candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let attempt = candidates
        .iter()
        .find(|item| {
            item["location"]["relative_path"]
                .as_str()
                .is_some_and(|path| path.ends_with("task-1-attempt-1"))
        })
        .unwrap_or_else(|| panic!("no attempt candidate: {preview}"));
    let reasons: Vec<&str> = attempt["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|reason| reason.as_str())
        .collect();
    assert!(reasons.contains(&"unknown-ignored"), "{attempt}");
    for absent in [
        "dirty-tracked",
        "dirty-untracked",
        "status-too-large",
        "status-unreadable",
    ] {
        assert!(!reasons.contains(&absent), "{absent}: {attempt}");
    }
}

/// Review of #196: the base has a submodule gitlink, so `worktree add` leaves its
/// directory empty. Files the worker writes there are invisible to status, `add` and
/// `clean`; the attempt is refused before staging and the check never runs.
#[test]
fn content_in_an_unpopulated_submodule_directory_is_refused_before_the_check() {
    let fixture = Fixture::new(
        "gitlink",
        "echo 'exit 0' > sub/payload.sh",
        "echo check-ran\n",
    );
    let out = bounded_output(
        Command::new("git")
            .current_dir(&fixture.repo)
            .args(["rev-parse", "HEAD"]),
    );
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    git(
        &fixture.repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},sub"),
        ],
    );
    git(&fixture.repo, &["commit", "--quiet", "-m", "gitlink"]);

    let out = fixture.run();
    assert_eq!(fixture.node_state(), "Failed", "{out}");
    let evidence = fixture.evidence();
    assert!(
        evidence.contains("content in an unpopulated gitlink"),
        "{evidence}"
    );
    assert!(!evidence.contains("check-ran"), "{evidence}");
    assert!(!evidence.contains("clean-ignored"), "{evidence}");
}
