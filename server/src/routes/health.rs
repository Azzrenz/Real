//! 健康检查（fullstack-dev 清单：/health liveness + /ready readiness）

use crate::error::AppResult;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok", "service": "real-server"}))
}

pub async fn ready(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    Ok(Json(json!({
        "status": if db_ok { "ok" } else { "degraded" },
        "checks": {
            "database": if db_ok { "ok" } else { "error" },
            "llm_mode": format!("{:?}", state.cfg.llm_mode),
            "tools": state.registry.tool_names(),
        }
    })))
}
