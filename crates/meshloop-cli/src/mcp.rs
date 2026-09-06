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
        "initialize" => json!({
            "jsonrpc":"2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "meshloop", "version": env!("CARGO_PKG_VERSION") }
            }
        })
        .to_string(),
        "notifications/initialized" => String::new(),
        "tools/list" => json!({
            "jsonrpc":"2.0",
            "id": id,
            "result": { "tools": tool_list() }
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

fn tool_list() -> Vec<Value> {
    bundled_commands()
        .iter()
        .filter(|c| **c != "meshloop:mcp")
        .filter_map(|c| MeshloopId::parse(c).ok())
        .map(|id| {
            json!({
                "name": id.mcp_tool(),
                "description": format!("Meshloop {} (canonical {})", id.cli_verb(), id.as_str()),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "json": { "type": "boolean", "const": true },
                        "args": { "type": "array", "items": { "type": "string" } }
                    }
                }
            })
        })
        .collect()
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
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg(id.cli_verb()).arg("--json");
    for a in extra {
        cmd.arg(a);
    }
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
    }
}
