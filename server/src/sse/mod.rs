//! SSE 事件域（收口：原 4 文件 → 1）

pub mod gate;

use crate::db::repos;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use axum::Router;
use futures_util::stream::{self, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

// ── 事件 kind 常量（单一事实源：编排内禁止裸字符串字面量）────────────────
pub const EV_LLM_USAGE: &str = "llm.usage";
pub const EV_THINKING: &str = "thinking";
pub const EV_REASONING: &str = "reasoning";
pub const EV_MESSAGE: &str = "message";
pub const EV_TOOL: &str = "tool";
pub const EV_COMPLETE: &str = "complete";
/// 0028+ 自动沉淀候选（B 档·只读版）：任务完成后复盘挑出的可复用经验，
pub const EV_EVOLUTION: &str = "evolution.candidates";
/// 后台长任务活性心跳（run 后台/采样/就绪等待期间每 4s 一拍，不落库只广播）
pub const EV_PROGRESS: &str = "progress";
/// 0028 用户插话注入事件：任务运行中的插话以事件身份进流，
pub const EV_USER_INTERJECTION: &str = "user_interjection";
/// 取消：编排接 CancellationToken 后提前返回 Err → chat 路由发本事件（前端取消分支等它）
pub const EV_CANCELLED: &str = "cancelled";
/// 会话级失败（run_agent 失败时发射；前端 error 分支处理轮次失败态）
pub const EV_ERROR: &str = "error";
pub const EV_LLM_RETRY: &str = "llm.retry";
/// 上下文构成快照（成本度量·一）：每轮 decide 前发射一次，
pub const EV_CTX_SNAPSHOT: &str = "ctx.snapshot";

/// 发射事件：先落库取 seq，再广播（顺序不可换——DB 是唯一事实源）
pub(crate) async fn emit(
    ctx: &AppState,
    session_id: &str,
    kind: &str,
    payload: Value,
) -> AppResult<()> {
    let seq = repos::insert_event(&ctx.pool, session_id, kind, &payload).await?;
    ctx.hub.publish(session_id, seq, kind, payload);
    Ok(())
}

/// 发射瞬时事件：**只广播不落库**——用于后台长任务活性心跳（每 4s 一拍，
pub(crate) fn emit_ephemeral(ctx: &AppState, session_id: &str, kind: &str, payload: Value) {
    ctx.hub.publish(session_id, -1, kind, payload);
}

/// 给事件载荷盖上 run 身份（**run 是事件的坐标系**）。
pub fn with_run(mut payload: Value, run_id: &str) -> Value {
    if run_id.is_empty() {
        return payload;
    }
    if let Value::Object(ref mut m) = payload {
        m.insert("run_id".to_string(), Value::String(run_id.to_string()));
    }
    payload
}

/// 工具动作中文映射（前端"正在做什么"文案；不认识的裸名兜底"正在执行"）
pub fn tool_action_zh(name: &str) -> &'static str {
    match name {
        // 改版：动作词是**纯动词**，不再带「正在」前缀与「文件」这类通用宾语。
        "read" => "读取",
        "write" => "写入",
        "edit" | "modify" => "编辑",
        "search" => "搜索",
        "find_files" => "查找",
        "list" => "列出",
        "run" => "执行",
        "verify" => "验证",
        "env" => "检查环境",
        "audit" => "审计项目",
        "db_query" => "查询数据",
        "web_fetch" => "抓取网页",
        "self_heal" => "诊断",
        "diagnose" => "诊断问题",
        _ => "执行",
    }
}

/// 从工具参数提取路径/命令（前端展示"正在读取文件 D:\xxx\yyy.rs"用）
pub fn tool_path_from_args(args: &Value) -> Option<String> {
    for k in ["file", "path", "target", "cwd"] {
        if let Some(s) = args
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            return Some(s.to_string());
        }
    }
    if let Some(p) = args
        .get("paths")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        return Some(p.to_string());
    }
    // 完整展示命令（不截断——run 长命令的路径部分被切掉后前端看不到在跑哪个文件）
    args.get("command")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

// ── 广播 Hub（每会话一个 broadcast channel；客户端只订阅自己的 session）──
#[derive(Clone)]
pub struct EventHub {
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
}

impl EventHub {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 订阅某会话事件流
    pub fn subscribe(&self, session_id: &str) -> broadcast::Receiver<String> {
        let mut map = self.channels.lock().unwrap_or_else(|e| e.into_inner());
        let tx = map
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(4096).0)
            .clone();
        tx.subscribe()
    }

    /// 发布事件（seq 由调用方从 DB 取得，保证与回放同源）
    pub fn publish(&self, session_id: &str, seq: i64, kind: &str, payload: Value) {
        let event = json!({
            "seq": seq,
            "kind": kind,
            "payload": payload,
            // 实时事件带后端时间戳（前端用作轮次 startedAt，与 messages.created_at 同源，
            "ts": chrono::Utc::now().to_rfc3339(),
        });
        let map = self.channels.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = map.get(session_id) {
            let _ = tx.send(event.to_string());
        }
    }

    /// 关闭会话频道（会话删除时调用）
    pub fn remove(&self, session_id: &str) {
        self.channels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

// ── HTTP 端点（SSE 流 + 轮询增量 + 断线回放）────────────────────────────
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/sessions/{id}/events", get(events))
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub after: Option<i64>,
    /// poll=true：非流式增量拉取——只回放 after 之后的事件、立即返回 JSON 数组，不订阅实时流。
    #[serde(default, deserialize_with = "parse_bool_loose")]
    pub poll: Option<bool>,
}

pub(crate) fn parse_bool_loose<'de, D>(d: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(d)?;
    Ok(s.map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }))
}

fn to_event(data: String) -> Event {
    Event::default().data(data)
}

pub async fn events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> AppResult<Response> {
    // 会话存在性校验（隔离第二道锁：不存在的会话直接 404）
    repos::get_session(&state.pool, &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {session_id} 不存在")))?;

    let mut rx = state.hub.subscribe(&session_id);
    let replay_rows =
        repos::list_events_after(&state.pool, &session_id, q.after.unwrap_or(0)).await?;

    // poll=1：返回与 SSE 同构的 {seq,kind,payload,ts} 数组，前端 applyEvent 直接消费
    if q.poll.unwrap_or(false) {
        let items: Vec<Value> = replay_rows
            .into_iter()
            .map(|e| {
                let payload = serde_json::from_str::<Value>(&e.payload_json).unwrap_or(json!({}));
                json!({"seq": e.id, "kind": e.kind, "payload": payload, "ts": e.created_at})
            })
            .collect();
        return Ok(Json(items).into_response());
    }

    // 回放事件带 DB 真实时间戳（前端用作轮次 startedAt，不等于"回放时刻"）
    let replay: Vec<Event> = replay_rows
        .into_iter()
        .map(|e| {
            let payload = serde_json::from_str::<Value>(&e.payload_json).unwrap_or(json!({}));
            to_event(
                json!({"seq": e.id, "kind": e.kind, "payload": payload, "ts": e.created_at})
                    .to_string(),
            )
        })
        .collect();

    let initial = stream::iter(replay).map(|e| Ok::<_, Infallible>(e));
    let live = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(msg) => yield Ok::<_, Infallible>(to_event(msg)),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    // Lagged = 广播缓冲已满、部分事件被跳过。仅告警会让实时 UI 缺事件，
                    tracing::warn!(session = %session_id, lagged = n, "SSE 消费过慢，断开连接触发游标回放");
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(Box::pin(initial.chain(live)))
        .keep_alive(KeepAlive::default().interval(std::time::Duration::from_secs(15)))
        .into_response())
}
/// 会话自动命名/漂移改名（{title}）——前端据此刷新侧栏标题
pub const EV_SESSION_TITLE: &str = "session.title";

#[cfg(test)]
#[path = "sse_tests.rs"]
mod sse_tests;
