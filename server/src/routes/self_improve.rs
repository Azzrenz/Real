//! 自改进闭环的事件注入接口

use crate::db::repos;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct EventReq {
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct StatusReq {
    pub session_id: String,
    pub status: String,
}

/// 注入一条步骤到会话流（tool 事件·append 语义——前端按 step_id 追加显示，历史全保留。
pub async fn push_event(
    State(state): State<AppState>,
    Json(req): Json<EventReq>,
) -> AppResult<Json<serde_json::Value>> {
    repos::get_session(&state.pool, &req.session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {} 不存在", req.session_id)))?;
    let step = crate::state::STEP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let payload = json!({"tools": [{
        "name": "self_improve",
        "action": "自改进步骤",
        "args": {"message": req.message},
        "status": "success",
        "step_id": format!("si_{step}"),
        "result_summary": req.message,
    }]});
    let seq = repos::insert_event(&state.pool, &req.session_id, "tool", &payload).await?;
    state.hub.publish(&req.session_id, seq, "tool", payload);
    Ok(Json(json!({"ok": true, "seq": seq})))
}

/// 上报终态（done/error）——前端据此结束 running 态
pub async fn push_status(
    State(state): State<AppState>,
    Json(req): Json<StatusReq>,
) -> AppResult<Json<serde_json::Value>> {
    repos::get_session(&state.pool, &req.session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {} 不存在", req.session_id)))?;
    let status = if req.status == "done" {
        "done"
    } else {
        "error"
    };
    repos::update_session_status(&state.pool, &req.session_id, status).await?;
    let payload = json!({"message": if status == "done" { "自改进完成" } else { "自改进失败" }});
    let kind = if status == "done" {
        "complete"
    } else {
        "error"
    };
    let seq = repos::insert_event(&state.pool, &req.session_id, kind, &payload).await?;
    state.hub.publish(&req.session_id, seq, kind, payload);
    Ok(Json(json!({"ok": true, "status": status})))
}
