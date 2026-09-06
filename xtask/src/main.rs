//! Repository checks using the pinned Cargo toolchain.
use std::{
    path::Path,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["check"] {
        eprintln!("Usage: cargo run -p xtask -- check");
        return ExitCode::from(2);
    }
    let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        eprintln!("Cannot resolve workspace root");
        return ExitCode::FAILURE;
    };
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
        println!("Running cargo {}", args.join(" "));
        match Command::new("cargo").args(*args).current_dir(root).status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                eprintln!("Check failed: {status}");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("Cannot run Cargo: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
