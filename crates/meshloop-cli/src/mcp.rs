//! Local stdio MCP server. Tool names are `meshloop_<leaf>` (colon is illegal in MCP).
//! Tools invoke this same binary; they contain no saga.

use std::io::{self, BufRead, Write};
use std::process::Command;

use serde_json::{Value, json};

use meshloop_domain::role::{MeshloopId, bundled_commands, bundled_roles};

pub fn run_stdio() -> i32 {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = handle_line(&line);
        if writeln!(stdout, "{reply}").is_err() {
            return 1;
        }
        let _ = stdout.flush();
    }
    0
}

fn handle_line(line: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(line) else {
        return json!({"jsonrpc":"2.0","error":{"code":-32700,"message":"parse error"}})
            .to_string();
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    match method {
        "initialize" => {
            let client_version = req
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            let negotiated_version = match client_version {
                v if v.starts_with("2024-") || v.starts_with("2025-") || v.starts_with("2026-") => {
                    client_version
                }
                _ => "2024-11-05",
            };
            json!({
                "jsonrpc":"2.0",
                "id": id,
                "result": {
                    "protocolVersion": negotiated_version,
                    "capabilities": {
                        "tools": { "listChanged": false },
                        "resources": { "listChanged": false },
                        "prompts": { "listChanged": false }
                    },
                    "serverInfo": { "name": "meshloop", "version": env!("CARGO_PKG_VERSION") }
                }
            })
            .to_string()
        }
        "notifications/initialized" => String::new(),
        "tools/list" => json!({
            "jsonrpc":"2.0",
            "id": id,
            "result": { "tools": tool_list() }
        })
        .to_string(),
        "resources/list" => json!({
            "jsonrpc":"2.0",
            "id": id,
            "result": { "resources": [] }
        })
        .to_string(),
        "prompts/list" => json!({
            "jsonrpc":"2.0",
            "id": id,
            "result": { "prompts": [] }
        })
        .to_string(),
        "tools/call" => {
            let name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = req
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            match dispatch_tool(name, &args) {
                Ok(text) => json!({
                    "jsonrpc":"2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": text }] }
                })
                .to_string(),
                Err(e) => json!({
                    "jsonrpc":"2.0",
                    "id": id,
                    "error": { "code": -32000, "message": e }
                })
                .to_string(),
            }
        }
        "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}).to_string(),
        other => json!({
            "jsonrpc":"2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("unknown method {other}") }
        })
        .to_string(),
    }
}

pub(crate) fn tool_list() -> Vec<Value> {
    bundled_commands()
        .iter()
        .filter(|c| **c != "meshloop:mcp")
        .filter_map(|c| MeshloopId::parse(c).ok())
        .map(|id| match id.as_str() {
            "meshloop:ast-skeleton" => json!({
                "name": id.mcp_tool(),
                "description": "Return the AST skeleton of one source or Markdown file inside the \
                    server's working directory: signatures, types, and docs kept, bodies elided. \
                    Reports original and pruned approximate token counts.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path, relative to the server's working directory"
                        }
                    },
                    "required": ["path"]
                }
            }),
            "meshloop:symbol-lookup" => json!({
                "name": id.mcp_tool(),
                "description": "Rank declared symbols across the server's working directory with the \
                    quantized FWHT signature index. Reports index build and search time separately. \
                    Multi-token queries or signature fragments rank best; a single short identifier \
                    can collide with unrelated entries in the 64-dimension sketch.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Free-text symbol query" },
                        "k": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": crate::context_tools::MAX_LOOKUP_K,
                            "default": crate::context_tools::DEFAULT_LOOKUP_K
                        },
                        "scope": {
                            "type": "string",
                            "description": "Subdirectory to index, relative to the working directory"
                        }
                    },
                    "required": ["query"]
                }
            }),
            _ => json!({
                "name": id.mcp_tool(),
                "description": format!("Meshloop {} (canonical {})", id.cli_verb(), id.as_str()),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "json": { "type": "boolean", "const": true },
                        "args": { "type": "array", "items": { "type": "string" } }
                    }
                }
            }),
        })
        .collect()
}

/// Map a context tool's typed arguments to CLI flags. Other tools pass `args` through.
fn typed_flags(id: &MeshloopId, arguments: &Value) -> Result<Vec<String>, String> {
    let text = |key: &str| arguments.get(key).and_then(|v| v.as_str());
    match id.as_str() {
        "meshloop:ast-skeleton" => {
            let path = text("path").ok_or("meshloop_ast_skeleton requires a string `path`")?;
            Ok(vec!["--path".into(), path.into()])
        }
        "meshloop:symbol-lookup" => {
            let query = text("query").ok_or("meshloop_symbol_lookup requires a string `query`")?;
            let mut flags = vec!["--query".to_string(), query.to_string()];
            if let Some(k) = arguments.get("k") {
                let k = k
                    .as_u64()
                    .ok_or("meshloop_symbol_lookup `k` must be a positive integer")?;
                flags.extend(["--k".to_string(), k.to_string()]);
            }
            if let Some(scope) = text("scope") {
                flags.extend(["--scope".to_string(), scope.to_string()]);
            }
            Ok(flags)
        }
        _ => Ok(Vec::new()),
    }
}

fn dispatch_tool(name: &str, arguments: &Value) -> Result<String, String> {
    let id =
        MeshloopId::parse(name).map_err(|e| format!("unprefixed or invalid MCP tool: {e:?}"))?;
    if id.as_str() == "meshloop:roles" {
        return Ok(serde_json::to_string_pretty(&bundled_roles()).unwrap_or_default());
    }
    let extra: Vec<String> = arguments
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let typed = typed_flags(&id, arguments)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg(id.cli_verb()).arg("--json");
    if let Some(h) = arguments.get("origin_harness").and_then(|v| v.as_str()) {
        cmd.arg("--origin-harness").arg(h);
    }
    if let Some(s) = arguments.get("origin_session").and_then(|v| v.as_str()) {
        cmd.arg("--origin-session").arg(s);
    }
    cmd.args(typed);
    for a in extra {
        cmd.arg(a);
    }
    meshloop_adapters::process::hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_names_are_meshloop_underscore() {
        let tools = tool_list();
        assert!(!tools.is_empty());
        for t in &tools {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap();
            assert!(name.starts_with("meshloop_"), "{name}");
            assert!(!name.contains(':'));
            assert_ne!(name, "reviewer");
            assert_ne!(name, "plan");
        }
    }

    #[test]
    fn initialize_is_jsonrpc() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let reply = handle_line(line);
        assert!(reply.contains("meshloop"));
        assert!(reply.contains("2024-11-05"));

        // Negotiation with modern 2026 version
        let line_2026 = r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2026-01-01"}}"#;
        let reply_2026 = handle_line(line_2026);
        assert!(reply_2026.contains("2026-01-01"));
    }

    #[test]
    fn context_tools_are_listed_with_typed_schemas() {
        let tools = tool_list();
        let find = |name: &str| {
            tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{name} missing from tools/list"))
                .clone()
        };
        let skel = find("meshloop_ast_skeleton");
        assert_eq!(skel["inputSchema"]["required"], json!(["path"]));
        let lookup = find("meshloop_symbol_lookup");
        assert_eq!(lookup["inputSchema"]["required"], json!(["query"]));
        assert_eq!(lookup["inputSchema"]["properties"]["k"]["type"], "integer");
    }

    #[test]
    fn typed_flags_map_and_validate_arguments() {
        let skel = MeshloopId::parse("meshloop_ast_skeleton").unwrap();
        assert_eq!(
            typed_flags(&skel, &json!({"path": "src/lib.rs"})).unwrap(),
            vec!["--path", "src/lib.rs"]
        );
        assert!(typed_flags(&skel, &json!({})).is_err());

        let lookup = MeshloopId::parse("meshloop_symbol_lookup").unwrap();
        assert_eq!(
            typed_flags(
                &lookup,
                &json!({"query": "Ledger", "k": 5, "scope": "crates"})
            )
            .unwrap(),
            vec!["--query", "Ledger", "--k", "5", "--scope", "crates"]
        );
        assert!(typed_flags(&lookup, &json!({"query": "x", "k": -1})).is_err());
        assert!(typed_flags(&lookup, &json!({"k": 3})).is_err());
    }

    #[test]
    fn resources_and_prompts_list_return_empty() {
        let res_line = r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#;
        let reply = handle_line(res_line);
        assert!(reply.contains(r#""resources":[]"#));

        let prompt_line = r#"{"jsonrpc":"2.0","id":4,"method":"prompts/list"}"#;
        let reply = handle_line(prompt_line);
        assert!(reply.contains(r#""prompts":[]"#));
    }
}
