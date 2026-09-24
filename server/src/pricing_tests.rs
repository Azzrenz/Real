//! pricing.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_prices_match_official_v4_flash() {
        let p = model_price("deepseek-flash").expect("默认配置应有 deepseek-flash（V4.1 Flash）");
        assert!((p.input_miss - 1.0).abs() < 0.0001);
        assert!((p.input_cached - 0.02).abs() < 0.0001);
        assert!((p.output - 4.0).abs() < 0.0001);
    }

    #[test]
    fn model_prefix_match() {
        // 带版本后缀仍应命中规格名（deepseek-flash-0731 → deepseek-flash）
        let p = model_price("deepseek-flash-0731").expect("前缀匹配应命中");
        assert!((p.input_miss - 1.0).abs() < 0.0001);
    }

    #[test]
    fn legacy_model_name_falls_back_to_same_price() {
        // 旧名（V4 Flash Vision Exp）已下线、且不再进价表 → 走"未知模型回退基价"
        assert!(model_price("deepseek-v4-flash-vision-exp").is_none(), "旧名不应再进价表");
        let legacy = cost_of("deepseek-v4-flash-vision-exp", 1_000_000, 1_000_000, 0, 1.0);
        let cur = cost_of("deepseek-flash", 1_000_000, 1_000_000, 0, 1.0);
        assert!((legacy - cur).abs() < 0.001, "旧名与规格名应同价: {legacy} vs {cur}");
        assert!((legacy - 5.0).abs() < 0.001, "应为 ¥5.0，实际 {legacy}");
    }

    #[test]
    fn normalize_legacy_model_name_to_current() {
        // 归一化（唯一入口）：旧名一律改走 deepseek-flash；新名与 glm 名原样返回。
        assert_eq!(crate::config::normalize_model_name("deepseek-v4-flash"), "deepseek-flash");
        assert_eq!(
            crate::config::normalize_model_name("deepseek-v4-flash-vision-exp"),
            "deepseek-flash"
        );
        assert_eq!(crate::config::normalize_model_name("  deepseek-flash  "), "deepseek-flash");
        assert_eq!(crate::config::normalize_model_name("glm-5.3-flash"), "glm-5.3-flash");
    }

    #[test]
    fn cost_flat_multiplier() {
        // 平峰 ×1：1M 输入未命中 + 1M 输出 = 1.0 + 4.0 = ¥5.0
        let c = cost_of("deepseek-flash", 1_000_000, 1_000_000, 0, 1.0);
        assert!((c - 5.0).abs() < 0.001, "应为 ¥5.0，实际 {c}");
        // 高峰 ×2 → ¥10.0
        let c2 = cost_of("deepseek-flash", 1_000_000, 1_000_000, 0, 2.0);
        assert!((c2 - 10.0).abs() < 0.001, "应为 ¥10.0，实际 {c2}");
    }

    #[test]
    fn cache_hit_priced() {
        // 全缓存命中：0.02 入 + 4.0 出 = ¥4.02
        let c = cost_of("deepseek-flash", 1_000_000, 1_000_000, 1_000_000, 1.0);
        assert!((c - 4.02).abs() < 0.001, "应为 ¥4.02，实际 {c}");
    }

    #[test]
    fn unknown_model_fallback() {
        // 未知模型回退到 flash 平峰基价：1.0 + 4.0 = ¥5.0
        let c = cost_of("some-future-model", 1_000_000, 1_000_000, 0, 1.0);
        assert!(
            (c - 5.0).abs() < 0.001,
            "未知模型回退 flash 平峰价，应为 ¥5.0，实际 {c}"
        );
    }

    #[test]
    fn peak_window_boundaries() {
        // 时段解析边界：09:00-12:00 含 09:00 不含 12:00
        let (sh, sm) = split_hhmm("09:00");
        assert_eq!((sh, sm), (9, 0));
        let (eh, em) = split_hhmm("12:00");
        assert_eq!((eh, em), (12, 0));
    }

    // ── 峰谷口径测试（官方口径修正：周末与法定节假日为空闲时段）──

    fn peak_cfg(weekends_off: bool, holidays: Vec<&str>) -> PeakConfig {
        PeakConfig {
            enabled: true,
            multiplier: 2.0,
            windows: vec![
                TimeWindow {
                    start: "09:00".into(),
                    end: "12:00".into(),
                },
                TimeWindow {
                    start: "14:00".into(),
                    end: "18:00".into(),
                },
            ],
            weekends_off,
            holidays: holidays.into_iter().map(String::from).collect(),
        }
    }

    fn utc_beijing_hour(
        y: i32,
        m: u32,
        d: u32,
        beijing_hour: u32,
    ) -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        // 北京时间 = UTC+8；beijing_hour 传北京时刻，自动减 8 得 UTC（跨日由 chrono 处理）
        chrono::Utc
            .with_ymd_and_hms(y, m, d, beijing_hour.saturating_sub(8), 0, 0)
            .unwrap()
    }

    #[test]
    fn workday_peak_window_is_peak() {
        // 周三，北京 10:00 → 高峰
        let utc = utc_beijing_hour(2026, 8, 19, 10);
        assert!(
            is_peak_at(&peak_cfg(true, vec![]), utc),
            "工作日高峰时段必须计高峰"
        );
    }

    #[test]
    fn workday_non_window_is_off_peak() {
        // 周三，北京 08:00 → 平峰
        let utc = utc_beijing_hour(2026, 8, 19, 8);
        assert!(
            !is_peak_at(&peak_cfg(true, vec![]), utc),
            "工作日非高峰时段必须平峰"
        );
    }

    #[test]
    fn weekend_is_off_peak_when_weekends_off() {
        // 周六，北京 10:00（官方口径：周末空闲）→ 平峰
        let utc = utc_beijing_hour(2026, 8, 22, 10);
        assert!(
            !is_peak_at(&peak_cfg(true, vec![]), utc),
            "周末高峰时段必须平峰（官方公告）"
        );
        // 周日同样
        let sunday = utc_beijing_hour(2026, 8, 23, 10);
        assert!(!is_peak_at(&peak_cfg(true, vec![]), sunday), "周日同样平峰");
    }

    #[test]
    fn weekend_peak_under_legacy_weeks_on() {
        // weekends_off=false（旧口径）时周末仍计高峰——该配置已被官方口径取代，仅保留兼容语义
        let utc = utc_beijing_hour(2026, 8, 22, 10);
        assert!(
            is_peak_at(&peak_cfg(false, vec![]), utc),
            "weekends_off=false 时周末仍计高峰"
        );
    }

    #[test]
    fn holiday_is_off_peak() {
        // 国庆节（周四），北京 10:00 → holidays 配置该日期时平峰
        let utc = utc_beijing_hour(2026, 10, 1, 10);
        assert!(
            !is_peak_at(&peak_cfg(true, vec!["2026-10-01"]), utc),
            "法定节假日必须平峰（官方公告）"
        );
        // 未配置 holidays 时，同一时刻（工作日高峰窗口）仍计高峰
        assert!(
            is_peak_at(&peak_cfg(true, vec![]), utc),
            "未配置节假日时按普通工作日计高峰"
        );
    }

    #[test]
    fn peak_window_inclusive_start_exclusive_end() {
        // 北京 09:00 整是高峰起点；12:00 整不是高峰
        let start = utc_beijing_hour(2026, 8, 19, 9);
        assert!(
            is_peak_at(&peak_cfg(true, vec![]), start),
            "09:00 整应算高峰"
        );
        let noon = utc_beijing_hour(2026, 8, 19, 12);
        assert!(
            !is_peak_at(&peak_cfg(true, vec![]), noon),
            "12:00 整不算高峰"
        );
    }

    #[test]
    fn default_holidays_cover_2026() {
        // 默认配置必须自带法定节假日：列表为空则国庆/春节/五一照收 ×2 高峰价
        let p = PeakConfig::default();
        assert!(!p.holidays.is_empty(), "默认 holidays 为空 = 节假日折扣形同虚设");
        for d in ["2026-01-01", "2026-02-17", "2026-05-01", "2026-09-25", "2026-10-01", "2026-10-07"] {
            assert!(p.holidays.iter().any(|h| h == d), "缺 {d}：该日将按工作日高峰计费");
        }
    }

    #[test]
    fn mid_autumn_2026_is_off_peak() {
        // 2026-09-25 中秋（周五）北京 10:00，落在 09:00-12:00 窗口内 → 必须平峰
        let utc = utc_beijing_hour(2026, 9, 25, 10);
        let cfg = PeakConfig::default();
        assert!(!is_peak_at(&cfg, utc), "中秋当天高峰窗口内必须平峰");
        // 对照：同为周五的 9/18，不在节假日列表 → 计高峰
        let workday = utc_beijing_hour(2026, 9, 18, 10);
        assert!(is_peak_at(&cfg, workday), "普通周五高峰窗口内应计高峰");
    }
}
