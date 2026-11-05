use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

struct Collector {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<Vec<serde_json::Value>>,
}
impl Collector {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("collector: {e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let n = socket.read(&mut chunk).unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(split) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..split]);
                        assert!(header.contains("Content-Type: application/json"));
                        let length: usize = header
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if request.len() >= split + 4 + length {
                            bodies.push(
                                serde_json::from_slice(&request[split + 4..split + 4 + length])
                                    .unwrap(),
                            );
                            break;
                        }
                    }
                }
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
            bodies
        });
        Self {
            address,
            stop,
            handle,
        }
    }
    fn finish(self) -> Vec<serde_json::Value> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().unwrap()
    }
}
fn config(tag: &str, address: std::net::SocketAddr) -> PathBuf {
    let root = std::env::temp_dir().join(format!("native-telemetry-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(root.join("observability")).unwrap();
    std::fs::write(root.join("observability/consumer.env"),format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{address}\nOTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf\nOTEL_EXPORTER_OTLP_HEADERS=do-not-export\n")).unwrap();
    root
}
fn metrics(bodies: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    bodies
        .iter()
        .flat_map(|b| {
            b["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
                .as_array()
                .unwrap()
        })
        .collect()
}
fn has(bodies: &[serde_json::Value], operation: &str, outcome: &str) -> bool {
    metrics(bodies).iter().any(|m| {
        m["sum"]["dataPoints"][0]["attributes"][0]["value"]["stringValue"] == operation
            && m["sum"]["dataPoints"][0]["attributes"][1]["value"]["stringValue"] == outcome
    })
}
fn command(root: &Path) -> Command {
    let mut c = Command::new(BINARY);
    c.env("XDG_CONFIG_HOME", root)
        .env_remove(OFF_SWITCH)
        .env_remove("OTEL_SDK_DISABLED");
    c.current_dir(root);
    c
}

const BINARY: &str = env!("CARGO_BIN_EXE_meshloop");
const OFF_SWITCH: &str = "MESHLOOP_TELEMETRY_DISABLED";
fn output(c: &mut Command) -> Output {
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    meshloop_adapters::process::spawn_command(c)
        .unwrap()
        .wait_with_output()
        .unwrap()
}
#[test]
fn cli_opt_in_off_switch_and_fail_open() {
    let collector = Collector::start();
    let root = config("mesh-cli", collector.address);
    let normal = output(command(&root).arg("--version"));
    assert!(normal.status.success());
    let failure = output(command(&root).args(["run", "--plan", "private-missing-plan"]));
    assert!(!failure.status.success());
    let bodies = collector.finish();
    assert!(has(&bodies, "cli.operation", "success"));
    assert!(has(&bodies, "cli.operation", "failure"));
    let all = serde_json::to_string(&bodies).unwrap();
    assert!(!all.contains("private-missing-plan"));
    assert!(!all.contains("do-not-export"));
    let collector = Collector::start();
    let disabled = config("mesh-off", collector.address);
    let off = output(command(&disabled).env(OFF_SWITCH, "1").arg("--version"));
    assert!(off.status.success());
    assert_eq!(normal.stdout, off.stdout);
    assert_eq!(normal.stderr, off.stderr);
    assert!(collector.finish().is_empty());
    let before = std::time::Instant::now();
    let refused = output(command(&disabled).arg("--version")); // closed collector
    assert!(refused.status.success());
    assert_eq!(normal.stdout, refused.stdout);
    assert_eq!(normal.stderr, refused.stderr);
    assert!(before.elapsed() < Duration::from_secs(2));
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(disabled).unwrap();
}
#[test]
fn fixture_graph_exports_native_committed_transitions_and_real_run_outcome() {
    let collector = Collector::start();
    let root = config("mesh-graph", collector.address);
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@test"],
        vec!["config", "user.name", "test"],
    ] {
        assert!(
            output(Command::new("git").current_dir(&root).args(args))
                .status
                .success()
        );
    }
    std::fs::write(root.join("README.md"), "fixture").unwrap();
    assert!(
        output(
            Command::new("git")
                .current_dir(&root)
                .args(["add", "README.md"])
        )
        .status
        .success()
    );
    assert!(
        output(
            Command::new("git")
                .current_dir(&root)
                .args(["commit", "-qm", "fixture"])
        )
        .status
        .success()
    );
    let fixture = Path::new(BINARY)
        .parent()
        .unwrap()
        .join(format!("fixture_harness{}", std::env::consts::EXE_SUFFIX));
    let path = fixture.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        root.join("meshloop.toml"),
        format!(
            r#"
selected_harnesses = ["fixture"]
[limits]
max_concurrent_workers = 1
max_retries = 1
task_timeout_seconds = 10
[verify]
verify_command = []
[harnesses.fixture]
executable = "{path}"
version_args = ["--version"]
invoke_args_template = ["--prompt-file", "{{prompt_file}}"]
model_ref = "fixture-model"
model_tier = "top"
"#
        ),
    )
    .unwrap();
    std::fs::write(root.join("plan.json"),r#"{"graph_id":"private-graph-name","nodes":[{"id":1,"description":"private-task-content","depends_on":[],"tier":null}]}"#).unwrap();
    let result = output(command(&root).args([
        "run",
        "--plan",
        "plan.json",
        "--accept-plan",
        "--fixture-only",
        "--config",
        "meshloop.toml",
        "--db",
        "state.sqlite",
        "--worktree-base",
        "worktrees",
    ]));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bodies = collector.finish();
    assert!(has(&bodies, "task.transition", "Running"));
    assert!(has(&bodies, "task.transition", "AwaitingReview"));
    assert!(has(&bodies, "task.dwell", "Ready"));
    assert!(has(&bodies, "graph.run", "idle")); // awaiting human review is not completion
    assert!(!has(&bodies, "graph.run", "complete"));
    let completed_collector = Collector::start();
    std::fs::write(
        root.join("observability/consumer.env"),
        format!(
            "OTEL_EXPORTER_OTLP_ENDPOINT=http://{}\n",
            completed_collector.address
        ),
    )
    .unwrap();
    let accepted = output(command(&root).args([
        "accept",
        "--task",
        "1",
        "--as",
        "fixture-reviewer",
        "--config",
        "meshloop.toml",
        "--db",
        "state.sqlite",
        "--worktree-base",
        "worktrees",
    ]));
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let resumed = output(command(&root).args([
        "resume",
        "--graph",
        "private-graph-name",
        "--fixture-only",
        "--config",
        "meshloop.toml",
        "--db",
        "state.sqlite",
        "--worktree-base",
        "worktrees",
    ]));
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let completed = completed_collector.finish();
    assert!(has(&completed, "task.transition", "Integrated"));
    assert!(has(&completed, "graph.resume", "complete"));
    let text = serde_json::to_string(&bodies).unwrap();
    for private in [
        "private-graph-name",
        "private-task-content",
        "fixture-model",
        "worktrees",
        "do-not-export",
    ] {
        assert!(!text.contains(private));
    }
    std::fs::remove_dir_all(root).unwrap();
}
