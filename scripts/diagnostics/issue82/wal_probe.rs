use meshloop_adapters::store::SqliteStore;
use meshloop_domain::evidence::AttemptId;
use meshloop_domain::state::{Event, TaskState};
use meshloop_domain::task_graph::TaskId;
use meshloop_engine::ports::TransitionRecord;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = std::env::temp_dir().join(format!(
        "meshloop_issue82_wal_{}_{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir(&fixture)?;
    // Fixtures are owned, unique and retained, including any failure state.
    eprintln!("fixture={}", fixture.display());
    let tmp_db = fixture.join("bench_wal.sqlite");
    let mut store = SqliteStore::open(&tmp_db).map_err(|e| format!("open sqlite wal: {e:?}"))?;
    let wal_trans_count = 50;
    let mut commit_latencies = Vec::with_capacity(wal_trans_count);
    for i in 0..wal_trans_count {
        let record = TransitionRecord {
            graph_id: "bench_g".to_string(),
            task_id: TaskId((i + 1) as u32),
            attempt_id: Some(AttemptId((i + 1) as u32)),
            from: TaskState::Pending,
            to: TaskState::Running,
            event: Event::AttemptStarted,
            reason: Some("bench".to_string()),
            executor: "xtask_bench".to_string(),
            occurred_at: (1000 + i as u64).to_string(),
        };
        let t_commit = Instant::now();
        store
            .record_transition_and_attempt(record, None)
            .expect("record transition");
        commit_latencies.push(t_commit.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sorted = commit_latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95_idx = (sorted.len() as f64 * 0.95) as usize;
    let output = serde_json::json!({
        "schema": "issue82-wal-probe-v1",
        "source_commit": env!("WAL_SOURCE_COMMIT"),
        "probe_source_sha256": env!("WAL_PROBE_SHA256"),
        "fixture": fixture,
        "sample_count": commit_latencies.len(),
        "ordered_samples_ms": commit_latencies,
        "p95_index_zero_based": p95_idx,
        "p95_ms": sorted[p95_idx.min(sorted.len() - 1)],
        "min_ms": sorted[0], "max_ms": sorted[sorted.len()-1],
        "mean_ms": sorted.iter().sum::<f64>() / sorted.len() as f64
    });
    println!("{}", serde_json::to_string(&output)?);
    drop(store);
    Ok(())
}