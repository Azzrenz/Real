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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::history::govern;

    /// 活跃会话常态规模（100K~150K token）：**远越水位线，但远未越发送安全线** ——
    /// 此刻窗口起点必须纹丝不动。一动就是删历史头部 ⇒ 前缀从下标 0 错位 ⇒ 缓存全量作废。
    #[test]
    fn active_session_within_send_safe_never_shifts_window() {
        let budget = govern::window_shift_budget();
        let input = (budget - 1) as i64;
        assert!(
            input > govern::window_safe_tokens(),
            "前提：该规模必须越水位线，否则测不到这条回归"
        );
        let (start, moved) = window_start_decision(Some(300), 900, 200, 600, input, budget);
        assert_eq!(start, 300, "锚点不该被移动");
        assert!(!moved, "未越发送安全线 ⇒ 不许前移");
    }

    /// 判据线必须**就是**发送安全线 —— 防将来被改回水位线（那正是击穿缓存的元凶）。
    #[test]
    fn shift_budget_is_send_safe_not_watermark() {
        assert_eq!(
            govern::window_shift_budget(),
            govern::send_safe_tokens() as u64
        );
        assert!(
            govern::window_shift_budget() > govern::window_safe_tokens() as u64,
            "窗口前移线必须高于水位线"
        );
    }

    /// 真越发送安全线才前移；且锚点**只前进不后退**（粘性，否则规模一回落就又动 = 又击穿）。
    #[test]
    fn beyond_send_safe_shifts_and_anchor_is_sticky() {
        let budget = govern::window_shift_budget();
        let (start, moved) = window_start_decision(Some(300), 900, 200, 600, (budget * 2) as i64, budget);
        assert!(moved && start > 300, "真越线 ⇒ 前移");
        let (start2, moved2) = window_start_decision(Some(start), 900, 200, 600, 1_000, budget);
        assert_eq!(start2, start, "规模回落 ⇒ 锚点保持不动");
        assert!(!moved2);
    }
}
