//! Windows PID liveness hints. PID reuse is a residual: Live only if the PID is listed
//! *and* the image name matches when known.

use std::process::Command;

use meshloop_engine::ports::{LiveCheck, ProcessHint, ProcessView};

pub struct WindowsProcessView;

impl ProcessView for WindowsProcessView {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck {
        let output = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("PID eq {}", hint.pid)])
            .output();
        let Ok(out) = output else {
            return LiveCheck::Ambiguous;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| !l.trim().is_empty());
        let Some(line) = line else {
            return LiveCheck::Dead;
        };
        // CSV: "image.exe","pid","session","session#","mem"
        let image = line.split(',').next().unwrap_or("").trim_matches('"');
        match &hint.image_name {
            Some(expected) if !expected.is_empty() => {
                if image.eq_ignore_ascii_case(expected) {
                    LiveCheck::Live
                } else {
                    LiveCheck::Ambiguous
                }
            }
            _ => LiveCheck::Live,
        }
    }
}

/// In-process view used when we still hold the Child (never claims Live for a NULL pid).
pub struct NullPidIsDead;

impl ProcessView for NullPidIsDead {
    fn is_live(&self, hint: &ProcessHint) -> LiveCheck {
        if hint.pid == 0 {
            LiveCheck::Dead
        } else {
            LiveCheck::Live
        }
    }
}
