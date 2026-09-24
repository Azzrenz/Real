//! 记忆管理 API（主题加权记忆）

use crate::agent::memory::journal;
use crate::db::repos;
use crate::error::AppResult;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

// 用户核心画像的 settings 键（snake_case，见 docs/20260915-数据落盘与命名规范.md §四）
const PERSONA_KEY: &str = "memory_persona";

#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    #[serde(default)]
    pub workspace: String,
}

#[derive(Debug, Deserialize)]
pub struct PrefBody {
    #[serde(default)]
    pub workspace: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct PrefDeleteQuery {
    #[serde(default)]
    pub workspace: String,
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct PinBody {
    #[serde(default)]
    pub workspace: String,
    pub id: i64,
}

/// workspace 兜底：前端不感知项目路径——为空时自动用最近活跃会话的 workspace
async fn resolve_workspace(pool: &sqlx::SqlitePool, q: &str) -> String {
    if !q.is_empty() {
        return q.to_string();
    }
    repos::get_recent_workspace(pool).await.unwrap_or_default()
}

fn parse_local_date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
}

// ---- 用户核心画像（L3 永恒记忆第一来源）----

pub async fn persona_get(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let raw = repos::get_setting(&state.pool, PERSONA_KEY).await?;
    let persona: Value = raw
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_else(|| json!({ "identity": "", "needs": "", "principles": "" }));
    Ok(Json(persona))
}

pub async fn persona_put(State(state): State<AppState>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let v = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    repos::set_setting(&state.pool, PERSONA_KEY, &v).await?;
    Ok(Json(body))
}

// ---- 偏好记忆 ----

pub async fn prefs_get(State(state): State<AppState>, Query(q): Query<WorkspaceQuery>) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &q.workspace).await;
    let prefs = repos::recall_by_type_workspace(&state.pool, &ws, "preference", 50).await?;
    Ok(Json(json!(prefs
        .iter()
        .map(|r| json!({
            "key": r.key,
            "value": r.value,
            "priority": r.priority,
            "created_at": r.created_at,
        }))
        .collect::<Vec<_>>())))
}

pub async fn prefs_post(State(state): State<AppState>, Json(body): Json<PrefBody>) -> AppResult<Json<Value>> {
    if body.value.trim().is_empty() {
        return Ok(Json(json!({"ok": false, "error": "value 不能为空"})));
    }
    let ws = resolve_workspace(&state.pool, &body.workspace).await;
    if ws.is_empty() {
        return Ok(Json(json!({"ok": false, "error": "未找到工作区（请先打开一个会话）"})));
    }
    let key = format!("偏好:{}", crate::agent::plan::truncate(&body.value, 30));
    repos::remember(&state.pool, &ws, "", &key, &body.value, "preference", 90).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn prefs_delete(State(state): State<AppState>, Query(q): Query<PrefDeleteQuery>) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &q.workspace).await;
    repos::forget_memory_by_key(&state.pool, &ws, &q.key).await?;
    Ok(Json(json!({"ok": true})))
}

// ---- 主题加权记忆 ----

pub async fn themes_get(State(state): State<AppState>, Query(q): Query<WorkspaceQuery>) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &q.workspace).await;
    let mut themes = repos::recall_by_type_workspace(&state.pool, &ws, "theme", 200).await?;
    themes.sort_by(|a, b| b.priority.cmp(&a.priority).then(b.created_at.cmp(&a.created_at)));
    Ok(Json(json!(themes
        .iter()
        .map(|r| json!({
            "id": r.id,
            "value": r.value,
            "priority": r.priority,
            "permanent": r.priority >= 90,
            "created_at": r.created_at,
            "session_id": r.session_id,
        }))
        .collect::<Vec<_>>())))
}

/// 手动标永久（priority → 95，与语义级永久重要同级，永不淘汰）
pub async fn theme_pin(State(state): State<AppState>, Json(body): Json<PinBody>) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &body.workspace).await;
    let themes = repos::recall_by_type_workspace(&state.pool, &ws, "theme", 200).await?;
    if let Some(t) = themes.iter().find(|t| t.id == body.id) {
        repos::remember(&state.pool, &ws, &t.session_id, &t.key, &t.value, "theme", 95).await?;
        Ok(Json(json!({"ok": true, "priority": 95})))
    } else {
        Ok(Json(json!({"ok": false, "error": "主题不存在"})))
    }
}

pub async fn theme_delete(State(state): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    repos::forget_memory_by_id(&state.pool, id).await?;
    Ok(Json(json!({"ok": true})))
}

// ---- 对话摘要 / 健康度 ----

pub async fn digests_get(State(state): State<AppState>, Query(q): Query<WorkspaceQuery>) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &q.workspace).await;
    let mut digests = repos::recall_by_type_workspace(&state.pool, &ws, "digest", 50).await?;
    digests.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(json!(digests
        .iter()
        .map(|r| json!({
            "id": r.id,
            "value": r.value,
            "created_at": r.created_at,
            "session_id": r.session_id,
        }))
        .collect::<Vec<_>>())))
}

pub async fn stats_get(State(state): State<AppState>, Query(q): Query<WorkspaceQuery>) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &q.workspace).await;
    let themes = repos::recall_by_type_workspace(&state.pool, &ws, "theme", 200).await?;
    let prefs = repos::recall_by_type_workspace(&state.pool, &ws, "preference", 50).await?;
    let permanent = themes.iter().filter(|t| t.priority >= 90).count();
    // 新鲜度分档（本地时区=UTC+8）：今天聊的 / 7 天内聊的 / 更早
    let today = chrono::Local::now().date_naive();
    let week_ago = today - chrono::Duration::days(7);
    let fresh_today = themes
        .iter()
        .filter(|t| parse_local_date(&t.created_at) == Some(today))
        .count();
    let fresh_week = themes
        .iter()
        .filter(|t| {
            let d = parse_local_date(&t.created_at);
            d.is_some() && d != Some(today) && d.unwrap() >= week_ago
        })
        .count();
    let old = themes.len().saturating_sub(fresh_today + fresh_week);
    Ok(Json(json!({
        "themes_total": themes.len(),
        "themes_permanent": permanent,
        "themes_regular": themes.len().saturating_sub(permanent),
        "prefs_count": prefs.len(),
        "freshness": { "today": fresh_today, "week": fresh_week, "old": old },
    })))
}

/// 记忆文件列表：该项目下的长期知识与任务记忆（`.md`），按修改时间倒序。
/// 日志页用。目录口径取自 `agent::memory::journal`，不另算一遍。
pub async fn files_get(
    State(state): State<AppState>,
    Query(q): Query<WorkspaceQuery>,
) -> AppResult<Json<Value>> {
    let ws = resolve_workspace(&state.pool, &q.workspace).await;
    let base = journal::project_dir(&ws);
    let mut out: Vec<Value> = Vec::new();
    // 长期知识（longterm/*.md）
    collect_md(&journal::longterm_dir(&ws), &base, &mut out);
    // 任务记忆（tasks/<task_key>/*.md）
    if let Ok(rd) = std::fs::read_dir(base.join("tasks")) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                collect_md(&e.path(), &base, &mut out);
            }
        }
    }
    out.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    out.truncate(200);
    Ok(Json(json!(out)))
}

/// 收集目录下的 `.md`（不递归）；`name` 用相对 `base` 的路径、统一正斜杠。
fn collect_md(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let is_md = p
            .extension()
            .map(|x| x.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !is_md {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        let name = p
            .strip_prefix(base)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(json!({
            "name": name,
            "path": p.to_string_lossy(),
            "size": m.len(),
            "modified": m.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }));
    }
}
