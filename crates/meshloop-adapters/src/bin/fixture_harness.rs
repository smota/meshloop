//! A controlled stand-in harness binary for adapter contract tests (runtime-design.md §2:
//! "a fixture/stub harness binary in test fixtures"). Never used against a real subscription.
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // `--model <MODEL> <mode...>`: echo the model the dispatcher passed on argv (issue
    // #134), then run the remaining mode. A model named `fail-*` stands in for a model
    // that cannot do the task: it exits 1 without touching the worktree.
    if args.first().map(String::as_str) == Some("--model") {
        let model = args.get(1).cloned().unwrap_or_default();
        println!("FIXTURE_MODEL:{model}");
        if model.starts_with("fail-") {
            std::process::exit(1);
        }
        args.drain(..args.len().min(2));
    }
    match args.first().map(String::as_str) {
        Some("--inspect-spawn") => {
            let environment: std::collections::BTreeMap<_, _> = env::vars().collect();
            println!(
                "{}",
                serde_json::json!({
                    "arguments": &args[1..],
                    "directory": env::current_dir().expect("fixture cwd"),
                    "executable": env::current_exe().expect("fixture executable"),
                    "environment": environment,
                })
            );
        }
        Some("--echo-stdin") => {
            use std::io::{Read, Write};
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .expect("fixture stdin");
            std::io::stdout().write_all(&bytes).expect("fixture stdout");
        }
        Some("--echo-args") => {
            let argc = args.len();
            println!("FIXTURE_ARGC:{argc}");
            for arg in &args {
                println!("FIXTURE_ARG:{arg}");
            }
        }
        Some("--large-output") => {
            use std::io::Write;
            let n: usize = args
                .get(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(128 * 1024);
            let stderr = std::thread::spawn(move || {
                let _ = std::io::stderr().write_all(&vec![b'e'; n]);
            });
            let _ = std::io::stdout().write_all(&vec![b'x'; n]);
            let _ = stderr.join();
        }
        Some("--emit-large-graph") => {
            use std::io::Write;
            let noise = std::thread::spawn(|| {
                let _ = std::io::stderr().write_all(&vec![b'e'; 128 * 1024]);
            });
            let json = format!(
                "{}{{\"graph_id\":\"large-planner\",\"nodes\":[{{\"id\":1,\"description\":\"task\",\"depends_on\":[],\"tier\":null}}]}}",
                " ".repeat(128 * 1024)
            );
            fs::write("meshloop-plan.json", &json).expect("write owned fixture plan");
            print!("{json}");
            let _ = noise.join();
        }
        Some("--exit-with-inherited-pipes") => {
            #[allow(
                clippy::zombie_processes,
                reason = "fixture descendant tests inherited pipe lifetime"
            )]
            let _child = meshloop_adapters::process::spawn_command(
                std::process::Command::new(env::current_exe().expect("fixture path"))
                    .args(["--sleep-ms", "2000"]),
            )
            .expect("spawn inherited writer");
        }
        Some("--version") => println!("fixture-harness 1.0.0"),
        Some("--hang") => loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        },
        Some("--fail") => std::process::exit(1),
        Some("--noop") => {}
        Some("--exhausted") => {
            eprintln!("capacity exhausted");
            std::process::exit(1);
        }
        Some("--emit-graph") => {
            let prompt = args.get(1).cloned().unwrap_or_default();
            let hash = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                prompt.hash(&mut h);
                format!("{:08x}", h.finish() as u32)
            };
            let graph_id = format!("fixture-{hash}");
            let json = format!(
                r#"{{"graph_id":"{graph_id}","nodes":[{{"id":1,"description":"first task","depends_on":[],"tier":null}},{{"id":2,"description":"second task, depends on first","depends_on":[1],"tier":null}}]}}"#
            );
            let _ = fs::write(Path::new("meshloop-plan.json"), &json);
            println!("{json}");
        }
        Some("--repair-scenario") => {
            let scenario = args.get(1).map(String::as_str).unwrap_or("");
            let prompt = prompt_from_args(&args);
            apply_repair_scenario(scenario, &prompt);
        }
        Some("--fill-stderr") => {
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8192);
            let chunk = "error[E0308]: mismatched types\n --> src/lib.rs:1:1\n";
            let mut out = String::with_capacity(n + chunk.len());
            while out.len() < n {
                out.push_str(chunk);
            }
            eprint!("{out}");
        }
        Some("--emit-transcript") => {
            // No worktree writes, so an empty_diff_ok node stays an empty diff.
            // Stdout exceeds the redact cap after a marker and a bounded `sk-` key.
            use std::io::Write;
            let marker = "MESHLOOP_HARNESS_MARKER_0123456789AB";
            let secret = format!("sk-{}", "a".repeat(40));
            let mut out = std::io::stdout();
            // Repeat the marker and key at both ends so either cap (head or tail)
            // still leaves them in the stored evidence.
            write!(
                out,
                "{marker}\n{secret}\n{}{marker}\n{secret}\n",
                "x".repeat(4096)
            )
            .expect("stdout");
            out.flush().expect("flush stdout");
            let mut err = std::io::stderr();
            write!(err, "STDERR_BODY_MARKER").expect("stderr");
            err.flush().expect("flush stderr");
            // Non-zero so evidence can show the process code without failing the gate.
            std::process::exit(7);
        }
        Some("--prompt-file") => {
            let path = args.get(1).expect("fixture requires a prompt file path");
            if let Some(ms) = args.get(2).and_then(|value| value.parse::<u64>().ok()) {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            deliver_prompt_file(path);
        }
        // `--gated-prompt-file <prompt> <gate>`: a prompt containing `GATED` waits
        // (at most 120s) for the file `<gate>` to exist before delivering, so a test
        // controls exactly when that node leaves Running. Other prompts deliver at once.
        Some("--gated-prompt-file") => {
            let path = args.get(1).expect("fixture requires a prompt file path");
            let gate = Path::new(args.get(2).expect("fixture requires a gate path"));
            if fs::read_to_string(path)
                .unwrap_or_default()
                .contains("GATED")
            {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
                while !gate.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            deliver_prompt_file(path);
        }
        Some("--record-verify") => {
            let log = args.get(1).expect("verify log path");
            let ms: u64 = args
                .get(2)
                .and_then(|value| value.parse().ok())
                .unwrap_or(1500);
            let now = || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0)
            };
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .expect("verify log");
            writeln!(file, "start {}", now()).expect("verify start");
            file.flush().expect("flush verify start");
            std::thread::sleep(std::time::Duration::from_millis(ms));
            writeln!(file, "end {}", now()).expect("verify end");
        }
        Some(arg) if arg.starts_with("--prompt-file=") => {
            let path = arg.strip_prefix("--prompt-file=").expect("prefix matched");
            let argc = args.len();
            println!("FIXTURE_ARGC:{argc}");
            println!("FIXTURE_ARG:{arg}");
            deliver_prompt_file(path);
        }
        Some("--sleep-ms") => {
            let ms: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        Some("--spawn-detached-sleeper") => {
            let ms = args.get(1).cloned().unwrap_or_else(|| "3000".to_string());
            let log = args
                .get(2)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::temp_dir()
                        .join(format!("meshloop-detached-{}.log", std::process::id()))
                });
            let exe = env::current_exe().expect("current exe");
            let mut cmd = std::process::Command::new(exe);
            cmd.arg("--sleep-ms").arg(ms);
            #[allow(
                clippy::zombie_processes,
                reason = "the detached sleeper is meant to outlive this fixture; the test kills it"
            )]
            let child =
                meshloop_adapters::process::spawn_detached(cmd, &log).expect("spawn detached");
            println!("{}", child.id());
        }
        Some("--spawn-detached-marker") => {
            let log = args.get(1).expect("log path");
            let exe = env::current_exe().expect("current exe");
            let mut cmd = std::process::Command::new(exe);
            cmd.arg("--write-stdio-markers");
            #[allow(
                clippy::zombie_processes,
                reason = "the marker child may still be exiting when this fixture returns"
            )]
            let child = meshloop_adapters::process::spawn_detached(cmd, std::path::Path::new(log))
                .expect("spawn detached");
            println!("{}", child.id());
        }
        Some("--write-stdio-markers") => {
            println!("STDOUT_MARKER");
            eprintln!("STDERR_MARKER");
        }
        Some("--grandchild-heartbeat") => {
            let path = args.get(1).expect("heartbeat file path required");
            let mut counter: u64 = 0;
            loop {
                counter += 1;
                let _ = fs::write(path, format!("{counter}\n"));
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        Some("--spawn-grandchild") => {
            let pid_file = args.get(1).expect("pid file path required");
            let hb_file = args.get(2).expect("heartbeat file path required");
            let exe = env::current_exe().expect("current exe");
            let mut child = meshloop_adapters::process::spawn_command(
                std::process::Command::new(exe)
                    .arg("--grandchild-heartbeat")
                    .arg(hb_file),
            )
            .expect("spawn grandchild");
            let _ = fs::write(pid_file, child.id().to_string());
            let _ = child.wait();
        }
        Some("--spawn-in-job-and-abort") => {
            let pid_file = args.get(1).expect("pid file path required");
            let hb_file = args.get(2).expect("heartbeat file path required");
            let exe = env::current_exe().expect("current exe");
            let mut cmd = meshloop_adapters::process::OwnedSpawnSpec::new(exe);
            cmd.arg("--spawn-grandchild").arg(pid_file).arg(hb_file);
            let _owned = meshloop_adapters::process::spawn_owned(cmd).expect("spawn owned");
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(5) {
                if Path::new(pid_file).exists() && Path::new(hb_file).exists() {
                    let content = fs::read_to_string(hb_file).unwrap_or_default();
                    if !content.trim().is_empty() {
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            std::process::abort();
        }
        _ => eprintln!("unknown fixture-harness invocation: {args:?}"),
    }
}

fn prompt_from_args(args: &[String]) -> String {
    if let Some(path) = args
        .windows(2)
        .find(|w| w[0] == "--prompt-file")
        .map(|w| w[1].as_str())
    {
        return fs::read_to_string(path).unwrap_or_default();
    }
    args.iter()
        .find_map(|arg| arg.strip_prefix("--prompt-file="))
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default()
}

fn deliver_prompt_file(path: &str) {
    let prompt = fs::read_to_string(path).unwrap_or_default();
    let _ = fs::write("fixture-touched.txt", "fixture wrote this file\n");
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "task".to_string());
    let _ = fs::write(format!("fixture-{stem}.txt"), "fixture wrote this file\n");
    println!("FIXTURE_HANDLED:{prompt}");
}

const SRC_TWO_ERRORS: &str = "pub fn f() {\n    let x: &str = 1;\n    let _ = (x, y);\n}\n";
const SRC_ONE_ERROR: &str = "pub fn f() {\n    let x: &str = 1;\n    let _ = x;\n}\n";
const SRC_CLEAN: &str = "pub fn f() {\n    let x: &str = \"ok\";\n    let _ = x;\n}\n";
const SRC_UNCLOSED: &str = "pub fn f() {\n    let x: &str = 1;\n    let _ = (x, y);\n";

fn apply_repair_scenario(scenario: &str, prompt: &str) {
    let is_repair = prompt.contains("PREVIOUS ATTEMPT FAILED CHECKS")
        || prompt.contains("NEGATIVE CONSTRAINT")
        || prompt.contains("REMAINING DIAGNOSTICS");
    let syntax_ban = prompt
        .to_ascii_lowercase()
        .contains("do not introduce new syntax")
        || prompt.contains("unclosed delimiter");
    let has_e0308 = prompt.contains("E0308");
    let has_e0425 = prompt.contains("E0425");

    let src = match scenario {
        "monotonic" => {
            if !is_repair {
                SRC_TWO_ERRORS
            } else if has_e0425 && has_e0308 {
                SRC_ONE_ERROR
            } else {
                SRC_CLEAN
            }
        }
        "rollback" => {
            if !is_repair {
                SRC_TWO_ERRORS
            } else if syntax_ban {
                SRC_CLEAN
            } else {
                SRC_UNCLOSED
            }
        }
        "oscillate" => SRC_TWO_ERRORS,
        other => {
            eprintln!("unknown repair scenario: {other}");
            std::process::exit(2);
        }
    };

    let _ = fs::create_dir_all("src");
    let _ = fs::write("src/lib.rs", src);
    let _ = fs::write(".gitignore", "check.rmeta\n*.rmeta\n/target/\n");
    let round = if !is_repair {
        0
    } else if syntax_ban {
        2
    } else if has_e0425 && has_e0308 {
        1
    } else {
        2
    };
    println!("REPAIR_SCENARIO:{scenario}:round{round}");
}
