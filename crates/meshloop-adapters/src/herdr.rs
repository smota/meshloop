//! Herdr 0.8.x CLI-subprocess transport (ADR 0005 / 0017 / 0019).
//! Structured arguments only. Origin space is never mutated. Live agents run in a
//! Meshloop-owned Herdr workspace (`meshloop-<repo>`), unfocused.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use meshloop_domain::capability::{Compatibility, HarnessError, HarnessProfile};
use meshloop_engine::agent::AgentSpec;
use meshloop_engine::ports::LiveCheck;
use meshloop_engine::ports::{
    HarnessCapabilities, HarnessHandle, HarnessOutcome, HerdrSessionPort, ReviewError,
    ReviewTransport, SessionHandle, SessionStatus,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitDecision {
    Continue,
    Settled,
    TimedOut,
}

/// Inactivity (`idle_limit`) applies only when the agent is not `working`/`blocked`.
/// A live thinking pane is never a timeout.
pub fn keep_waiting_for_agent(
    status: Option<&str>,
    saw_working: bool,
    idle_means_done: bool,
    idle_for: Duration,
    idle_limit: Duration,
) -> WaitDecision {
    match status {
        Some("working" | "blocked") => WaitDecision::Continue,
        Some("done") => WaitDecision::Settled,
        Some("idle") if saw_working || idle_means_done => WaitDecision::Settled,
        _ if idle_for >= idle_limit => WaitDecision::TimedOut,
        _ => WaitDecision::Continue,
    }
}

pub struct HerdrCliAdapter {
    herdr_path: PathBuf,
    origin_pane: Option<String>,
    repo_root: Option<PathBuf>,
}

impl HerdrCliAdapter {
    pub fn new(herdr_path: PathBuf) -> Self {
        Self {
            herdr_path,
            origin_pane: None,
            repo_root: None,
        }
    }

    pub fn with_origin(
        herdr_path: PathBuf,
        origin_pane: Option<String>,
        repo_root: Option<PathBuf>,
    ) -> Self {
        Self {
            herdr_path,
            origin_pane,
            repo_root,
        }
    }

    /// `herdr pane split --direction right --cwd <path> --no-focus`
    /// Not used for live placement (ADR 0019). Kept for argv-shape tests.
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

    pub fn workspace_create_args(cwd: &Path, label: &str) -> Vec<String> {
        vec![
            "workspace".into(),
            "create".into(),
            "--cwd".into(),
            cwd.to_string_lossy().into_owned(),
            "--label".into(),
            label.into(),
            "--no-focus".into(),
        ]
    }

    pub fn tab_create_args(workspace: &str, cwd: &Path, label: &str) -> Vec<String> {
        vec![
            "tab".into(),
            "create".into(),
            "--workspace".into(),
            workspace.into(),
            "--cwd".into(),
            cwd.to_string_lossy().into_owned(),
            "--label".into(),
            label.into(),
            "--no-focus".into(),
        ]
    }

    pub fn workspace_list_args() -> Vec<String> {
        vec!["workspace".into(), "list".into()]
    }

    pub fn workspace_close_args(workspace: &str) -> Vec<String> {
        vec!["workspace".into(), "close".into(), workspace.into()]
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
        vec!["pane".into(), "list".into()]
    }

    pub fn status_args() -> Vec<String> {
        vec!["status".into()]
    }

    pub fn current_pane_args() -> Vec<String> {
        vec!["pane".into(), "current".into()]
    }

    pub fn prompt_submit_args(target: &str, text: &str) -> Vec<String> {
        vec!["agent".into(), "prompt".into(), target.into(), text.into()]
    }

    pub fn agent_get_args(target: &str) -> Vec<String> {
        vec!["agent".into(), "get".into(), target.into()]
    }

    pub fn agent_wait_args(target: &str, until: &[&str], timeout_ms: u64) -> Vec<String> {
        let mut args = vec!["agent".into(), "wait".into(), target.into()];
        for state in until {
            args.push("--until".into());
            args.push((*state).into());
        }
        args.push("--timeout".into());
        args.push(timeout_ms.to_string());
        args
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

    fn transport(err: HarnessError) -> ReviewError {
        ReviewError::Transport(format!("{err:?}"))
    }

    fn origin_id<'a>(&'a self, avoid_pane: Option<&'a str>) -> Result<&'a str, ReviewError> {
        avoid_pane
            .or(self.origin_pane.as_deref())
            .filter(|s| !s.trim().is_empty())
            .ok_or(ReviewError::OriginPane)
    }

    fn ensure_loop_workspace(
        &self,
        cwd: &Path,
        origin_ws: &str,
    ) -> Result<(String, Option<String>), ReviewError> {
        let label = loop_workspace_label(self.repo_root.as_deref(), cwd);
        if let Some(cached) = self.read_cached_loop_ws()
            && cached != origin_ws
            && self.workspace_is_live(&cached)?
        {
            return Ok((cached, None));
        }
        if let Some(existing) = self.find_workspace_by_label(&label)?
            && existing != origin_ws
        {
            self.write_cached_loop_ws(&existing, &label, None);
            return Ok((existing, None));
        }
        let out = self
            .run(&Self::workspace_create_args(cwd, &label))
            .map_err(Self::transport)?;
        let (ws, pane) = parse_workspace_create(&out).ok_or_else(|| {
            ReviewError::Transport(format!("workspace create produced no ids: {out}"))
        })?;
        if ws == origin_ws {
            return Err(ReviewError::Transport(
                "loop workspace must not be the origin workspace".into(),
            ));
        }
        self.write_cached_loop_ws(&ws, &label, Some(&pane));
        Ok((ws, Some(pane)))
    }

    fn workspace_is_live(&self, workspace_id: &str) -> Result<bool, ReviewError> {
        Ok(self
            .list_workspaces()?
            .iter()
            .any(|(id, _)| id == workspace_id))
    }

    fn find_workspace_by_label(&self, label: &str) -> Result<Option<String>, ReviewError> {
        Ok(self
            .list_workspaces()?
            .into_iter()
            .find(|(_, l)| l == label)
            .map(|(id, _)| id))
    }

    fn list_workspaces(&self) -> Result<Vec<(String, String)>, ReviewError> {
        let raw = self
            .run(&Self::workspace_list_args())
            .map_err(Self::transport)?;
        Ok(parse_workspace_list(&raw))
    }

    fn read_cached_loop_ws(&self) -> Option<String> {
        let path = loop_space_path(Some(self.repo_root.as_ref()?))?;
        let raw = std::fs::read_to_string(path).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        v.get("workspace_id")
            .and_then(|x| x.as_str())
            .map(str::to_string)
    }

    fn write_cached_loop_ws(&self, workspace_id: &str, label: &str, pane_id: Option<&str>) {
        let Some(root) = &self.repo_root else {
            return;
        };
        let dir = root.join(".meshloop");
        let _ = std::fs::create_dir_all(&dir);
        let mut body = serde_json::json!({
            "workspace_id": workspace_id,
            "label": label,
        });
        if let Some(pane) = pane_id
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert("last_pane_id".into(), Value::String(pane.into()));
        }
        let _ = std::fs::write(dir.join("loop-space.json"), body.to_string());
    }

    fn remember_last_pane(&self, pane_id: &str, workspace_id: &str) {
        let Some(root) = &self.repo_root else {
            return;
        };
        let path = root.join(".meshloop").join("loop-space.json");
        let mut body = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = body.as_object_mut() {
            obj.insert("workspace_id".into(), Value::String(workspace_id.into()));
            obj.insert("last_pane_id".into(), Value::String(pane_id.into()));
            if !obj.contains_key("label") {
                obj.insert(
                    "label".into(),
                    Value::String(loop_workspace_label(self.repo_root.as_deref(), root)),
                );
            }
        }
        let _ = std::fs::create_dir_all(root.join(".meshloop"));
        let _ = std::fs::write(path, body.to_string());
    }

    fn place_in_loop_space(
        &self,
        cwd: &Path,
        origin: &str,
    ) -> Result<(String, String), ReviewError> {
        let origin_ws = workspace_id_from_pane(origin).ok_or_else(|| {
            ReviewError::Transport(format!("cannot parse workspace from origin pane {origin}"))
        })?;
        let (loop_ws, fresh_root) = self.ensure_loop_workspace(cwd, &origin_ws)?;
        let pane = if let Some(root) = fresh_root.filter(|p| {
            workspace_id_from_pane(p).as_deref() == Some(loop_ws.as_str()) && p != origin
        }) {
            root
        } else {
            let out = self
                .run(&Self::tab_create_args(
                    &loop_ws,
                    cwd,
                    &tab_label_for_cwd(cwd),
                ))
                .map_err(Self::transport)?;
            parse_tab_root_pane(&out).ok_or_else(|| {
                ReviewError::Transport(format!("tab create produced no pane id: {out}"))
            })?
        };
        if pane == origin {
            let _ = self.close_pane(&pane);
            return Err(ReviewError::OriginPane);
        }
        if workspace_id_from_pane(&pane).as_deref() != Some(loop_ws.as_str()) {
            let _ = self.close_pane(&pane);
            return Err(ReviewError::Transport(format!(
                "placed pane {pane} is not in loop workspace {loop_ws}"
            )));
        }
        self.remember_last_pane(&pane, &loop_ws);
        Ok((pane, loop_ws))
    }

    fn agent_status(&self, target: &str) -> Option<String> {
        let raw = self.run(&Self::agent_get_args(target)).ok()?;
        parse_agent_status(&raw)
    }

    fn pane_exists(&self, pane_id: &str) -> bool {
        self.list_pane_ids()
            .ok()
            .is_some_and(|ids| ids.iter().any(|id| id == pane_id))
    }

    pub fn pane_for_worktree_path(&self, worktree: &Path) -> Option<String> {
        let raw = self.run(&self.list_panes_args()).ok()?;
        parse_pane_cwds(&raw)
            .into_iter()
            .find(|(_, cwd)| paths_equal(Path::new(cwd), worktree))
            .map(|(id, _)| id)
    }

    pub fn session_live_pane(&self, pane_id: &str) -> LiveCheck {
        match self.agent_status(pane_id).as_deref() {
            Some("working" | "blocked" | "idle" | "done") => LiveCheck::Live,
            _ if self.pane_exists(pane_id) => LiveCheck::Ambiguous,
            _ => LiveCheck::Dead,
        }
    }

    pub fn wait_until_settled(
        &self,
        target: &str,
        idle_limit: Duration,
        idle_means_done: bool,
    ) -> Result<String, ReviewError> {
        let mut saw_working = false;
        let mut last_progress = Instant::now();
        loop {
            let status = self.agent_status(target);
            if matches!(status.as_deref(), Some("working" | "blocked")) {
                saw_working = true;
                last_progress = Instant::now();
            }
            match keep_waiting_for_agent(
                status.as_deref(),
                saw_working,
                idle_means_done,
                last_progress.elapsed(),
                idle_limit,
            ) {
                WaitDecision::Settled => {
                    return self.read_output(target).or_else(|_| Ok(String::new()));
                }
                WaitDecision::TimedOut => {
                    if matches!(status.as_deref(), Some("working" | "blocked")) {
                        last_progress = Instant::now();
                        thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    return Err(ReviewError::Timeout);
                }
                WaitDecision::Continue => thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    pub fn close_workspace(&self, workspace_id: &str) -> Result<(), ReviewError> {
        let _ = self.run(&Self::workspace_close_args(workspace_id));
        Ok(())
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

pub fn workspace_id_from_pane(pane: &str) -> Option<String> {
    let (ws, rest) = pane.trim().split_once(':')?;
    if ws.starts_with('w') && rest.starts_with('p') {
        Some(ws.to_string())
    } else {
        None
    }
}

pub fn loop_workspace_label(repo_root: Option<&Path>, cwd: &Path) -> String {
    let raw = repo_root
        .and_then(|p| p.file_name())
        .or_else(|| cwd.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("meshloop-{safe}")
}

fn tab_label_for_cwd(cwd: &Path) -> String {
    match cwd.file_name().and_then(|s| s.to_str()) {
        Some("plan") => "meshloop-plan".into(),
        Some(name) if name.contains("review") => "meshloop-review".into(),
        _ => "meshloop-work".into(),
    }
}

fn loop_space_path(repo_root: Option<&PathBuf>) -> Option<PathBuf> {
    Some(repo_root?.join(".meshloop").join("loop-space.json"))
}

pub fn parse_pane_cwds(json_or_text: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<Value>(json_or_text) else {
        return Vec::new();
    };
    let Some(panes) = v
        .pointer("/result/panes")
        .or_else(|| v.get("panes"))
        .and_then(|p| p.as_array())
    else {
        return Vec::new();
    };
    panes
        .iter()
        .filter_map(|pane| {
            let id = pane.get("pane_id")?.as_str()?.to_string();
            let cwd = pane.get("cwd")?.as_str()?.to_string();
            Some((id, cwd))
        })
        .collect()
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
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

pub fn parse_workspace_list(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(list) = v
        .pointer("/result/workspaces")
        .or_else(|| v.get("workspaces"))
        .and_then(|w| w.as_array())
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|ws| {
            let id = ws.get("workspace_id")?.as_str()?.to_string();
            let label = ws
                .get("label")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            Some((id, label))
        })
        .collect()
}

pub fn parse_workspace_create(stdout: &str) -> Option<(String, String)> {
    let v = serde_json::from_str::<Value>(stdout).ok()?;
    let ws = v
        .pointer("/result/workspace/workspace_id")
        .and_then(|x| x.as_str())?
        .to_string();
    let pane = v
        .pointer("/result/root_pane/pane_id")
        .and_then(|x| x.as_str())?
        .to_string();
    Some((ws, pane))
}

pub fn parse_tab_root_pane(stdout: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(stdout)
        && let Some(id) = v
            .pointer("/result/root_pane/pane_id")
            .or_else(|| v.pointer("/result/pane/pane_id"))
            .and_then(|x| x.as_str())
    {
        return Some(id.to_string());
    }
    parse_split_pane_id(stdout)
}

pub fn parse_agent_status(stdout: &str) -> Option<String> {
    let v = serde_json::from_str::<Value>(stdout).ok()?;
    v.pointer("/result/agent/agent_status")
        .or_else(|| v.pointer("/result/agent_status"))
        .and_then(|x| x.as_str())
        .map(str::to_string)
}

pub fn parse_split_pane_id(stdout: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(stdout)
        && let Some(id) = v
            .pointer("/result/pane/pane_id")
            .or_else(|| v.pointer("/result/pane_id"))
            .or_else(|| v.pointer("/result/root_pane/pane_id"))
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
        let (pane_id, _) = self
            .place_in_loop_space(
                &spec.worktree_path,
                self.origin_id(None).map_err(map_review)?,
            )
            .map_err(map_review)?;
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
        let origin = self.origin_id(avoid_pane)?;
        let (pane, _) = self.place_in_loop_space(cwd, origin)?;
        Ok(pane)
    }

    fn start_agent(&self, name: &str, kind: &str, pane_id: &str) -> Result<(), ReviewError> {
        self.run(&Self::agent_start_args(name, kind, pane_id))
            .map(|_| ())
            .map_err(Self::transport)
    }

    fn prompt_and_wait(
        &self,
        target: &str,
        prompt: &str,
        timeout_ms: u64,
    ) -> Result<String, ReviewError> {
        match self.run(&Self::prompt_submit_args(target, prompt)) {
            Ok(_) => {}
            Err(e) => {
                let detail = format!("{e:?}");
                if detail.contains("timeout") {
                    return Err(ReviewError::Timeout);
                }
                if !detail.contains("agent_prompt_stalled") {
                    return Err(ReviewError::Transport(detail));
                }
            }
        }
        self.wait_until_settled(target, Duration::from_millis(timeout_ms.max(1)), false)
    }

    fn read_output(&self, target: &str) -> Result<String, ReviewError> {
        self.run(&Self::read_args(target)).map_err(Self::transport)
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

struct LiveSlot {
    timeout: Duration,
}

/// Live worker/planner dispatch via Herdr panes. Isolation is the attempt worktree, not the pane.
pub struct HerdrWorkerHarness {
    adapter: HerdrCliAdapter,
    kind: String,
    harness_name: String,
    origin_pane: Option<String>,
    outputs: Mutex<HashMap<u32, String>>,
    slots: Mutex<HashMap<u32, LiveSlot>>,
}

impl HerdrWorkerHarness {
    pub fn new(
        herdr_path: PathBuf,
        harness_name: impl Into<String>,
        kind: impl Into<String>,
        origin_pane: Option<String>,
        repo_root: PathBuf,
    ) -> Self {
        Self {
            adapter: HerdrCliAdapter::with_origin(herdr_path, origin_pane.clone(), Some(repo_root)),
            kind: kind.into(),
            harness_name: harness_name.into(),
            origin_pane,
            outputs: Mutex::new(HashMap::new()),
            slots: Mutex::new(HashMap::new()),
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
        if let Err(e) = self
            .adapter
            .start_agent(&self.agent_name(spec), &self.kind, &pane)
        {
            let _ = self.adapter.close_pane(&pane);
            return Err(map_review(e));
        }
        match self
            .adapter
            .run(&HerdrCliAdapter::prompt_submit_args(&pane, &spec.prompt))
        {
            Ok(_) => {}
            Err(e) => {
                let detail = format!("{e:?}");
                if !detail.contains("agent_prompt_stalled") {
                    let _ = self.adapter.close_pane(&pane);
                    return Err(map_review(ReviewError::Transport(detail)));
                }
            }
        }
        self.slots
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "herdr worker registry mutex poisoned".into(),
            })?
            .insert(
                spec.attempt_id.0,
                LiveSlot {
                    timeout: spec.timeout,
                },
            );
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
        let idle_limit = self
            .slots
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "herdr worker registry mutex poisoned".into(),
            })?
            .remove(&handle.attempt_id.0)
            .map(|s| s.timeout)
            .unwrap_or(Duration::from_secs(300));
        let Some(pane) = handle.pane_id.as_deref() else {
            return Ok(HarnessOutcome {
                exit_code: 0,
                output_redacted: String::new(),
                worktree_changed: true,
            });
        };
        let idle_means_done = matches!(
            self.adapter.agent_status(pane).as_deref(),
            Some("idle" | "done")
        );
        let output = self
            .adapter
            .wait_until_settled(pane, idle_limit, idle_means_done)
            .map_err(map_review)?;
        self.outputs
            .lock()
            .map_err(|_| HarnessError::ProcessFault {
                detail: "herdr worker registry mutex poisoned".into(),
            })?
            .insert(handle.attempt_id.0, output.clone());
        Ok(HarnessOutcome {
            exit_code: 0,
            output_redacted: output,
            worktree_changed: true,
        })
    }

    fn session_live(&self, handle: &HarnessHandle) -> LiveCheck {
        match &handle.pane_id {
            Some(pane) => self.adapter.session_live_pane(pane),
            None => LiveCheck::Dead,
        }
    }

    fn pane_for_worktree(&self, worktree: &Path) -> Option<String> {
        self.adapter.pane_for_worktree_path(worktree)
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
        assert!(!split.contains(&"--current".into()));
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
    fn parse_pane_cwds_json() {
        let raw = r#"{"result":{"panes":[{"pane_id":"w6:p8","cwd":"C:\\wt\\task-1-attempt-1"}]}}"#;
        assert_eq!(
            parse_pane_cwds(raw),
            vec![("w6:p8".into(), "C:\\wt\\task-1-attempt-1".into())]
        );
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
    fn prompt_submit_does_not_use_wait_gate() {
        let args = HerdrCliAdapter::prompt_submit_args("w5:p2", "read the pack");
        assert!(!args.contains(&"--wait".into()));
        assert_eq!(args[3], "read the pack");
    }

    #[test]
    fn agent_wait_uses_timeout_and_settled_states() {
        let args = HerdrCliAdapter::agent_wait_args("w5:p2", &["idle", "done", "blocked"], 120_000);
        assert!(args.contains(&"--timeout".into()));
        assert!(args.contains(&"idle".into()));
        assert!(args.contains(&"120000".into()));
    }

    #[test]
    fn workspace_create_args_are_unfocused_and_labeled() {
        let args =
            HerdrCliAdapter::workspace_create_args(Path::new("C:\\tmp\\wt"), "meshloop-demo");
        assert_eq!(args[0], "workspace");
        assert_eq!(args[1], "create");
        assert!(args.contains(&"--no-focus".into()));
        assert!(args.contains(&"--label".into()));
        assert!(args.contains(&"meshloop-demo".into()));
        assert!(!args.contains(&"--current".into()));
    }

    #[test]
    fn tab_create_args_pin_loop_workspace() {
        let args =
            HerdrCliAdapter::tab_create_args("w9", Path::new("C:\\tmp\\wt"), "meshloop-plan");
        assert_eq!(args[0], "tab");
        assert!(args.contains(&"--workspace".into()));
        assert!(args.contains(&"w9".into()));
        assert!(args.contains(&"--no-focus".into()));
        assert!(!args.contains(&"--current".into()));
    }

    #[test]
    fn workspace_id_parses_from_pane() {
        assert_eq!(workspace_id_from_pane("w7:p1").as_deref(), Some("w7"));
        assert_eq!(workspace_id_from_pane("w6:p8").as_deref(), Some("w6"));
        assert!(workspace_id_from_pane("not-a-pane").is_none());
    }

    #[test]
    fn loop_label_uses_repo_name() {
        let label = loop_workspace_label(
            Some(Path::new("C:\\Users\\samue\\code\\agentflow-demo")),
            Path::new("C:\\tmp\\plan"),
        );
        assert_eq!(label, "meshloop-agentflow-demo");
    }

    #[test]
    fn parse_workspace_create_json() {
        let raw = r#"{"id":"cli:workspace:create","result":{"workspace":{"workspace_id":"w9"},"tab":{"tab_id":"w9:t1"},"root_pane":{"pane_id":"w9:p1"}}}"#;
        assert_eq!(
            parse_workspace_create(raw),
            Some(("w9".into(), "w9:p1".into()))
        );
    }

    #[test]
    fn parse_workspace_list_labels() {
        let raw = r#"{"result":{"workspaces":[{"workspace_id":"w7","label":"agentflow-demo"},{"workspace_id":"w9","label":"meshloop-agentflow-demo"}]}}"#;
        let list = parse_workspace_list(raw);
        assert_eq!(list[1], ("w9".into(), "meshloop-agentflow-demo".into()));
    }

    #[test]
    fn parse_agent_status_json() {
        let raw = r#"{"result":{"agent":{"agent_status":"working","pane_id":"w9:p1"}}}"#;
        assert_eq!(parse_agent_status(raw).as_deref(), Some("working"));
    }

    #[test]
    fn wait_never_times_out_while_working() {
        assert_eq!(
            keep_waiting_for_agent(
                Some("working"),
                true,
                false,
                Duration::from_secs(999),
                Duration::from_secs(5)
            ),
            WaitDecision::Continue
        );
        assert_eq!(
            keep_waiting_for_agent(
                Some("blocked"),
                true,
                false,
                Duration::from_secs(999),
                Duration::from_secs(5)
            ),
            WaitDecision::Continue
        );
    }

    #[test]
    fn wait_settles_on_idle_after_working() {
        assert_eq!(
            keep_waiting_for_agent(
                Some("idle"),
                true,
                false,
                Duration::from_secs(1),
                Duration::from_secs(5)
            ),
            WaitDecision::Settled
        );
    }

    #[test]
    fn wait_times_out_only_when_never_working() {
        assert_eq!(
            keep_waiting_for_agent(
                Some("idle"),
                false,
                false,
                Duration::from_secs(5),
                Duration::from_secs(5)
            ),
            WaitDecision::TimedOut
        );
        assert_eq!(
            keep_waiting_for_agent(
                Some("idle"),
                false,
                true,
                Duration::from_secs(5),
                Duration::from_secs(5)
            ),
            WaitDecision::Settled
        );
    }
}
