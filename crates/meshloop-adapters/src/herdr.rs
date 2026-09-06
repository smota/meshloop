//! Herdr 0.8.x CLI-subprocess transport (ADR 0005 / 0017). Structured arguments only.
//! Never split the origin supervisor pane.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::{
    HarnessCapabilities, HarnessHandle, HarnessOutcome, HerdrSessionPort, ReviewError,
    ReviewTransport, SessionHandle, SessionStatus,
};
use serde_json::Value;

pub struct HerdrCliAdapter {
    herdr_path: PathBuf,
    /// Workspace to split from; None means `--current` at call time.
    workspace_hint: Option<String>,
}

impl HerdrCliAdapter {
    pub fn new(herdr_path: PathBuf) -> Self {
        Self {
            herdr_path,
            workspace_hint: None,
        }
    }

    pub fn with_workspace(herdr_path: PathBuf, workspace: impl Into<String>) -> Self {
        Self {
            herdr_path,
            workspace_hint: Some(workspace.into()),
        }
    }

    /// `herdr pane split --direction right --cwd <path> --no-focus`
    pub fn split_args(&self, cwd: &Path) -> Vec<String> {
        vec![
            "pane".into(),
            "split".into(),
            "--direction".into(),
            "right".into(),
            "--cwd".into(),
            cwd.to_string_lossy().into_owned(),
            "--no-focus".into(),
        ]
    }

    /// `herdr pane run <PANE_ID> <COMMAND>...`
    pub fn run_args(&self, pane_id: &str, command: &[String]) -> Vec<String> {
        let mut args = vec!["pane".into(), "run".into(), pane_id.to_string()];
        args.extend(command.iter().cloned());
        args
    }

    pub fn close_pane_args(&self, pane_id: &str) -> Vec<String> {
        vec!["pane".into(), "close".into(), pane_id.to_string()]
    }

    pub fn list_panes_args(&self) -> Vec<String> {
        let mut args = vec!["pane".into(), "list".into()];
        if let Some(ws) = &self.workspace_hint {
            args.push("--workspace".into());
            args.push(ws.clone());
        }
        args
    }

    pub fn status_args() -> Vec<String> {
        vec!["status".into()]
    }

    pub fn current_pane_args() -> Vec<String> {
        vec!["pane".into(), "current".into()]
    }

    pub fn prompt_args(target: &str, text: &str, timeout_ms: u64) -> Vec<String> {
        vec![
            "agent".into(),
            "prompt".into(),
            target.into(),
            text.into(),
            "--wait".into(),
            "--until".into(),
            "idle".into(),
            "--until".into(),
            "done".into(),
            "--until".into(),
            "blocked".into(),
            "--timeout".into(),
            timeout_ms.to_string(),
        ]
    }

    pub fn read_args(target: &str) -> Vec<String> {
        vec![
            "agent".into(),
            "read".into(),
            target.into(),
            "--lines".into(),
            "200".into(),
            "--format".into(),
            "text".into(),
        ]
    }

    pub fn agent_start_args(name: &str, kind: &str, pane_id: &str) -> Vec<String> {
        vec![
            "agent".into(),
            "start".into(),
            name.into(),
            "--kind".into(),
            kind.into(),
            "--pane".into(),
            pane_id.into(),
        ]
    }

    fn run(&self, args: &[String]) -> Result<String, HarnessError> {
        let output = Command::new(&self.herdr_path)
            .args(args)
            .output()
            .map_err(|e| HarnessError::ProcessFault {
                detail: e.to_string(),
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(HarnessError::ProcessFault {
                detail: format!("{}{}", stdout, String::from_utf8_lossy(&output.stderr)),
            })
        }
    }

    pub fn probe_status(&self) -> Result<HerdrDoctor, HarnessError> {
        let raw = self.run(&Self::status_args())?;
        Ok(HerdrDoctor::parse(&raw))
    }

    pub fn list_pane_ids(&self) -> Result<Vec<String>, HarnessError> {
        let raw = self.run(&self.list_panes_args())?;
        Ok(parse_pane_ids(&raw))
    }

    pub fn current_pane_id(&self) -> Result<Option<String>, HarnessError> {
        let raw = self.run(&Self::current_pane_args())?;
        Ok(parse_split_pane_id(&raw).or_else(|| parse_pane_ids(&raw).into_iter().next()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrDoctor {
    pub server_running: bool,
    pub version: Option<String>,
    pub raw_ok: bool,
}

impl HerdrDoctor {
    pub fn parse(status_text: &str) -> Self {
        let running =
            status_text.contains("status: running") || status_text.contains("status:running");
        let version = status_text.lines().find_map(|l| {
            let t = l.trim();
            t.strip_prefix("version:").map(|v| v.trim().to_string())
        });
        Self {
            server_running: running,
            version,
            raw_ok: !status_text.trim().is_empty(),
        }
    }
}

pub fn parse_pane_ids(json_or_text: &str) -> Vec<String> {
    if let Ok(v) = serde_json::from_str::<Value>(json_or_text) {
        let mut ids = Vec::new();
        if let Some(panes) = v
            .pointer("/result/panes")
            .or_else(|| v.get("panes"))
            .and_then(|p| p.as_array())
        {
            for pane in panes {
                if let Some(id) = pane.get("pane_id").and_then(|x| x.as_str()) {
                    ids.push(id.to_string());
                }
            }
        }
        return ids;
    }
    Vec::new()
}

pub fn parse_split_pane_id(stdout: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(stdout)
        && let Some(id) = v
            .pointer("/result/pane/pane_id")
            .or_else(|| v.pointer("/result/pane_id"))
            .and_then(|x| x.as_str())
    {
        return Some(id.to_string());
    }
    stdout
        .lines()
        .map(str::trim)
        .find(|l| l.contains(":p") || l.starts_with("w"))
        .map(ToString::to_string)
}

impl HerdrSessionPort for HerdrCliAdapter {
    fn spawn(&self, spec: &AgentSpec) -> Result<SessionHandle, HarnessError> {
        let split_out = self.run(&self.split_args(&spec.worktree_path))?;
        let pane_id =
            parse_split_pane_id(&split_out).ok_or_else(|| HarnessError::ProcessFault {
                detail: format!("herdr pane split produced no pane id: {split_out}"),
            })?;
        let cmd = vec![spec.harness.clone()];
        let _ = self.run(&self.run_args(&pane_id, &cmd))?;
        Ok(SessionHandle { id: pane_id })
    }

    fn status(&self, handle: &SessionHandle) -> Result<SessionStatus, HarnessError> {
        let ids = self.list_pane_ids()?;
        if ids.iter().any(|id| id == &handle.id) {
            Ok(SessionStatus::Running)
        } else {
            Ok(SessionStatus::Exited(0))
        }
    }

    fn cancel(&self, handle: &SessionHandle) -> Result<(), HarnessError> {
        let _ = self.run(&self.close_pane_args(&handle.id));
        Ok(())
    }

    fn cleanup(&self, handle: &SessionHandle) -> Result<(), HarnessError> {
        self.cancel(handle)
    }
}

impl ReviewTransport for HerdrCliAdapter {
    fn split_pane(&self, cwd: &Path, avoid_pane: Option<&str>) -> Result<String, ReviewError> {
        let ids = self
            .list_pane_ids()
            .map_err(|e| ReviewError::Transport(format!("{e:?}")))?;
        let from = ids.iter().find(|id| Some(id.as_str()) != avoid_pane);
        let Some(from) = from else {
            return Err(ReviewError::OriginPane);
        };
        let mut args = self.split_args(cwd);
        args.push("--pane".into());
        args.push(from.clone());
        let out = self
            .run(&args)
            .map_err(|e| ReviewError::Transport(format!("{e:?}")))?;
        let pane_id = parse_split_pane_id(&out)
            .ok_or_else(|| ReviewError::Transport(format!("split produced no pane id: {out}")))?;
        if Some(pane_id.as_str()) == avoid_pane {
            let _ = self.close_pane(&pane_id);
            return Err(ReviewError::OriginPane);
        }
        Ok(pane_id)
    }

    fn start_agent(&self, name: &str, kind: &str, pane_id: &str) -> Result<(), ReviewError> {
        self.run(&Self::agent_start_args(name, kind, pane_id))
            .map(|_| ())
            .map_err(|e| ReviewError::Transport(format!("{e:?}")))
    }

    fn prompt_and_wait(
        &self,
        target: &str,
        prompt: &str,
        timeout_ms: u64,
    ) -> Result<String, ReviewError> {
        match self.run(&Self::prompt_args(target, prompt, timeout_ms)) {
            Ok(out) => {
                if out.contains("timeout") {
                    return Err(ReviewError::Timeout);
                }
                let read = self.read_output(target).unwrap_or(out);
                Ok(read)
            }
            Err(e) => {
                let detail = format!("{e:?}");
                if detail.contains("timeout") {
                    Err(ReviewError::Timeout)
                } else {
                    Err(ReviewError::Transport(detail))
                }
            }
        }
    }

    fn read_output(&self, target: &str) -> Result<String, ReviewError> {
        self.run(&Self::read_args(target))
            .map_err(|e| ReviewError::Transport(format!("{e:?}")))
    }

    fn close_pane(&self, pane_id: &str) -> Result<(), ReviewError> {
        let _ = self.run(&self.close_pane_args(pane_id));
        Ok(())
    }
}

fn map_review(err: ReviewError) -> HarnessError {
    match err {
        ReviewError::Timeout => HarnessError::Timeout,
        ReviewError::OriginPane => HarnessError::ProcessFault {
            detail: "refusing to split meshloop:origin supervisor pane".into(),
        },
        ReviewError::Transport(detail) => HarnessError::ProcessFault { detail },
    }
}

/// Live worker/planner dispatch via Herdr panes. Isolation is the attempt worktree, not the pane.
pub struct HerdrWorkerHarness {
    adapter: HerdrCliAdapter,
    kind: String,
    harness_name: String,
    origin_pane: Option<String>,
    outputs: Mutex<HashMap<u32, String>>,
}

impl HerdrWorkerHarness {
    pub fn new(
        herdr_path: PathBuf,
        harness_name: impl Into<String>,
        kind: impl Into<String>,
        origin_pane: Option<String>,
    ) -> Self {
        Self {
            adapter: HerdrCliAdapter::new(herdr_path),
            kind: kind.into(),
            harness_name: harness_name.into(),
            origin_pane,
            outputs: Mutex::new(HashMap::new()),
        }
    }

    fn agent_name(&self, spec: &AgentSpec) -> String {
        let leaf = if spec.worktree_path.file_name().and_then(|s| s.to_str()) == Some("plan") {
            "planner"
        } else {
            "worker"
        };
        format!(
            "meshloop-{leaf}-{}-a{}",
            self.harness_name, spec.attempt_id.0
        )
    }
}

impl HarnessCapabilities for HerdrWorkerHarness {
    fn probe(&self) -> Result<HarnessProfile, HarnessError> {
        let status = self.adapter.probe_status()?;
        if !status.server_running {
            return Err(HarnessError::ProcessFault {
                detail: "herdr server is not running".into(),
            });
        }
        Ok(HarnessProfile {
            harness: self.harness_name.clone(),
            version: status.version.unwrap_or_else(|| "herdr".into()),
            compatibility: Compatibility::Compatible,
            supports_noninteractive: true,
            supports_structured_output: false,
            supports_cancellation: true,
        })
    }

    fn invoke(&self, spec: &AgentSpec) -> Result<HarnessHandle, HarnessError> {
        let avoid = self.origin_pane.as_deref();
        let pane = self
            .adapter
            .split_pane(&spec.worktree_path, avoid)
            .map_err(map_review)?;
        if Some(pane.as_str()) == avoid {
            let _ = self.adapter.close_pane(&pane);
            return Err(HarnessError::ProcessFault {
                detail: "refusing to split meshloop:origin supervisor pane".into(),
            });
        }
        self.adapter
            .start_agent(&self.agent_name(spec), &self.kind, &pane)
            .map_err(map_review)?;
        let timeout_ms = spec.timeout.as_millis().max(1) as u64;
        let output = self
            .adapter
            .prompt_and_wait(&pane, &spec.prompt, timeout_ms)
            .map_err(map_review)?;
        self.outputs
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "herdr worker registry mutex poisoned".into(),
            })?
            .insert(spec.attempt_id.0, output);
        Ok(HarnessHandle {
            attempt_id: spec.attempt_id,
            pid: None,
            pane_id: Some(pane),
        })
    }

    fn cancel(&self, handle: &HarnessHandle) -> Result<(), HarnessError> {
        if let Some(pane) = &handle.pane_id {
            let _ = self.adapter.close_pane(pane);
        }
        Ok(())
    }

    fn collect(&self, handle: &HarnessHandle) -> Result<HarnessOutcome, HarnessError> {
        let stored = self
            .outputs
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "herdr worker registry mutex poisoned".into(),
            })?
            .remove(&handle.attempt_id.0);
        let output = match stored {
            Some(text) => text,
            None => handle
                .pane_id
                .as_deref()
                .and_then(|pane| self.adapter.read_output(pane).ok())
                .unwrap_or_default(),
        };
        Ok(HarnessOutcome {
            exit_code: 0,
            output_redacted: output,
            worktree_changed: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_run_args_match_herdr_0_8() {
        let adapter = HerdrCliAdapter::new(PathBuf::from("herdr"));
        let split = adapter.split_args(Path::new("C:\\tmp\\wt"));
        assert_eq!(split[0], "pane");
        assert_eq!(split[1], "split");
        assert!(split.contains(&"--no-focus".into()));
        assert!(!split.contains(&"--session".into()));
        let run = adapter.run_args("w5:p2", &["claude".into(), "--print".into()]);
        assert_eq!(run, vec!["pane", "run", "w5:p2", "claude", "--print"]);
    }

    #[test]
    fn close_pane_args_are_positional_id() {
        let adapter = HerdrCliAdapter::new(PathBuf::from("herdr"));
        assert_eq!(
            adapter.close_pane_args("w5:p2"),
            vec!["pane", "close", "w5:p2"]
        );
    }

    #[test]
    fn run_args_keep_metacharacters_as_one_argv_element() {
        let adapter = HerdrCliAdapter::new(PathBuf::from("herdr"));
        let args = adapter.run_args("w1:p1", &["claude".into(), "--dangerous; rm -rf /".into()]);
        assert_eq!(args.last().unwrap(), "--dangerous; rm -rf /");
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn parse_pane_list_json() {
        let raw = r#"{"id":"cli:pane:list","result":{"panes":[{"pane_id":"w5:p1"},{"pane_id":"w3:p1"}],"type":"pane_list"}}"#;
        assert_eq!(parse_pane_ids(raw), vec!["w5:p1", "w3:p1"]);
    }

    #[test]
    fn doctor_detects_running_server() {
        let text = "server:\n  status: running\n  version: 0.8.2\n";
        let d = HerdrDoctor::parse(text);
        assert!(d.server_running);
        assert_eq!(d.version.as_deref(), Some("0.8.2"));
    }

    #[test]
    fn agent_start_uses_kind_and_pane() {
        let args = HerdrCliAdapter::agent_start_args("meshloop-worker-1", "grok", "w5:p2");
        assert!(args.contains(&"--kind".into()));
        assert!(args.contains(&"grok".into()));
        assert!(args.contains(&"--pane".into()));
        assert!(!args.iter().any(|a| a == "reviewer" || a == "planner"));
    }

    #[test]
    fn prompt_wait_uses_timeout_and_settled_states() {
        let args = HerdrCliAdapter::prompt_args("w5:p2", "read the pack", 120_000);
        assert!(args.contains(&"--wait".into()));
        assert!(args.contains(&"--timeout".into()));
        assert!(args.contains(&"idle".into()));
        assert_eq!(args[3], "read the pack");
    }
}
