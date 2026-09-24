//! 会话 CRUD（多任务/多会话管理）

use crate::db::repos;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct CreateSessionReq {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default)]
    pub system_prompt: String,
    /// (可选)显式绑定工作区；缺省回退 settings default_workspace
    #[serde(default)]
    pub workspace: String,
}

/// reasoning 事件历史回放聚合（适配增量协议）
pub(crate) fn sample_reasoning_spanned(
    events: Vec<repos::EventRow>,
) -> Vec<(repos::EventRow, i64, i64)> {
    let mut out: Vec<(repos::EventRow, i64, i64)> = Vec::with_capacity(events.len());
    for e in events {
        if e.kind == "reasoning" {
            let txt = serde_json::from_str::<serde_json::Value>(&e.payload_json)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
                .unwrap_or_default();
            if txt.trim().is_empty() {
                continue;
            }
            if let Some((last, _from, to)) = out.last_mut() {
                if last.kind == "reasoning" {
                    let last_txt =
                        serde_json::from_str::<serde_json::Value>(&last.payload_json)
                            .ok()
                            .and_then(|v| {
                                v.get("text").and_then(|t| t.as_str()).map(str::to_string)
                            })
                            .unwrap_or_default();
                    // 旧累计全文残留：后一条含前一条全文 → 替换
                    if txt.starts_with(&last_txt) && txt.len() > last_txt.len() {
                        *to = e.id;
                        *last = e;
                        continue;
                    }
                    // 增量碎片（新协议）：拼接还原全文
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&last.payload_json) {
                        if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                            let merged = format!("{}{}", t, txt);
                            last.payload_json = serde_json::json!({ "text": merged }).to_string();
                            *to = e.id;
                            continue;
                        }
                    }
                }
            }
            // 先取 id 再 move（EventRow 无 Copy，同表达式内先 move 后取字段会编译错）
            let (from, to) = (e.id, e.id);
            out.push((e, from, to));
        } else {
            let (from, to) = (e.id, e.id);
            out.push((e, from, to));
        }
    }
    out
}

/// 兼容包装：只要聚合后的行（不含 span 信息）
pub(crate) fn sample_reasoning_events(events: Vec<repos::EventRow>) -> Vec<repos::EventRow> {
    sample_reasoning_spanned(events)
        .into_iter()
        .map(|(e, _from, _to)| e)
        .collect()
}

fn default_title() -> String {
    "新任务".to_string()
}

// ---- 任务中插话（0026）：忙时不拒收，入队等轮边界注入 ----

#[derive(Debug, Deserialize)]
pub struct InterjectBody {
    pub text: String,
    #[serde(default = "default_interject_mode")]
    pub mode: String,
}
fn default_interject_mode() -> String {
    "append".into()
}

pub async fn interject(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<InterjectBody>,
) -> AppResult<Json<Value>> {
    repos::get_session(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {id} 不存在")))?;
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::Validation("插话内容不能为空".into()));
    }
    if req.mode != "append" && req.mode != "interrupt" {
        return Err(AppError::Validation(
            "mode 仅支持 append（追加）/interrupt（停下换活）".into(),
        ));
    }
    // 会话原生插话（对齐平台体验）：插话同时落一条真实用户消息——
    let item_json = json!({
        "type": "message",
        "role": "user",
        "interjection": true,
        "content": [{"type": "input_text", "text": text.clone()}],
    });
    let running = repos::get_session(&state.pool, &id)
        .await?
        .map(|s| s.status != "done")
        .unwrap_or(false);
    let msg_id = if running {
        None
    } else {
        let msg = repos::insert_message(
            &state.pool,
            &id,
            "user",
            &text,
            Some(&serde_json::to_string(&item_json).unwrap()),
        )
        .await?;
        Some(msg.id)
    };
    // interjection 行带 message_id 关联（cancel 时据此作废消息，防新轮历史重建死而复生）
    repos::insert_interjection(&state.pool, &id, &text, &req.mode, msg_id.as_deref()).await?;
    let queued = repos::count_pending_interjections(&state.pool, &id).await?;
    Ok(Json(json!({"ok": true, "queued": queued})))
}

#[derive(Debug, Deserialize)]
pub struct InterjectQuery {
    #[serde(default)]
    pub status: String,
}

pub async fn interjections(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<InterjectQuery>,
) -> AppResult<Json<Value>> {
    let rows = repos::list_interjections(&state.pool, &id, Some(q.status).filter(|s| !s.is_empty()))
        .await?;
    Ok(Json(json!({"interjections": rows})))
}

pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionReq>,
) -> AppResult<Json<Value>> {
    // 未传 system_prompt 时使用运行时设置的默认提示词
    let system_prompt = if req.system_prompt.trim().is_empty() {
        state.settings.read().await.default_system_prompt.clone()
    } else {
        req.system_prompt.clone()
    };
    // （删外部验收）：会话不再携带验收命令——验收由模型自己跑、自己判。
    let session = repos::create_session(&state.pool, &req.title, &system_prompt).await?;
    let mut ws = req.workspace.trim().to_string();
    if ws.is_empty() {
        ws = repos::get_setting(&state.pool, "default_workspace")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
    }
    if !ws.is_empty() {
        let _ = repos::set_session_workspace(&state.pool, &session.id, ws.trim()).await;
    }
    tracing::info!(session = %session.id, "会话已创建");
    Ok(Json(json!({"session": session})))
}

/// PATCH /api/sessions/{id} — 改名 / 更新系统提示词（运行时设置与改名持久化）
#[derive(Debug, Deserialize)]
pub struct UpdateSessionReq {
    pub title: Option<String>,
    pub system_prompt: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionReq>,
) -> AppResult<Json<Value>> {
    if req.title.is_none() && req.system_prompt.is_none() {
        return Err(AppError::Validation(
            "至少提供 title 或 system_prompt 之一".into(),
        ));
    }
    let title = req
        .title
        .as_deref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty());
    let sp = req.system_prompt.as_deref();
    let updated = repos::update_session(&state.pool, &id, title, sp)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {id} 不存在")))?;
    tracing::info!(session = %id, title = ?title, "会话已更新");
    Ok(Json(json!({"session": updated})))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let sessions = repos::list_sessions(&state.pool).await?;
    Ok(Json(json!({"sessions": sessions})))
}

/// 面板每批事件条数（首屏与"加载更早"共用）。
const EVENTS_PAGE: i64 = 10_000;

/// 事件行 → 前端契约 JSON。详情接口与"加载更早"端点共用同一次转换，
fn events_to_json(rows: Vec<repos::EventRow>, skeleton: bool) -> Vec<serde_json::Value> {
    sample_reasoning_spanned(rows)
        .into_iter()
        .map(|(e, from, to)| {
            let mut payload = serde_json::from_str::<serde_json::Value>(&e.payload_json)
                .unwrap_or_else(|_| json!({}));
            // 骨架回放：思考全文替换为占位（长度 + 预览 + seq 区间），其余事件原样
            if skeleton && e.kind == "reasoning" {
                let text = payload
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default();
                let chars: Vec<char> = text.chars().collect();
                let mut preview: String = chars.iter().take(120).collect();
                if chars.len() > 120 {
                    preview.push('…');
                }
                payload = json!({
                    "omitted": true,
                    "len": chars.len(),
                    "preview": preview,
                    "from": from,
                    "to": to,
                });
            }
            json!({"seq": e.id, "kind": e.kind, "payload": payload, "ts": e.created_at})
        })
        .collect()
}

/// GET /api/sessions/{id} 查询参数
#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    /// skeleton=true：骨架回放（会话加载提速）——思考全文不随响应下发，
    #[serde(default, deserialize_with = "crate::sse::parse_bool_loose")]
    pub skeleton: Option<bool>,

    /// events=0：不返回 events 数组（冷启动减负）——前端重建会话过程区已
    #[serde(default, deserialize_with = "crate::sse::parse_bool_loose")]
    pub events: Option<bool>,
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SessionQuery>,
) -> AppResult<Json<Value>> {
    let session = repos::get_session(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {id} 不存在")))?;
    let messages = repos::list_messages(&state.pool, &id).await?;
    let tool_calls = repos::list_tool_calls(&state.pool, &id).await?;
    let plans = repos::list_plans(&state.pool, &id).await?;
    let skeleton = q.skeleton.unwrap_or(false);
    let include_events = q.events.unwrap_or(true);
    // 坑位 J55：历史回放靠 events（thinking/verify/tool.result/cost 都持久化在这里），
    let (events, events_has_earlier) = if !include_events {
        (Vec::new(), false)
    } else {
        let raw = repos::list_events_tail(&state.pool, &id, EVENTS_PAGE).await?;
        let has_earlier = match raw.first() {
            Some(first) => !repos::list_events_before(&state.pool, &id, first.id, 1)
                .await?
                .is_empty(),
            None => false,
        };
        (events_to_json(raw, skeleton), has_earlier)
    };

    Ok(Json(json!({
        "session": session,
        "messages": messages,
        "tool_calls": tool_calls,
        "plans": plans,
        "events": events,
        "events_has_earlier": events_has_earlier,
    })))
}

/// GET /api/sessions/{id}/thinking?from=&to= — 思考全文懒加载（骨架回放的配套端点）。
#[derive(Debug, Deserialize)]
pub struct ThinkingQuery {
    pub from: i64,
    pub to: i64,
}

pub async fn thinking(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ThinkingQuery>,
) -> AppResult<Json<Value>> {
    // 会话存在性校验（与 events 端点同口径：不存在的会话直接 404）
    repos::get_session(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {id} 不存在")))?;
    let rows = repos::list_reasoning_between(&state.pool, &id, q.from, q.to).await?;
    // 区间内全部聚合后按序拼接（正常情况一轮思考聚合为一条；碎片被 thinking 隔开等
    let text = sample_reasoning_events(rows)
        .iter()
        .filter_map(|e| {
            serde_json::from_str::<serde_json::Value>(&e.payload_json)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
        })
        .collect::<String>();
    Ok(Json(json!({"text": text, "len": text.chars().count()})))
}

/// GET /api/sessions/{id}/events/before?before=&limit= — 面板"加载更早"（往前分页）。
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub before: i64,
    pub limit: Option<i64>,
}

pub async fn events_before(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
) -> AppResult<Json<Value>> {
    repos::get_session(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {id} 不存在")))?;
    let limit = q.limit.unwrap_or(EVENTS_PAGE).clamp(1, EVENTS_PAGE);
    let rows = if q.before > 0 {
        repos::list_events_before(&state.pool, &id, q.before, limit).await?
    } else {
        repos::list_events_tail(&state.pool, &id, limit).await?
    };
    let has_earlier = match rows.first() {
        Some(first) => !repos::list_events_before(&state.pool, &id, first.id, 1)
            .await?
            .is_empty(),
        None => false,
    };
    // 与详情接口走同一个转换函数；这里一律给思考全文——翻历史的人正是在找细节。
    Ok(Json(json!({
        "events": events_to_json(rows, false),
        "has_earlier": has_earlier,
    })))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let _ = state.cancel_session(&id).await;
    let deleted = repos::delete_session(&state.pool, &id).await?;
    if deleted {
        crate::agent::memory::journal::purge_task_workspace(&state.pool, &id).await;
    }
    state.hub.remove(&id);
    state.forget_run(&id);
    // 审计 A3：删除会话时级联清理该 session 的记忆 + 工作区
    let _ = repos::forget_memory_by_session(&state.pool, &id).await;
    crate::agent::workspace::clear_session(&id);
    // 级联清理写授权记忆（确认门批准过的目录随会话作废）
    crate::path::clear_write_grants(&id);
    if !deleted {
        return Err(AppError::NotFound(format!("会话 {id} 不存在")));
    }
    Ok(Json(json!({"deleted": true, "id": id})))
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod sessions_tests;
