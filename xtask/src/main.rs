//! Repository checks, session-control-plane smoke, session pack, and publish dry-run.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        eprintln!("Cannot resolve workspace root");
        return ExitCode::FAILURE;
    };
    match args.first().map(String::as_str) {
        Some("check") => check(root),
        Some("smoke") => smoke(root),
        Some("bundle") => bundle(root),
        Some("live") => live(root),
        Some("publish-dry") => publish_dry(root),
        _ => {
            eprintln!("Usage: cargo run -p xtask -- check|smoke|bundle|live|publish-dry");
            ExitCode::from(2)
        }
    }
}

fn cargo(root: &Path, args: &[&str]) -> bool {
    println!("Running cargo {}", args.join(" "));
    match Command::new("cargo").args(args).current_dir(root).status() {
        Ok(s) if s.success() => true,
        Ok(s) => {
            eprintln!("Check failed: {s}");
            false
        }
        Err(e) => {
            eprintln!("Cannot run Cargo: {e}");
            false
        }
    }
}

fn check(root: &Path) -> ExitCode {
    if let Err(e) = assert_embedded_skills_match(root) {
        eprintln!("Check failed: {e}");
        return ExitCode::FAILURE;
    }
    let checks: &[&[&str]] = &[
        &["fmt", "--all", "--", "--check"],
        &[
            "check",
            "--workspace",
            "--all-targets",
            "--locked",
            "--offline",
        ],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--offline",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace", "--locked", "--offline"],
    ];
    for args in checks {
        if !cargo(root, args) {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn meshloop_bin(root: &Path) -> PathBuf {
    root.join("target/debug/meshloop")
        .with_extension(std::env::consts::EXE_EXTENSION)
}

fn smoke(root: &Path) -> ExitCode {
    if !cargo(
        root,
        &["build", "-p", "meshloop-cli", "--locked", "--offline"],
    ) {
        return ExitCode::FAILURE;
    }
    let bin = meshloop_bin(root);
    let cases: &[(&[&str], &str)] = &[
        (&["--help"], "meshloop:plan"),
        (&["--help"], "meshloop:bundle"),
        (&["--help"], "meshloop:review-plan"),
        (&["roles", "--json"], "meshloop:reviewer"),
        (&["roles", "--json"], "meshloop:planner"),
        (&["doctor", "--json"], "meshloop:doctor"),
        (&["meshloop:roles", "--json"], "meshloop:origin"),
        (&["reviewer"], "unprefixed"),
    ];
    for (args, marker) in cases {
        let output = Command::new(&bin).args(*args).current_dir(root).output();
        let Ok(output) = output else {
            eprintln!("smoke: failed to spawn meshloop {args:?}");
            return ExitCode::FAILURE;
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !text.contains(marker) {
            eprintln!("smoke: expected {marker:?} in meshloop {args:?}, got:\n{text}");
            return ExitCode::FAILURE;
        }
    }
    println!("smoke: session-control-plane surface ok");
    ExitCode::SUCCESS
}

fn bundle(root: &Path) -> ExitCode {
    let skills_src = root.join("skills");
    let embedded = root.join("crates/meshloop-cli/session-bundle/skills");
    let _ = fs::remove_dir_all(&embedded);
    copy_dir(&skills_src, &embedded);
    if !cargo(
        root,
        &["build", "-p", "meshloop-cli", "--locked", "--offline"],
    ) {
        return ExitCode::FAILURE;
    }
    let dest = root.join("dist/meshloop-session-bundle");
    let bin = meshloop_bin(root);
    match Command::new(&bin)
        .args(["bundle", "--dest"])
        .arg(&dest)
        .current_dir(root)
        .status()
    {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => {
            eprintln!("bundle: meshloop bundle failed: {s}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("bundle: cannot run meshloop: {e}");
            ExitCode::FAILURE
        }
    }
}

const PUBLISH_CRATES: &[&str] = &[
    "meshloop-domain",
    "meshloop-engine",
    "meshloop-adapters",
    "meshloop-cli",
];

fn publish_dry(root: &Path) -> ExitCode {
    if let Err(e) = assert_embedded_skills_match(root) {
        eprintln!("publish-dry: {e}");
        return ExitCode::FAILURE;
    }
    for name in PUBLISH_CRATES {
        if let Err(e) = assert_publish_manifest(root, name) {
            eprintln!("publish-dry: {e}");
            return ExitCode::FAILURE;
        }
        println!("Running cargo package -p {name} --locked --allow-dirty");
        let output = Command::new("cargo")
            .args(["package", "-p", name, "--locked", "--allow-dirty"])
            .current_dir(root)
            .output();
        let Ok(output) = output else {
            eprintln!("publish-dry: cannot run cargo");
            return ExitCode::FAILURE;
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{text}");
        if output.status.success() {
            continue;
        }
        if text.contains("no matching package named `meshloop-") {
            println!(
                "publish-dry: {name} packaging waits for its meshloop-* dependency on crates.io (first upload is domain → engine → adapters → cli)"
            );
            continue;
        }
        eprintln!("publish-dry: cargo package -p {name} failed");
        return ExitCode::FAILURE;
    }
    println!("publish-dry: manifests ready; packaged what the index currently allows");
    ExitCode::SUCCESS
}

fn assert_publish_manifest(root: &Path, name: &str) -> Result<(), String> {
    let path = root.join("crates").join(name).join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if text.contains("publish = false") {
        return Err(format!("{name} is marked publish = false"));
    }
    if !text.contains("description = ") {
        return Err(format!("{name} is missing description"));
    }
    if !text.contains("repository.workspace = true") {
        return Err(format!("{name} is missing repository"));
    }
    Ok(())
}

fn assert_embedded_skills_match(root: &Path) -> Result<(), String> {
    let src = root.join("skills");
    let embedded = root.join("crates/meshloop-cli/session-bundle/skills");
    let src_files = collect_files(&src)?;
    let embedded_files = collect_files(&embedded)?;
    if src_files.len() != embedded_files.len() {
        return Err(format!(
            "embedded session pack file count {} != repo skills {}; run cargo run -p xtask -- bundle",
            embedded_files.len(),
            src_files.len()
        ));
    }
    for (rel, src_path) in &src_files {
        let Some(emb_path) = embedded_files.get(rel) else {
            return Err(format!(
                "embedded session pack missing {rel}; run cargo run -p xtask -- bundle"
            ));
        };
        let a = fs::read(src_path).map_err(|e| format!("read {}: {e}", src_path.display()))?;
        let b = fs::read(emb_path).map_err(|e| format!("read {}: {e}", emb_path.display()))?;
        if a != b {
            return Err(format!(
                "embedded session pack drifted at {rel}; run cargo run -p xtask -- bundle"
            ));
        }
    }
    Ok(())
}

fn collect_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut out = BTreeMap::new();
    collect_files_into(dir, dir, &mut out)?;
    Ok(out)
}

fn collect_files_into(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_into(root, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| format!("strip prefix {}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, path);
        }
    }
    Ok(())
}

fn live(root: &Path) -> ExitCode {
    if !cargo(
        root,
        &["build", "-p", "meshloop-cli", "--locked", "--offline"],
    ) {
        return ExitCode::FAILURE;
    }
    let herdr = Command::new("herdr")
        .arg("status")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if !herdr.contains("status: running") && !herdr.contains("status:running") {
        eprintln!("xtask live: Herdr server is not running. Launch sign-off requires live Herdr.");
        return ExitCode::FAILURE;
    }

    let bin = meshloop_bin(root);
    let doctor = Command::new(&bin)
        .args(["doctor", "--json"])
        .current_dir(root)
        .output();
    let Ok(doctor) = doctor else {
        eprintln!("xtask live: failed to run meshloop doctor");
        return ExitCode::FAILURE;
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    if !text.contains("\"herdr_server_running\": true") {
        eprintln!("xtask live: doctor did not report herdr_server_running true:\n{text}");
        return ExitCode::FAILURE;
    }
    if !text.contains("origin_session") {
        eprintln!("xtask live: doctor JSON missing origin_session:\n{text}");
        return ExitCode::FAILURE;
    }

    let origin = std::env::var("MESHLOOP_ORIGIN_SESSION")
        .ok()
        .filter(|s| !s.is_empty());
    let origin = match origin {
        Some(s) => s,
        None => {
            let cur = Command::new("herdr").args(["pane", "current"]).output();
            match cur {
                Ok(o) if o.status.success() => {
                    let raw = String::from_utf8_lossy(&o.stdout);
                    raw.lines()
                        .map(str::trim)
                        .find(|l| l.contains(":p") || l.starts_with('w'))
                        .unwrap_or("")
                        .to_string()
                }
                _ => String::new(),
            }
        }
    };
    if origin.is_empty() {
        eprintln!("xtask live: set MESHLOOP_ORIGIN_SESSION so the supervisor pane is never split");
        return ExitCode::FAILURE;
    }

    let list = Command::new("herdr").args(["pane", "list"]).output();
    let Ok(list) = list else {
        eprintln!("xtask live: herdr pane list failed");
        return ExitCode::FAILURE;
    };
    let list_text = String::from_utf8_lossy(&list.stdout);
    if !list_text.contains(&origin) && !list_text.contains("pane") {
        eprintln!("xtask live: could not list panes to split from a non-origin pane");
        return ExitCode::FAILURE;
    }

    println!(
        "xtask live: Herdr running; doctor reports origin_session; origin pane {origin} will not be split"
    );
    ExitCode::SUCCESS
}

fn copy_dir(src: &Path, dst: &Path) {
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    let _ = fs::create_dir_all(dst);
    for entry in entries.flatten() {
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            let _ = fs::copy(entry.path(), to);
        }
    }
}
