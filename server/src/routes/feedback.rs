//! 回复反馈记录（设计决定："赞/踩要反馈到模型"）

use crate::db::repos;
use crate::error::AppError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct FeedbackReq {
    pub session_id: String,
    #[serde(default)]
    pub round_seq: Option<u32>,
    /// like | dislike
    pub vote: String,
    /// 该轮时间锚（startedAt/endedAt），便于回放定位
    #[serde(default)]
    pub ts: Option<String>,
}

pub async fn rate(
    State(state): State<AppState>,
    Json(req): Json<FeedbackReq>,
) -> Result<Json<Value>, AppError> {
    if req.vote != "like" && req.vote != "dislike" {
        return Err(AppError::Validation("vote 只接受 like/dislike".into()));
    }
    if req.session_id.trim().is_empty() {
        return Err(AppError::Validation("session_id 不能为空".into()));
    }
    repos::insert_event(
        &state.pool,
        &req.session_id,
        "user.rating",
        &json!({
            "round_seq": req.round_seq,
            "vote": req.vote,
            "ts": req.ts,
        }),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}
