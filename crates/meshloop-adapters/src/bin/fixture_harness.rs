//! A controlled stand-in harness binary for adapter contract tests (runtime-design.md §2:
//! "a fixture/stub harness binary in test fixtures"). Never used against a real subscription.
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
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
        Some("--prompt-file") => {
            let path = args.get(1).expect("fixture requires a prompt file path");
            let prompt = fs::read_to_string(path).unwrap_or_default();
            let _ = fs::write("fixture-touched.txt", "fixture wrote this file\n");
            println!("FIXTURE_HANDLED:{prompt}");
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
            let mut child = std::process::Command::new(exe)
                .arg("--grandchild-heartbeat")
                .arg(hb_file)
                .spawn()
                .expect("spawn grandchild");
            let _ = fs::write(pid_file, child.id().to_string());
            let _ = child.wait();
        }
        Some("--spawn-in-job-and-abort") => {
            let pid_file = args.get(1).expect("pid file path required");
            let hb_file = args.get(2).expect("heartbeat file path required");
            let exe = env::current_exe().expect("current exe");
            let mut cmd = std::process::Command::new(exe);
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
