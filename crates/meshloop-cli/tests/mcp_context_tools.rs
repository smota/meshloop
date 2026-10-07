//! Stdio MCP round trip for the context tools (issue #7). Drives the real binary
//! over JSON-RPC against a disposable source tree; no harness is involved.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meshloop-mcp-ctx-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/ledger.rs"),
        "/// Running balance.\npub struct Ledger { entries: Vec<u64> }\n\
         impl Ledger {\n    pub fn reconcile_balance(&mut self, delta: u64) -> u64 {\n\
         \x20       let mut total = 0;\n        for e in &self.entries { total += e; }\n\
         \x20       self.entries.push(delta);\n        total + delta\n    }\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/render.ts"),
        "export function drawWidget(canvas: Canvas): void {\n  canvas.paint();\n  canvas.flush();\n}\n",
    )
    .unwrap();
    dir
}

/// Send newline-delimited requests, close stdin, and parse one reply per request.
fn mcp_session(cwd: &Path, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_meshloop"))
        .arg("mcp")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn meshloop mcp");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for req in requests {
            writeln!(stdin, "{req}").unwrap();
        }
    }
    let out = child.wait_with_output().expect("mcp output");
    assert!(out.status.success(), "mcp exited {:?}", out.status);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("non-JSON reply {l}: {e}")))
        .collect()
}

fn tool_envelope(reply: &Value) -> Value {
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text content in {reply}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("tool text is not JSON ({e}): {text}"))
}

#[test]
fn mcp_lists_and_calls_context_tools() {
    let dir = scratch("roundtrip");
    let replies = mcp_session(
        &dir,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"meshloop_ast_skeleton","arguments":{"path":"src/ledger.rs"}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":"meshloop_symbol_lookup","arguments":{"query":"reconcile_balance Ledger","k":3}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
                "name":"meshloop_ast_skeleton","arguments":{"path":"../outside.rs"}}}),
        ],
    );
    assert_eq!(replies.len(), 4);

    let names: Vec<&str> = replies[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"meshloop_ast_skeleton"), "{names:?}");
    assert!(names.contains(&"meshloop_symbol_lookup"), "{names:?}");

    let skel = tool_envelope(&replies[1]);
    assert_eq!(skel["ok"], true, "{skel}");
    assert_eq!(skel["command"], "meshloop:ast-skeleton");
    let text = skel["data"]["skeleton"].as_str().unwrap();
    assert!(text.contains("pub fn reconcile_balance"));
    assert!(!text.contains("self.entries.push(delta)"));
    assert!(
        skel["data"]["pruned_approx_tokens"].as_u64().unwrap()
            < skel["data"]["original_approx_tokens"].as_u64().unwrap()
    );

    let lookup = tool_envelope(&replies[2]);
    assert_eq!(lookup["ok"], true, "{lookup}");
    assert_eq!(lookup["data"]["files_indexed"], 2);
    assert_eq!(lookup["data"]["hits"][0]["path"], "src/ledger.rs");
    assert!(lookup["data"]["search_us"].is_u64());

    let escaped = tool_envelope(&replies[3]);
    assert_eq!(escaped["ok"], false, "{escaped}");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn bundle_catalog_carries_context_tool_schemas() {
    let dir = scratch("catalog");
    let dest = dir.join("pack");
    let out = Command::new(env!("CARGO_BIN_EXE_meshloop"))
        .args(["bundle", "--dest"])
        .arg(&dest)
        .output()
        .expect("meshloop bundle");
    assert!(out.status.success());
    let catalog: Value =
        serde_json::from_str(&fs::read_to_string(dest.join("meshloop-mcp-tools.json")).unwrap())
            .expect("catalog is JSON");
    let tools = catalog["tools"].as_array().expect("tools array");
    let skel = tools
        .iter()
        .find(|t| t["name"] == "meshloop_ast_skeleton")
        .expect("ast skeleton tool");
    assert_eq!(skel["inputSchema"]["required"], json!(["path"]));
    assert!(tools.iter().any(|t| t["name"] == "meshloop_symbol_lookup"));
    assert!(tools.iter().all(|t| t["name"] != "meshloop_mcp"));
    fs::remove_dir_all(&dir).ok();
}
