//! `retire.rs` 的测试外置（部门盘查：测试全部移出核心文件）

use super::*;
use sqlx::SqlitePool;

/// 建一个内存库，跑真实迁移（保证 SQL 与生产同构）。
async fn mem_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.expect("内存库");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("迁移");
    pool
}

/// 播下一个会话：`active_secs_ago` 秒前有过 `llm.usage`，并带若干流式碎片事件。
async fn seed_session(pool: &SqlitePool, sid: &str, active_secs_ago: i64) {
    sqlx::query(
        "INSERT INTO sessions (id, title, status, system_prompt, created_at, updated_at)
         VALUES (?1, 't', 'done', '', datetime('now'), datetime('now'))",
    )
    .bind(sid)
    .execute(pool)
    .await
    .unwrap();

    // 账本事件：时间戳按 active_secs_ago 回拨
    let ts = format!("datetime('now', '-{active_secs_ago} seconds')");
    sqlx::query(&format!(
        "INSERT INTO events (session_id, kind, payload_json, created_at)
         VALUES (?1, 'llm.usage', '{{\"input\":100,\"cached\":80,\"output\":10}}', {ts})"
    ))
    .bind(sid)
    .execute(pool)
    .await
    .unwrap();

    // 流式碎片：reasoning x5 / message x3 / thinking x2（这些是清场对象）
    for (kind, n) in [("reasoning", 5), ("message", 3), ("thinking", 2)] {
        for i in 0..n {
            sqlx::query(&format!(
                "INSERT INTO events (session_id, kind, payload_json, created_at)
                 VALUES (?1, ?2, '{{\"text\":\"tok{i}\"}}', {ts})"
            ))
            .bind(sid)
            .bind(kind)
            .execute(pool)
            .await
            .unwrap();
        }
    }
    // 断裂归因事件：必须保留
    sqlx::query(&format!(
        "INSERT INTO events (session_id, kind, payload_json, created_at)
         VALUES (?1, 'ctx.rewrite', '{{\"source\":\"archive\"}}', {ts})"
    ))
    .bind(sid)
    .execute(pool)
    .await
    .unwrap();
    // 回合日志：摘要的数据源，也绝不能被清
    sqlx::query(
        "INSERT INTO turn_logs (session_id, seq, user_input, answer_digest, keywords, files, created_at)
         VALUES (?1, 1, '修个 bug', '已修好', '[]', '[\"a.rs\"]', datetime('now'))",
    )
    .bind(sid)
    .execute(pool)
    .await
    .unwrap();
}

/// 某会话某 kind 的事件行数。
async fn count_kind(pool: &SqlitePool, sid: &str, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = ?2")
        .bind(sid)
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 契约 1：温档活跃会话（静默不足温档）**什么都不做**。
#[tokio::test]
async fn active_session_is_untouched() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-active", 60).await;

    sweep(&pool, 86400, 7 * 86400).await.unwrap();

    assert_eq!(count_kind(&pool, "s-active", "reasoning").await, 5, "活跃会话不得动事件");
    let r = retirement_of(&pool, "s-active").await.unwrap();
    assert!(r.is_none(), "活跃会话不该进退场台账");
}

/// 契约 2：静默满温档 → 生成摘要 + 台账留痕，但**一行 events 都不动**。
#[tokio::test]
async fn warm_pass_summarizes_without_touching_events() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-warm", 2 * 86400).await;

    let rep = sweep(&pool, 86400, 7 * 86400).await.unwrap();

    assert_eq!(rep.summarized, 1);
    assert_eq!(rep.purged, 0, "温档不得清场");
    assert_eq!(count_kind(&pool, "s-warm", "reasoning").await, 5, "温档必须保留原始事件");

    let (summarized, purged, err) = retirement_of(&pool, "s-warm").await.unwrap().unwrap();
    assert!(summarized.is_some(), "温档必须记 summarized_at");
    assert!(purged.is_none(), "温档不得记 purged_at");
    assert!(err.is_none());
}

/// 摘要内容必须自带每回合索引与 pack 指针 —— 这是"摘要是索引不是全文"的落地。
#[tokio::test]
async fn digest_carries_turn_index_and_pointer_note() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-digest", 2 * 86400).await;
    sweep(&pool, 86400, 7 * 86400).await.unwrap();

    let (_, _, _) = retirement_of(&pool, "s-digest").await.unwrap().unwrap();
    let raw: String = sqlx::query_scalar(
        "SELECT digest_json FROM session_retirement WHERE session_id = 's-digest'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&raw).expect("摘要必须是合法 JSON");

    assert_eq!(v["schema"], 1);
    assert_eq!(v["turns"], 1, "一轮 → turns=1");
    assert_eq!(v["index"][0]["ask"], "修个 bug", "索引必须带用户原话");
    assert_eq!(v["index"][0]["digest"], "已修好", "索引必须带结论");
    assert_eq!(v["index"][0]["files"][0], "a.rs", "索引必须带涉及文件");
    assert!(
        v["note"].as_str().unwrap().contains("read"),
        "摘要必须自证：要细节 read 包，勿凭记忆猜"
    );
}

/// 契约 3：静默满冷档 → 清流式碎片，**账本/归因/回合日志一行不动**。
#[tokio::test]
async fn cold_pass_purges_fragments_but_keeps_ledger() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-cold", 8 * 86400).await;

    let rep = sweep(&pool, 86400, 7 * 86400).await.unwrap();

    assert_eq!(rep.summarized, 1, "冷档必须先补做温档摘要");
    assert_eq!(rep.purged, 1);
    assert_eq!(rep.purged_rows, 10, "5+3+2 = 10 条碎片");

    for k in purgeable_kinds() {
        assert_eq!(count_kind(&pool, "s-cold", k).await, 0, "{k} 应被清掉");
    }
    // 保留清单逐项验：删错任何一个都是静默的数据损失
    assert_eq!(count_kind(&pool, "s-cold", "llm.usage").await, 1, "成本账本不得删");
    assert_eq!(count_kind(&pool, "s-cold", "ctx.rewrite").await, 1, "断裂归因不得删");
    let tl: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM turn_logs WHERE session_id = 's-cold'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tl, 1, "回合日志不得删");

    let (summarized, purged, _) = retirement_of(&pool, "s-cold").await.unwrap().unwrap();
    assert!(summarized.is_some());
    assert!(purged.is_some(), "冷档必须记 purged_at");
}

/// 幂等：连扫两次，第二次不得重复摘/重复清。
#[tokio::test]
async fn sweep_is_idempotent() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-idem", 8 * 86400).await;

    let first = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!((first.summarized, first.purged), (1, 1));

    let second = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!(
        (second.summarized, second.purged),
        (0, 0),
        "已完成两档的会话不得重做（崩溃重启后靠台账续跑）"
    );
}

/// 「摘完还要等」：温档与冷档之间隔着一整个窗口，中间那次扫描只摘不清。
#[tokio::test]
async fn warm_to_cold_is_two_separate_passes() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-two", 2 * 86400).await;

    let r1 = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!((r1.summarized, r1.purged), (1, 0));
    assert_eq!(count_kind(&pool, "s-two", "reasoning").await, 5, "温档后原文仍在");

    // 时间推进到 8 天前活动
    sqlx::query("UPDATE events SET created_at = datetime('now', '-8 days') WHERE session_id = 's-two'")
        .execute(&pool)
        .await
        .unwrap();

    let r2 = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!((r2.summarized, r2.purged), (0, 1), "温档已完成，这轮只做冷档");
    assert_eq!(count_kind(&pool, "s-two", "reasoning").await, 0);
}

/// 阈值边界：静默恰好等于温档线 → 应触发（判据是 `>=`，不是 `>`）。
#[tokio::test]
async fn boundary_is_inclusive() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-edge", 86400).await;

    let rep = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!(rep.summarized, 1, "恰好到线应触发");
}

/// 无 `llm.usage` 的会话不进退场流程 —— 从没调用过模型，没有前缀要摘。
#[tokio::test]
async fn session_without_usage_is_skipped() {
    let pool = mem_pool().await;
    sqlx::query(
        "INSERT INTO sessions (id, title, status, system_prompt, created_at, updated_at)
         VALUES ('s-empty', 't', 'idle', '', datetime('now','-30 days'), datetime('now','-30 days'))",
    )
    .execute(&pool)
    .await
    .unwrap();

    let rep = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!((rep.summarized, rep.purged), (0, 0));
}

/// 保留清单与清场清单必须互斥 —— 同 kind 同时出现在两边就是逻辑错误。
#[test]
fn purgeable_and_keep_lists_are_disjoint() {
    for p in purgeable_kinds() {
        assert!(!keep_kinds().contains(p), "{p} 不得同时在两张清单里");
    }
}

/// 硬闸门：摘要未就绪时**绝不盲清**。
#[tokio::test]
async fn cold_pass_refuses_to_purge_without_digest() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-nodigest", 8 * 86400).await;

    // 预置台账：冷档已到（idle=8天），但摘要列为空 → 闸门应拦住
    sqlx::query(
        "INSERT INTO session_retirement (session_id, last_active_at, summarized_at, digest_json)
         VALUES ('s-nodigest', datetime('now'), NULL, NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 把 turn_logs 抽掉 → 温档仍能拼出摘要（空索引也是合法摘要），
    sqlx::query(
        "UPDATE session_retirement
            SET summarized_at = datetime('now'), digest_json = ''
          WHERE session_id = 's-nodigest'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let rep = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!(rep.purged, 0, "digest_json 为空时不得清场（硬闸门）");
    assert_eq!(
        count_kind(&pool, "s-nodigest", "reasoning").await,
        5,
        "闸门拦住后原始事件必须完好"
    );
}

/// 闸门放行：摘要齐备（summarized_at + 非空 digest_json）才允许清。
#[tokio::test]
async fn cold_pass_proceeds_only_with_complete_digest() {
    let pool = mem_pool().await;
    seed_session(&pool, "s-ok", 8 * 86400).await;

    // 8 天静默已经同时越过温档与冷档，一轮就会「先摘后清」走完两档。
    let rep = sweep(&pool, 86400, 7 * 86400).await.unwrap();
    assert_eq!((rep.summarized, rep.purged), (1, 1), "一轮内先摘后清，两档都要完成");

    // 清完之后：碎片没了，但摘要与账本都还在
    assert_eq!(count_kind(&pool, "s-ok", "reasoning").await, 0);
    assert_eq!(count_kind(&pool, "s-ok", "llm.usage").await, 1, "账本仍须保留");

    let raw: String = sqlx::query_scalar(
        "SELECT digest_json FROM session_retirement WHERE session_id = 's-ok'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&raw).expect("摘要必须落库且合法");
    assert_eq!(v["turns"], 1, "摘要内容必须在清场后仍可取用");
    assert_eq!(count_kind(&pool, "s-ok", "ctx.rewrite").await, 1, "归因事件仍须保留");
}
