//! 成本追踪：按 LLM 调用累计 token 与费用（计费走 pricing.rs 峰谷），事件透出前端

use crate::model::types::Usage;

#[derive(Debug, Clone)]
pub struct CostTracker {
    /// 计费模型（E6：按模型从 pricing.json 取价 + 峰谷倍率）
    pub model: String,
    /// LLM 调用次数（规划 1 + 求解 1 + 反思 1，重规划再加）
    pub call_count: u32,
    /// 最近一次调用（精确 token，来自 API usage）
    pub last_input: u64,
    pub last_output: u64,
    pub last_cache_hit: u64,
    /// 累计
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_hit_tokens: u64,
    /// 累计费用（元）—— 逐笔累加，不重算（见 `total_yuan`）
    pub total_cost_yuan: f64,
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CostTracker {
    pub fn new() -> Self {
        Self::with_model(crate::config::DEFAULT_MODEL)
    }

    /// 指定计费模型（E6：编排层传 cfg.llm_model）
    pub fn with_model(model: &str) -> Self {
        Self {
            model: model.to_string(),
            call_count: 0,
            last_input: 0,
            last_output: 0,
            last_cache_hit: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_hit_tokens: 0,
            total_cost_yuan: 0.0,
        }
    }

    /// 记录一次 LLM 调用（输入/输出/缓存命中，来自响应 usage）
    pub fn record(&mut self, input: u64, output: u64, cached: u64) {
        self.call_count += 1;
        self.last_input = input;
        self.last_output = output;
        self.last_cache_hit = cached;
        self.total_input_tokens += input;
        self.total_output_tokens += output;
        self.total_cache_hit_tokens += cached;
        // 逐笔入账：此刻的模型与峰谷倍率就是这一笔的真实单价（事后不再重算）
        let m = crate::pricing::current_multiplier();
        self.total_cost_yuan += crate::pricing::cost_of(&self.model, input, output, cached, m);
    }

    /// 累计花费（元）——前端"已消耗/共消耗"显示（坑位 J38）
    pub fn total_yuan(&self) -> f64 {
        self.total_cost_yuan
    }

    /// 金额四舍五入到分：钱没有低于"分"的单位——
    pub fn round_to_fen(v: f64) -> f64 {
        (v * 100.0).round() / 100.0
    }

    /// 缓存命中率（%）：缓存命中 token / 总输入 token——DeepSeek 硬盘缓存命中价仅为未命中的
    pub fn cache_hit_ratio(&self) -> f64 {
        if self.total_input_tokens == 0 {
            0.0
        } else {
            self.total_cache_hit_tokens as f64 / self.total_input_tokens as f64 * 100.0
        }
    }
}

/// 从 Usage 拆分 (input, output, cached)
pub fn usage_breakdown(usage: &Option<Usage>) -> (u64, u64, u64) {
    let Some(u) = usage else { return (0, 0, 0) };
    let cached = u
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.get("cached_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    (u.input_tokens, u.output_tokens, cached)
}
