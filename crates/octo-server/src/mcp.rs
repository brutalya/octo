//! Minimal MCP server (JSON-RPC over Streamable HTTP, single JSON responses) that gives
//! Claude one tool, `approve`, used as its `--permission-prompt-tool`. The path carries a
//! per-run secret; see `Hub::mcp_approve`.
use crate::Shared;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

const PROTOCOL: &str = "2025-03-26";

fn ok(id: &Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn err(id: &Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
        .into_response()
}

fn tool_list() -> Value {
    json!({"tools": [{
        "name": "approve",
        "description": "Ask the user whether a tool use may run.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": {"type": "string"},
                "input": {"type": "object"},
                "tool_use_id": {"type": "string"}
            },
            "required": ["tool_name", "input"]
        }
    }]})
}

pub(crate) async fn handle(
    State(hub): State<Shared>,
    Path(token): Path<String>,
    body: Bytes,
) -> Response {
    if !hub.run_token_valid(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(req) = serde_json::from_slice::<Value>(&body) else {
        return err(&Value::Null, -32700, "parse error");
    };
    let id = req.get("id").cloned();
    let method = req["method"].as_str().unwrap_or_default();
    let Some(id) = id else {
        // notification (no id): nothing to answer
        return StatusCode::ACCEPTED.into_response();
    };
    match method {
        "initialize" => {
            let version = req["params"]["protocolVersion"]
                .as_str()
                .unwrap_or(PROTOCOL);
            ok(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "octo", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
        }
        "ping" => ok(&id, json!({})),
        "tools/list" => ok(&id, tool_list()),
        "tools/call" => call(&hub, &token, &id, &req["params"]).await,
        _ => err(&id, -32601, "method not found"),
    }
}

async fn call(hub: &Shared, token: &str, id: &Value, params: &Value) -> Response {
    let args = &params["arguments"];
    let (Some("approve"), Some(tool)) = (params["name"].as_str(), args["tool_name"].as_str())
    else {
        return err(id, -32602, "invalid params");
    };
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let Ok(allow) = hub.mcp_approve(token, tool, &input).await else {
        return err(id, -32602, "run has ended");
    };
    let decision = if allow {
        json!({"behavior": "allow", "updatedInput": input})
    } else {
        json!({"behavior": "deny", "message": "The user denied this action in Octo."})
    };
    ok(
        id,
        json!({"content": [{"type": "text", "text": decision.to_string()}]}),
    )
}
