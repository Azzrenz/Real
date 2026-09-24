//! 任务名的自动生成。**不调模型**，全是确定性变换。

use sqlx::SqlitePool;

/// 新建任务时前端写进去的占位标题。只有这些值才允许被自动改写。
const PLACEHOLDERS: [&str; 3] = ["", "新任务", "New task"];

/// 标题长度上限，按字符数算（一个汉字算一个）。
const MAX_CHARS: usize = 20;

/// 路径 → 领域名。按表的顺序匹配，**第一个命中的赢**（所以更具体的必须排在更笼统的前面）。
const AREA_RULES: [(&str, &str); 14] = [
    ("client/src/features/chat", "聊天面板"),
    ("client/src/features/settings", "设置面板"),
    ("client/src/features/sessions", "任务与侧栏"),
    ("client/src/features/tools", "工具卡"),
    ("client/src/features/memory", "记忆面板"),
    ("client/src/features/mcp", "MCP"),
    ("client/src/features/schedule", "定时任务"),
    ("client/src/styles", "样式"),
    ("client/src", "前端"),
    ("server/src/agent", "Agent 内核"),
    ("server/src/tools", "工具层"),
    ("server/src/routes", "后端接口"),
    ("server/prompts", "话术"),
    ("Docs", "文档"),
];

/// 标题是否仍是占位——也就是"用户还没给它起过名字"。
pub fn is_placeholder(title: &str) -> bool {
    PLACEHOLDERS.contains(&title.trim())
}

/// 从用户的话里浓缩一个任务名。
pub fn condense(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let body = flat
        .split(' ')
        .filter(|seg| !seg.starts_with('/'))
        .collect::<Vec<_>>()
        .join(" ");
    let body = drop_leading_path(body.trim())?.trim();
    if body.is_empty() {
        return None;
    }
    if body.chars().count() > MAX_CHARS {
        Some(format!("{}…", body.chars().take(MAX_CHARS).collect::<String>()))
    } else {
        Some(body.to_string())
    }
}

/// 开头是一条路径（盘符 `D:\` 或 UNC `\\srv\`）就切掉它。
fn drop_leading_path(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let unc = text.starts_with(r"\\");
    let drive = bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
    if !unc && !drive {
        return Some(text);
    }
    let (i, _) = text.char_indices().find(|(_, c)| !c.is_ascii())?;
    Some(&text[i..])
}

/// 从这一轮改动过的文件路径推断"这件事属于哪一块"，给任务列表当宏观标题。
pub fn area_from_paths(paths: &[String]) -> Option<String> {
    let mut tally: Vec<(usize, usize)> = Vec::new();
    for p in paths {
        let norm = p.replace('\\', "/");
        let Some(idx) = AREA_RULES.iter().position(|(frag, _)| norm.contains(frag)) else {
            continue;
        };
        match tally.iter_mut().find(|(i, _)| *i == idx) {
            Some((_, n)) => *n += 1,
            None => tally.push((idx, 1)),
        }
    }
    // 票数相同时取规则表里靠前的那个（表本身已按具体程度排好序）。
    let (idx, _) = tally.into_iter().max_by_key(|(i, n)| (*n, std::cmp::Reverse(*i)))?;
    Some(AREA_RULES[idx].1.to_string())
}

/// 改名：仍占位才改。
pub async fn apply(pool: &SqlitePool, session_id: &str, user_input: &str) {
    let current: Option<String> = sqlx::query_scalar("SELECT title FROM sessions WHERE id = ?1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let Some(current) = current else {
        return;
    };
    if !is_placeholder(&current) {
        return;
    }
    let Some(title) = condense(user_input) else {
        return;
    };
    let _ = crate::db::repos::update_session(pool, session_id, Some(&title), None).await;
}

#[cfg(test)]
#[path = "naming_tests.rs"]
mod naming_tests;
