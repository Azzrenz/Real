//! MCP 服务器管理接口（生态运维面：配置持久化 + 状态可见 + 热同步）

use crate::config::McpServerConfig;
use crate::db::repos;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// GET /api/mcp/servers —— 运行状态 + 已保存配置（配置用于界面回填，编辑/删除需要它）
pub async fn list(State(state): State<AppState>) -> Json<Value> {
    let servers = state.registry.mcp_server_status();
    let saved: Value = match repos::get_setting(&state.pool, "mcp_servers").await {
        Ok(Some(raw)) => serde_json::from_str(&raw).unwrap_or_else(|_| json!([])),
        _ => json!([]),
    };
    Json(json!({ "servers": servers, "saved": saved }))
}

#[derive(Debug, Deserialize)]
pub struct SaveServersReq {
    /// 全量服务器配置（覆盖式保存）
    pub servers: Vec<McpServerConfig>,
}

/// PUT /api/mcp/servers —— 保存配置（写 DB，不自动热同步；调 apply 生效）
pub async fn save(
    State(state): State<AppState>,
    Json(req): Json<SaveServersReq>,
) -> Json<Value> {
    let raw = serde_json::to_string(&req.servers).unwrap_or_else(|_| "[]".to_string());
    match repos::set_setting(&state.pool, "mcp_servers", &raw).await {
        Ok(_) => Json(json!({ "ok": true, "count": req.servers.len() })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

/// POST /api/mcp/servers/apply —— 按 DB 配置热同步
pub async fn apply(State(state): State<AppState>) -> Json<Value> {
    let wanted: Vec<McpServerConfig> =
        match repos::get_setting(&state.pool, "mcp_servers").await {
            Ok(Some(raw)) => serde_json::from_str(&raw).unwrap_or_default(),
            _ => Vec::new(),
        };
    // env 兜底：DB 无配置时按启动 env 的清单同步
    let wanted = if wanted.is_empty() { state.cfg.mcp_servers.clone() } else { wanted };

    let mut applied: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for cfg in &wanted {
        let cfg_ref = crate::mcp::client::McpServerConfigRef {
            name: cfg.name.clone(),
            transport: cfg.transport.clone(),
            command: cfg.command.clone(),
            args: cfg.args.clone(),
            url: cfg.url.clone(),
            headers: cfg.headers.clone(),
        };
        match state.registry.upsert_mcp_server(&cfg_ref).await {
            Ok(_) => applied.push(cfg.name.clone()),
            Err(e) => failed.push(json!({ "name": cfg.name, "error": e })),
        }
    }
    // 配置里已不存在的服务器 → 摘除
    let mut removed: Vec<String> = Vec::new();
    for st in state.registry.mcp_server_status() {
        if !wanted.iter().any(|c| c.name == st.name) {
            state.registry.remove_mcp_server(&st.name);
            removed.push(st.name);
        }
    }
    Json(json!({
        "ok": failed.is_empty(),
        "applied": applied,
        "removed": removed,
        "failed": failed,
        "servers": state.registry.mcp_server_status(),
    }))
}

/// POST /api/mcp/servers/{name}/reconnect —— 单服务器重连（按 DB 配置；env 兜底）
pub async fn reconnect(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Json<Value> {
    let db_cfg: Option<McpServerConfig> = match repos::get_setting(&state.pool, "mcp_servers").await
    {
        Ok(Some(raw)) => serde_json::from_str::<Vec<McpServerConfig>>(&raw)
            .ok()
            .and_then(|v| v.into_iter().find(|c| c.name == name)),
        _ => None,
    };
    let cfg = db_cfg.or_else(|| state.cfg.mcp_servers.iter().find(|c| c.name == name).cloned());
    let Some(cfg) = cfg else {
        return Json(json!({ "ok": false, "error": format!("配置中不存在服务器 {name}") }));
    };
    let cfg_ref = crate::mcp::client::McpServerConfigRef {
        name: cfg.name.clone(),
        transport: cfg.transport.clone(),
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        url: cfg.url.clone(),
        headers: cfg.headers.clone(),
    };
    match state.registry.upsert_mcp_server(&cfg_ref).await {
        Ok(_) => Json(json!({ "ok": true, "servers": state.registry.mcp_server_status() })),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}
