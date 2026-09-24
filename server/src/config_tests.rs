//! config.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod thinking_tests {
    use super::*;

    #[test]
    fn compression_thresholds_default() {
        // E8 压缩阈值调优：无 env 时应落到经验默认值
        std::env::remove_var("REAL_COMPACT_TRIGGER");
        std::env::remove_var("REAL_COMPACT_KEEP_RAW");
        std::env::remove_var("REAL_COMPACT_COOLDOWN_SECS");
        assert_eq!(env_parse("REAL_COMPACT_TRIGGER", 8usize), 8);
        assert_eq!(env_parse("REAL_COMPACT_KEEP_RAW", 4usize), 4);
        assert_eq!(env_parse("REAL_COMPACT_COOLDOWN_SECS", 30i64), 30);
    }
}
