//! 定时任务管理接口（Cron / Heartbeat）

use crate::error::{AppError, AppResult};
use crate::scheduler;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct CreateScheduleReq {
    pub name: String,
    pub prompt: String,
    /// 标准 5 字段 cron（如 "0 9 * * *"）或自然语言（如 "每天早上9点检查依赖安全"）
    pub schedule: String,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleReq {
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateScheduleReq>,
) -> AppResult<Json<Value>> {
    if req.name.trim().is_empty() {
        return Err(AppError::Validation("name 不能为空".into()));
    }
    if req.prompt.trim().is_empty() {
        return Err(AppError::Validation("prompt 不能为空".into()));
    }
    let (cron, nl) = scheduler::normalize_schedule(&req.schedule).map_err(AppError::Validation)?;
    let ws = req.workspace.filter(|w| !w.trim().is_empty());
    let job = scheduler::create_job(
        &state.pool,
        &req.name,
        &req.prompt,
        &cron,
        &nl,
        ws.as_deref(),
    )
    .await?;
    tracing::info!(job = %job.id, cron = %cron, "定时任务已创建");
    Ok(Json(json!({ "job": job })))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let jobs = scheduler::list_jobs(&state.pool).await?;
    Ok(Json(json!({ "jobs": jobs })))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let ok = scheduler::delete_job(&state.pool, &id).await?;
    if !ok {
        return Err(AppError::NotFound(format!("定时任务 {id} 不存在")));
    }
    Ok(Json(json!({ "deleted": true, "id": id })))
}

pub async fn toggle(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ToggleReq>,
) -> AppResult<Json<Value>> {
    let enabled = req.enabled.unwrap_or(true);
    let ok = scheduler::set_enabled(&state.pool, &id, enabled).await?;
    if !ok {
        return Err(AppError::NotFound(format!("定时任务 {id} 不存在")));
    }
    // 启用时补算下次触发时间
    if enabled {
        if let Some(job) = scheduler::get_job(&state.pool, &id).await? {
            if let Some(next) = scheduler::next_occurrence_cron(&job.cron_expr, &chrono::Utc::now())
            {
                scheduler::reschedule(&state.pool, &id, &next.to_rfc3339()).await?;
            }
        }
    }
    Ok(Json(json!({ "id": id, "enabled": enabled })))
}

pub async fn run_now(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let job = scheduler::get_job(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("定时任务 {id} 不存在")))?;
    let st = state.clone();
    let j = job.clone();
    tokio::spawn(async move {
        scheduler::run_job(st, j).await;
    });
    Ok(Json(json!({ "triggered": true, "id": id })))
}
