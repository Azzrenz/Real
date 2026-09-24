//! agent/memory/theme.rs 的测试外置（部门盘查：核心文件测试全部移出，此文件单管）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 纯函数：打分 / 钩子 / 投入度 ----

    #[test]
    fn score_architecture_is_permanent() {
        assert_eq!(score_theme("重构 engine 的事件循环架构"), 95);
        assert_eq!(score_theme("确定技术选型"), 95);
    }

    #[test]
    fn score_emphasis_and_plain() {
        assert_eq!(score_theme("这个功能最重要，必须先做"), 90);
        assert_eq!(score_theme("帮我写一个计算器"), 70);
        assert_eq!(score_theme("嗯"), 50);
    }

    #[test]
    fn keywords_chinese_trigram_and_english() {
        let kws = extract_keywords("重构 RecycleBin 回收站删除逻辑");
        assert!(kws.contains(&"recyclebin".to_string()), "英文词应提取: {kws:?}");
        assert!(kws.iter().any(|k| k.contains("回收站") || k.contains("站删除")), "中文 3-gram 应提取: {kws:?}");
        // 停用词剔除："帮我检查" 的 help/check 不应出现
        let kws2 = extract_keywords("please help me check it");
        assert!(kws2.is_empty(), "纯停用词应剔空: {kws2:?}");
    }

    #[test]
    fn same_domain_across_wording() {
        // 同一架构话题的不同措辞 → 主题域归并
        assert!(same_theme_domain("重构事件循环", "优化事件循环的性能"));
        assert!(!same_theme_domain("做一个五子棋游戏", "重构事件循环"));
    }

    #[test]
    fn engagement_reaches_permanent() {
        // 70 → +10=80 → 再聊 90 → 定性 95 永恒
        assert_eq!(engagement_score(70, 70), 80);
        assert_eq!(engagement_score(80, 70), 95);
        assert_eq!(engagement_score(95, 70), 95);
    }

    #[test]
    fn preference_and_credential_extraction() {
        let prefs = extract_preferences("探讨这个方案要具体到细节，不要 emoji");
        assert_eq!(prefs.len(), 2, "粒度+表情禁忌两条: {prefs:?}");
        // 语言不再是记忆层的职责：输出语言由全局设置 `output_lang` 唯一裁决。
        assert!(
            extract_preferences("不要用英文，用中文回答").is_empty(),
            "语言禁忌不应再沉淀成用户偏好"
        );
        let creds = extract_credentials("token 是 ghp_abcdefghijklmnopqrst 已经给你了");
        assert_eq!(creds.len(), 1);
        assert!(creds[0].starts_with("ghp_"));
        // 无明文描述也记"已提供"
        assert!(!extract_credentials("github token 我已经给你了").is_empty());
        assert!(extract_credentials("帮我看看这个函数").is_empty());
    }

    // ---- 回环：写入 → 注入（机制转得起来的证明）----

    /// 测试用工作区：真实盘符路径会让 journal 在磁盘上真建目录（内存库 ≠ 文件系统隔离）。
    fn tmp_workspace(tag: &str) -> String {
        std::env::temp_dir()
            .join(format!("real-test-{tag}-{}", crate::path::data_root::stamp_ms()))
            .to_string_lossy()
            .to_string()
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// 端到端回环：store_theme/store_preferences/store_credentials 写入 →
    #[tokio::test]
    async fn write_then_inject_roundtrip() {
        let pool = test_pool().await;
        let session = crate::db::repos::create_session(&pool, "t", "t")
            .await
            .unwrap();
        let sid = session.id.clone();
        let ws = tmp_workspace("roundtrip");
        crate::db::repos::set_session_workspace(&pool, &sid, &ws).await.unwrap();

        // 写入侧（接线后的真实入口；各跑两次验证投入度累积/UPSERT 覆盖）
        store_theme(&pool, &sid, "重构 engine 的事件循环架构", None).await.unwrap();
        store_theme(&pool, &sid, "优化事件循环的性能", None).await.unwrap();
        store_preferences(&pool, &sid, "回答要具体到细节").await.unwrap();
        store_preferences(&pool, &sid, "回答要简洁直接").await.unwrap();
        store_credentials(&pool, &sid, "token ghp_abcdefghijklmnopqrst 已经给你了").await.unwrap();

        // 注入侧：续接语义 → 保底主题注入；偏好/凭证无条件注入
        let ctx = crate::agent::memory::build_memory_prompt(&pool, &sid, "继续 重构 engine 的事件循环")
            .await
            .unwrap()
            .expect("应有记忆注入");
        assert!(ctx.contains("【对话主题】"), "保底主题应注入: {ctx}");
        assert!(ctx.contains("事件循环"), "主题文本应在注入中: {ctx}");
        assert!(ctx.contains("【用户偏好】"), "偏好应注入: {ctx}");
        assert!(ctx.contains("【用户凭证】") && ctx.contains("GitHub 凭证"), "凭证应注入: {ctx}");
    }

    /// 钩子检索：非续接的新对话，只要消息关键词勾到老主题 → 仍注入（防"换个说法就失忆"）
    #[tokio::test]
    async fn hook_hits_theme_without_resume() {
        let pool = test_pool().await;
        let session = crate::db::repos::create_session(&pool, "t", "t")
            .await
            .unwrap();
        let sid = session.id.clone();
        let ws = tmp_workspace("hook");
        crate::db::repos::set_session_workspace(&pool, &sid, &ws).await.unwrap();
        store_theme(&pool, &sid, "重构 RecycleBin 的回收站删除逻辑", None).await.unwrap();

        // 新对话（无"继续"），但消息里含"回收站"关键词 → 钩子命中
        let ctx = crate::agent::memory::build_memory_prompt(&pool, &sid, "回收站删除还有问题")
            .await
            .unwrap()
            .expect("钩子命中应有注入");
        assert!(ctx.contains("【钩子命中的历史主题】"), "钩子应命中: {ctx}");
        assert!(ctx.contains("回收站"), "主题文本应在注入中: {ctx}");
    }

    /// 投入度定性：同主题域多次出现 → priority 升至 95 永恒；DB 里确实只有一条（域归并）
    #[tokio::test]
    async fn theme_engagement_accumulates_and_merges() {
        let pool = test_pool().await;
        let session = crate::db::repos::create_session(&pool, "t", "t")
            .await
            .unwrap();
        let sid = session.id.clone();
        let ws = tmp_workspace("merge");
        crate::db::repos::set_session_workspace(&pool, &sid, &ws).await.unwrap();
        store_theme(&pool, &sid, "重构事件循环", None).await.unwrap();
        store_theme(&pool, &sid, "优化事件循环的性能", None).await.unwrap();
        store_theme(&pool, &sid, "事件循环再改一轮", None).await.unwrap();

        let themes = repos::recall_by_type_workspace(&pool, &ws, "theme", 50)
            .await
            .unwrap();
        assert_eq!(themes.len(), 1, "同主题域应归并为一条: {themes:?}");
        assert!(themes[0].priority >= 90, "投入度应累积到永恒级: {}", themes[0].priority);
    }
}
