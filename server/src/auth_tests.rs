//! auth.rs 的测试外置（测试不进核心文件）。
//! 覆盖：配对码过期 / 超尝试次数 / 用后即弃 / 限速 / token 校验与存储 / SSE 票据 / 设备档案。

use super::*;
use sqlx::SqlitePool;

async fn test_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

// ── 迁移清单 ────────────────────────────────────────────────────────────

#[test]
fn 迁移清单_版本唯一且严格递增() {
    // ⚠️ `sqlx::migrate!` 是**编译期嵌入**：已升级过的库会按版本核对 checksum。
    // 版本重复/回退/改名都会让老库启动即报
    // 「migration N was previously applied but has been modified」——本测试提前拦住。
    let migrator = sqlx::migrate!("./migrations");
    let versions: Vec<i64> = migrator.iter().map(|m| m.version).collect();
    assert!(!versions.is_empty(), "迁移清单不得为空");
    for pair in versions.windows(2) {
        assert!(pair[0] < pair[1], "迁移版本必须严格递增：{versions:?}");
    }
    assert!(versions.contains(&18), "设备表迁移 0018 必须在清单中：{versions:?}");
}

// ── 配对码 ──────────────────────────────────────────────────────────────

#[test]
fn 配对码_六位纯数字() {
    let store = AuthStore::new();
    for _ in 0..64 {
        let (code, _) = store.issue_pair_code(1_000);
        assert_eq!(code.len(), 6, "必须是 6 位: {code}");
        assert!(code.chars().all(|c| c.is_ascii_digit()), "必须是纯数字: {code}");
    }
}

#[test]
fn 配对码_过期即失效且不可再用() {
    let store = AuthStore::new();
    let t0 = 1_000_000;
    let (code, expires_at) = store.issue_pair_code(t0);
    assert_eq!(expires_at, t0 + PAIR_CODE_TTL_SECS);

    // 恰好到期：仍在有效窗口内（now <= expires_at）
    assert!(store
        .redeem_pair_code(&code, expires_at)
        .is_ok(), "到期末刻仍应有效");

    // 重新发一枚，过期后必须 Expired，且该码随即作废（第二次 NoCode）
    store.issue_pair_code(t0);
    assert_eq!(
        store.redeem_pair_code(&code, t0 + PAIR_CODE_TTL_SECS + 1),
        Err(PairError::Expired)
    );
    assert_eq!(
        store.redeem_pair_code(&code, t0 + PAIR_CODE_TTL_SECS + 2),
        Err(PairError::NoCode),
        "过期码必须被清掉"
    );
}

#[test]
fn 配对码_用后即弃() {
    let store = AuthStore::new();
    let (code, _) = store.issue_pair_code(0);
    let token = store.redeem_pair_code(&code, 1).expect("首次配对成功");
    assert_eq!(token.len(), TOKEN_BYTES * 2, "token 应是 64 位十六进制");
    assert_eq!(
        store.redeem_pair_code(&code, 2),
        Err(PairError::NoCode),
        "同一枚码不能换第二个 token"
    );
}

#[test]
fn 配对码_尝试超限即作废() {
    let store = AuthStore::new();
    let (code, _) = store.issue_pair_code(0);
    // 前 4 次错误都是 Rejected
    for i in 1..PAIR_CODE_MAX_ATTEMPTS {
        assert_eq!(store.redeem_pair_code("000000", i as i64), Err(PairError::Rejected));
    }
    // 第 5 次错误 ⇒ 作废
    assert_eq!(
        store.redeem_pair_code("000000", PAIR_CODE_MAX_ATTEMPTS as i64),
        Err(PairError::Exhausted)
    );
    // 作废后连正确码也没用了
    assert_eq!(store.redeem_pair_code(&code, 10), Err(PairError::NoCode));
}

#[test]
fn 配对码_未生成时返回_no_code() {
    let store = AuthStore::new();
    assert_eq!(store.redeem_pair_code("123456", 0), Err(PairError::NoCode));
}

#[test]
fn 配对码_重新生成会覆盖上一枚() {
    let store = AuthStore::new();
    let (old, _) = store.issue_pair_code(0);
    let (new, _) = store.issue_pair_code(1);
    assert_ne!(old, new, "两枚码应不同（概率 1e-6 相同，可接受）");
    // 旧码作废（它已不在 store 里；若新旧相同则仍可通过——所以先断言不同）
    assert!(store.redeem_pair_code(&new, 2).is_ok());
}

#[test]
fn 配对限速_窗口内超限拒绝_跨窗口恢复() {
    let store = AuthStore::new();
    let t0 = 0;
    for i in 0..PAIR_RATE_MAX {
        assert!(store.allow_pair_attempt(t0 + i as i64), "第 {i} 次应放行");
    }
    assert!(!store.allow_pair_attempt(t0 + 1), "同一窗口内第 N+1 次必须拒");
    // 跨窗口后恢复
    assert!(store.allow_pair_attempt(t0 + PAIR_RATE_WINDOW_SECS));
}

// ── token 摘要与常量时间比较 ─────────────────────────────────────────────

#[test]
fn 常量时间比较_正确与错误与长度不同() {
    assert!(constant_time_eq(b"abc", b"abc"));
    assert!(!constant_time_eq(b"abc", b"abd"));
    assert!(!constant_time_eq(b"abc", b"abcd"), "长度不同必须 false");
    assert!(constant_time_eq(b"", b""));
}

#[test]
fn 摘要_token不可逆且稳定() {
    let t = "deadbeef";
    let h = hash_token(t);
    assert_eq!(h.len(), 64);
    assert_ne!(h, t, "摘要不得等于明文");
    assert_eq!(h, hash_token(t), "同一输入摘要稳定");
    assert_ne!(h, hash_token("deadbeee"), "不同输入摘要不同");
}

// ── 设备档案（落库） ─────────────────────────────────────────────────────

#[tokio::test]
async fn token_校验正确与错误() {
    let pool = test_pool().await;
    let dev = create_device(&pool, "我的手机", "tok-abc").await.unwrap();
    assert_eq!(dev.name, "我的手机");

    let hit = verify_token(&pool, "tok-abc").await.unwrap();
    assert_eq!(hit.map(|d| d.id), Some(dev.id.clone()), "正确 token 必须命中");

    assert!(
        verify_token(&pool, "tok-wrong").await.unwrap().is_none(),
        "错误 token 必须不命中"
    );
    assert!(
        verify_token(&pool, "").await.unwrap().is_none(),
        "空 token 必须不命中"
    );
}

#[tokio::test]
async fn token_只存哈希不存明文() {
    let pool = test_pool().await;
    let plain = "super-secret-token";
    let dev = create_device(&pool, "平板", plain).await.unwrap();
    let stored: String = sqlx::query_as::<_, (String,)>("SELECT token_hash FROM devices WHERE id = ?1")
        .bind(&dev.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .0;
    assert_ne!(stored, plain, "库里绝不能出现明文 token");
    assert_eq!(stored, hash_token(plain), "库里应是 SHA-256 摘要");
    // 序列化设备时也不得带出哈希
    let js = serde_json::to_value(&dev).unwrap();
    assert!(js.get("token_hash").is_none(), "设备 JSON 不得含 token_hash");
}

#[tokio::test]
async fn 设备名_清洗与空值兜底() {
    let pool = test_pool().await;
    let d1 = create_device(&pool, "  厨房 平板 ", "t1").await.unwrap();
    assert_eq!(d1.name, "厨房 平板", "应 trim");
    let d2 = create_device(&pool, "\u{7} \n\t ", "t2").await.unwrap();
    assert_eq!(d2.name, "未命名设备", "控制字符/空白应兜底");
    let long: String = "x".repeat(200);
    let d3 = create_device(&pool, &long, "t3").await.unwrap();
    assert_eq!(d3.name.chars().count(), 64, "名称应截断到 64 字符");
}

#[tokio::test]
async fn 设备_列出与删除() {
    let pool = test_pool().await;
    assert!(list_devices(&pool).await.unwrap().is_empty());
    let a = create_device(&pool, "A", "ta").await.unwrap();
    let _b = create_device(&pool, "B", "tb").await.unwrap();
    assert_eq!(list_devices(&pool).await.unwrap().len(), 2);

    assert!(delete_device(&pool, &a.id).await.unwrap(), "删除应命中");
    assert!(!delete_device(&pool, &a.id).await.unwrap(), "重复删除不命中");
    assert_eq!(list_devices(&pool).await.unwrap().len(), 1);
    // 删掉的设备其 token 立即失效
    assert!(verify_token(&pool, "ta").await.unwrap().is_none());
}

#[tokio::test]
async fn last_seen_节流写入() {
    let pool = test_pool().await;
    let dev = create_device(&pool, "手机", "tz").await.unwrap();
    let seen0 = dev.last_seen;

    // 间隔不足 ⇒ 不写
    touch_device(&pool, &dev.id, seen0 + TOUCH_INTERVAL_MS - 1).await.unwrap();
    let seen1: i64 = sqlx::query_as::<_, (i64,)>("SELECT last_seen FROM devices WHERE id = ?1")
        .bind(&dev.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .0;
    assert_eq!(seen1, seen0, "间隔不足不得写入");

    // 达到间隔 ⇒ 写入
    let later = seen0 + TOUCH_INTERVAL_MS;
    touch_device(&pool, &dev.id, later).await.unwrap();
    let seen2: i64 = sqlx::query_as::<_, (i64,)>("SELECT last_seen FROM devices WHERE id = ?1")
        .bind(&dev.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .0;
    assert_eq!(seen2, later, "达到间隔应写入");
}

// ── SSE 一次性票据 ──────────────────────────────────────────────────────

#[test]
fn sse票据_一次性且绑定会话() {
    let store = AuthStore::new();
    let (ticket, _) = store.issue_sse_ticket("sess-1", 1_000);
    assert!(store.consume_sse_ticket(&ticket, "sess-1", 1_001), "首次消费应成功");
    assert!(
        !store.consume_sse_ticket(&ticket, "sess-1", 1_002),
        "票据必须一次性（第二次失败）"
    );

    // 绑定会话：给 sess-1 的票据不能在 sess-2 上用
    let (t2, _) = store.issue_sse_ticket("sess-1", 1_000);
    assert!(!store.consume_sse_ticket(&t2, "sess-2", 1_001), "会话不匹配必须失败");
}

#[test]
fn sse票据_过期即失效() {
    let store = AuthStore::new();
    let (ticket, exp) = store.issue_sse_ticket("sess-1", 1_000);
    assert_eq!(exp, 1_000 + SSE_TICKET_TTL_SECS);
    assert!(!store.consume_sse_ticket(&ticket, "sess-1", exp + 1), "过期票据必须失败");
}

#[test]
fn sse票据_未知票据失败() {
    let store = AuthStore::new();
    assert!(!store.consume_sse_ticket("nope", "sess-1", 0));
}

#[test]
fn cookie_必须_httponly_且_same_site() {
    let c = device_cookie("tok123");
    assert!(c.starts_with("real_device=tok123"), "cookie: {c}");
    assert!(c.contains("HttpOnly"), "必须 HttpOnly: {c}");
    assert!(c.contains("SameSite=Lax"), "必须 SameSite: {c}");
    assert!(!c.contains("Secure"), "局域网明文 HTTP 下不能带 Secure: {c}");
}
