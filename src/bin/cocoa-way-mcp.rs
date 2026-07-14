use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

#[path = "../control_client.rs"]
mod control_client;
#[allow(dead_code)]
#[path = "../control_protocol.rs"]
mod control_protocol;

use control_protocol::ControlRequest;

const LATEST_PROTOCOL: &str = "2025-11-25";
const SUPPORTED_PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

fn main() {
    let socket = parse_socket_argument().unwrap_or_else(|error| fail(&error));
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut initialized = false;

    for line in stdin.lock().lines() {
        let response = match line {
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => match serde_json::from_str::<Value>(&line) {
                Ok(message) => handle_message(&message, &socket, &mut initialized),
                Err(error) => Some(json_rpc_error(
                    Value::Null,
                    -32700,
                    format!("Parse error: {error}"),
                )),
            },
            Err(error) => {
                eprintln!("cocoa-way-mcp: failed to read stdin: {error}");
                break;
            }
        };
        if let Some(response) = response {
            match serde_json::to_string(&response) {
                Ok(encoded) => {
                    if writeln!(stdout, "{encoded}").is_err() || stdout.flush().is_err() {
                        break;
                    }
                }
                Err(error) => {
                    eprintln!("cocoa-way-mcp: failed to encode response: {error}");
                    break;
                }
            }
        }
    }
}

fn parse_socket_argument() -> Result<PathBuf, String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() {
        return Ok(control_client::default_socket_path());
    }
    if arguments.len() == 2 && arguments[0] == "--socket" {
        return Ok(PathBuf::from(&arguments[1]));
    }
    Err("usage: cocoa-way-mcp [--socket PATH]".into())
}

fn handle_message(message: &Value, socket: &Path, initialized: &mut bool) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(json_rpc_error(Value::Null, -32600, "Invalid Request"));
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(json_rpc_error(
            object.get("id").cloned().unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    }
    let method = object.get("method").and_then(Value::as_str)?;
    let id = object.get("id").cloned();
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));

    if id.is_none() {
        if method == "notifications/initialized" {
            *initialized = true;
        }
        return None;
    }
    let id = id.unwrap_or(Value::Null);
    if id.is_null() {
        return Some(json_rpc_error(id, -32600, "Request id must not be null"));
    }

    match method {
        "initialize" => Some(json_rpc_success(id, initialize_result(&params))),
        "ping" => Some(json_rpc_success(id, json!({}))),
        "tools/list" if *initialized => Some(json_rpc_success(id, json!({ "tools": tools() }))),
        "tools/call" if *initialized => match call_tool(&params, socket) {
            Ok(result) => Some(json_rpc_success(id, result)),
            Err(error) => Some(json_rpc_error(id, -32602, error)),
        },
        "tools/list" | "tools/call" => {
            Some(json_rpc_error(id, -32002, "Server is not initialized"))
        }
        _ => Some(json_rpc_error(id, -32601, "Method not found")),
    }
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(LATEST_PROTOCOL);
    let protocol = if SUPPORTED_PROTOCOLS.contains(&requested) {
        requested
    } else {
        LATEST_PROTOCOL
    };
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "cocoa-way-mcp",
            "title": "Cocoa-Way Local Control",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Read-only local diagnostics for Cocoa-Way sessions and container resources."
        },
        "instructions": "This server is local and read-only. It does not expose launch, stop, or delete operations."
    })
}

fn tools() -> Vec<Value> {
    vec![
        read_only_tool(
            "cocoa_way_status",
            "Cocoa-Way status",
            "Read compositor, transport, performance, and recent activity status.",
            empty_schema(),
        ),
        read_only_tool(
            "cocoa_way_sessions",
            "Cocoa-Way sessions",
            "List configured GUI sessions and their active process/display state.",
            empty_schema(),
        ),
        read_only_tool(
            "cocoa_way_displays",
            "Cocoa-Way displays",
            "List the default and active dedicated display assignments.",
            empty_schema(),
        ),
        read_only_tool(
            "cocoa_way_images",
            "Cocoa-Way images",
            "List local Apple Container and Docker-compatible images.",
            empty_schema(),
        ),
        read_only_tool(
            "cocoa_way_logs",
            "Cocoa-Way session logs",
            "Read recent captured logs for one configured GUI session.",
            json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Exact session name or zero-based session index."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1000,
                        "default": 100
                    }
                },
                "required": ["session"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn read_only_tool(name: &str, title: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn empty_schema() -> Value {
    json!({ "type": "object", "additionalProperties": false })
}

fn call_tool(params: &Value, socket: &Path) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tools/call requires a tool name".to_string())?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let (command, session, limit) = match name {
        "cocoa_way_status" => ("status", None, 10),
        "cocoa_way_sessions" => ("sessions", None, 100),
        "cocoa_way_displays" => ("displays", None, 100),
        "cocoa_way_images" => ("images", None, 100),
        "cocoa_way_logs" => {
            let session = arguments
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| "cocoa_way_logs requires a string session argument".to_string())?;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 1000) as usize;
            ("logs", Some(session), limit)
        }
        _ => return Err(format!("Unknown tool: {name}")),
    };

    let request = ControlRequest {
        command: command.into(),
        session,
        limit,
    };
    match control_client::send_request(socket, &request) {
        Ok(response) => {
            let structured = serde_json::to_value(&response)
                .map_err(|error| format!("failed to encode Cocoa-Way response: {error}"))?;
            let text = serde_json::to_string_pretty(&structured)
                .map_err(|error| format!("failed to format Cocoa-Way response: {error}"))?;
            Ok(json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": structured,
                "isError": !response.ok
            }))
        }
        Err(error) => Ok(json!({
            "content": [{ "type": "text", "text": error }],
            "structuredContent": { "error": error },
            "isError": true
        })),
    }
}

fn json_rpc_success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn json_rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn fail(message: &str) -> ! {
    eprintln!("cocoa-way-mcp: {message}");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_negotiates_the_current_protocol() {
        let mut initialized = false;
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2025-11-25" }
            }),
            Path::new("/tmp/missing.sock"),
            &mut initialized,
        )
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tool_catalog_is_read_only() {
        let catalog = tools();
        assert_eq!(catalog.len(), 5);
        assert!(catalog.iter().all(|tool| {
            tool["annotations"]["readOnlyHint"] == true
                && tool["annotations"]["destructiveHint"] == false
        }));
        assert!(
            catalog.iter().all(|tool| {
                !matches!(tool["name"].as_str(), Some("launch" | "stop" | "delete"))
            })
        );
    }

    #[test]
    fn tools_are_blocked_until_initialized_notification() {
        let mut initialized = false;
        let response = handle_message(
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            Path::new("/tmp/missing.sock"),
            &mut initialized,
        )
        .unwrap();
        assert_eq!(response["error"]["code"], -32002);
    }
}
