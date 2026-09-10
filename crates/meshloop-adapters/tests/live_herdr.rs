//! Live Herdr integration. Skips when the server is down (`xtask check` stays green).
//! `cargo run -p xtask -- live` fails if Herdr is required for launch sign-off.
//! Never splits the origin supervisor pane. Live agents land in a Meshloop space (ADR 0019).

use std::path::PathBuf;

use meshloop_adapters::herdr::{HerdrCliAdapter, workspace_id_from_pane};
use meshloop_engine::ports::{ReviewError, ReviewTransport};

fn herdr() -> Option<HerdrCliAdapter> {
    let adapter = HerdrCliAdapter::new(PathBuf::from("herdr"));
    match adapter.probe_status() {
        Ok(d) if d.server_running => Some(adapter),
        _ => None,
    }
}

fn origin_pane(adapter: &HerdrCliAdapter) -> Option<String> {
    std::env::var("MESHLOOP_ORIGIN_SESSION")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| adapter.current_pane_id().ok().flatten())
}

#[test]
fn live_split_never_uses_origin_pane() {
    let Some(probe) = herdr() else {
        eprintln!("skip: herdr server not running");
        return;
    };
    let Some(origin) = origin_pane(&probe) else {
        eprintln!("skip: no origin pane id");
        return;
    };
    let cwd = std::env::temp_dir().join(format!("meshloop-live-place-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cwd);
    let adapter = HerdrCliAdapter::with_origin(
        PathBuf::from("herdr"),
        Some(origin.clone()),
        Some(cwd.clone()),
    );
    match adapter.split_pane(&cwd, Some(&origin)) {
        Ok(pane) => {
            assert_ne!(pane, origin, "split produced origin pane {pane}");
            let origin_ws = workspace_id_from_pane(&origin);
            let pane_ws = workspace_id_from_pane(&pane);
            assert_ne!(
                pane_ws, origin_ws,
                "loop pane {pane} must not share origin workspace {origin}"
            );
            let _ = adapter.close_pane(&pane);
            if let Some(ws) = pane_ws {
                let _ = adapter.close_workspace(&ws);
            }
        }
        Err(ReviewError::OriginPane) => {
            panic!("origin-only space must create a Meshloop workspace, not OriginPane");
        }
        Err(e) => panic!("unexpected live split error: {e:?}"),
    }
    let _ = std::fs::remove_dir_all(&cwd);
}
