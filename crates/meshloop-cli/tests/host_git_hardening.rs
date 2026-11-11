//! Issue #180: host-side git that Meshloop runs in a worker's worktree must not run
//! anything the worker chose. Each test is a worker (a `/bin/sh` script run as the
//! harness) that plants one minimal, non-weaponised trigger whose only effect is to
//! create a marker file. After verify, integrate and cleanup, no marker may exist.
//!
//! Unix only: the worker and the trigger are shell scripts. Offline, no sockets:
//! git, `/bin/sh` and this crate's binary only.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GRAPH: &str = "hostgit";

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    plan: PathBuf,
    config: PathBuf,
    marker: PathBuf,
    /// The operator's global git config handed to Meshloop (`GIT_CONFIG_GLOBAL`).
    global: PathBuf,
}

impl Fixture {
    /// `attack` is the body of the worker script. It runs in the attempt worktree with
    /// `$1` = the prompt file, `$TRIGGER` = the marker-writing script, `$BASE` = the
    /// fixture directory.
    fn new(tag: &str, attack: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "meshloop-host-git-{tag}-{}-{nonce}",
            std::process::id()
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let marker = base.join("MARKER");
        // The trigger: create the marker, then pass stdin through (a clean filter or
        // textconv must still produce output). Nothing else.
        let trigger = base.join("trigger.sh");
        fs::write(
            &trigger,
            format!("#!/bin/sh\n: > '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&trigger, fs::Permissions::from_mode(0o755)).unwrap();
        // An operator global config that defines drivers a worker could select with
        // `.gitattributes`. Meshloop must not run them on worker content.
        let global = base.join("operator.gitconfig");
        let t = trigger.display();
        fs::write(
            &global,
            format!(
                "[filter \"marker\"]\n\tclean = {t}\n\tsmudge = {t}\n\
                 [diff \"marker\"]\n\tcommand = {t}\n\ttextconv = {t}\n\
                 [merge \"marker\"]\n\tdriver = {t}\n\
                 [user]\n\tname = Operator\n\temail = operator@example.invalid\n"
            ),
        )
        .unwrap();
        let worker = base.join("worker.sh");
        fs::write(
            &worker,
            format!(
                "#!/bin/sh\nset -e\nTRIGGER='{t}'\nBASE='{}'\necho work > work.txt\n{attack}\n",
                base.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();

        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        fs::write(repo.join("README.md"), "seed\n").unwrap();
        git(&repo, &["add", "README.md"]);
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
verify_command = []

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
            marker,
            global,
        }
    }

    fn meshloop(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_meshloop"));
        command
            .current_dir(&self.repo)
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .env("GIT_CONFIG_GLOBAL", &self.global);
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
        self.assert_no_marker(args, &text);
        text
    }

    /// Runs a command whose success is not the point; only the marker is checked.
    fn any(&self, args: &[&str]) -> String {
        let out = self.meshloop(args);
        let text = Self::text(&out);
        self.assert_no_marker(args, &text);
        text
    }

    fn assert_no_marker(&self, args: &[&str], text: &str) {
        assert!(
            !self.marker.exists(),
            "meshloop {args:?} ran a worker-chosen program (marker created): {text}"
        );
    }

    fn plan_arg(&self) -> String {
        self.plan.to_string_lossy().into_owned()
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

    /// Verify, accept, integrate into the integrate worktree, land on `main` and
    /// reclaim (cleanup). Every step is checked for the marker.
    fn land(&self) {
        let plan = self.plan_arg();
        self.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
        self.ok(&["run", "--plan", &plan]);
        assert_eq!(self.node_state(), "AwaitingReview");
        self.ok(&["accept", "--task", "1", "--as", "probe"]);
        self.ok(&["resume"]);
        let out = self.ok(&[
            "integrate",
            "--graph",
            GRAPH,
            "--into",
            "main",
            "--accept-integrate",
        ]);
        assert!(out.contains(&format!("reclaimed run {GRAPH}")), "{out}");
        self.ok(&["workspace", "preview", "--cleanup"]);
        assert!(!self.marker.exists());
    }

    /// The attempt (and its retry) must fail; cleanup preview must still be safe.
    fn refused(&self) -> String {
        let plan = self.plan_arg();
        self.ok(&["review-plan", "--plan", &plan, "--accept", "--as", "probe"]);
        let out = self.any(&["run", "--plan", &plan]);
        assert_eq!(self.node_state(), "Failed", "{out}");
        self.any(&["events", "--graph", GRAPH, "--json"]);
        self.any(&["workspace", "inventory"]);
        self.any(&["workspace", "preview", "--cleanup"]);
        self.any(&["resume"]);
        assert!(!self.marker.exists());
        self.evidence()
    }

    /// Every recorded evidence payload, for the failing check's name and reason.
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

/// Fixture setup git, before any worker ran: plain git, no operator global config.
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

/// (i) The worker points its worktree's `.git` link at a repository it controls,
/// whose config and hooks would run the trigger. Meshloop refuses the worktree and
/// the attempt fails; nothing follows the link.
#[test]
fn rewritten_git_link_is_refused_and_fails_the_attempt() {
    let fixture = Fixture::new(
        "link",
        r#"EVIL="$BASE/evil-$(basename "$1")"
git init -q "$EVIL"
git -C "$EVIL" config core.fsmonitor "$TRIGGER"
for h in pre-commit post-commit post-checkout reference-transaction; do cp "$TRIGGER" "$EVIL/.git/hooks/$h"; done
printf 'gitdir: %s\n' "$EVIL/.git" > .git"#,
    );
    let evidence = fixture.refused();
    assert!(
        evidence.contains("git-link") && evidence.contains(".git link was modified"),
        "{evidence}"
    );
}

/// (ii) The worker installs hooks where host-side git would look for them (the
/// repository's hooks directory, shared by every worktree).
#[test]
fn hooks_planted_by_the_worker_never_run() {
    let fixture = Fixture::new(
        "hooks",
        r#"HOOKS="$(git rev-parse --path-format=absolute --git-path hooks)"
mkdir -p "$HOOKS"
for h in pre-commit commit-msg post-commit post-checkout post-merge reference-transaction post-rewrite pre-auto-gc; do cp "$TRIGGER" "$HOOKS/$h"; done"#,
    );
    fixture.land();
}

/// (iii) The worker sets executable keys in the repository config.
#[test]
fn executable_repository_config_never_runs() {
    let fixture = Fixture::new(
        "config",
        r#"git config core.fsmonitor "$TRIGGER"
git config diff.external "$TRIGGER"
git config core.pager "$TRIGGER"
git config core.editor "$TRIGGER"
git config sequence.editor "$TRIGGER"
git config commit.gpgSign true
git config gpg.program "$TRIGGER"
git config merge.verifySignatures true"#,
    );
    fixture.land();
}

/// (iv) The worker commits attributes that select filter, diff and merge drivers
/// the operator's global config defines.
#[test]
fn attributes_cannot_select_global_config_drivers() {
    let fixture = Fixture::new(
        "attrs",
        r#"printf '* filter=marker diff=marker merge=marker\n' > .gitattributes"#,
    );
    fixture.land();
}

/// (v) The worker commits symlinks at future prompt-file names pointing at an
/// operator-owned file. The change is refused by the diff gate and the file is
/// never written through.
#[test]
fn symlink_at_a_future_prompt_file_name_is_refused() {
    let fixture = Fixture::new(
        "prompt",
        r#"for n in 1 2 3 4 5 6 7 8; do ln -sf "$BASE/victim.txt" ".meshloop-prompt-$n" 2>/dev/null || true; done"#,
    );
    let victim = fixture.base.join("victim.txt");
    fs::write(&victim, "operator bytes\n").unwrap();
    let evidence = fixture.refused();
    assert!(evidence.contains("reserved-paths"), "{evidence}");
    assert_eq!(fs::read(&victim).unwrap(), b"operator bytes\n");
}

/// (vi) The worker creates a nested repository whose own config defines a filter its
/// own `.gitattributes` selects, and leaves a file dirty in it. Host-side status/diff
/// recursing into it would run the filter. The attempt is refused instead.
#[test]
fn nested_repository_in_the_worktree_is_refused() {
    let fixture = Fixture::new(
        "nested",
        r#"mkdir sub
cd sub
git init -q
git config user.name w
git config user.email w@example.invalid
git config filter.marker.clean "$TRIGGER"
printf '* filter=marker\n' > .gitattributes
echo one > f
GIT_CONFIG_GLOBAL=/dev/null git -c filter.marker.clean=cat add .gitattributes f
GIT_CONFIG_GLOBAL=/dev/null git -c filter.marker.clean=cat commit -q -m nested
echo two > f
cd .."#,
    );
    let evidence = fixture.refused();
    assert!(evidence.contains("nested-git"), "{evidence}");
}
