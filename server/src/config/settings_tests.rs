//! config/settings.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 官方端点（测试用本地常量；运行时真值在厂商档案 providers/*.json 里）
    const GLM_BASE: &str = "https://open.bigmodel.cn/api/paas/v4";
    const DS_BASE: &str = "https://api.deepseek.com";

    fn base_settings() -> RuntimeSettings {
        RuntimeSettings {
            llm_mode: LlmMode::Real,
            api_key: Some("ds-key-245d".into()),
            base_url: DS_BASE.into(),
            model: "deepseek-flash".into(),
            default_system_prompt: String::new(),
            thinking_mode: "auto".into(),
            thinking_effort: "medium".into(),
            provider_keys: HashMap::new(),
            provider_bases: HashMap::new(),
        }
    }

    #[test]
    fn deepseek_model_uses_main_credentials_even_with_glm_archive() {
        let mut s = base_settings();
        s.provider_keys.insert("glm".into(), "glm-key-gZ61".into());
        s.provider_bases.insert("glm".into(), GLM_BASE.into());
        let (url, key) = s.credentials_for("deepseek-flash");
        assert_eq!(url, DS_BASE);
        assert_eq!(key, "ds-key-245d");
    }

    #[test]
    fn glm_model_uses_glm_archive() {
        let mut s = base_settings();
        s.provider_keys.insert("glm".into(), "glm-key-gZ61".into());
        s.provider_bases.insert("glm".into(), GLM_BASE.into());
        let (url, key) = s.credentials_for("glm-5.3-flash");
        assert_eq!(url, GLM_BASE);
        assert_eq!(key, "glm-key-gZ61");
    }

    #[test]
    fn glm_model_without_archive_falls_back_to_official_base_and_main_key() {
        let s = base_settings();
        let (url, key) = s.credentials_for("glm-5.3-flash");
        assert_eq!(url, GLM_BASE);
        assert_eq!(key, "ds-key-245d");
    }

    #[test]
    fn glm_custom_proxy_is_respected() {
        // 自建代理（非官方主机）不受错配校验限制
        let mut s = base_settings();
        s.provider_keys.insert("glm".into(), "k".into());
        s.provider_bases
            .insert("glm".into(), "https://proxy.example.com/v4".into());
        let (url, _) = s.credentials_for("GLM-5.3");
        assert_eq!(url, "https://proxy.example.com/v4");
    }

    #[test]
    fn apply_overrides_reads_generic_archive_keys() {
        let mut s = base_settings();
        s.apply_overrides(vec![
            ("glm_api_key".into(), "glm-key-gZ61".into()),
            ("glm_base_url".into(), GLM_BASE.into()),
            ("model".into(), "glm-5.3-flash".into()),
        ]);
        let (url, key) = s.credentials_for("glm-5.3-flash");
        assert_eq!(url, GLM_BASE);
        assert_eq!(key, "glm-key-gZ61");
    }

    #[test]
    fn apply_overrides_empty_values_clear_slots() {
        let mut s = base_settings();
        s.apply_overrides(vec![
            ("glm_api_key".into(), "  ".into()),
            ("glm_base_url".into(), String::new()),
        ]);
        assert!(s.provider_key("glm").is_none());
        assert!(s.provider_base("glm").is_none());
        // 回退链不 panic：glm 档案空 → 档案里的官方端点 + 主 Key
        let (url, key) = s.credentials_for("glm-5.3-flash");
        assert_eq!(url, GLM_BASE);
        assert_eq!(key, "ds-key-245d");
    }

    #[test]
    fn db_legacy_model_name_is_normalized_on_override() {
        let mut s = base_settings();
        s.apply_overrides(vec![(
            "model".into(),
            "deepseek-v4-flash-vision-exp".into(),
        )]);
        assert_eq!(
            s.model, crate::config::DEFAULT_MODEL,
            "DB 里的退役模型名必须被归一，不得原样生效"
        );
    }

    #[test]
    fn db_current_model_name_passes_through() {
        // 边界：新名不被误改（归一化不能把好名字改坏）
        let mut s = base_settings();
        s.apply_overrides(vec![("model".into(), "glm-5.3-flash".into())]);
        assert_eq!(s.model, "glm-5.3-flash");
    }

    #[test]
    fn corrupted_glm_archive_pointing_to_deepseek_self_heals() {
        let mut s = base_settings();
        s.provider_keys.insert("glm".into(), "glm-key-gZ61".into());
        s.provider_bases.insert("glm".into(), DS_BASE.into());
        let (url, key) = s.credentials_for("glm-5.3-flash");
        assert_eq!(url, GLM_BASE);
        assert_eq!(key, "glm-key-gZ61");
    }

    #[test]
    fn corrupted_main_base_pointing_to_bigmodel_self_heals() {
        let mut s = base_settings();
        s.base_url = GLM_BASE.into();
        let (url, _) = s.credentials_for("deepseek-flash");
        assert_eq!(url, DS_BASE);
    }

    // "接入新公司"回归：全部靠档案，代码零改动
    #[test]
    fn third_party_provider_resolves_from_archive_alone() {
        // 模拟用户丢了一个 providers/moonshot.json 后
        let mut s = base_settings();
        s.apply_overrides(vec![
            ("moonshot_api_key".into(), "sk-moon-abc".into()),
            ("moonshot_base_url".into(), "https://api.moonshot.cn/v1".into()),
        ]);
        assert_eq!(s.provider_key("moonshot").as_deref(), Some("sk-moon-abc"));
        assert_eq!(
            s.provider_base("moonshot").as_deref(),
            Some("https://api.moonshot.cn/v1")
        );
    }

    #[test]
    fn slot_names_are_derived_not_hardcoded() {
        // 槽名由厂商 id 派生（唯一取名点）——新增厂商无需登记
        assert_eq!(crate::model::catalog::key_slot("moonshot"), "moonshot_api_key");
        assert_eq!(crate::model::catalog::base_slot("moonshot"), "moonshot_base_url");
        assert_eq!(crate::model::catalog::key_slot("glm"), "glm_api_key");
    }

    #[test]
    fn archive_keys_do_not_collide_with_fixed_keys() {
        // 边界：主键 `api_key` / `base_url` 不得被误当成某厂商的档案槽
        let mut s = base_settings();
        s.apply_overrides(vec![
            ("api_key".into(), "main-key".into()),
            ("base_url".into(), "https://main.example.com".into()),
        ]);
        assert!(s.provider_keys.is_empty(), "主键不得落进厂商档案槽");
        assert!(s.provider_bases.is_empty(), "主键不得落进厂商档案槽");
        assert_eq!(s.api_key.as_deref(), Some("main-key"));
        assert_eq!(s.base_url, "https://main.example.com");
    }

    /// 回归：env 缺省回退值必须等于 `DEF_*` 常量（即 `tuning_defaults()`）。
    #[test]
    fn env_fallback_matches_declared_defaults() {
        init_tuning_from_env();
        let live = tuning_snapshot();
        let defs = tuning_defaults();
        let cases: [(&str, u64, u64); 8] = [
            ("REAL_MAX_ROUNDS", live.max_rounds as u64, defs.max_rounds as u64),
            ("REAL_COMPACT_TRIGGER", live.compact_trigger as u64, defs.compact_trigger as u64),
            ("REAL_COMPACT_KEEP_RAW", live.compact_keep_raw as u64, defs.compact_keep_raw as u64),
            (
                "REAL_COMPACT_PRESSURE_CHARS",
                live.compact_pressure_chars as u64,
                defs.compact_pressure_chars as u64,
            ),
            ("REAL_TASK_KEEP_OUTPUTS", live.task_keep_outputs as u64, defs.task_keep_outputs as u64),
            (
                "REAL_ARCHIVE_KEEP_RECENT",
                live.archive_keep_recent as u64,
                defs.archive_keep_recent as u64,
            ),
            (
                "REAL_MAX_OUTPUT_TOKENS",
                live.max_output_tokens as u64,
                defs.max_output_tokens as u64,
            ),
            (
                "REAL_TASK_STUB_MIN_BYTES",
                live.task_stub_min_bytes as u64,
                defs.task_stub_min_bytes as u64,
            ),
        ];
        for (env_key, got, want) in cases {
            if std::env::var(env_key).is_ok() {
                continue;
            }
            assert_eq!(
                got, want,
                "{env_key} 的缺省回退值与 DEF_* 常量不一致——默认值又长出第二个名字了"
            );
        }
    }

    /// **`auto` 必须真的自适应** —— 这是它存在的全部意义。
    #[test]
    fn explicit_effort_wins_over_complexity() {
        // 显式 low：即便输入复杂，也不抬档（这是"面板设 low 要真的 low"）
        assert_eq!(resolve_effort("auto", "low", "帮我修复这个 bug"), "low");
        let long = "啊".repeat(201);
        assert_eq!(resolve_effort("auto", "low", &long), "low");
        // 显式 high / max：原样生效，不被降档也不被抬档
        assert_eq!(resolve_effort("auto", "high", "你好"), "high");
        assert_eq!(resolve_effort("auto", "max", "帮我修复这个 bug"), "max");
    }

    /// `configured = auto`（或面板值非法）时才按任务复杂度自动决策。
    #[test]
    fn auto_configured_adapts_to_task_complexity() {
        // 简单输入（短、无代码意图词）⇒ low
        assert_eq!(resolve_effort("auto", "auto", "你好"), "low");
        // 复杂输入（含改动类关键词）⇒ high
        assert_eq!(resolve_effort("auto", "auto", "帮我修复这个 bug"), "high");
        // 复杂输入（够长）⇒ high
        let long = "啊".repeat(201);
        assert_eq!(resolve_effort("auto", "auto", &long), "high");
        // 面板值非法 ⇒ 与 auto 同口径（不看面板值，按复杂度决策）
        assert_eq!(resolve_effort("auto", "bogus", "你好"), "low");
        assert_eq!(resolve_effort("auto", "bogus", "帮我修复这个 bug"), "high");
    }

    /// **`off` / `on` 不做复杂度判断** —— 用户显式要求优先于启发式。
    #[test]
    fn explicit_on_off_bypass_complexity() {
        assert_eq!(resolve_effort("off", "high", "帮我重构这个模块"), "none");
        assert_eq!(resolve_effort("on", "low", "你好"), "max");
    }

    /// `is_complex_input` 的两条判据与边界（>200 字符 · 代码意图词）。
    #[test]
    fn is_complex_input_boundaries() {
        assert!(!is_complex_input("你好"), "短且无意图词 ⇒ 不复杂");
        assert!(!is_complex_input(&"啊".repeat(200)), "正好 200 字符 ⇒ 不算（判据是 >200）");
        assert!(is_complex_input(&"啊".repeat(201)), "201 字符 ⇒ 复杂");
        assert!(
            !is_complex_input("看一下这个"),
            "短且无意图词 ⇒ 不复杂。**这是启发式的已知边界**：语义复杂但字面简单的输入会被判成简单 —— \
             判据只有「长度」与「意图词」两条，不做语义理解"
        );
        assert!(is_complex_input("帮我修复"), "含「修复」⇒ 复杂");
        assert!(is_complex_input("run cargo build"), "含 cargo ⇒ 复杂");
    }
}
