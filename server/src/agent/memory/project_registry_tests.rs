//! agent/memory/project_registry.rs 的测试外置（部门盘查：核心文件测试全部移出）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_name_takes_last_segment_lowercased() {
        assert_eq!(dir_name(r"D:\proj"), "proj");
        assert_eq!(dir_name("D:/project-c/"), "project-c");
        assert_eq!(dir_name(""), "");
    }

    #[test]
    fn aliases_skip_generic_and_short_names() {
        assert!(aliases_for(r"D:\Docs").is_empty(), "通用目录名不入名录");
        assert!(aliases_for("D:\\").is_empty(), "盘符根无目录名");
        let a = aliases_for(r"D:\Alpha-Build-20260916");
        assert!(a.contains(&"alpha-build-20260916".to_string()));
        assert!(a.contains(&"alphabuild20260916".to_string()), "紧凑形式: {a:?}");
    }

    #[test]
    fn contains_word_respects_ascii_boundary() {
        // 中文夹英文——用户真实说法（"你给我查real这个问题"）
        assert!(contains_word("你给我查real这个问题是在哪儿", "real"));
        // 贴 ASCII 字母不算命中（real 不该吃 realtime 里的 real）
        assert!(!contains_word("realtime 库的问题", "real"));
        assert!(contains_word("upgrade real now", "real"));
    }

    #[test]
    fn hit_alias_prefers_longest_and_switches_project() {
        let entries = vec![
            ProjectEntry {
                path: r"D:\proj".into(),
                aliases: vec!["real".into()],
                alias: None,
            },
            ProjectEntry {
                path: r"D:\project-c".into(),
                aliases: vec!["project-c".into()],
                alias: None,
            },
        ];
        // 上一轮在另一个项目，本轮口语点名 Real → 命中 Real（换项目切得过来）
        let hit = hit_alias(&entries, "你给我查real这个问题是在哪儿").expect("应命中");
        assert_eq!(hit, (r"D:\proj".to_string(), "real".to_string()));
        let hit2 = hit_alias(&entries, "b,z 这个项目要跟我对照 project-c 这个项目").expect("应命中");
        assert_eq!(hit2.0, r"D:\project-c");
        assert!(hit_alias(&entries, "这个函数处理一下").is_none(), "无项目名不误切");
    }

    #[test]
    fn workspace_hygiene_rejects_dirty_values() {
        // 实报脏值样本
        assert!(!is_usable_workspace("s://www.bilibili.com/video"), "URL 协议串不算工作区");
        assert!(
            !is_usable_workspace(
                r"C:\Users\x\AppData\Roaming\real-agent\projects\uifoundry-1\tasks\2026\attachments"
            ),
            "任务附件目录不算项目根"
        );
        assert!(!is_usable_workspace("relative/path"), "非盘符绝对路径不算");
        assert!(!is_usable_workspace(""), "空值不算");
        // 真实存在的目录应通过
        let td = std::env::temp_dir();
        assert!(is_usable_workspace(&td.display().to_string()), "真实目录应可用");
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

    #[tokio::test]
    async fn registry_lookup_reads_settings_channel() {
        let pool = test_pool().await;
        // 空名录不命中（旧实现就是这个状态：表从未被写入）
        assert!(lookup(&pool, "查 real 的问题").await.is_none());
        // 写入一条（模拟已积累）后应命中
        repos::set_setting(
            &pool,
            SETTINGS_KEY,
            r#"[{"path":"D:\\proj","aliases":["real"]}]"#,
        )
        .await
        .unwrap();
        assert_eq!(
            lookup(&pool, "你给我查real这个问题是在哪儿").await,
            Some((r"D:\proj".to_string(), "real".to_string()))
        );
    }

    #[tokio::test]
    async fn register_skips_invalid_root_and_is_idempotent() {
        let pool = test_pool().await;
        // 不存在的目录 → 静默跳过，不污染名录
        register(&pool, r"D:\NoSuchProjectXyz").await.unwrap();
        assert!(load(&pool).await.is_empty());
        // 真实目录但无工程标记 → 也不登记（避免附件/素材目录混入）
        let td = std::env::temp_dir();
        register(&pool, &td.display().to_string()).await.unwrap();
        assert!(load(&pool).await.is_empty(), "无工程标记不入名录");
    }
}
