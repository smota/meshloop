//! `--json` envelope for session-control-plane callers (ADR 0017).

use serde::Serialize;
use serde_json::Value;

use meshloop_engine::origin::Origin;

#[derive(Debug, Serialize)]
pub struct Envelope {
    pub ok: bool,
    pub command: String,
    pub origin: Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub data: Value,
}

pub fn ok(command: &str, origin: Origin, data: Value) -> String {
    serde_json::to_string_pretty(&Envelope {
        ok: true,
        command: command.into(),
        origin,
        error: None,
        data,
    })
    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"serialize\"}".into())
}

pub fn err(command: &str, origin: Origin, error: impl ToString) -> String {
    serde_json::to_string_pretty(&Envelope {
        ok: false,
        command: command.into(),
        origin,
        error: Some(error.to_string()),
        data: Value::Null,
    })
    .unwrap_or_else(|_| "{\"ok\":false}".into())
}
