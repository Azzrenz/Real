//! 配置化定价（E6，规范 v1.0 §2.6）

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input_miss: f64,
    pub input_cached: f64,
    pub output: f64,
    #[serde(default = "default_window")]
    pub context_window: usize,
    /// 一口价模型（如 glm 系列）：不受峰谷倍率影响，cost_of 恒按基价计
    #[serde(default)]
    pub flat_pricing: bool,
}

fn default_window() -> usize {
    1_048_576
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakConfig {
    #[serde(default = "default_peak_enabled")]
    pub enabled: bool,
    #[serde(default = "default_multiplier")]
    pub multiplier: f64,
    #[serde(default)]
    pub windows: Vec<TimeWindow>,
    /// 周六/周日是否不算高峰（官方 公告：周末为空闲时段 → 应为 true）
    #[serde(default = "default_weekends_off")]
    pub weekends_off: bool,
    /// 法定节假日日期列表（"YYYY-MM-DD"，北京时间；官方公告：节假日为空闲时段。
    #[serde(default)]
    pub holidays: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start: String,
    pub end: String,
}

fn default_peak_enabled() -> bool {
    true
}
fn default_multiplier() -> f64 {
    2.0
}
// 官方 调价公告：「其余时段（含夜间、周末及节假日）为空闲时段」→ 周末不计高峰
fn default_weekends_off() -> bool {
    true
}

impl Default for PeakConfig {
    fn default() -> Self {
        Self {
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
            weekends_off: true,
            holidays: [
                "2026-01-01",
                "2026-01-02",
                "2026-01-03",
                "2026-02-16",
                "2026-02-17",
                "2026-02-18",
                "2026-02-19",
                "2026-02-20",
                "2026-02-21",
                "2026-02-22",
                "2026-04-04",
                "2026-04-05",
                "2026-04-06",
                "2026-05-01",
                "2026-05-02",
                "2026-05-03",
                "2026-05-04",
                "2026-05-05",
                "2026-06-19",
                "2026-06-20",
                "2026-06-21",
                "2026-09-25",
                "2026-09-26",
                "2026-09-27",
                "2026-10-01",
                "2026-10-02",
                "2026-10-03",
                "2026-10-04",
                "2026-10-05",
                "2026-10-06",
                "2026-10-07",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    pub models: HashMap<String, ModelPrice>,
    #[serde(default)]
    pub peak: PeakConfig,
}

fn default_pricing() -> PricingConfig {
    let mut models = HashMap::new();
    // 平峰基价（RMB/百万 token），官方峰谷计价；高峰 = 此价 × peak.multiplier(2)。
    models.insert(
        "deepseek-flash".to_string(),
        ModelPrice {
            input_miss: 1.0,
            input_cached: 0.02,
            output: 4.0,
            context_window: 1_048_576,
            flat_pricing: false,
        },
    );
    // （GLM 接入）：glm-5.3-flash 一口价（无峰谷），1M 上下文。
    models.insert(
        "glm-5.3-flash".to_string(),
        ModelPrice {
            input_miss: 0.8,
            input_cached: 0.23,
            output: 2.8,
            context_window: 1_048_576,
            flat_pricing: true,
        },
    );
    PricingConfig {
        models,
peak: PeakConfig::default(),
    }
}

fn pricing_file_path() -> PathBuf {
    if let Ok(p) = std::env::var("REAL_PRICING_FILE") {
        return PathBuf::from(p);
    }
    crate::path::data_root::data_root().join("pricing.json")
}

static PRICING: OnceLock<PricingConfig> = OnceLock::new();

/// 加载定价配置（首次调用时读文件，缺失自动生成默认文件）
fn load_pricing() -> &'static PricingConfig {
    PRICING.get_or_init(|| {
        let path = pricing_file_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<PricingConfig>(&text) {
                Ok(mut cfg) => {
                    for (k, v) in default_pricing().models {
                        cfg.models.entry(k).or_insert(v);
                    // 峰谷口径补默认：老配置文件缺新假期时自动跟上（否则改代码不生效）。
                    for h in &default_pricing().peak.holidays {
                        if !cfg.peak.holidays.contains(h) {
                            cfg.peak.holidays.push(h.clone());
                        }
                    }
                    }
                    cfg
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "pricing.json 解析失败，使用默认配置");
                    default_pricing()
                }
            },
            Err(_) => {
                // 首次运行：生成默认文件
                let cfg = default_pricing();
                if let Ok(text) = serde_json::to_string_pretty(&cfg) {
                    let _ = std::fs::write(&path, text);
                    tracing::info!(path = %path.display(), "已生成默认 pricing.json（涨价/峰谷改此文件）");
                }
                cfg
            }
        }
    })
}

/// 模型前缀匹配：deepseek-flash-0731 → deepseek-flash
pub fn model_price(model: &str) -> Option<&'static ModelPrice> {
    let cfg = load_pricing();
    let m = model.to_lowercase();
    if let Some(p) = cfg.models.get(&m) {
        return Some(p);
    }
    // 前缀匹配（按 key 长度降序，避免 v4-flash 误配 v4-flash-xxx 之类）
    let mut keys: Vec<&String> = cfg.models.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    for k in keys {
        if m.starts_with(k) {
            return cfg.models.get(k);
        }
    }
    None
}

/// 全部已注册模型名（前端设置面板下拉列表用；单一事实源 = pricing 表）
pub fn model_list() -> Vec<String> {
    let cfg = load_pricing();
    let mut keys: Vec<String> = cfg.models.keys().cloned().collect();
    keys.sort();
    keys
}

/// 当前是否为高峰时段（北京时间 UTC+8，9-12 / 14-18；周六日与法定节假日不计高峰）
pub fn is_peak_hour() -> bool {
    let cfg = load_pricing();
    is_peak_at(&cfg.peak, chrono::Utc::now())
}

/// 是否一口价模型（flat_pricing）——事件透出 "flat" 标签，前端显示"一口价"而非"平峰价"
pub fn is_flat_pricing(model: &str) -> bool {
    model_price(model).map(|p| p.flat_pricing).unwrap_or(false)
}

/// 纯函数判定：给定配置与时刻（UTC），是否处于高峰时段。
fn is_peak_at(cfg: &PeakConfig, utc: chrono::DateTime<chrono::Utc>) -> bool {
    if !cfg.enabled {
        return false;
    }
    let now = utc + chrono::Duration::hours(8);
    use chrono::Datelike;
    // 周末（周六=5 周日=6）
    if cfg.weekends_off && now.weekday().num_days_from_monday() >= 5 {
        return false;
    }
    // 法定节假日
    if !cfg.holidays.is_empty() {
        let date = now.format("%Y-%m-%d").to_string();
        if cfg.holidays.iter().any(|h| h == &date) {
            return false;
        }
    }
    use chrono::Timelike;
    let hm = now.hour() * 60 + now.minute();
    cfg.windows.iter().any(|w| {
        let (sh, sm) = split_hhmm(&w.start);
        let (eh, em) = split_hhmm(&w.end);
        let s = sh * 60 + sm;
        let e = eh * 60 + em;
        hm >= s && hm < e
    })
}

fn split_hhmm(s: &str) -> (u32, u32) {
    let mut it = s.split(':');
    let h = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let m = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    (h, m)
}

/// 当前峰谷倍率（高峰 ×N，平峰 ×1）
pub fn current_multiplier() -> f64 {
    let cfg = load_pricing();
    if is_peak_hour() {
        cfg.peak.multiplier
    } else {
        1.0
    }
}

/// 计算一次调用费用（已含峰谷倍率；multiplier 可注入供测试）
pub fn cost_of(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached: u64,
    multiplier: f64,
) -> f64 {
    match model_price(model) {
        Some(p) => {
            let miss = input_tokens.saturating_sub(cached);
            let base = (miss as f64 * p.input_miss
                + cached as f64 * p.input_cached
                + output_tokens as f64 * p.output)
                / 1_000_000.0;
            // 一口价模型不吃峰谷倍率（glm 系列官方一口价，无峰谷分时）
            let mult = if p.flat_pricing { 1.0 } else { multiplier };
            base * mult
        }
        None => {
            // 未知模型：回退到配置里的 DeepSeek 平峰基价（取当前规格名，不再写死旧名）
            let fb = model_price(crate::config::DEFAULT_MODEL)
                .cloned()
                .unwrap_or(ModelPrice {
                    input_miss: 1.0,
                    input_cached: 0.02,
                    output: 4.0,
                    context_window: 1_048_576,
                    flat_pricing: false,
                });
            let miss = input_tokens.saturating_sub(cached);
            let base = (miss as f64 * fb.input_miss
                + cached as f64 * fb.input_cached
                + output_tokens as f64 * fb.output)
                / 1_000_000.0;
            base * multiplier
        }
    }
}

/// 便捷：按当前峰谷倍率计价
pub fn cost_of_now(model: &str, input_tokens: u64, output_tokens: u64, cached: u64) -> f64 {
    cost_of(
        model,
        input_tokens,
        output_tokens,
        cached,
        current_multiplier(),
    )
}

/// 分项计价（元）：（未命中输入, 缓存命中输入, 输出）各自金额，已含峰谷倍率。
pub fn cost_split_now(model: &str, input_tokens: u64, output_tokens: u64, cached: u64) -> (f64, f64, f64) {
    cost_split(model, input_tokens, output_tokens, cached, current_multiplier())
}

/// 分项计价（元），multiplier 可注入供测试。
pub fn cost_split(model: &str, input_tokens: u64, output_tokens: u64, cached: u64, multiplier: f64) -> (f64, f64, f64) {
    let p = model_price(model).cloned().unwrap_or(ModelPrice {
        input_miss: 1.0,
        input_cached: 0.02,
        output: 4.0,
        context_window: 1_048_576,
        flat_pricing: false,
    });
    let mult = if p.flat_pricing { 1.0 } else { multiplier };
    let miss = input_tokens.saturating_sub(cached);
    (
        miss as f64 * p.input_miss / 1_000_000.0 * mult,
        cached as f64 * p.input_cached / 1_000_000.0 * mult,
        output_tokens as f64 * p.output / 1_000_000.0 * mult,
    )
}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod pricing_tests;
