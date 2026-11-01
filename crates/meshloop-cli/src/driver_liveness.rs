//! Log path and liveness label for a detached `run` driver.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use meshloop_domain::state::TaskState;
use meshloop_engine::ports::{DriverLiveness, LiveCheck, ProcessHint, ProcessView, RunStore};
use meshloop_engine::run_loop::{DRIVER_HEARTBEAT, RunStatus};

pub const LOG_ENV: &str = "MESHLOOP_DRIVER_LOG";
pub const PANIC_ENV: &str = "MESHLOOP_DRIVER_PANIC";

pub struct DriverReport {
    pub label: String,
    pub log_path: String,
    pub suggest_resume: bool,
}

impl DriverReport {
    pub fn text(&self) -> String {
        let mut out = format!("driver: {} log={}\n", self.label, self.log_path);
        if self.suggest_resume {
            out.push_str("driver is gone and a node is non-terminal; meshloop resume\n");
        }
        out
    }
}

pub fn driver_log_path(repo_root: &Path, db: Option<&Path>, graph_id: &str) -> PathBuf {
    mesh_dir(repo_root, db)
        .join("runs")
        .join(graph_component(graph_id))
        .join("driver.log")
}

pub fn prepare_log(path: &Path) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    std::fs::canonicalize(path).or_else(|_| Ok(path.to_path_buf()))
}

pub fn register(
    store: &mut dyn RunStore,
    graph_id: &str,
    log_path: &Path,
) -> Result<(), meshloop_engine::ports::StoreError> {
    let now = unix_secs();
    store.upsert_driver(&DriverLiveness {
        graph_id: graph_id.to_string(),
        pid: std::process::id(),
        started_at: now.clone(),
        heartbeat_at: now,
        exit_code: None,
        log_path: log_path.display().to_string(),
    })
}

pub fn finish(store: &mut dyn RunStore, graph_id: &str, code: i32) {
    let _ = store.record_driver_exit(graph_id, std::process::id(), code, &unix_secs());
}

pub fn panic_if_requested(fixture_only: bool) {
    if fixture_only && std::env::var(PANIC_ENV).ok().as_deref() == Some("1") {
        eprintln!("meshloop driver panic");
        let _ = std::io::Write::flush(&mut std::io::stderr());
        panic!("meshloop driver panic");
    }
}

pub fn observe(
    store: &dyn RunStore,
    processes: &dyn ProcessView,
    status: &RunStatus,
) -> Option<DriverReport> {
    let row = store.load_driver(&status.graph_id).ok()??;
    let image = std::env::current_exe()
        .ok()
        .and_then(|path| meshloop_adapters::process::process_image_name(&path));
    let live = processes.is_live(&ProcessHint {
        pid: row.pid,
        image_name: image,
    });
    let label = driver_label(&row, live, unix_now(), DRIVER_HEARTBEAT);
    let suggest_resume = label != "alive" && needs_driver(status);
    Some(DriverReport {
        label,
        log_path: row.log_path,
        suggest_resume,
    })
}

pub fn driver_label(
    row: &DriverLiveness,
    live: LiveCheck,
    now_unix: u64,
    interval: Duration,
) -> String {
    if let Some(code) = row.exit_code {
        return format!("exited({code})");
    }
    let beat = row.heartbeat_at.parse::<u64>().unwrap_or(0);
    let stale = Duration::from_secs(now_unix.saturating_sub(beat)) > interval;
    match live {
        LiveCheck::Dead => "lost".into(),
        LiveCheck::Live => "alive".into(),
        LiveCheck::Ambiguous if stale => "lost".into(),
        LiveCheck::Ambiguous => "alive".into(),
    }
}

fn needs_driver(status: &RunStatus) -> bool {
    status.nodes.iter().any(|node| {
        !matches!(
            node.state,
            TaskState::Integrated | TaskState::Failed | TaskState::Cancelled | TaskState::Blocked
        )
    })
}

fn mesh_dir(repo_root: &Path, db: Option<&Path>) -> PathBuf {
    if let Some(db) = db
        && let Some(parent) = db.parent()
        && parent.file_name().is_some_and(|name| name == ".meshloop")
    {
        return parent.to_path_buf();
    }
    repo_root.join(".meshloop")
}

fn graph_component(graph_id: &str) -> String {
    let mut parts = Path::new(graph_id).components();
    if let (Some(Component::Normal(name)), None) = (parts.next(), parts.next())
        && name == graph_id
    {
        return graph_id.to_string();
    }
    let mut hash = 0xcbf29ce484222325u64;
    for byte in graph_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("g{hash:016x}")
}

fn unix_secs() -> String {
    unix_now().to_string()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(exit_code: Option<i32>, heartbeat_at: &str) -> DriverLiveness {
        DriverLiveness {
            graph_id: "g".into(),
            pid: 7,
            started_at: "1".into(),
            heartbeat_at: heartbeat_at.into(),
            exit_code,
            log_path: "driver.log".into(),
        }
    }

    #[test]
    fn dead_pid_is_lost_without_waiting_out_a_fresh_heartbeat() {
        let label = driver_label(
            &row(None, "100"),
            LiveCheck::Dead,
            100,
            Duration::from_secs(5),
        );
        assert_eq!(label, "lost");
    }

    #[test]
    fn recorded_exit_is_exited_even_if_the_pid_is_dead() {
        let label = driver_label(
            &row(Some(0), "1"),
            LiveCheck::Dead,
            100,
            Duration::from_secs(5),
        );
        assert_eq!(label, "exited(0)");
    }

    #[test]
    fn live_pid_stays_alive_and_stale_ambiguous_is_lost() {
        assert_eq!(
            driver_label(
                &row(None, "1"),
                LiveCheck::Live,
                100,
                Duration::from_secs(5)
            ),
            "alive"
        );
        assert_eq!(
            driver_label(
                &row(None, "1"),
                LiveCheck::Ambiguous,
                100,
                Duration::from_millis(1)
            ),
            "lost"
        );
        assert_eq!(
            driver_label(
                &row(None, "100"),
                LiveCheck::Ambiguous,
                100,
                Duration::from_secs(5)
            ),
            "alive"
        );
    }

    #[test]
    fn gone_driver_text_names_resume_for_a_nonterminal_node() {
        let report = DriverReport {
            label: "lost".into(),
            log_path: "/repo/.meshloop/runs/g/driver.log".into(),
            suggest_resume: true,
        };
        let text = report.text();
        assert!(text.contains("driver: lost log=/repo/.meshloop/runs/g/driver.log"));
        assert!(text.contains("meshloop resume"));
    }

    #[test]
    fn log_path_follows_the_store_meshloop_directory() {
        let repo = std::env::temp_dir().join("repo");
        let db = std::env::temp_dir()
            .join("other")
            .join(".meshloop")
            .join("state.sqlite");
        let inside = driver_log_path(&repo, Some(&db), "g1");
        assert_eq!(
            inside,
            db.parent()
                .unwrap()
                .join("runs")
                .join("g1")
                .join("driver.log")
        );
        let beside = driver_log_path(&repo, Some(&repo.join("state.sqlite")), "g1");
        assert_eq!(
            beside,
            repo.join(".meshloop")
                .join("runs")
                .join("g1")
                .join("driver.log")
        );
        let escaped = driver_log_path(&repo, None, "..");
        assert!(escaped.starts_with(repo.join(".meshloop").join("runs")));
        assert_eq!(
            escaped.file_name().and_then(|name| name.to_str()),
            Some("driver.log")
        );
        assert_ne!(
            escaped
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str()),
            Some("..")
        );
    }
}
