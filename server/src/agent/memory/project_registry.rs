//! 项目名录：口语项目名（"Real" / 中文别名）→ 项目根 的映射。

use crate::db::repos;
use crate::error::AppResult;
use sqlx::SqlitePool;

/// settings 键名（与前端/手工编辑共用）
pub const SETTINGS_KEY: &str = "project_registry";
/// 名录上限（防无限增长；超出按登记顺序截断）
const MAX_ENTRIES: usize = 64;
/// 别名长度下限（1 个字符太泛，易误命中）
const MIN_ALIAS_LEN: usize = 2;
/// 通用目录名黑名单：这些"项目名"太泛，命中会误切项目
const GENERIC: &[&str] = &[
    "desktop",
    "downloads",
    "documents",
    "docs",
    "attachments",
    "tasks",
    "tmp",
    "temp",
    "windows",
    "users",
    "src",
    "target",
    "node_modules",
    "dist",
    "build",
    "projects",
    "real-agent",
    "appdata",
    "data",
    "backup",
    "backups",
];

/// 项目名录条目
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProjectEntry {
    /// 项目根（绝对路径，正斜杠或反斜杠均可，比较时归一）
    pub path: String,
    /// 口语别名（目录名及其紧凑形式，自动积累）
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 手工/旧格式单别名（中文口语名走这里，如 "视频工厂"）——匹配时与 aliases 合并
    #[serde(default)]
    pub alias: Option<String>,
}

/// 条目全部别名：新格式 `aliases` 合并旧格式 `alias`
pub fn effective_aliases(e: &ProjectEntry) -> Vec<String> {
    let mut out = e.aliases.clone();
    if let Some(a) = e.alias.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            out.push(a.to_string());
        }
    }
    out
}

/// 目录名归一：取末段、小写、去首尾空白；**盘符根（`D:` / `D:\`）返回空**——
pub fn dir_name(path: &str) -> String {
    let s = path.trim().trim_end_matches(['\\', '/']);
    if s.len() == 2 && s.as_bytes().get(1) == Some(&b':') {
        return String::new();
    }
    s.rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
}

/// 由路径派生别名集合：目录名小写 + 去分隔符的紧凑形式
pub fn aliases_for(path: &str) -> Vec<String> {
    let base = dir_name(path);
    let mut out: Vec<String> = Vec::new();
    if base.len() >= MIN_ALIAS_LEN && !GENERIC.contains(&base.as_str()) {
        out.push(base.clone());
    }
    let compact: String = base
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' ' | '.'))
        .collect();
    if compact.len() >= MIN_ALIAS_LEN && compact != base && !GENERIC.contains(&compact.as_str()) {
        out.push(compact);
    }
    out
}

/// 工作区值卫生：必须是**存在的目录**、非 URL 协议串、非 Real 自己的任务附件目录。
pub fn is_usable_workspace(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() || p.contains("://") {
        return false;
    }
    // 盘符绝对路径（`X:\` / `X:/`）
    if p.as_bytes().get(1) != Some(&b':') {
        return false;
    }
    let lower = p.replace('/', "\\").to_lowercase();
    if lower.contains("\\tasks\\") && lower.ends_with("\\attachments") {
        return false;
    }
    std::path::Path::new(p).is_dir()
}

/// 可登记为项目根：工作区可用 **且** 带工程标记（或盘符根）。
pub fn is_project_root(path: &str) -> bool {
    if !is_usable_workspace(path) {
        return false;
    }
    let p = std::path::Path::new(path.trim());
    // 盘符根（`D:\`）单独放行
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    if trimmed.len() == 2 && trimmed.as_bytes().get(1) == Some(&b':') {
        return true;
    }
    const MARKERS: &[&str] = &[
        ".git",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "AGENTS.md",
    ];
    MARKERS.iter().any(|m| p.join(m).exists())
}

/// `needle`（小写）作为**独立词**出现在 `hay`（小写）中——
pub fn contains_word(hay_lower: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while from <= hay_lower.len() {
        let Some(i) = hay_lower[from..].find(needle) else {
            return false;
        };
        let s = from + i;
        let e = s + needle.len();
        let before_ok = hay_lower[..s]
            .chars()
            .last()
            .map(|c| !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        let after_ok = hay_lower[e..]
            .chars()
            .next()
            .map(|c| !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        from = e;
    }
    false
}

/// 在名录里找口语名命中（纯函数，便于单测）：返回 (路径, 命中的别名)。
pub fn hit_alias(entries: &[ProjectEntry], user_lower: &str) -> Option<(String, String)> {
    let mut best: Option<(usize, String, String)> = None;
    for e in entries {
        for a in effective_aliases(e) {
            let a_low = a.to_lowercase();
            if !contains_word(user_lower, &a_low) {
                continue;
            }
            let better = match &best {
                Some((len, _, _)) => a_low.len() > *len,
                None => true,
            };
            if better {
                best = Some((a_low.len(), e.path.clone(), a_low));
            }
        }
    }
    best.map(|(_, p, a)| (p, a))
}

/// 载入名录（解析失败/损坏一律当空表，不打断主流程）
pub async fn load(pool: &SqlitePool) -> Vec<ProjectEntry> {
    let Ok(Some(raw)) = repos::get_setting(pool, SETTINGS_KEY).await else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<ProjectEntry>>(&raw).unwrap_or_default()
}

async fn save(pool: &SqlitePool, entries: &[ProjectEntry]) -> AppResult<()> {
    let raw = serde_json::to_string(entries).unwrap_or_else(|_| "[]".to_string());
    repos::set_setting(pool, SETTINGS_KEY, &raw).await
}

/// 登记（upsert）：路径已存在则合并别名，否则追加。非法路径静默跳过。
pub async fn register(pool: &SqlitePool, path: &str) -> AppResult<()> {
    let path = path.trim();
    if !is_project_root(path) {
        return Ok(());
    }
    let mut entries = load(pool).await;
    let key = norm_key(path);
    if let Some(e) = entries.iter_mut().find(|e| norm_key(&e.path) == key) {
        let mut changed = false;
        for a in aliases_for(path) {
            if !e.aliases.contains(&a) {
                e.aliases.push(a);
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
    } else {
        entries.push(ProjectEntry {
            path: path.to_string(),
            aliases: aliases_for(path),
            alias: None,
        });
        if entries.len() > MAX_ENTRIES {
            let drop_n = entries.len() - MAX_ENTRIES;
            entries.drain(0..drop_n);
        }
    }
    save(pool, &entries).await
}

/// 口语名命中查询：返回 (路径, 命中的别名)
pub async fn lookup(pool: &SqlitePool, user_input: &str) -> Option<(String, String)> {
    let entries = load(pool).await;
    hit_alias(&entries, &user_input.to_lowercase())
}

/// 每进程只补登一次（幂等：名录无变化时 register 不写库）
static BACKFILLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 冷启动补登：把历史会话工作区里的项目根补进名录。
pub async fn ensure_backfilled(pool: &SqlitePool) {
    if BACKFILLED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let before = load(pool).await.len();
    let added = backfill_from_sessions(pool).await;
    if added > 0 || load(pool).await.len() != before {
        tracing::info!(added, "项目名录：从历史会话工作区补登完成");
    }
}

/// 从历史会话工作区补登（幂等，冷启动/首轮调用一次即可）
pub async fn backfill_from_sessions(pool: &SqlitePool) -> usize {
    let Ok(ws_list) = repos::list_session_workspaces(pool).await else {
        return 0;
    };
    let before = load(pool).await.len();
    for ws in ws_list {
        let _ = register(pool, &ws).await;
    }
    load(pool).await.len().saturating_sub(before)
}

/// 同一项目根判定（统一分隔符/去尾/小写）——锚行"本轮切换"标注据此，
pub fn same_root(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && !b.trim().is_empty() && norm_key(a) == norm_key(b)
}

/// 路径比较键：统一分隔符 + 小写 + 去尾分隔符（Windows 大小写不敏感）
fn norm_key(path: &str) -> String {
    path.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

#[cfg(test)]
#[path = "project_registry_tests.rs"]
mod project_registry_tests;
