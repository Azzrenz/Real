//! 仓储层：全部查询强制 WHERE session_id = ?（行级隔离）

use crate::error::AppResult;
use chrono::Utc;
use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use sqlx::SqlitePool;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub system_prompt: String,
    pub created_at: String,
    pub updated_at: String,
    /// 本会话选定的模型；None = 跟随全局默认（settings.model）。
    pub model: Option<String>,
    /// 这一块属于哪个领域（任务列表的宏观标题）；None = 还没推断出来或用户没让它写。
    pub area: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct MessageRow {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub item_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ToolCallRow {
    pub id: String,
    pub session_id: String,
    pub plan_id: Option<String>,
    pub step_id: Option<String>,
    pub name: String,
    pub arguments: String,
    pub result_json: Option<String>,
    pub status: String,
    pub duration_ms: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct PlanRow {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub steps_json: String,
    pub status: String,
    pub attempt: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct EventRow {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub payload_json: String,
    pub created_at: String,
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

// Sessions

pub async fn create_session(
    pool: &SqlitePool,
    title: &str,
    system_prompt: &str,
) -> AppResult<SessionRow> {
    let id = uuid::Uuid::new_v4().to_string();
    let ts = now();
    // （删外部验收）：不再写 acceptance_cmd/acceptance_cwd 列。
    sqlx::query(
        "INSERT INTO sessions (id, title, status, system_prompt, created_at, updated_at)
         VALUES (?1, ?2, 'idle', ?3, ?4, ?4)",
    )
    .bind(&id)
    .bind(title)
    .bind(system_prompt)
    .bind(&ts)
    .execute(pool)
    .await?;
    Ok(SessionRow {
        id,
        title: title.into(),
        status: "idle".into(),
        system_prompt: system_prompt.into(),
        created_at: ts.clone(),
        updated_at: ts,
        model: None,
        area: None,
    })
}

pub async fn list_sessions(pool: &SqlitePool) -> AppResult<Vec<SessionRow>> {
    Ok(sqlx::query_as::<_, SessionRow>(
        "SELECT id, title, status, system_prompt, created_at, updated_at, model, area
         FROM sessions ORDER BY updated_at DESC"
    ).fetch_all(pool).await?)
}

pub async fn all_session_ids(pool: &SqlitePool) -> AppResult<std::collections::HashSet<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM sessions")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn get_session(pool: &SqlitePool, id: &str) -> AppResult<Option<SessionRow>> {
    Ok(sqlx::query_as::<_, SessionRow>(
        "SELECT id, title, status, system_prompt, created_at, updated_at, model, area
         FROM sessions WHERE id = ?1"
    ).bind(id).fetch_optional(pool).await?)
}

/// 会话选定的模型（None = 未选过，调用方回退全局默认）
pub async fn get_session_model(pool: &SqlitePool, id: &str) -> AppResult<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT model FROM sessions WHERE id = ?1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0).filter(|m| !m.is_empty()))
}

/// 写入会话选定的模型；Some("") 视为清除（回到跟随全局默认）
pub async fn set_session_model(
    pool: &SqlitePool,
    id: &str,
    model: Option<&str>,
) -> AppResult<()> {
    let value = model.map(|m| m.trim()).filter(|m| !m.is_empty());
    sqlx::query("UPDATE sessions SET model = ?2, updated_at = ?3 WHERE id = ?1")
        .bind(id)
        .bind(value)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

/// 写入会话的领域名（任务列表的宏观标题）。只在该位置还空着时调——
pub async fn set_session_area(pool: &SqlitePool, id: &str, area: &str) -> AppResult<()> {
    sqlx::query("UPDATE sessions SET area = ?2, updated_at = ?3 WHERE id = ?1 AND area IS NULL")
        .bind(id)
        .bind(area.trim())
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_session_status(pool: &SqlitePool, id: &str, status: &str) -> AppResult<()> {
    sqlx::query("UPDATE sessions SET status = ?2, updated_at = ?3 WHERE id = ?1")
        .bind(id)
        .bind(status)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

/// 会话/任务收敛终态（收敛真理外移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    #[allow(dead_code)]
    Executing,
    Done,
    Blocked,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Executing => "executing",
            SessionStatus::Done => "done",
            SessionStatus::Blocked => "blocked",
        }
    }
}

/// 启动自愈：进程崩溃/被强杀后，DB 会话状态会停留在运行中
pub async fn reset_stale_running_sessions(pool: &SqlitePool) -> AppResult<u64> {
    // 启动对账：运行态会话除重置 idle 外，还要给事件流补一条
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE status IN ('planning','executing','solving','reflecting')",
    )
    .fetch_all(pool)
    .await?;
    // 尾段对账（线上反馈"插话气泡重启后丢失"）：status 现已不标记运行中，
    let dangling: Vec<String> = sqlx::query_scalar(
        "SELECT s.id FROM sessions s \
         WHERE EXISTS (SELECT 1 FROM events e WHERE e.session_id = s.id) \
         AND (SELECT e2.kind FROM events e2 WHERE e2.session_id = s.id ORDER BY e2.id DESC LIMIT 1) \
             NOT IN ('complete','error','cancelled','interrupted')",
    )
    .fetch_all(pool)
    .await?;
    for sid in dangling.iter().chain(stale.iter()) {
        let payload = serde_json::json!({ "message": "应用重启，任务被中断" });
        if let Err(e) = insert_event(pool, sid, "interrupted", &payload).await {
            tracing::warn!(session = %sid, error = %e, "补发 interrupted 终态失败（跳过）");
        }
    }
    let r = sqlx::query(
        "UPDATE sessions SET status = 'idle', updated_at = ?1 \
         WHERE status IN ('planning','executing','solving','reflecting')",
    )
    .bind(now())
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// 删除会话 + 级联清理（**DB 与磁盘两侧都要清**）。
pub async fn delete_session(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    let mut tx = pool.begin().await?;
    for table in [
        "events",
        "messages",
        "tool_calls",
        "plans",
        "checkpoints",
        "memories",
        "turn_logs",
        "interjections",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE session_id = ?1"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    // 会话级的 settings 键（历史起点 / 改写计数）—— 原先全部残留
    sqlx::query(
        "DELETE FROM settings WHERE key LIKE 'hist_start:' || ?1 || '%' \
         OR key LIKE 'session_rewrite_mark:' || ?1 || '%'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let r = sqlx::query("DELETE FROM sessions WHERE id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(r.rows_affected() > 0)
}

// Messages（item_json 原样存储，历史重建零失真 —— 无状态对策）

/// 从 item_json 判定**这一行到底是什么**：应有的 role + 是否携带 call_id。
fn item_shape(item_json: &str) -> (Option<String>, bool) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(item_json) else {
        return (None, false);
    };
    let has_call_id = v.get("call_id").is_some() || v.get("tool_calls").is_some();
    let role = match v.get("type").and_then(|x| x.as_str()) {
        Some("function_call_output") => Some("tool".to_string()),
        // 声明属于 assistant（三种载体里的 Raw/FunctionCall 形态）
        Some("function_call") => Some("assistant".to_string()),
        Some("reasoning") => Some("reasoning".to_string()),
        _ => {
            if let Some(r) = v.get("role").and_then(|x| x.as_str()) {
                Some(r.to_string())
            } else if v.get("call_id").is_some() && v.get("output").is_some() {
                Some("tool".to_string())
            } else if v.get("call_id").is_some() && v.get("name").is_some() {
                Some("assistant".to_string())
            } else {
                None
            }
        }
    };
    (role, has_call_id)
}

/// 从 item_json 里取出**它就是 `content` 那一份**的正文（若有）。
fn body_inside_item(item_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(item_json).ok()?;
    let joined = |key: &str| -> Option<String> {
        let arr = v.get(key)?.as_array()?;
        let t: String = arr
            .iter()
            .filter_map(|p| p.get("text").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("");
        (!t.is_empty()).then_some(t)
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("function_call_output") => {
            let o = v.get("output").and_then(|x| x.as_str())?;
            (!o.is_empty()).then(|| o.to_string())
        }
        Some("message") => joined("content"),
        Some("reasoning") => joined("summary").or_else(|| joined("content")),
        _ => None,
    }
}

/// 落库（**唯一入口**）。两件事在这里一处收口
pub async fn insert_message(
    pool: &SqlitePool,
    session_id: &str,
    role: &str,
    content: &str,
    item_json: Option<&str>,
) -> AppResult<MessageRow> {
    let (derived, has_call_id) = item_json.map(item_shape).unwrap_or((None, false));
    let role: String = match derived {
        Some(r) if r != role => {
            tracing::warn!(
                session = session_id,
                given = role,
                derived = %r,
                "落库 role 与 item_json 不同源——已按 item_json 归正（role 的唯一事实源是 item_json）"
            );
            r
        }
        _ => role.to_string(),
    };
    if let Some(ij) = item_json {
        if role != "user" && has_call_id {
            let existed = sqlx::query_as::<_, MessageRow>(
                "SELECT id, session_id, role, content, item_json, created_at FROM messages
                 WHERE session_id = ?1 AND role = ?2 AND item_json = ?3 LIMIT 1",
            )
            .bind(session_id)
            .bind(&role)
            .bind(ij)
            .fetch_optional(pool)
            .await?;
            if let Some(mut row) = existed {
                tracing::debug!(session = session_id, role = %role, "落库幂等：同一条声明/回执已在库，跳过重复插入");
                // 命中的是**库里的行**：它的 content 可能是去重存的空串，
                if row.content.is_empty() {
                    if let Some(body) = row.item_json.as_deref().and_then(body_inside_item) {
                        row.content = body;
                    }
                }
                return Ok(row);
            }
        }
    }
    // **去重**：`content` 若与 `item_json` 里的正文逐字节相同 ⇒ 存空串，
    let stored_content: &str = match item_json.and_then(body_inside_item) {
        Some(body) if body == content => "",
        _ => content,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let ts = now();
    sqlx::query(
        "INSERT INTO messages (id, session_id, role, content, item_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    )
        .bind(&id).bind(session_id).bind(&role).bind(stored_content).bind(item_json).bind(&ts)
        .execute(pool).await?;
    Ok(MessageRow {
        id,
        session_id: session_id.into(),
        role,
        // 返回给调用方的仍是**完整正文**（不是落库那个空串）——
        content: content.into(),
        item_json: item_json.map(String::from),
        created_at: ts,
    })
}

/// 按时间顺序取会话全部消息
pub async fn list_messages(pool: &SqlitePool, session_id: &str) -> AppResult<Vec<MessageRow>> {
    let mut rows = sqlx::query_as::<_, MessageRow>(
        "SELECT id, session_id, role, content, item_json, created_at FROM messages
         WHERE session_id = ?1 ORDER BY rowid ASC",
    // 09-10：改用插入序而非事件时间——created_at 有 6 位/9 位两种格式且压缩重写会
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    for m in rows.iter_mut() {
        if !m.content.is_empty() {
            continue;
        }
        if let Some(body) = m.item_json.as_deref().and_then(body_inside_item) {
            m.content = body;
        }
    }
    Ok(rows)
}

/// 重建 Responses API input items：只取带 item_json 的消息（含 function_call / function_call_output）

// Tool Calls（工具执行审计）

pub async fn list_tool_calls(pool: &SqlitePool, session_id: &str) -> AppResult<Vec<ToolCallRow>> {
    Ok(sqlx::query_as::<_, ToolCallRow>(
        "SELECT id, session_id, plan_id, step_id, name, arguments, result_json, status, duration_ms, created_at
         FROM tool_calls WHERE session_id = ?1 ORDER BY created_at ASC"
    ).bind(session_id).fetch_all(pool).await?)
}

// Plans

/// 取会话全部计划（会话详情接口用；裸 sqlx 收敛进 repos）
pub async fn list_plans(pool: &SqlitePool, session_id: &str) -> AppResult<Vec<PlanRow>> {
    Ok(sqlx::query_as::<_, PlanRow>(
        "SELECT id, session_id, objective, steps_json, status, attempt, created_at
         FROM plans WHERE session_id = ?1 ORDER BY created_at DESC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?)
}

// Events（审计 + SSE 回放游标）

/// 写入事件，返回自增 id（作为 SSE sequence）
pub async fn insert_event(
    pool: &SqlitePool,
    session_id: &str,
    kind: &str,
    payload: &Value,
) -> AppResult<i64> {
    let ts = now();
    let payload_json = payload.to_string();
    let r = sqlx::query(
        "INSERT INTO events (session_id, kind, payload_json, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(session_id)
    .bind(kind)
    .bind(&payload_json)
    .bind(&ts)
    .execute(pool)
    .await?;
    Ok(r.last_insert_rowid())
}

/// 按 id 增量拉取事件（SSE 断线回放，坑位 D3）
pub async fn list_events_after(
    pool: &SqlitePool,
    session_id: &str,
    after_id: i64,
) -> AppResult<Vec<EventRow>> {
    Ok(sqlx::query_as::<_, EventRow>(
        "SELECT id, session_id, kind, payload_json, created_at FROM events
         WHERE session_id = ?1 AND id > ?2 ORDER BY id ASC",
    )
    .bind(session_id)
    .bind(after_id)
    .fetch_all(pool)
    .await?)
}

/// 最近 limit 条事件（面板首屏）。
pub async fn list_events_tail(
    pool: &SqlitePool,
    session_id: &str,
    limit: i64,
) -> AppResult<Vec<EventRow>> {
    let mut rows = sqlx::query_as::<_, EventRow>(
        "SELECT id, session_id, kind, payload_json, created_at FROM events
         WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    Ok(rows)
}

/// 更早的一批事件（面板"加载更早"；`before_id` 传已加载的最早一条的 id）。
pub async fn list_events_before(
    pool: &SqlitePool,
    session_id: &str,
    before_id: i64,
    limit: i64,
) -> AppResult<Vec<EventRow>> {
    let mut rows = sqlx::query_as::<_, EventRow>(
        "SELECT id, session_id, kind, payload_json, created_at FROM events
         WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
    )
    .bind(session_id)
    .bind(before_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    Ok(rows)
}

/// 事件封顶裁剪（0029 事件生命周期）：会话事件超 cap 时删最旧的，只留最新 cap 条。
pub async fn trim_events_to_cap(pool: &SqlitePool, session_id: &str, cap: i64) -> AppResult<u64> {
    let cnt: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;
    if cnt.0 <= cap {
        return Ok(0);
    }
    let floor: (i64,) = sqlx::query_as(
        "SELECT id FROM events WHERE session_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2",
    )
    .bind(session_id)
    .bind(cap)
    .fetch_one(pool)
    .await?;
    let r = sqlx::query("DELETE FROM events WHERE session_id = ?1 AND id < ?2")
        .bind(session_id)
        .bind(floor.0)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

/// 按 seq 区间取某会话的 reasoning 事件（思考全文懒加载）。
pub async fn list_reasoning_between(
    pool: &SqlitePool,
    session_id: &str,
    from_id: i64,
    to_id: i64,
) -> AppResult<Vec<EventRow>> {
    Ok(sqlx::query_as::<_, EventRow>(
        "SELECT id, session_id, kind, payload_json, created_at FROM events
         WHERE session_id = ?1 AND kind = 'reasoning' AND id BETWEEN ?2 AND ?3 ORDER BY id ASC",
    )
    .bind(session_id)
    .bind(from_id)
    .bind(to_id)
    .fetch_all(pool)
    .await?)
}

/// 清理 confirm 瞬时事件（回放会把旧确认请求重新推前端 → 遮罩常驻；启动时清一次兜底）
pub async fn purge_confirm_events(pool: &SqlitePool) -> AppResult<u64> {    let r = sqlx::query("DELETE FROM events WHERE kind IN ('confirm.request', 'confirm.cancelled')")
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

// Session 更新（改名 / 系统提示词）

/// 更新会话 title / system_prompt（仅更新提供字段），返回更新后的会话
pub async fn update_session(
    pool: &SqlitePool,
    id: &str,
    title: Option<&str>,
    system_prompt: Option<&str>,
) -> AppResult<Option<SessionRow>> {
    let ts = now();
    if let Some(t) = title {
        sqlx::query("UPDATE sessions SET title = ?2, updated_at = ?3 WHERE id = ?1")
            .bind(id)
            .bind(t)
            .bind(&ts)
            .execute(pool)
            .await?;
    }
    if let Some(sp) = system_prompt {
        sqlx::query("UPDATE sessions SET system_prompt = ?2, updated_at = ?3 WHERE id = ?1")
            .bind(id)
            .bind(sp)
            .bind(&ts)
            .execute(pool)
            .await?;
    }
    get_session(pool, id).await
}

// Settings（运行时设置，环境变量为默认，DB 为运行期覆盖）

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> AppResult<()> {
    let ts = now();
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
    )
    .bind(key)
    .bind(value)
    .bind(&ts)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_setting(pool: &SqlitePool, key: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM settings WHERE key = ?1")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// 读单条设置（不存在 → None）——provider 凭据档案（{provider}_api_key 等）按 key 精确取
pub async fn get_setting(pool: &SqlitePool, key: &str) -> AppResult<Option<String>> {
    Ok(sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?
        .map(|r| r.0))
}

pub async fn list_settings(pool: &SqlitePool) -> AppResult<Vec<(String, String)>> {
    Ok(
        sqlx::query_as::<_, (String, String)>("SELECT key, value FROM settings")
            .fetch_all(pool)
            .await?,
    )
}

// 多轮任务记忆（跨会话：路径锚定 + 结论 + 修改记录）

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct MemoryRow {
    pub id: i64,
    pub workspace: String,
    pub session_id: String,
    pub key: String,
    pub value: String,
    pub record_type: String,
    pub priority: i64,
    pub created_at: String,
    pub scope: String,
}

/// 由 record_type 推导隔离维度（规范 v1.0 §2.2）
pub fn scope_for(record_type: &str) -> &'static str {
    match record_type {
        "conclusion" | "modification" | "files_read"
        // 主题加权/偏好/凭证 = workspace 级跨会话共享
        | "theme" | "preference" | "credential" => "workspace",
        "user_memory" => "user",
        _ => "session",
    }
}

/// 记忆作用域开关（`settings` 表键 `memory_scope`）—— **长期记忆是否跨会话共享**。
pub async fn memory_scope_shared(pool: &SqlitePool) -> bool {
    match get_setting(pool, "memory_scope").await {
        Ok(Some(v)) => !v.trim().eq_ignore_ascii_case("session"),
        _ => true,
    }
}

/// 写入一条记忆（workspace 为项目路径锚定；record_type: path_anchor|conclusion|modification|error|tool_result|note）
pub async fn remember(
    pool: &SqlitePool,
    workspace: &str,
    session_id: &str,
    key: &str,
    value: &str,
    record_type: &str,
    priority: i64,
) -> AppResult<()> {
    let ts = now();
    // **写侧作用域开关**：本该跨会话的（`scope_for` 判为 `"workspace"`）在关掉共享时
    let mut scope = scope_for(record_type);
    if scope == "workspace" && !memory_scope_shared(pool).await {
        scope = "session";
    }
    // （声明与执行不对称 P0-1.5b）：A4 覆盖语义兑现——注释声称"同 key 记忆
    sqlx::query(
        "INSERT INTO memories (workspace, session_id, key, value, record_type, priority, scope, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(workspace, key) DO UPDATE SET
            value = excluded.value,
            record_type = excluded.record_type,
            priority = excluded.priority,
            scope = excluded.scope,
            created_at = excluded.created_at"
    )
        .bind(workspace).bind(session_id).bind(key).bind(value).bind(record_type).bind(priority).bind(scope).bind(&ts)
        .execute(pool).await?;
    Ok(())
}

/// 按 workspace + key 精确取一条记忆（readmemo 读缓存回填用；键即寻址，不用扫表）
pub async fn recall_memory_by_key(
    pool: &SqlitePool,
    workspace: &str,
    key: &str,
) -> AppResult<Option<MemoryRow>> {
    Ok(sqlx::query_as::<_, MemoryRow>(
        "SELECT * FROM memories WHERE workspace = ?1 AND key = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(workspace)
    .bind(key)
    .fetch_optional(pool)
    .await?)
}

/// 按 workspace + scope 检索（跨会话注入用：只取 workspace 级共享记忆）
pub async fn recall_workspace_scope(
    pool: &SqlitePool,
    workspace: &str,
    scope: &str,
    limit: i64,
) -> AppResult<Vec<MemoryRow>> {
    Ok(sqlx::query_as::<_, MemoryRow>(
        "SELECT * FROM memories WHERE workspace = ?1 AND scope = ?2 ORDER BY priority DESC, created_at DESC, id DESC LIMIT ?3"
    )
        .bind(workspace).bind(scope).bind(limit)
        .fetch_all(pool).await?)
}

/// 按会话检索记忆（任务重跑时找回本轮上下文）
pub async fn recall_session(
    pool: &SqlitePool,
    session_id: &str,
    limit: i64,
) -> AppResult<Vec<MemoryRow>> {
    Ok(sqlx::query_as::<_, MemoryRow>(
        "SELECT * FROM memories WHERE session_id = ?1 ORDER BY priority DESC, created_at DESC, id DESC LIMIT ?2"
    )
        .bind(session_id).bind(limit)
        .fetch_all(pool).await?)
}

/// 按 workspace + record_type 检索（滚动压缩用：取某类记忆全量）
pub async fn recall_by_type_workspace(
    pool: &SqlitePool,
    workspace: &str,
    record_type: &str,
    limit: i64,
) -> AppResult<Vec<MemoryRow>> {
    Ok(sqlx::query_as::<_, MemoryRow>(
        "SELECT * FROM memories WHERE workspace = ?1 AND record_type = ?2 ORDER BY created_at DESC, id DESC LIMIT ?3"
    )
        .bind(workspace).bind(record_type).bind(limit)
        .fetch_all(pool).await?)
}

/// 删除指定 id 的记忆（滚动压缩：旧 conversation 压缩后移除）
pub async fn forget_memory_by_id(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM memories WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除同 (workspace, key) 的记忆（偏好/凭证同类别覆盖——
pub async fn forget_memory_by_key(pool: &SqlitePool, workspace: &str, key: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM memories WHERE workspace = ?1 AND key = ?2")
        .bind(workspace)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除某会话的全部记忆（审计 A3：删会话时级联清理，防孤儿记忆污染锚定兜底）
pub async fn forget_memory_by_session(pool: &SqlitePool, session_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM memories WHERE session_id = ?1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除某 workspace 下同 key 的记忆（审计 A4：user_fact 同工具覆盖——最新陈述胜出）

// 多轮锚定（checkpoint 恢复 + 已读清单回取）

/// 持久化 session 的 workspace（任务容器级锚定）
pub async fn set_session_workspace(
    pool: &SqlitePool,
    session_id: &str,
    workspace: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE sessions SET workspace = ?2, updated_at = ?3 WHERE id = ?1")
        .bind(session_id)
        .bind(workspace)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

/// 全部会话的历史工作区（distinct、非空）——项目名录冷启动补登用
pub async fn list_session_workspaces(pool: &SqlitePool) -> AppResult<Vec<String>> {
    Ok(sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT workspace FROM sessions \
         WHERE workspace IS NOT NULL AND TRIM(workspace) != ''",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect())
}

/// 读取 session 持久化的 workspace（每轮入口恢复链第一层）
pub async fn get_session_workspace(
    pool: &SqlitePool,
    session_id: &str,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT workspace FROM sessions WHERE id = ?1")
        .bind(session_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0).filter(|w| !w.is_empty()))
}

/// 会话创建时间（RFC3339）——**任务目录名由它派生**（`journal::task_key_from_created_at`）。
pub async fn get_session_created_at(pool: &SqlitePool, session_id: &str) -> AppResult<String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT created_at FROM sessions WHERE id = ?1")
        .bind(session_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0).unwrap_or_default())
}

/// 最近活跃会话的 workspace（记忆管理面板兜底：前端不感知项目路径）
pub async fn get_recent_workspace(pool: &SqlitePool) -> AppResult<String> {    let row: Option<(String,)> = sqlx::query_as(
        "SELECT workspace FROM sessions WHERE workspace IS NOT NULL AND workspace != '' ORDER BY updated_at DESC, id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0).unwrap_or_default())
}

/// 保存中断断点（INSERT OR REPLACE：同一 session 只保留最新）
pub async fn checkpoint_save(
    pool: &SqlitePool,
    session_id: &str,
    round: i64,
    summary: &str,
    plan_exists: bool,
    files_read: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO checkpoints (session_id, round, summary, plan_exists, files_read, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(session_id) DO UPDATE SET
            round = excluded.round,
            summary = excluded.summary,
            plan_exists = excluded.plan_exists,
            files_read = excluded.files_read,
            created_at = excluded.created_at",
    )
    .bind(session_id)
    .bind(round)
    .bind(summary)
    .bind(plan_exists as i32)
    .bind(files_read)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

// Tool Guard Events（工具护栏事件审计 —— 后端兜底达成度机读）

/// 工具定义类问题（后端未兜住、需模型自纠）的错误码集合。

pub async fn recent_tool_args(
    pool: &SqlitePool,
    session_id: &str,
    limit: i64,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT payload_json FROM events WHERE session_id = ?1 AND kind = 'tool' \
         ORDER BY id DESC LIMIT ?2",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut out: Vec<String> = Vec::new();
    for (pj,) in rows {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&pj) {
            if let Some(args) = v
                .get("tools")
                .and_then(|t| t.as_array())
                .and_then(|a| a.first())
                .and_then(|t| t.get("args"))
            {
                if !args.is_null() {
                    out.push(args.to_string());
                }
            }
        }
    }
    Ok(out)
}

/// 召回会话内已成功 read 的文件路径（多轮锚定：模型"继续"时知道读过哪些）
pub async fn recall_read_paths(
    pool: &SqlitePool,
    session_id: &str,
    limit: i64,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT name, arguments FROM tool_calls
         WHERE session_id = ?1 AND name = 'read' AND status = 'success'
         ORDER BY created_at DESC, id DESC LIMIT ?2",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut out: Vec<String> = Vec::new();
    for (_name, args) in rows {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&args) {
            if let Some(paths) = v.get("paths").and_then(|p| p.as_array()) {
                for p in paths.iter().filter_map(|x| x.as_str()) {
                    if !out.contains(&p.to_string()) {
                        out.push(p.to_string());
                    }
                }
            }
            if let Some(f) = v.get("path").and_then(|x| x.as_str()) {
                if !out.contains(&f.to_string()) {
                    out.push(f.to_string());
                }
            }
        }
    }
    Ok(out)
}

// ── 断点续传：取消时汇总真实进度（工具调用 / 计划步数 / 已确认结论）──

/// 判定工具输出是否为"低信息量结论"（不应注入断点）
fn is_low_value_finding(t: &str) -> bool {
    let s = t.trim();
    if s.chars().count() < 4 {
        return true;
    }
    let stripped: String = s
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '"' && *c != '\'')
        .collect();
    if !stripped.is_empty()
        && stripped
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_punctuation())
    {
        return true;
    }
    let status_words = [
        "done", "ok", "ok.", "yes", "no", "success", "succeeded", "finished", "complete",
        "completed", "all done", "done.", "finished.", "true", "false", "null", "none",
    ];
    if status_words.iter().any(|w| s.to_lowercase() == *w) {
        return true;
    }
    let lower = s.to_lowercase();
    let fail_kws = [
        "timeout", "timed out", "connection refused", "could not resolve",
        "exit code", "error:", "failed", "no such host", "http_code",
        "operation timed out", "connection reset", "closed",
    ];
    if fail_kws.iter().any(|k| lower.contains(k)) && s.chars().count() < 40 {
        return true;
    }
    let sig: String = s
        .chars()
        .filter(|c| {
            !c.is_ascii_whitespace()
                && !matches!(c, '"' | '\'' | '{' | '}' | '(' | ')' | '[' | ']' | '`' | '-' | '_' | '|')
        })
        .collect();
    if sig.is_empty() {
        return true;
    }
    false
}

/// 从工具结果信信封（JSON）提取一条结论性文本
fn extract_finding_text(name: &str, result_json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(result_json).ok()?;
    let raw = v
        .get("render_full")
        .and_then(|t| t.as_str())
        .map(String::from)
        .or_else(|| {
            v.get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| {
                    let texts: Vec<String> = arr
                        .iter()
                        .filter_map(|it| it.get("text").and_then(|t| t.as_str()).map(String::from))
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(texts.join("\n"))
                    }
                })
        })?;
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if is_low_value_finding(t) {
        return None;
    }
    let t: String = t.chars().take(140).collect();
    Some(if t.chars().count() > 120 {
        format!("{name}: {t}…")
    } else {
        format!("{name}: {t}")
    })
}

/// 提取最近关键工具的结果要点（read/search/run/find_files/list/edit/write 等），
pub async fn recent_tool_findings(
    pool: &SqlitePool,
    session_id: &str,
    limit: i64,
) -> Vec<String> {
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT name, result_json FROM tool_calls
         WHERE session_id = ?1 AND status = 'success'
           AND name IN ('read', 'search', 'run', 'find_files', 'list', 'edit', 'write')
         ORDER BY id DESC LIMIT ?2",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for (name, result) in rows.into_iter().rev() {
        if let Some(rj) = result {
            if let Some(t) = extract_finding_text(&name, &rj) {
                out.push(t);
            }
        }
    }
    out
}

/// 汇总当前进度为断点摘要：已完成工具调用数 + 计划步数 + 最近关键结论。
pub async fn build_checkpoint_summary(
    pool: &SqlitePool,
    session_id: &str,
) -> (i64, String) {
    let tools: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM tool_calls WHERE session_id = ?1 ORDER BY id")
            .bind(session_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut recent: Vec<String> = Vec::new();
    for (name,) in tools.iter().rev() {
        if seen.insert(name.clone()) {
            recent.push(name.clone());
        }
        if recent.len() >= 8 {
            break;
        }
    }
    recent.reverse();
    let steps: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans WHERE session_id = ?1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let round = tools.len() as i64;
    let findings = recent_tool_findings(pool, session_id, 4).await;
    let mut summary = if tools.is_empty() {
        format!("任务中断于开始阶段（计划 {steps} 步，尚无工具执行）。请按计划从头执行。")
    } else {
        format!(
            "任务中断于执行中：已完成 {round} 次工具调用（{}），计划共 {steps} 步。",
            recent.join(", ")
        )
    };
    if !findings.is_empty() {
        summary.push_str(" 已确认的关键结论：");
        summary.push_str(&findings.join("；"));
        summary.push('。');
    }
    summary.push_str("（断点：以上为本会话已确认的进度）");
    (round, summary)
}

// Interjections（0026 任务中插话：忙时不拒收，队列持久化，轮边界原子消费）

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct InterjectionRow {
    pub id: i64,
    pub session_id: String,
    pub text: String,
    pub mode: String,
    pub status: String,
    pub created_at: String,
}

/// 入队：任何会话状态都接受（任务运行中 = 排队等轮边界；空闲 = 存着下次消费）
pub async fn consumed_interjections_without_message(
    pool: &SqlitePool,
    session_id: &str,
) -> AppResult<Vec<(i64, String)>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, text FROM interjections          WHERE session_id = ?1 AND status = 'consumed' AND message_id IS NULL ORDER BY id",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_message_item_json(
    pool: &SqlitePool,
    message_id: &str,
    item_json: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE messages SET item_json = ?2 WHERE id = ?1")
        .bind(message_id)
        .bind(item_json)
        .execute(pool)
        .await?;
    Ok(())
}

/// 只替换 `item_json.content`，**保留其余键**（如 `attachments`）。
pub async fn update_message_item_content(
    pool: &SqlitePool,
    message_id: &str,
    content_json: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE messages SET item_json = json_set(item_json, '$.content', json(?2)) WHERE id = ?1",
    )
    .bind(message_id)
    .bind(content_json)
    .execute(pool)
    .await?;
    Ok(())
}

/// 回填插话对应的 message 行 id
pub async fn attach_interjection_message(
    pool: &SqlitePool,
    interjection_id: i64,
    message_id: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE interjections SET message_id = ?2 WHERE id = ?1")
        .bind(interjection_id)
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_interjection(
    pool: &SqlitePool,
    session_id: &str,
    text: &str,
    mode: &str,
    message_id: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO interjections (session_id, text, mode, status, created_at, message_id) \
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5)",
    )
    .bind(session_id)
    .bind(text)
    .bind(mode)
    .bind(now())
    .bind(message_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 停止回收（cancel_session 调用）：把该会话全部 **pending**（未被消费）插话标记
pub async fn cancel_pending_interjections(
    pool: &SqlitePool,
    session_id: &str,
) -> AppResult<u64> {
    // ① 取 pending 行的 message_id
    let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
        "SELECT id, message_id FROM interjections \
         WHERE session_id = ?1 AND status = 'pending'",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    // ② 标记 cancelled（保留记录作审计）
    let mut n = 0u64;
    for (id, message_id) in &rows {
        sqlx::query("UPDATE interjections SET status='cancelled', consumed_at=?2 WHERE id=?1")
            .bind(id)
            .bind(now())
            .execute(pool)
            .await?;
        // ③ 对应 user 消息作废（item_json 打 cancelled 标记，历史重建跳过）
        if let Some(mid) = message_id {
            let msg: Option<(Option<String>,)> =
                sqlx::query_as("SELECT item_json FROM messages WHERE id = ?1")
                    .bind(mid)
                    .fetch_optional(pool)
                    .await?;
            if let Some((Some(raw),)) = msg {
                if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    v["cancelled"] = serde_json::Value::Bool(true);
                    let _ = sqlx::query("UPDATE messages SET item_json=?2 WHERE id=?1")
                        .bind(mid)
                        .bind(v.to_string())
                        .execute(pool)
                        .await;
                }
            }
        }
        n += 1;
    }
    Ok(n)
}

/// 原子取走全部 pending 插话并标记 consumed（多端并发插话按序消费，不重复注入）
pub async fn drain_interjections(
    pool: &SqlitePool,
    session_id: &str,
    consumed_round: i64,
) -> AppResult<Vec<InterjectionRow>> {
    let rows: Vec<InterjectionRow> = sqlx::query_as(
        "SELECT id, session_id, text, mode, status, created_at FROM interjections \
         WHERE session_id = ?1 AND status = 'pending' ORDER BY id",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(rows);
    }
    for r in &rows {
        sqlx::query("UPDATE interjections SET status='consumed', consumed_at=?2, consumed_round=?3 WHERE id=?1")
            .bind(r.id)
            .bind(now())
            .bind(consumed_round)
            .execute(pool)
            .await?;
    }
    Ok(rows)
}

/// 待注入条数（前端徽标用）
pub async fn count_pending_interjections(pool: &SqlitePool, session_id: &str) -> AppResult<i64> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM interjections WHERE session_id = ?1 AND status = 'pending'",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// 列插话（status 过滤可选：pending/consumed/全部）
pub async fn list_interjections(
    pool: &SqlitePool,
    session_id: &str,
    status: Option<String>,
) -> AppResult<Vec<InterjectionRow>> {
    match status.as_deref() {
        Some(s) if !s.is_empty() => Ok(sqlx::query_as(
            "SELECT id, session_id, text, mode, status, created_at FROM interjections \
             WHERE session_id = ?1 AND status = ?2 ORDER BY id DESC LIMIT 50",
        )
        .bind(session_id)
        .bind(s)
        .fetch_all(pool)
        .await?),
        _ => Ok(sqlx::query_as(
            "SELECT id, session_id, text, mode, status, created_at FROM interjections \
             WHERE session_id = ?1 ORDER BY id DESC LIMIT 50",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?),
    }
}

#[cfg(test)]
mod repos_tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// item_shape 是 role 的唯一事实源，七种载体必须全判对。
    #[test]
    fn item_shape_判全七种载体() {
        let cases: &[(&str, Option<&str>, bool)] = &[
            (r#"{"type":"function_call_output","call_id":"c","output":"x"}"#, Some("tool"), true),
            (r#"{"type":"function_call","call_id":"c","name":"run","arguments":"{}"}"#, Some("assistant"), true),
            (r#"{"type":"reasoning","content":[]}"#, Some("reasoning"), false),
            (r#"{"type":"message","role":"assistant","tool_calls":[{"call_id":"c"}]}"#, Some("assistant"), true),
            (r#"{"type":"message","role":"user","content":[]}"#, Some("user"), false),
            (r#"{"call_id":"c","output":"x"}"#, Some("tool"), true),
            (r#"{"call_id":"c","name":"run","arguments":"{}"}"#, Some("assistant"), true),
        ];
        for (js, role, has) in cases {
            let (r, h) = item_shape(js);
            assert_eq!(r.as_deref(), *role, "role 判错: {js}");
            assert_eq!(h, *has, "has_call_id 判错: {js}");
        }
    }

    #[tokio::test]
    async fn 落库_role以itemjson为准_且带callid条目幂等() {
        let pool = test_pool().await;
        let s = create_session(&pool, "t", "").await.unwrap();
        let sid = s.id.clone();

        let decl = r#"{"type":"message","role":"assistant","content":[],"tool_calls":[{"call_id":"c1","name":"run","arguments":"{}","type":"function_call"}]}"#;
        // ① 错位归正：调用点说 reasoning，item_json 是 assistant 声明
        let m1 = insert_message(&pool, &sid, "reasoning", "x", Some(decl)).await.unwrap();
        assert_eq!(m1.role, "assistant", "role 必须按 item_json 归正");
        // ② 幂等：同一条声明换个 role 再落一次，不得新增
        let m2 = insert_message(&pool, &sid, "tool", "x", Some(decl)).await.unwrap();
        assert_eq!(m2.id, m1.id, "带 call_id 的同一条声明不得重复落库");

        let out = r#"{"type":"function_call_output","call_id":"c1","output":"x"}"#;
        let o1 = insert_message(&pool, &sid, "assistant", "x", Some(out)).await.unwrap();
        assert_eq!(o1.role, "tool", "回执的 role 必须归正为 tool");
        let o2 = insert_message(&pool, &sid, "tool", "x", Some(out)).await.unwrap();
        assert_eq!(o2.id, o1.id, "同一条回执不得重复落库");

        // ③ 用户连发同一句 = 合法重复，必须保留两行
        let u = r#"{"type":"message","role":"user","content":[{"text":"继续","type":"input_text"}]}"#;
        let u1 = insert_message(&pool, &sid, "user", "继续", Some(u)).await.unwrap();
        let u2 = insert_message(&pool, &sid, "user", "继续", Some(u)).await.unwrap();
        assert_ne!(u1.id, u2.id, "用户的合法重复不得被吞掉");

        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = ?1")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 4, "声明 1 + 回执 1 + 用户 2 = 4");
    }

    /// 三种载体的正文都能从 item_json 里取出来（tool / assistant / reasoning）。
    #[test]
    fn body_inside_item_认三种载体() {
        let out = r#"{"type":"function_call_output","call_id":"c","output":"命令输出全文"}"#;
        assert_eq!(body_inside_item(out).as_deref(), Some("命令输出全文"));

        let msg = r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"前"},{"type":"output_text","text":"后"}]}"#;
        assert_eq!(body_inside_item(msg).as_deref(), Some("前后"), "多段应拼接");

        let rs = r#"{"type":"reasoning","summary":[{"type":"summary_text","text":"思考正文"}]}"#;
        assert_eq!(body_inside_item(rs).as_deref(), Some("思考正文"));

        // 拿不准的一律 None（宁可不省，也不能让读出来的 content 变空）
        assert_eq!(body_inside_item(r#"{"type":"function_call","call_id":"c","arguments":"{}"}"#), None);
        assert_eq!(body_inside_item(r#"{"type":"message","role":"user","content":[]}"#), None);
        assert_eq!(body_inside_item("not json"), None);
        assert_eq!(body_inside_item(r#"{"type":"function_call_output","call_id":"c"}"#), None);
    }

    /// 逐字节相同 ⇒ 库里 content 存空串；读出来仍要还原成完整正文。
    #[tokio::test]
    async fn 逐字节相同的内容只存一份_读出来完整() {
        let pool = test_pool().await;
        let s = create_session(&pool, "t", "").await.unwrap();
        let sid = s.id.clone();
        let body = "命令输出".repeat(500);
        let ij = format!(r#"{{"type":"function_call_output","call_id":"c1","output":"{body}"}}"#);

        let m = insert_message(&pool, &sid, "tool", &body, Some(&ij))
            .await
            .unwrap();
        // ① 返回给调用方的仍是完整正文（去重是存储细节，不外泄）
        assert_eq!(m.content, body, "insert 返回值必须是完整正文");

        // ② 库里那一列确实是空的（省下了）
        let stored: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = ?1")
            .bind(&m.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(stored.is_empty(), "重复内容应存空串，实得 {} 字节", stored.len());

        // ③ 读出来完整还原 —— 所有读方无感
        let read = list_messages(&pool, &sid).await.unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].content, body, "读出来必须与写入时逐字相同");
    }

    /// content 与 item_json 正文**不同**时（例如前端要的回显文本 ≠ 模型看到的），
    #[tokio::test]
    async fn 内容不同时两个都保留() {
        let pool = test_pool().await;
        let s = create_session(&pool, "t", "").await.unwrap();
        let sid = s.id.clone();
        let ij = r#"{"type":"function_call_output","call_id":"c1","output":"模型看到的那份"}"#;
        let shown = "前端要显示的那份（不同）";
        let m = insert_message(&pool, &sid, "tool", shown, Some(ij)).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = ?1")
            .bind(&m.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, shown, "两份不同就不能省，必须原样留 content");
        let read = list_messages(&pool, &sid).await.unwrap();
        assert_eq!(read[0].content, shown, "读出来还是前端那份，不得被 item_json 覆盖");
    }

    /// 幂等分支返回的既有行也要给完整 content（不能把库里那个空串漏出去）。
    #[tokio::test]
    async fn 幂等命中时返回的也是完整正文() {
        let pool = test_pool().await;
        let s = create_session(&pool, "t", "").await.unwrap();
        let sid = s.id.clone();
        let body = "回执正文".repeat(300);
        let ij = format!(r#"{{"type":"function_call_output","call_id":"c9","output":"{body}"}}"#);
        let first = insert_message(&pool, &sid, "tool", &body, Some(&ij)).await.unwrap();
        // 第二次落入幂等分支，返回的是**库里的行**（content 为空串）
        let again = insert_message(&pool, &sid, "tool", &body, Some(&ij)).await.unwrap();
        assert_eq!(again.id, first.id, "应命中幂等");
        assert_eq!(again.content, body, "幂等返回的也必须还原成完整正文");
    }
}
