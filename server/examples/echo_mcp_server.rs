//! 最小 MCP stdio Server 示例（演示用）
//!
//! 协议：JSON-RPC 2.0，stdin 读请求、stdout 写响应（每行一个 JSON 消息）
//! 工具：echo_mcp（回显）
//!
//! 运行：
//!   cargo run --example echo_mcp_server
//! 接入 Real：
//!   REAL_MCP_SERVERS='[{"name":"demo","transport":"stdio","command":"real-server.exe","args":["--example","echo_mcp_server"]}]'
//!   （或直接编译为独立 exe 后填 command 路径）

use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let result: Value = match method {
            "tools/list" => json!({
                "tools": [{
                    "name": "echo_mcp",
                    "description": "回显消息（来自 stdio MCP server 示例）",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "message": {"type": "string", "description": "要回显的内容"}
                        },
                        "required": ["message"]
                    }
                }]
            }),
            "tools/call" => {
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(空)");
                if name == "echo_mcp" {
                    json!({
                        "content": [{"type": "text", "text": format!("MCP-ECHO: {message}")}],
                        "isError": false
                    })
                } else {
                    json!({
                        "content": [{"type": "text", "text": format!("未知工具: {name}")}],
                        "isError": true
                    })
                }
            }
            // 兼容旧版握手（我们的客户端不需要，但友好处理）
            "initialize" => json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "real-demo-mcp", "version": "0.1.0"}
            }),
            _ => json!({}),
        };

        let response = json!({"jsonrpc": "2.0", "id": id, "result": result});
        if writeln!(stdout, "{}", response).is_err() {
            break;
        }
        stdout.flush().ok();
    }
}
