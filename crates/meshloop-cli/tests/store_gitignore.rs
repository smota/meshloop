//! Issue #44: the local `.meshloop/` store must stay out of consuming repositories' Git
//! status. Uses the fixture harness only.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_path() -> String {
    let exe = format!("fixture_harness{}", std::env::consts::EXE_SUFFIX);
    let mut dir = std::env::current_exe().expect("current test executable path");
    dir.pop();
    dir.pop();
    let path = dir.join(&exe);
    assert!(path.exists(), "expected fixture harness at {}", path.display());
    path.to_string_lossy().to_string()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run git");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn temp_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meshloop-gi-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "test@test"]);
    git(&dir, &["config", "user.name", "test"]);
    // Isolate from the developer's global excludes file.
    git(&dir, &["config", "core.excludesFile", ""]);
    fs::write(dir.join("README.md"), "seed").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "seed"]);
    dir
}

fn write_config(dir: &Path) -> PathBuf {
    let path = dir.join("meshloop.toml");
    let contents = format!(
        r#"
selected_harnesses = ["fixture"]

[limits]
max_concurrent_workers = 1
max_retries = 2
task_timeout_seconds = 30

[verify]
verify_command = []

[harnesses.fixture]
executable = "{}"
version_args = ["--version"]
invoke_args_template = ["--emit-graph"]
model_ref = "fixture-model"
model_tier = "top"
"#,
        fixture_path().replace('\\', "\\\\")
    );
    fs::write(&path, contents).unwrap();
    path
}

fn meshloop(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_meshloop"));
    c.current_dir(dir);
    c
}

fn doctor_store_ignored(dir: &Path, cfg: &Path, db: Option<&Path>) -> bool {
    let mut c = meshloop(dir);
    c.args(["doctor", "--json", "--config"]).arg(cfg);
    if let Some(db) = db {
        c.arg("--db").arg(db);
    }
    let out = c.output().unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json prints JSON");
    v["data"]["store_ignored"]
        .as_bool()
        .expect("store_ignored is a bool")
}

#[test]
fn plan_in_fresh_repo_never_shows_store_as_untracked() {
    let dir = temp_repo("plan");
    let cfg = write_config(&dir);
    let out = meshloop(&dir)
        .args(["plan", "--fixture-only", "--objective", "x", "--config"])
        .arg(&cfg)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join(".meshloop").join("state.sqlite").exists());
    assert_eq!(
        fs::read_to_string(dir.join(".meshloop").join(".gitignore")).unwrap(),
        "*\n"
    );
    assert!(!dir.join(".gitignore").exists(), "root .gitignore untouched");
    let status = git(&dir, &["status", "--porcelain", "-uall"]);
    assert!(!status.contains(".meshloop"), "status: {status}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(".gitignore"),
        "one-time notice expected on stderr"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn bundle_reports_missing_and_only_edits_with_flag() {
    let dir = temp_repo("bundle");
    let root_ignore = dir.join(".gitignore");
    fs::write(&root_ignore, "target/").unwrap();
    let before = fs::read(&root_ignore).unwrap();

    let out = meshloop(&dir)
        .args(["bundle", "--dest", ".", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["gitignore"], "missing");
    assert_eq!(v["data"]["gitignore_line"], "/.meshloop/");
    assert_eq!(fs::read(&root_ignore).unwrap(), before);

    let human = meshloop(&dir).args(["bundle", "--dest", "."]).output().unwrap();
    assert!(String::from_utf8_lossy(&human.stdout).contains("gitignore: missing"));
    assert_eq!(fs::read(&root_ignore).unwrap(), before);

    let out = meshloop(&dir)
        .args(["bundle", "--dest", ".", "--gitignore", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["gitignore"], "added");
    let after = fs::read_to_string(&root_ignore).unwrap();
    assert_eq!(after.matches("# >>> meshloop >>>").count(), 1);
    assert!(after.contains("/.meshloop/"));
    assert!(after.starts_with("target/\n"));

    let out = meshloop(&dir)
        .args(["bundle", "--dest", ".", "--gitignore", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["gitignore"], "present");
    assert_eq!(fs::read_to_string(&root_ignore).unwrap(), after);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn doctor_reports_store_ignored_before_and_after() {
    let dir = temp_repo("doctor");
    let cfg = write_config(&dir);
    assert!(!doctor_store_ignored(&dir, &cfg, None));
    let human = meshloop(&dir)
        .args(["doctor", "--config"])
        .arg(&cfg)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&human.stdout).contains("/.meshloop/"));

    let out = meshloop(&dir)
        .args(["plan", "--objective", "x", "--config"])
        .arg(&cfg)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(doctor_store_ignored(&dir, &cfg, None));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn custom_db_outside_work_tree_is_not_flagged_or_modified() {
    let dir = temp_repo("custom");
    let cfg = write_config(&dir);
    let outside = std::env::temp_dir().join(format!("meshloop-gi-out-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outside);
    let db = outside.join(".meshloop").join("state.sqlite");
    assert!(doctor_store_ignored(&dir, &cfg, Some(&db)));

    let out = meshloop(&dir)
        .args(["plan", "--objective", "x", "--config"])
        .arg(&cfg)
        .arg("--db")
        .arg(&db)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(db.exists());
    assert!(!outside.join(".meshloop").join(".gitignore").exists());
    assert!(doctor_store_ignored(&dir, &cfg, Some(&db)));
    fs::remove_dir_all(&dir).ok();
    fs::remove_dir_all(&outside).ok();
}

#[test]
fn install_docs_list_the_ignore_step() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for name in ["install.md", "start.md"] {
        let text = fs::read_to_string(docs.join(name)).unwrap();
        assert!(text.contains("/.meshloop/"), "{name} missing ignore step");
        assert!(text.contains("--gitignore"), "{name} missing --gitignore");
    }
}
