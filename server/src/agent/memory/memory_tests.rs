//! agent/memory/memory.rs 的测试外置（部门盘查：核心文件测试全部移出，此文件单管）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_path_extracted_from_windows_path() {
        let input = r#"修复 D:\AI Doc\node-v26.5.0-win-x64 下的 BAT 脚本"#;
        assert_eq!(
            extract_anchor_path(input).as_deref(),
            Some(r"D:\AI Doc\node-v26.5.0-win-x64")
        );
    }

    #[test]
    fn anchor_path_extracted_with_forward_slash() {
        // 用户用正斜杠 D:/xxx 也必须提取（否则 fallback 捞错锚定）
        let input = "审查 D:/proj-test-projects/a2-concurrent-race 项目";
        assert_eq!(
            extract_anchor_path(input).as_deref(),
            Some(r"D:/proj-test-projects/a2-concurrent-race")
        );
    }

    // ---- 会话工作区挂接（机制接线验收：写端死列 → 解析链回填）----

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn ws_of(v: Option<String>) -> String {
        v.unwrap_or_default()
    }

    #[tokio::test]
    async fn ensure_消息含路径_回填锚定() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        // 样本用编译期注入的工程根：绝对存在，且不含 workspace marker（不会被上溯）
        const WS: &str = env!("CARGO_MANIFEST_DIR");
        let ws = ensure_session_workspace(&pool, &s.id, &format!("看一下 {WS} 的改动")).await;
        assert_eq!(ws_of(ws), WS);
        // 已绑不覆盖：换个无关输入，工作区保持
        let again = ensure_session_workspace(&pool, &s.id, "继续").await;
        assert_eq!(ws_of(again), WS);
    }

    /// 用户改口做别的项目 —— 显式路径必须当场覆盖开局锚定。
    #[tokio::test]
    async fn ensure_已绑后改口_新路径立即覆盖() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        let ws = ensure_session_workspace(&pool, &s.id, "看一下 D:\\proj 的改动").await;
        assert_eq!(ws_of(ws), r"D:\proj");
        let ws2 = ensure_session_workspace(&pool, &s.id, "现在改做 D:\\OtherProj").await;
        assert_eq!(
            ws_of(ws2),
            r"D:\OtherProj",
            "已绑定的工作区必须能被本轮显式路径覆盖"
        );
    }

    #[tokio::test]
    async fn ensure_注册表命中_强锚点() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        repos::set_setting(&pool, "project_registry",
            r#"[{"path":"D:\\proj","alias":"视频工厂"}]"#).await.unwrap();
        // 消息不含路径，但提到别名/项目名场景下按注册表路径匹配
        let ws = ensure_session_workspace(&pool, &s.id, "继续视频工厂的流水线任务").await;
        assert_eq!(ws_of(ws), r"D:\proj");
    }

    #[tokio::test]
    async fn ensure_默认工作区兜底_显式锚定优先() {
        let pool = test_pool().await;
        repos::set_setting(&pool, "default_workspace", r"D:\proj").await.unwrap();
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        // 无锚定无注册表 → 默认兜底
        let ws = ensure_session_workspace(&pool, &s.id, "继续").await;
        assert_eq!(ws_of(ws), r"D:\proj");
        // 有显式路径 → 路径优先于默认
        let s2 = crate::db::repos::create_session(&pool, "t2", "t").await.unwrap();
        let ws2 = ensure_session_workspace(&pool, &s2.id, "看看 D:\\OtherProj 里的问题").await;
        assert_eq!(ws_of(ws2), r"D:\OtherProj");
    }

    // 停止回收未消费插话
    use crate::agent::history::build_history;

    #[tokio::test]
    async fn cancel_pending_marks_interjection_and_message() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        // 模拟 interject 端点：消息（interjection 标记）+ 插话行（带 message_id 关联）
        let item = serde_json::json!({
            "type": "message", "role": "user", "interjection": true,
            "content": [{"type": "input_text", "text": "旧插话"}]
        });
        let msg = crate::db::repos::insert_message(
            &pool, &s.id, "user", "旧插话", Some(&item.to_string()),
        )
        .await
        .unwrap();
        crate::db::repos::insert_interjection(&pool, &s.id, "旧插话", "append", Some(&msg.id))
            .await
            .unwrap();
        assert_eq!(
            crate::db::repos::count_pending_interjections(&pool, &s.id).await.unwrap(),
            1
        );
        // 停止回收
        let n = crate::db::repos::cancel_pending_interjections(&pool, &s.id).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            crate::db::repos::count_pending_interjections(&pool, &s.id).await.unwrap(),
            0,
            "pending 应清空"
        );
        // 对应消息被打 cancelled 标记（历史重建据此跳过）
        let rows = crate::db::repos::list_messages(&pool, &s.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        let v: serde_json::Value =
            serde_json::from_str(rows[0].item_json.as_deref().unwrap()).unwrap();
        assert_eq!(v["cancelled"], serde_json::Value::Bool(true), "消息应作废");
    }

    #[tokio::test]
    async fn build_history_skips_cancelled_interjection() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        // 已作废的旧插话（cancel 后形态）
        let dead = serde_json::json!({
            "type": "message", "role": "user", "interjection": true, "cancelled": true,
            "content": [{"type": "input_text", "text": "旧插话"}]
        });
        crate::db::repos::insert_message(&pool, &s.id, "user", "旧插话", Some(&dead.to_string()))
            .await
            .unwrap();
        // 正常的新用户消息
        crate::db::repos::insert_message(&pool, &s.id, "user", "新消息", None).await.unwrap();
        let items = build_history(&pool, &s.id).await.unwrap().items;
        assert_eq!(items.len(), 1, "作废插话不得进重建历史: {items:?}");
        let text = match &items[0] {
            crate::model::types::InputItem::Message { content: serde_json::Value::Array(arr), .. } => {
                arr.iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            _ => String::new(),
        };
        assert!(text.contains("新消息"), "保留的应是新消息: {text}");
    }

    // ---- 记忆作用域开关（`memory_scope`：长期记忆是否跨会话共享）----

    /// 缺省（表里没这个键）必须是**共享** —— 加开关不能改变既有行为。
    #[tokio::test]
    async fn memory_scope_defaults_to_shared() {
        let pool = test_pool().await;
        assert!(
            crate::db::repos::memory_scope_shared(&pool).await,
            "缺省必须是共享（加开关不得改变既有行为）"
        );
    }

    /// 显式写 `"session"` ⇒ 关掉跨会话共享；大小写与空白不该影响判定。
    #[tokio::test]
    async fn memory_scope_session_disables_sharing() {
        let pool = test_pool().await;
        crate::db::repos::set_setting(&pool, "memory_scope", "session")
            .await
            .unwrap();
        assert!(!crate::db::repos::memory_scope_shared(&pool).await, "写了 session 就该关");
        crate::db::repos::set_setting(&pool, "memory_scope", "  SESSION  ")
            .await
            .unwrap();
        assert!(!crate::db::repos::memory_scope_shared(&pool).await, "大小写与空白不该影响判定");
    }

    /// **非法值回退到"共享"，不是回退到"关"** —— 拼错一个字符不该让记忆静默消失。
    #[tokio::test]
    async fn memory_scope_invalid_value_falls_back_to_shared() {
        let pool = test_pool().await;
        crate::db::repos::set_setting(&pool, "memory_scope", "sesion")
            .await
            .unwrap();
        assert!(
            crate::db::repos::memory_scope_shared(&pool).await,
            "拼错必须回退到共享；静默关掉记忆比报错更危险"
        );
    }

    /// **写侧也必须隔离** —— 这是"只属于当下"能成立的另一半。
    #[tokio::test]
    async fn write_side_downgrades_workspace_scope_when_sharing_off() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        let ws = r"D:\ScopeProbe";

        // ① 共享态（缺省）：conclusion 应落成 workspace 级
        crate::db::repos::remember(&pool, ws, &s.id, "k1", "v1", "conclusion", 50)
            .await
            .unwrap();
        let sc: String = sqlx::query_scalar("SELECT scope FROM memories WHERE key = 'k1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sc, "workspace", "缺省共享：本该跨会话的记录是 workspace 级");

        // ② 关掉共享：同一条记录必须降级为 session 级
        crate::db::repos::set_setting(&pool, "memory_scope", "session")
            .await
            .unwrap();
        crate::db::repos::remember(&pool, ws, &s.id, "k2", "v2", "conclusion", 50)
            .await
            .unwrap();
        let sc2: String = sqlx::query_scalar("SELECT scope FROM memories WHERE key = 'k2'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            sc2, "session",
            "关掉共享后必须落成 session 级 —— 否则重开共享会把旧账一起翻出来"
        );

        // ③ 降级不等于丢弃：本会话仍读得到（recall_session 按 session_id 查）
        let rows = crate::db::repos::recall_session(&pool, &s.id, 20).await.unwrap();
        assert!(
            rows.iter().any(|r| r.key == "k2"),
            "降级后本会话仍须读得到（不能被丢掉）"
        );

        // ④ 而按工作区查的通道拿不到它 —— 这才是"隔离"的判据
        let ws_rows = crate::db::repos::recall_workspace_scope(&pool, ws, "workspace", 20)
            .await
            .unwrap();
        assert!(
            !ws_rows.iter().any(|r| r.key == "k2"),
            "关掉共享期间写的记录，不得出现在工作区级通道里"
        );
    }
}
