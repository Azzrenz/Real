//! MCP 适配层端到端自检：连真正的 MCP server 子进程走完整链路

use super::*;
use serde_json::json;

/// stdio 链路往返（连接 → 列工具 → 调用 → 回显断言）
#[tokio::test]
#[ignore = "需先 cargo build --example echo_mcp_server"]
async fn echo_server_stdio_roundtrip() {
    let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug/examples/echo_mcp_server.exe");
    assert!(
        exe.exists(),
        "自检样例未编译：先执行 cargo build --example echo_mcp_server"
    );

    let cfg = McpServerConfigRef {
        name: "demo".to_string(),
        transport: "stdio".to_string(),
        command: Some(exe.to_string_lossy().to_string()),
        args: vec![],
        url: None,
        headers: Default::default(),
    };

    let client = McpClient::connect(&cfg).await.expect("连接自检样例服务器失败");
    let tools = client.list_tools().await.expect("列工具失败");
    assert!(
        tools.iter().any(|t| t.name == "echo_mcp"),
        "应看到 echo_mcp 工具，实际: {tools:?}"
    );

    let res = client
        .call_tool("echo_mcp", json!({ "message": "hi" }))
        .await
        .expect("调用工具失败");
    assert_eq!(res["isError"], json!(false), "调用应成功，实际: {res}");
    let text = res["content"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains("MCP-ECHO"), "应回显消息，实际: {text}");
}
