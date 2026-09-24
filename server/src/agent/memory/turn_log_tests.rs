//! agent/memory/turn_log.rs 的测试外置（部门盘查：核心文件测试全部移出，此文件单管）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_cjk_and_latin() {
        let kws = extract_keywords("缓存命中太低了，先看看 mysql 后端代码", &[]);
        assert!(kws.contains(&"缓存命中太低了".to_string()) || kws.iter().any(|k| k.contains("缓存")), "CJK 段应入关键词: {kws:?}");
        assert!(kws.contains(&"mysql".to_string()), "拉丁词应小写入关键词: {kws:?}");
    }

    #[test]
    fn keywords_from_files() {
        let kws = extract_keywords("继续修", &["D:/x/app/audit_leakage.py".into()]);
        assert!(kws.contains(&"audit_leakage".to_string()), "文件主干应入关键词: {kws:?}");
    }

    #[test]
    fn digest_prefers_structure() {
        let a = "# 结论\n\n## 根因\n- 缺口一\n- 缺口二\n\n正文段落忽略。";
        let d = extract_digest(a);
        assert!(d.contains("# 结论"), "标题应入要点: {d}");
        assert!(d.contains("- 缺口一"), "列表项应入要点: {d}");
        assert!(!d.contains("正文段落忽略"), "纯段落不入要点: {d}");
        assert!(d.contains("L1: # 结论"), "标题带原行号: {d}");
        assert!(d.contains("L4: - 缺口一"), "列表项带原行号: {d}");
    }

    #[test]
    fn digest_falls_back_to_first_paragraph() {
        let d = extract_digest("没有结构的一句话结论，比较长的一些描述文字。");
        assert!(d.contains("没有结构"), "无结构取首段: {d}");
        assert!(d.starts_with("L1: "), "首段同样带行号: {d}");
    }

    #[test]
    fn truncate_chars_safe_on_cjk() {
        let s = "中文截断测试".repeat(100);
        let t = truncate_chars(&s, 50);
        assert_eq!(t.chars().count(), 51, "50 字 + 省略号");
    }

    #[test]
    fn stopwords_filtered() {
        let kws = extract_keywords("这个然后继续看看那个audit", &[]);
        assert!(kws.contains(&"audit".to_string()), "实词保留: {kws:?}");
        assert!(!kws.contains(&"这个".to_string()), "虚词停用: {kws:?}");
        assert!(!kws.contains(&"然后".to_string()), "虚词停用: {kws:?}");
        assert!(kws.contains(&"继续".to_string()), "续接信号有意保留: {kws:?}");
    }

    #[test]
    fn hits_are_inclusive_matches() {
        // 短语级精确匹配太脆：query 是"缓存命中"、stored 是"缓存命中太低了"也应命中
        let q = vec!["缓存命中".to_string(), "audit".to_string()];
        let stored = vec!["缓存命中太低了".to_string(), "后端代码".to_string()];
        assert_eq!(keyword_hits(&q, &stored), 1, "包含式命中算 1");
        let q2 = vec!["缓存命中".to_string(), "后端".to_string()];
        assert_eq!(keyword_hits(&q2, &stored), 2, "两个都包含命中");
    }

    async fn anchor_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn anchor_emitted_when_no_keyword_hit() {
        let pool = anchor_pool().await;
        let sid = "s-anchor";
        sqlx::query(
            "INSERT INTO turn_logs (session_id, seq, user_input, answer_digest, keywords, files, created_at)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(sid)
        .bind("给 project-b 布置考试题目：把题库里的题恢复后跑一遍")
        .bind("- L1: 已布场 22 题\n- L2: 判分脚本已就绪")
        .bind(r#"["project-b","考试题目","题库"]"#)
        .bind(r#"["D:/wb-run/judge.py"]"#)
        .bind("2026-09-11T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

        // 关键词数 = 2 ⇒ 非弱信号；与上面零重叠 ⇒ 旧实现返回空串
        let out = activate(&pool, sid, "为什么会出现这种错误？是不是记忆系统的问题？")
            .await
            .unwrap();
        assert!(!out.is_empty(), "无命中时也必须给回合锚，不得返回空串");
        assert!(out.contains("最近回合锚"), "应给锚而非静默：{out}");
        assert!(out.contains("回合#1"), "应带最近回合序号：{out}");
        assert!(out.contains("给 project-b 布置考试题目"), "应带最近回合诉求：{out}");
        assert!(out.contains("judge.py"), "应带「上一轮动过哪些文件」（产出回程）：{out}");
        assert!(!out.contains("用户持续指令"), "强信号无命中不追加持续指令块（与弱信号分支区分）：{out}");
    }

    /// 反向：弱信号（关键词 <2）分支的既有行为不得被改坏——仍追加持续指令与边界声明。
    #[tokio::test]
    async fn weak_signal_keeps_standing_rules_block() {
        let pool = anchor_pool().await;
        let sid = "s-weak";
        sqlx::query(
            "INSERT INTO turn_logs (session_id, seq, user_input, answer_digest, keywords, files, created_at)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(sid)
        .bind("不要动 build-run.bat，只改 prompts")
        .bind("- L1: 已改 prompts")
        .bind(r#"["build-run","prompts"]"#)
        .bind(r#"[]"#)
        .bind("2026-09-11T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

        let out = activate(&pool, sid, "继续").await.unwrap();
        assert!(out.contains("最近回合续接"), "弱信号仍走续接分支：{out}");
        assert!(out.contains("用户持续指令"), "弱信号仍给持续指令：{out}");
        assert!(out.contains("不构成编辑资格"), "边界声明必须紧跟回灌块（它锚定「以上」）：{out}");
        assert!(
            !out.contains("执行前自检·硬门"),
            "执行纪律已收口到 prompts/workflow/system.md，回灌块里不得再留一份：{out}"
        );
    }

    /// 回灌字面隔离（正向）：台账文本里的可执行占位符字面必须被换掉。
    #[test]
    fn isolate_literals_strips_executable_placeholders() {
        let s = "- L1: file/find 填 #En 或自造 #NEW1 的兜底尚未落地\n- L2: 待补 #E12 引用解析";
        let out = isolate_literals(s);
        assert!(!out.contains("#NEW"), "不得残留 #NEW 字面：{out}");
        assert!(!out.contains("#En"), "不得残留 #En 泛指：{out}");
        assert!(!out.contains("#E12"), "不得残留 #E数字：{out}");
        assert!(out.contains("「新增占位」"), "#NEW 应换成可读等价说法：{out}");
        assert!(out.contains("「第N步」"), "#En 泛指应换成可读等价说法：{out}");
        assert!(out.contains("「第12步」"), "#E12 应换成可读等价说法：{out}");
        assert!(out.contains("尚未落地"), "只隔离字面，正文内容不得被砍：{out}");
    }

    /// 反向：不含占位符字面的文本必须**逐字不变**（老路径不许被改坏）。
    #[test]
    fn isolate_literals_leaves_plain_text_untouched() {
        let s = "- L1: ## 根因\n- L2: 见 #123 issue 与 #fff 色值\n- L3: #E2E 覆盖已补齐\n- L4: #English 文档已补\n- L5: file.rs:120 已改";
        assert_eq!(isolate_literals(s), s, "标题/编号/色值/#E2E/#English 一律不得误伤");
        assert_eq!(isolate_literals(""), "", "空串保持不变");
        assert_eq!(isolate_literals("普通结论，无符号"), "普通结论，无符号");
    }

    /// 端到端钉住管道：digest 含占位符字面时，`activate` 回灌出去的文本不得含该字面。
    #[tokio::test]
    async fn activate_never_leaks_placeholder_literal() {
        let pool = anchor_pool().await;
        let sid = "s-literal";
        sqlx::query(
            "INSERT INTO turn_logs (session_id, seq, user_input, answer_digest, keywords, files, created_at)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(sid)
        .bind("编辑引用解析还没落地")
        .bind("- L1: file/find 填 #E1 或自造 #NEW1 的兜底尚未落地\n- L2: 待补 #E12 引用解析")
        .bind(r#"["编辑引用解析"]"#)
        .bind(r#"[]"#)
        .bind("2026-09-11T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

        // 与本行零关键词重叠 ⇒ 走「最近回合锚」出口（即当时扩大后的那条）
        let out = activate(&pool, sid, "为什么会出现这种错误？是不是记忆系统的问题？")
            .await
            .unwrap();
        assert!(!out.is_empty(), "无命中也要给锚");
        assert!(!out.contains("#NEW"), "回灌文本不得含 #NEW 字面：{out}");
        assert!(!out.contains("#E1"), "回灌文本不得含 #E1 字面：{out}");
        assert!(out.contains("结论要点"), "锚本身仍要在（只隔离字面，不砍内容）：{out}");
        assert!(out.contains("尚未落地"), "结论正文仍在：{out}");
    }
}
