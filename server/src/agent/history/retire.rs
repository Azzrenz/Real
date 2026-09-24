//! 会话退场与摘要沉淀（**缓慢退出清场**）。

use crate::error::AppResult;
use serde_json::{json, Value};
use sqlx::SqlitePool;

/// 清场时**允许**删除的流式碎片事件 kind。
const PURGEABLE_KINDS: &[&str] = &["reasoning", "message", "thinking"];

/// **绝不删除**的事件 kind（保留清单，与 PURGEABLE_KINDS 互斥）。
const KEEP_KINDS: &[&str] = &[
    "llm.usage",
    "ctx.rewrite",
    "ctx.snapshot",
    "error",
    "cancelled",
    "evolution.candidates",
    "user_interjection",
];

/// 一次退场扫描的结果（供调用方记日志与测试断言）。
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct SweepReport {
    /// 本轮判定为温档、已生成摘要的会话数
    pub summarized: usize,
    /// 本轮判定为冷档、已清事件的会话数
    pub purged: usize,
    /// 清掉的事件行总数
    pub purged_rows: u64,
    /// 摘要生成的失败次数（不阻断，下轮重试）
    pub failures: usize,
}

/// 退场扫描主体。**幂等**：同一会话可反复调用，已完成的档位不会重做。
pub async fn sweep(pool: &SqlitePool, warm_secs: i64, cold_secs: i64) -> AppResult<SweepReport> {
    let mut report = SweepReport::default();

    // 待办会话 = 有活动记录、且未走完两档的会话。
    let rows: Vec<(String, i64, bool, bool)> = sqlx::query_as(
        "SELECT s.id,
                CAST((julianday('now') - julianday(MAX(e.created_at))) * 86400.0 AS INTEGER) AS idle,
                r.summarized_at IS NOT NULL AS done_warm,
                r.purged_at IS NOT NULL AS done_cold
           FROM sessions s
           JOIN events e ON e.session_id = s.id AND e.kind = 'llm.usage'
           LEFT JOIN session_retirement r ON r.session_id = s.id
          GROUP BY s.id
         HAVING done_warm = 0 OR done_cold = 0",
    )
    .fetch_all(pool)
    .await?;

    for (sid, idle, done_warm, done_cold) in rows {
        if idle < warm_secs {
            continue;
        }
        // ── 温档：做摘要（不动 events）──────────────────────────────
        if !done_warm {
            match build_session_digest(pool, &sid).await {
                Ok(digest) => {
                    record_summarized(pool, &sid, &digest).await?;
                    report.summarized += 1;
                    tracing::info!(
                        session = %sid, idle_hours = idle as f64 / 3600.0,
                        "会话退场·温档：已生成会话级摘要（原文包已落盘，events 不动）"
                    );
                }
                Err(e) => {
                    report.failures += 1;
                    let _ = record_error(pool, &sid, &e.to_string()).await;
                    tracing::warn!(session = %sid, error = %e,
                        "会话退场·温档摘要失败（不阻断，下轮重试）");
                    continue;
                }
            }
        }

        // ── 冷档：摘要自证够用，才清事件行 ───────────────────────────
        if !done_cold && idle >= cold_secs {
            let has_digest: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM session_retirement
                  WHERE session_id = ?1 AND summarized_at IS NOT NULL
                    AND digest_json IS NOT NULL AND digest_json <> ''",
            )
            .bind(&sid)
            .fetch_optional(pool)
            .await?;
            if has_digest.is_none() {
                tracing::warn!(
                    session = %sid,
                    "会话退场·冷档：摘要未就绪，本轮不清场（宁可占盘，不可盲清）"
                );
                continue;
            }
            let (rows, bytes) = purge_stream_events(pool, &sid).await?;
            record_purged(pool, &sid, rows, bytes).await?;
            report.purged += 1;
            report.purged_rows += rows;
            tracing::info!(
                session = %sid, idle_days = idle as f64 / 86400.0, rows, bytes,
                "会话退场·冷档：已清流式碎片事件（账本/摘要/历史重建源全部保留）"
            );
        }
    }

    Ok(report)
}

/// 温档产物：会话级一页纸摘要。
async fn build_session_digest(pool: &SqlitePool, sid: &str) -> AppResult<String> {
    let turns: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT seq, user_input, answer_digest, files FROM turn_logs
          WHERE session_id = ?1 ORDER BY seq",
    )
    .bind(sid)
    .fetch_all(pool)
    .await?;

    // 时间跨度：会话内第一条与最后一条 llm.usage
    let span: Option<(String, String)> = sqlx::query_as(
        "SELECT MIN(created_at), MAX(created_at) FROM events
          WHERE session_id = ?1 AND kind = 'llm.usage'",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await?;

    // 原文包：复用 spill 目录里该会话已有的回合包（不重新生成）
    let packs: Vec<String> = list_turn_packs(sid);

    let index: Vec<Value> = turns
        .iter()
        .map(|(seq, ask, digest, files)| {
            json!({
                "seq": seq,
                "ask": ask,
                "digest": digest,
                "files": serde_json::from_str::<Vec<String>>(files).unwrap_or_default(),
            })
        })
        .collect();

    // 一页纸的开头一句：取首轮诉求做标题（截断），让摘要自身可读
    let headline = turns
        .first()
        .map(|(_, ask, _, _)| truncate_chars(ask, 120))
        .unwrap_or_default();

    let digest = json!({
        "schema": 1,
        "session_id": sid,
        "turns": turns.len(),
        "first_active": span.as_ref().map(|(a, _)| a.clone()),
        "last_active": span.as_ref().map(|(_, b)| b.clone()),
        "headline": headline,
        "index": index,
        "packs": packs,
        "note": "本摘要是索引：要某轮的细节时 read 对应 pack 文件取全文，勿凭记忆猜。",
    });
    Ok(digest.to_string())
}

/// 列出该会话 spill 目录里已有的回合包文件名（**不重新生成**）。
fn list_turn_packs(sid: &str) -> Vec<String> {
    let dir = crate::mcp::spill::spill_dir(sid);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.contains("-task-") && n.ends_with(".json"))
        .collect();
    v.sort();
    v
}

/// 冷档清场：删掉流式碎片事件行，返回（行数, 估算字节）。
async fn purge_stream_events(pool: &SqlitePool, sid: &str) -> AppResult<(u64, u64)> {
    // 先量体积（删完就量不到了）
    let bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(LENGTH(payload_json)), 0) FROM events
          WHERE session_id = ?1 AND kind IN ('reasoning','message','thinking')",
    )
    .bind(sid)
    .fetch_one(pool)
    .await?;

    // 逐 kind 删（不用 IN 展开拼串，避免 SQL 注入面与转义问题）
    let mut total = 0u64;
    for k in PURGEABLE_KINDS {
        let r = sqlx::query("DELETE FROM events WHERE session_id = ?1 AND kind = ?2")
            .bind(sid)
            .bind(k)
            .execute(pool)
            .await?;
        total += r.rows_affected();
    }
    Ok((total, bytes.max(0) as u64))
}

/// 台账写入：温档完成。
async fn record_summarized(pool: &SqlitePool, sid: &str, digest: &str) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO session_retirement (session_id, last_active_at, summarized_at, digest_json)
         VALUES (?1, datetime('now'), datetime('now'), ?2)
         ON CONFLICT(session_id) DO UPDATE SET
             summarized_at = datetime('now'),
             digest_json   = excluded.digest_json,
             last_error    = NULL",
    )
    .bind(sid)
    .bind(digest)
    .execute(pool)
    .await?;
    Ok(())
}

/// 台账写入：冷档完成。
async fn record_purged(
    pool: &SqlitePool,
    sid: &str,
    rows: u64,
    bytes: u64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO session_retirement (session_id, last_active_at, purged_at, purged_rows, purged_bytes)
         VALUES (?1, datetime('now'), datetime('now'), ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET
             purged_at    = datetime('now'),
             purged_rows  = excluded.purged_rows,
             purged_bytes = excluded.purged_bytes",
    )
    .bind(sid)
    .bind(rows as i64)
    .bind(bytes as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// 台账写入：失败留痕（不覆盖已完成档位的成果）。
async fn record_error(pool: &SqlitePool, sid: &str, err: &str) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO session_retirement (session_id, last_active_at, last_error)
         VALUES (?1, datetime('now'), ?2)
         ON CONFLICT(session_id) DO UPDATE SET last_error = excluded.last_error",
    )
    .bind(sid)
    .bind(err)
    .execute(pool)
    .await?;
    Ok(())
}

/// 按**字符**截断（不是字节）：中文按字节截会截出半个字。
fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// 读取某会话的退场台账（供会话详情接口展示"这次会话处于哪一档"）。
#[allow(dead_code)]
pub async fn retirement_of(
    pool: &SqlitePool,
    sid: &str,
) -> AppResult<Option<(Option<String>, Option<String>, Option<String>)>> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT summarized_at, purged_at, last_error FROM session_retirement WHERE session_id = ?1",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 退场扫描周期。静默档位以「天」计，扫描频率取小时级足够——
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// 挂后台周期扫描器。
pub fn spawn_retirement_sweeper(pool: SqlitePool) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(SWEEP_INTERVAL);
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            iv.tick().await;
            // 每轮都重读阈值：env → DB settings 覆盖可热生效，不用重启服务
            let warm = crate::config::settings::tuning()
                .retire_warm_secs
                .load(std::sync::atomic::Ordering::Relaxed);
            let cold = crate::config::settings::tuning()
                .retire_cold_secs
                .load(std::sync::atomic::Ordering::Relaxed);
            match sweep(&pool, warm, cold).await {
                Ok(rep) if rep.summarized > 0 || rep.purged > 0 => tracing::info!(
                    summarized = rep.summarized, purged = rep.purged,
                    purged_rows = rep.purged_rows, failures = rep.failures,
                    "会话退场扫描完成"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "会话退场扫描失败（下轮重试）"),
            }
        }
    });
}

/// 常量暴露给测试与文档，避免测试里硬编码字符串。
#[allow(dead_code)]
pub fn purgeable_kinds() -> &'static [&'static str] {
    PURGEABLE_KINDS
}
#[allow(dead_code)]
pub fn keep_kinds() -> &'static [&'static str] {
    KEEP_KINDS
}

#[cfg(test)]
#[path = "retire_tests.rs"]
mod retire_tests;
