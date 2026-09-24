//! 锚定窗口 —— 历史窗口起点决策 + 该窗口自己的设置键读写。

pub(crate) async fn put_setting(pool: &sqlx::SqlitePool, key: &str, value: &str) {
    if let Err(e) = sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = datetime('now')",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    {
        tracing::warn!(key, error = %e, "写设置失败——依赖它的机制将静默失效");
    }
}

/// 锚定窗口低线：**超预算时**保留的 user/assistant 条数下限。
pub(crate) const HISTORY_WINDOW_LOW: usize = 200;

/// 窗口起点决策（纯函数，可测）——"锚定窗口"的核心。
pub(crate) fn window_start_decision(
    stored: Option<usize>,
    talk_len: usize,
    low: usize,
    high: usize,
    input_tokens: i64,
    budget: u64,
) -> (usize, bool) {
    let stored = stored.map(|s| s.min(talk_len));
    let Some(anchor) = stored else {
        return (talk_len.saturating_sub(low), true);
    };
    // ② 没超预算（或取不到真实 input）→ 不动 —— 这一条就掐掉了绝大部分前移
    if budget == 0 || input_tokens <= 0 || (input_tokens as u64) <= budget {
        return (anchor, false);
    }
    // ③ 超了：超出 1×~2× → 保留条数从 high 线性收到 low；≥2× → 收到 low
    let ratio = input_tokens as f64 / budget as f64;
    let keep = if ratio >= 2.0 {
        low as f64
    } else {
        high as f64 - (ratio - 1.0) * (high.saturating_sub(low)) as f64
    };
    let keep = (keep as usize).max(1);
    let start = talk_len.saturating_sub(keep);
    if start > anchor {
        (start, true)
    } else {
        (anchor, false)
    }
}
