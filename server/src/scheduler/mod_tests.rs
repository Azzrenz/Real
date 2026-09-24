//! scheduler/mod.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_parse_cron_basic() {
        let spec = parse_cron("0 9 * * *").unwrap();
        assert!(spec.minutes.contains(&0));
        assert!(spec.hours.contains(&9));
        assert!(parse_cron("bad").is_err());
        assert!(parse_cron("0 9 * *").is_err());
        assert!(parse_cron("99 9 * * *").is_err());
    }

    #[test]
    fn test_next_daily_9am() {
        let spec = parse_cron("0 9 * * *").unwrap();
        let after = Utc.with_ymd_and_hms(2026, 8, 10, 8, 0, 0).unwrap();
        let next = next_occurrence(&spec, &after).unwrap();
        assert_eq!(
            (next.month(), next.day(), next.hour(), next.minute()),
            (8, 10, 9, 0)
        );
        let after2 = Utc.with_ymd_and_hms(2026, 8, 10, 9, 1, 0).unwrap();
        let next2 = next_occurrence(&spec, &after2).unwrap();
        assert_eq!((next2.month(), next2.day(), next2.hour()), (8, 11, 9));
    }

    #[test]
    fn test_next_weekly_monday() {
        let spec = parse_cron("0 9 * * 1").unwrap();
        // 是周一，9:01 之后 → 下周一 9:00
        let after = Utc.with_ymd_and_hms(2026, 8, 10, 9, 1, 0).unwrap();
        let next = next_occurrence(&spec, &after).unwrap();
        assert_eq!(next.weekday(), chrono::Weekday::Mon);
        assert_eq!((next.hour(), next.minute()), (9, 0));
    }

    #[test]
    fn test_nl_parsing() {
        assert_eq!(
            parse_nl_to_cron("每天早上9点检查依赖安全").unwrap(),
            "0 9 * * *"
        );
        assert_eq!(parse_nl_to_cron("每小时").unwrap(), "0 * * * *");
        assert_eq!(parse_nl_to_cron("每周一").unwrap(), "0 9 * * 1");
        assert_eq!(parse_nl_to_cron("每周三15:30").unwrap(), "30 15 * * 3");
        assert_eq!(parse_nl_to_cron("每月1号").unwrap(), "0 9 1 * *");
        assert_eq!(parse_nl_to_cron("每天22点半").unwrap(), "30 22 * * *");
    }

    #[test]
    fn test_normalize_schedule() {
        let (cron, nl) = normalize_schedule("0 9 * * *").unwrap();
        assert_eq!(cron, "0 9 * * *");
        assert!(nl.is_empty());
        let (cron2, nl2) = normalize_schedule("每天早上9点检查依赖安全").unwrap();
        assert_eq!(cron2, "0 9 * * *");
        assert!(!nl2.is_empty());
        assert!(normalize_schedule("随便跑").is_err());
    }

    // 内存库集成测试：真实跑迁移 + scheduled_jobs CRUD，验证表结构与 next_run_at 计算。
    #[tokio::test]
    async fn test_job_crud_against_migration() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let job = create_job(
            &pool,
            "依赖安全巡检",
            "检查 Cargo 依赖漏洞",
            "0 9 * * *",
            "每天9点",
            None,
        )
        .await
        .unwrap();
        assert_eq!(job.name, "依赖安全巡检");
        assert!(job.next_run_at.is_some(), "新建任务应自动算好下次触发时间");
        assert_eq!(job.enabled, true);

        let listed = list_jobs(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);

        // 停用再启用，next_run_at 应被补算
        assert!(set_enabled(&pool, &job.id, false).await.unwrap());
        assert!(set_enabled(&pool, &job.id, true).await.unwrap());

        assert!(delete_job(&pool, &job.id).await.unwrap());
        assert!(list_jobs(&pool).await.unwrap().is_empty());
    }
}
