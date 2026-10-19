//! 健康检查（fullstack-dev 清单：/health liveness + /ready readiness）
//!
//! ⚠️ 模糊化：未鉴权调用只回 `{"status": …}`，**不回服务名 / 工具表 / LLM 模式**——
//! 否则远程探测者凭 `/ready` 就能拿到本机的工具面清单，等于给攻击者递地图。
//! （`/health`、`/ready` 不在 `auth_guard` 覆盖范围，故在此自行判定鉴权。）

use crate::error::AppResult;
use crate::routes::auth_guard;
use crate::state::AppState;
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};
use std::net::SocketAddr;

pub async fn health(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Json<Value> {
    if auth_guard::is_authenticated(&state, &headers, Some(addr)).await {
        return Json(json!({"status": "ok", "service": "real-server"}));
    }
    Json(json!({"status": "ok"}))
}

pub async fn ready(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    let status = if db_ok { "ok" } else { "degraded" };
    if !auth_guard::is_authenticated(&state, &headers, Some(addr)).await {
        return Ok(Json(json!({ "status": status })));
    }
    Ok(Json(json!({
        "status": status,
        "checks": {
            "database": if db_ok { "ok" } else { "error" },
            "llm_mode": format!("{:?}", state.cfg.llm_mode),
            "tools": state.registry.tool_names(),
        }
    })))
}
