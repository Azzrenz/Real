//! 设备配对与鉴权接口（`/api/auth/*`）。
//!
//! 安全边界由 `auth_guard` 在网关层强制：
//! - `POST /api/auth/pair/code` **仅本机**（远程拿不到配对码 ⇒ 无法自配对）；
//! - `POST /api/auth/pair` 是**唯一未鉴权入口**，自保靠配对码 + 尝试上限 5 次 + 每分钟限速；
//! - `sse-ticket` / `devices` 系列需有效 token（回环自动放行）。

use crate::auth;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// `POST /api/auth/pair/code` —— 生成 6 位配对码（**仅本机**；守卫层已保证来源）。
/// 码在电脑界面上显示给用户，用户在自己的手机上输入。
pub async fn pair_code(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let (code, expires_at) = state.auth.issue_pair_code(auth::now_secs());
    tracing::info!(
        expires_at,
        attempts = auth::PAIR_CODE_MAX_ATTEMPTS,
        "已生成配对码（5 分钟有效，尝试上限 {} 次）",
        auth::PAIR_CODE_MAX_ATTEMPTS
    );
    Ok(Json(json!({
        "code": code,
        "expires_in": auth::PAIR_CODE_TTL_SECS,
        "expires_at": expires_at,
        "max_attempts": auth::PAIR_CODE_MAX_ATTEMPTS,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PairReq {
    /// 用户手机输入的 6 位配对码
    pub code: String,
    /// 设备人读名（可选，缺省「未命名设备」）
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /api/auth/pair` —— 用配对码换设备 token（**唯一未鉴权入口**）。
/// 响应体回明文 token（**仅此一次**）并同时下发 HttpOnly Cookie。
pub async fn pair(State(state): State<AppState>, Json(req): Json<PairReq>) -> AppResult<Response> {
    if !state.auth.allow_pair_attempt(auth::now_secs()) {
        return Err(AppError::TooManyRequests(
            auth::PairError::RateLimited.message().to_string(),
        ));
    }
    let token = state
        .auth
        .redeem_pair_code(&req.code, auth::now_secs())
        .map_err(|e| {
            tracing::warn!(reason = ?e, "配对失败");
            AppError::Unauthorized(e.message().to_string())
        })?;

    let device = auth::create_device(&state.pool, req.name.as_deref().unwrap_or(""), &token).await?;
    tracing::info!(device = %device.id, name = %device.name, "设备配对成功");
    // token 明文只在响应体出现这一次；Cookie 是 HttpOnly，前端脚本读不到
    let cookie = auth::device_cookie(&token);
    let mut resp = Json(json!({
        "ok": true,
        "device_id": device.id,
        "name": device.name,
        "token": token,
    }))
    .into_response();
    match HeaderValue::from_str(&cookie) {
        Ok(v) => {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
        Err(_) => tracing::error!("设备 Cookie 构造失败（token 未下发到 Cookie，仅响应体可用）"),
    }
    Ok(resp)
}

#[derive(Debug, Deserialize)]
pub struct SseTicketReq {
    pub session_id: String,
}

/// `POST /api/auth/sse-ticket` —— 换一次性 SSE 票据（需 token）。
/// 为什么要它：前端用 `new EventSource()`（`ChatView.tsx`），**EventSource 不能自定义 header**，
/// 所以既不能用 Bearer，也不该把长期 token 放 URL（会进日志/历史/代理）。
pub async fn sse_ticket(
    State(state): State<AppState>,
    Json(req): Json<SseTicketReq>,
) -> AppResult<Json<Value>> {
    let sid = req.session_id.trim();
    if sid.is_empty() {
        return Err(AppError::Validation("session_id 不能为空".into()));
    }
    // 会话必须存在（与 SSE 端点同一道隔离：不存在的会话不发票）
    if crate::db::repos::get_session(&state.pool, sid).await?.is_none() {
        return Err(AppError::NotFound(format!("会话 {sid} 不存在")));
    }
    let (ticket, _expires_at) = state.auth.issue_sse_ticket(sid, auth::now_secs());
    Ok(Json(json!({
        "ticket": ticket,
        "expires_in": auth::SSE_TICKET_TTL_SECS,
    })))
}

/// `GET /api/auth/devices` —— 列出已配对设备（不含 token / 哈希）。
pub async fn list_devices(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let devices = auth::list_devices(&state.pool).await?;
    Ok(Json(json!({ "devices": devices })))
}

/// `DELETE /api/auth/devices/{id}` —— 踢掉某设备（其 token 立即失效）。
pub async fn delete_device(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    if !auth::delete_device(&state.pool, &id).await? {
        return Err(AppError::NotFound(format!("设备 {id} 不存在")));
    }
    tracing::info!(device = %id, "已踢掉设备");
    Ok(Json(json!({ "ok": true, "device_id": id })))
}
