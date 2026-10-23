//! `orbit-aigent approve`: the MCP server Claude Code asks for permission
//! (`--permission-prompt-tool mcp__orbit_aigent__approve`), over stdio,
//! one JSON-RPC message a line. Its one tool, `approve`, takes the tool
//! Claude wants to use and its input, asks the daemon (`Command::Approve`)
//! and waits for the answer, which the daemon gets by voice or a click.
//! Without the daemon, or with no answer, the answer is no.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};

use orbit_aigent::Command;
use serde_json::{Value, json};

pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        // Notifications have no id and get no answer.
        let Some(id) = message.get("id").cloned() else { continue };
        let reply = match message["method"].as_str() {
            Some("initialize") => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": message["params"]["protocolVersion"],
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "orbit-aigent", "version": env!("CARGO_PKG_VERSION") },
                },
            }),
            Some("tools/list") => json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "tools": [{
                    "name": "approve",
                    "description": "Asks the user, by voice, whether a tool may be used.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tool_name": { "type": "string" },
                            "input": { "type": "object" },
                        },
                        "required": ["tool_name", "input"],
                    },
                }]},
            }),
            Some("tools/call") => {
                let arguments = &message["params"]["arguments"];
                let tool = arguments["tool_name"].as_str().unwrap_or_default().to_owned();
                let input = arguments["input"].clone();
                let decision = if ask(&tool, &input) {
                    json!({ "behavior": "allow", "updatedInput": input })
                } else {
                    json!({ "behavior": "deny", "message": "The user said no." })
                };
                json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": decision.to_string() }] },
                })
            }
            Some("ping") => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            _ => json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": "method not found" },
            }),
        };
        if writeln!(stdout, "{reply}").and_then(|()| stdout.flush()).is_err() {
            break;
        }
    }
    0
}

/// Asks the daemon; `true` when the user said yes.
fn ask(tool: &str, input: &Value) -> bool {
    let Ok(mut sock) = UnixStream::connect(orbit_aigent::socket_path()) else { return false };
    let command = Command::Approve {
        tool: tool.to_owned(),
        input: input.clone(),
    };
    let mut line = serde_json::to_vec(&command).expect("a command serialises");
    line.push(b'\n');
    if sock.write_all(&line).is_err() {
        return false;
    }
    // The frames every client gets go by; the answer is the line with
    // `approved` in it.
    for line in BufReader::new(sock).lines() {
        let Ok(line) = line else { break };
        if let Ok(reply) = serde_json::from_str::<Value>(&line)
            && let Some(approved) = reply["approved"].as_bool()
        {
            return approved;
        }
    }
    false
}
