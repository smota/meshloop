//! Repository checks, session-control-plane smoke, and wrapper bundle.
use std::{
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
        _ => {
            eprintln!("Usage: cargo run -p xtask -- check|smoke|bundle");
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
    let dest = root.join("dist/meshloop-session-bundle");
    let _ = fs::remove_dir_all(&dest);
    if fs::create_dir_all(dest.join("skills")).is_err() {
        eprintln!("bundle: cannot create dist");
        return ExitCode::FAILURE;
    }
    let skills_src = root.join("skills");
    copy_dir(&skills_src, &dest.join("skills"));
    let catalog = format!(
        "{{\n  \"name\": \"meshloop-session-bundle\",\n  \"version\": \"{}\",\n  \"namespace\": \"meshloop:\",\n  \"mcp\": \"meshloop mcp\",\n  \"slash_prefix\": \"/meshloop:\",\n  \"roles\": [\"meshloop:origin\",\"meshloop:planner\",\"meshloop:scout\",\"meshloop:worker\",\"meshloop:reviewer\"]\n}}\n",
        env!("CARGO_PKG_VERSION")
    );
    if fs::write(dest.join("meshloop-mcp-tools.json"), catalog).is_err() {
        return ExitCode::FAILURE;
    }
    let readme = "Meshloop session bundle\n\nInstall skills from skills/.\nRun local MCP: meshloop mcp\nNever use unprefixed plan/reviewer/scout tools.\n";
    let _ = fs::write(dest.join("README.md"), readme);
    println!("bundled to {}", dest.display());
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
