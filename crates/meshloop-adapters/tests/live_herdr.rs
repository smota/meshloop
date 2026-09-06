//! Live Herdr integration. Skips when the server is down (`xtask check` stays green).
//! `cargo run -p xtask -- live` fails if Herdr is required for launch sign-off.
//! Never splits the origin supervisor pane.

use std::path::PathBuf;

use meshloop_adapters::herdr::HerdrCliAdapter;
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
    let Some(adapter) = herdr() else {
        eprintln!("skip: herdr server not running");
        return;
    };
    let Some(origin) = origin_pane(&adapter) else {
        eprintln!("skip: no origin pane id");
        return;
    };
    let cwd = std::env::temp_dir();
    match adapter.split_pane(&cwd, Some(&origin)) {
        Ok(pane) => {
            assert_ne!(pane, origin, "split produced origin pane {pane}");
            let _ = adapter.close_pane(&pane);
        }
        Err(ReviewError::OriginPane) => {
            // Only the origin pane exists; refusing is the product rule.
        }
        Err(e) => panic!("unexpected live split error: {e:?}"),
    }
}
