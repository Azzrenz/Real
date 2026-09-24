//! 运行时设置（RuntimeSettings）

use crate::config::{Config, LlmMode};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct RuntimeSettings {
    pub llm_mode: LlmMode,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub default_system_prompt: String,
    /// 思考模式开关（E7：auto/on/off，默认 auto）
    pub thinking_mode: String,
    /// 思考强度（E7：low/high/max，默认 low—— 设计决定：简单任务降档；会话级可拉高）
    pub thinking_effort: String,
    /// 厂商凭据档案：厂商 id → api_key。
    pub provider_keys: HashMap<String, String>,
    /// 厂商凭据档案：厂商 id → base_url（同上）
    pub provider_bases: HashMap<String, String>,
}

impl RuntimeSettings {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            llm_mode: cfg.llm_mode,
            api_key: cfg.deepseek_api_key.clone(),
            base_url: cfg.deepseek_base_url.clone(),
            model: cfg.llm_model.clone(),
            default_system_prompt: "你是一个严谨、可靠的多任务 Agent。".into(),
            thinking_mode: cfg.thinking_mode.clone(),
            thinking_effort: cfg.thinking_effort.clone(),
            provider_keys: HashMap::new(),
            provider_bases: HashMap::new(),
        }
    }

    /// 应用 DB 中的覆盖项。
    pub fn apply_overrides(&mut self, db_values: Vec<(String, String)>) {
        for (k, v) in db_values {
            match k.as_str() {
                "llm_mode" => {
                    self.llm_mode = match v.as_str() {
                        "real" => LlmMode::Real,
                        _ => LlmMode::Mock,
                    };
                }
                "api_key" => self.api_key = if v.trim().is_empty() { None } else { Some(v) },
                "base_url" => self.base_url = v,
                // 模型名必须走归一化入口：DB 里存的可能是**已下线的旧名**
                "model" => self.model = crate::config::normalize_model_name(&v),
                "default_system_prompt" => self.default_system_prompt = v,
                "thinking_mode" => {
                    // 只接受契约值，DB 脏数据回退默认，不进 LLM 调用
                    if matches!(v.as_str(), "auto" | "on" | "off") {
                        self.thinking_mode = v;
                    } else {
                        tracing::warn!(value = %v, "thinking_mode 非法，回退 auto");
                        self.thinking_mode = "auto".into();
                    }
                }
                "thinking_effort" => {
                    // GLM「始终思考」模型契约：low/high/max（其余值 API 400）
                    if matches!(v.as_str(), "low" | "high" | "max") {
                        self.thinking_effort = v;
                    } else {
                        tracing::warn!(value = %v, "thinking_effort 非法，回退 high");
                        self.thinking_effort = "high".into();
                    }
                }
                _ => {
                    // 厂商档案槽（通用后缀解析）：`{id}_api_key` / `{id}_base_url`。
                    if let Some(id) = k.strip_suffix("_api_key").filter(|s| !s.is_empty()) {
                        if v.trim().is_empty() {
                            self.provider_keys.remove(id);
                        } else {
                            self.provider_keys.insert(id.to_string(), v);
                        }
                    } else if let Some(id) = k.strip_suffix("_base_url").filter(|s| !s.is_empty()) {
                        if v.trim().is_empty() {
                            self.provider_bases.remove(id);
                        } else {
                            self.provider_bases.insert(id.to_string(), v);
                        }
                    }
                }
            }
        }
    }

    /// 档案槽读取（空串视为未配置）
    pub fn provider_key(&self, id: &str) -> Option<String> {
        self.provider_keys
            .get(id)
            .filter(|k| !k.trim().is_empty())
            .cloned()
    }
    pub fn provider_base(&self, id: &str) -> Option<String> {
        self.provider_bases
            .get(id)
            .filter(|u| !u.trim().is_empty())
            .cloned()
    }

    /// 当前生效模型所属厂商 id（判定"主凭据是不是这家的"）
    fn active_provider_id(&self) -> Option<String> {
        crate::model::catalog::provider_of(&self.model).map(|p| p.id.clone())
    }

    /// 按 model 解析 (base_url, api_key)——读路径的**单一凭据解析点**。
    pub fn credentials_for(&self, model: &str) -> (String, String) {
        let Some(p) = crate::model::catalog::provider_of(model) else {
            return (
                self.base_url.clone(),
                self.api_key.clone().unwrap_or_default(),
            );
        };
        let is_active = self.active_provider_id().as_deref() == Some(p.id.as_str());
        let raw_base = self
            .provider_base(&p.id)
            // 当前生效模型就是这家 → 主 base_url 是真身（可能被用户改成自建代理）
            .or_else(|| if is_active { Some(self.base_url.clone()) } else { None })
            .unwrap_or_else(|| p.base_url.clone());
        let base = match crate::model::catalog::host_mismatch(&p.id, &raw_base) {
            Some(reason) => {
                tracing::warn!(
                    url = %raw_base,
                    provider = %p.id,
                    reason = %reason,
                    "凭据档案与官方端点错配（历史脏数据），回退该厂商官方端点"
                );
                p.base_url.clone()
            }
            None => raw_base,
        };
        let key = self
            .provider_key(&p.id)
            .or_else(|| self.api_key.clone())
            .unwrap_or_default();
        (base, key)
    }
}

pub type SettingsRef = Arc<RwLock<RuntimeSettings>>;

// Tuning — 收敛循环 / 记忆压缩调优参数（运行时可调，热生效）
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

// ── 调优参数默认值：**唯一事实源** ───────────────────────────────────────────
pub const DEF_MAX_ROUNDS: u32 = 100;
/// 上下文 token 硬预算。
pub const DEF_TOKEN_BUDGET: u64 = 58_880;
pub const DEF_COMPACT_TRIGGER: usize = 20;
pub const DEF_COMPACT_KEEP_RAW: usize = 20;
pub const DEF_COMPACT_PRESSURE_CHARS: usize = 60_000;
/// 任务内「绝不动」的最近工具输出条数（出厂默认）。
pub const DEF_TASK_KEEP_OUTPUTS: usize = 3;
pub const DEF_ARCHIVE_KEEP_RECENT: usize = 4;
pub const DEF_BODY_ARCHIVE_FLOOR_BYTES: usize = 60_000;

/// 会话退场·温档静默秒数（默认 1 天）—— 静默满此值做会话级摘要 + 回合包落盘，不动 events。
pub const DEF_RETIRE_WARM_SECS: i64 = 24 * 3600;

/// 会话退场·冷档静默秒数（默认 7 天，必须 ≥ 温档）—— 摘要已自证够用，才清原始事件行。
pub const DEF_RETIRE_COLD_SECS: i64 = 7 * 24 * 3600;
pub const DEF_MAX_OUTPUT_TOKENS: u32 = 65_536;

/// 任务内瘦身起切线（字节）—— 同一任务里、比"最近 N 条工具输出"更早的那些，
pub const DEF_TASK_STUB_MIN_BYTES: usize = 2_304;

// ── spill（全文落盘）阈值：**设置面板「调优档位」可切的那两套值** ──────────
// 出厂默认 = 极简那套（8,000 线，上下文最省）：一次都没设过时生效的就是它。
// 平衡档用 16,000——2026-09-25 按 13 条 read 落盘采样（8,471–31,765，中位 ≈15.5K）定的。
// 其余四项两档同值（40,000 / 200 / 120 / 8 MB），经复核未动。
/// read 全文（mode != lines）落盘线（字符，信封口径）
pub const DEF_READ_SPILL_THRESHOLD_CHARS: usize = 8_000;
/// read 精读（mode = lines）落盘线（≈1000 行）
pub const DEF_READ_LINES_SPILL_THRESHOLD_CHARS: usize = 40_000;
/// 落盘预览保留的头部原文（字符）
pub const DEF_SPILL_PREVIEW_HEAD_CHARS: usize = 200;
/// 落盘预览保留的尾部原文（字符）
pub const DEF_SPILL_PREVIEW_TAIL_CHARS: usize = 120;
/// 单个 spill 全文文件的字节硬上限
pub const DEF_SPILL_MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

pub const DEF_STALL_WARN_ROUNDS: u32 = 4;
/// 收窄档：达此数进入只读收窄（禁 write/modify/run，逼收束总结/向用户提问——一问续命）
pub const DEF_STALL_NARROW_ROUNDS: u32 = 8;
/// 触底档：达此数收束停机（reason 文案带「回复继续可续跑」指引）
pub const DEF_STALL_STOP_ROUNDS: u32 = 12;
/// 纯诊断兜底：从未改文件的任务满此轮数收口（原 STALL_DRY_ROUNDS）
pub const DEF_STALL_DRY_ROUNDS: u32 = 30;
/// 触底止损总门限：轮数不足此值一律不判停机（避免短任务误伤，原 STALL_MIN_ROUNDS）
pub const DEF_STALL_MIN_ROUNDS: u32 = 20;
/// 交账宽限：触底后先注「交账指令」再收口的缓冲轮数（0 = 触底即停，即原行为）
pub const DEF_STALL_GRACE_ROUNDS: u32 = 3;

pub struct Tuning {
    /// 收敛循环最大轮数（1..=200；env REAL_MAX_ROUNDS；默认 `DEF_MAX_ROUNDS`）
    pub max_rounds: AtomicU32,
    /// L3 硬顶的 **token 路线**（`archive.rs::archive_hard_cap`，按 `budget × 60% × BYTES_PER_TOKEN_EST` 折算）。
    pub token_budget: AtomicU64,
    /// 记忆压缩触发条数（≥2；env REAL_COMPACT_TRIGGER；默认 `DEF_COMPACT_TRIGGER`）
    pub compact_trigger: AtomicUsize,
    /// 记忆压缩保留原文条数（env REAL_COMPACT_KEEP_RAW；默认 `DEF_COMPACT_KEEP_RAW`）
    pub compact_keep_raw: AtomicUsize,
    /// 记忆压缩压力阈值（累计字符；env REAL_COMPACT_PRESSURE_CHARS；默认 `DEF_COMPACT_PRESSURE_CHARS`）
    pub compact_pressure_chars: AtomicUsize,
    /// 任务内保留最近工具输出条数（≥1；env REAL_TASK_KEEP_OUTPUTS；默认 `DEF_TASK_KEEP_OUTPUTS`）
    pub task_keep_outputs: AtomicUsize,
    /// 正文保留最近**回合**数（2..=200；env REAL_ARCHIVE_KEEP_RECENT；默认 `DEF_ARCHIVE_KEEP_RECENT`）
    pub archive_keep_recent: AtomicUsize,
    /// 正文归档启动字节门槛（≥1_000；env REAL_BODY_ARCHIVE_FLOOR_BYTES；默认 `DEF_BODY_ARCHIVE_FLOOR_BYTES`）
    pub body_archive_floor_bytes: AtomicUsize,
    /// 单次 LLM 输出 token 上限（≥1_024；env REAL_MAX_OUTPUT_TOKENS；默认 `DEF_MAX_OUTPUT_TOKENS`）。
    pub max_output_tokens: AtomicU32,
    /// 任务内瘦身起切线（字节；env REAL_TASK_STUB_MIN_BYTES；默认 `DEF_TASK_STUB_MIN_BYTES`）
    pub task_stub_min_bytes: AtomicUsize,
    /// read 全文落盘线（字符；env REAL_READ_SPILL_THRESHOLD_CHARS；默认 `DEF_READ_SPILL_THRESHOLD_CHARS`）
    pub read_spill_threshold_chars: AtomicUsize,
    /// read 精读落盘线（字符；env REAL_READ_LINES_SPILL_THRESHOLD_CHARS；默认 `DEF_READ_LINES_SPILL_THRESHOLD_CHARS`）
    pub read_lines_spill_threshold_chars: AtomicUsize,
    /// 落盘预览·头部（字符；env REAL_SPILL_PREVIEW_HEAD_CHARS；默认 `DEF_SPILL_PREVIEW_HEAD_CHARS`）
    pub spill_preview_head_chars: AtomicUsize,
    /// 落盘预览·尾部（字符；env REAL_SPILL_PREVIEW_TAIL_CHARS；默认 `DEF_SPILL_PREVIEW_TAIL_CHARS`）
    pub spill_preview_tail_chars: AtomicUsize,
    /// spill 单文件字节硬上限（env REAL_SPILL_MAX_FILE_BYTES；默认 `DEF_SPILL_MAX_FILE_BYTES`）
    pub spill_max_file_bytes: AtomicUsize,
    /// 会话退场·温档静默秒数（≥60；env REAL_RETIRE_WARM_SECS；默认 `DEF_RETIRE_WARM_SECS`）
    pub retire_warm_secs: std::sync::atomic::AtomicI64,
    /// 会话退场·冷档静默秒数（≥温档；env REAL_RETIRE_COLD_SECS；默认 `DEF_RETIRE_COLD_SECS`）
    pub retire_cold_secs: std::sync::atomic::AtomicI64,
    /// 止损·唤醒档（连续零进展轮数；env REAL_STALL_WARN_ROUNDS；默认 `DEF_STALL_WARN_ROUNDS`）
    pub stall_warn_rounds: AtomicU32,
    /// 止损·收窄档（进入只读收窄；env REAL_STALL_NARROW_ROUNDS；默认 `DEF_STALL_NARROW_ROUNDS`）
    pub stall_narrow_rounds: AtomicU32,
    /// 止损·触底档（收束停机+续命指引；env REAL_STALL_STOP_ROUNDS；默认 `DEF_STALL_STOP_ROUNDS`）
    pub stall_stop_rounds: AtomicU32,
    /// 止损·纯诊断兜底轮数（env REAL_STALL_DRY_ROUNDS；默认 `DEF_STALL_DRY_ROUNDS`）
    pub stall_dry_rounds: AtomicU32,
    /// 止损·总门限（轮数不足不判停机；env REAL_STALL_MIN_ROUNDS；默认 `DEF_STALL_MIN_ROUNDS`）
    pub stall_min_rounds: AtomicU32,
    /// 止损·交账宽限（触底后先收尾再停的缓冲轮数；env REAL_STALL_GRACE_ROUNDS；默认 `DEF_STALL_GRACE_ROUNDS`）
    pub stall_grace_rounds: AtomicU32,
}

static TUNING: Tuning = Tuning {
    max_rounds: AtomicU32::new(DEF_MAX_ROUNDS),
    token_budget: AtomicU64::new(DEF_TOKEN_BUDGET),
    compact_trigger: AtomicUsize::new(DEF_COMPACT_TRIGGER),
    compact_keep_raw: AtomicUsize::new(DEF_COMPACT_KEEP_RAW),
    compact_pressure_chars: AtomicUsize::new(DEF_COMPACT_PRESSURE_CHARS),
    task_keep_outputs: AtomicUsize::new(DEF_TASK_KEEP_OUTPUTS),
    archive_keep_recent: AtomicUsize::new(DEF_ARCHIVE_KEEP_RECENT),
    body_archive_floor_bytes: AtomicUsize::new(DEF_BODY_ARCHIVE_FLOOR_BYTES),
    max_output_tokens: AtomicU32::new(DEF_MAX_OUTPUT_TOKENS),
    task_stub_min_bytes: AtomicUsize::new(DEF_TASK_STUB_MIN_BYTES),
    read_spill_threshold_chars: AtomicUsize::new(DEF_READ_SPILL_THRESHOLD_CHARS),
    read_lines_spill_threshold_chars: AtomicUsize::new(DEF_READ_LINES_SPILL_THRESHOLD_CHARS),
    spill_preview_head_chars: AtomicUsize::new(DEF_SPILL_PREVIEW_HEAD_CHARS),
    spill_preview_tail_chars: AtomicUsize::new(DEF_SPILL_PREVIEW_TAIL_CHARS),
    spill_max_file_bytes: AtomicUsize::new(DEF_SPILL_MAX_FILE_BYTES),
    stall_warn_rounds: AtomicU32::new(DEF_STALL_WARN_ROUNDS),
    stall_narrow_rounds: AtomicU32::new(DEF_STALL_NARROW_ROUNDS),
    stall_stop_rounds: AtomicU32::new(DEF_STALL_STOP_ROUNDS),
    stall_dry_rounds: AtomicU32::new(DEF_STALL_DRY_ROUNDS),
    stall_min_rounds: AtomicU32::new(DEF_STALL_MIN_ROUNDS),
    stall_grace_rounds: AtomicU32::new(DEF_STALL_GRACE_ROUNDS),
    retire_warm_secs: std::sync::atomic::AtomicI64::new(DEF_RETIRE_WARM_SECS),
    retire_cold_secs: std::sync::atomic::AtomicI64::new(DEF_RETIRE_COLD_SECS),
};

/// 便宜模型选择（成本治理）：短输出辅助任务（标题/进化）路由到**档案声明的廉价模型**。
pub fn cheap_model(s: &RuntimeSettings, main: &str) -> String {
    crate::model::catalog::cheap_model_of(|id| s.provider_key(id).is_some(), main)
        .unwrap_or_else(|| main.to_string())
}

pub fn tuning() -> &'static Tuning {
    &TUNING
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct TuningSnapshot {
    pub max_rounds: u32,
    pub compact_trigger: usize,
    pub compact_keep_raw: usize,
    pub compact_pressure_chars: usize,
    pub task_keep_outputs: usize,
    pub archive_keep_recent: usize,
    pub max_output_tokens: u32,
    pub task_stub_min_bytes: usize,
    pub read_spill_threshold_chars: usize,
    pub read_lines_spill_threshold_chars: usize,
    pub spill_preview_head_chars: usize,
    pub spill_preview_tail_chars: usize,
    pub spill_max_file_bytes: usize,
    pub stall_warn_rounds: u32,
    pub stall_narrow_rounds: u32,
    pub stall_stop_rounds: u32,
    pub stall_dry_rounds: u32,
    pub stall_min_rounds: u32,
    pub stall_grace_rounds: u32,
}

pub fn tuning_snapshot() -> TuningSnapshot {
    let t = tuning();
    TuningSnapshot {
        max_rounds: t.max_rounds.load(Ordering::Relaxed),
        compact_trigger: t.compact_trigger.load(Ordering::Relaxed),
        compact_keep_raw: t.compact_keep_raw.load(Ordering::Relaxed),
        compact_pressure_chars: t.compact_pressure_chars.load(Ordering::Relaxed),
        task_keep_outputs: t.task_keep_outputs.load(Ordering::Relaxed),
        archive_keep_recent: t.archive_keep_recent.load(Ordering::Relaxed),
        max_output_tokens: t.max_output_tokens.load(Ordering::Relaxed),
        task_stub_min_bytes: t.task_stub_min_bytes.load(Ordering::Relaxed),
        read_spill_threshold_chars: t.read_spill_threshold_chars.load(Ordering::Relaxed),
        read_lines_spill_threshold_chars: t.read_lines_spill_threshold_chars.load(Ordering::Relaxed),
        spill_preview_head_chars: t.spill_preview_head_chars.load(Ordering::Relaxed),
        spill_preview_tail_chars: t.spill_preview_tail_chars.load(Ordering::Relaxed),
        spill_max_file_bytes: t.spill_max_file_bytes.load(Ordering::Relaxed),
        stall_warn_rounds: t.stall_warn_rounds.load(Ordering::Relaxed),
        stall_narrow_rounds: t.stall_narrow_rounds.load(Ordering::Relaxed),
        stall_stop_rounds: t.stall_stop_rounds.load(Ordering::Relaxed),
        stall_dry_rounds: t.stall_dry_rounds.load(Ordering::Relaxed),
        stall_min_rounds: t.stall_min_rounds.load(Ordering::Relaxed),
        stall_grace_rounds: t.stall_grace_rounds.load(Ordering::Relaxed),
    }
}

/// 出厂默认值快照（与 `tuning_snapshot()` 同构）。
pub fn tuning_defaults() -> TuningSnapshot {
    TuningSnapshot {
        max_rounds: DEF_MAX_ROUNDS,
        compact_trigger: DEF_COMPACT_TRIGGER,
        compact_keep_raw: DEF_COMPACT_KEEP_RAW,
        compact_pressure_chars: DEF_COMPACT_PRESSURE_CHARS,
        task_keep_outputs: DEF_TASK_KEEP_OUTPUTS,
        archive_keep_recent: DEF_ARCHIVE_KEEP_RECENT,
        max_output_tokens: DEF_MAX_OUTPUT_TOKENS,
        task_stub_min_bytes: DEF_TASK_STUB_MIN_BYTES,
        read_spill_threshold_chars: DEF_READ_SPILL_THRESHOLD_CHARS,
        read_lines_spill_threshold_chars: DEF_READ_LINES_SPILL_THRESHOLD_CHARS,
        spill_preview_head_chars: DEF_SPILL_PREVIEW_HEAD_CHARS,
        spill_preview_tail_chars: DEF_SPILL_PREVIEW_TAIL_CHARS,
        spill_max_file_bytes: DEF_SPILL_MAX_FILE_BYTES,
        stall_warn_rounds: DEF_STALL_WARN_ROUNDS,
        stall_narrow_rounds: DEF_STALL_NARROW_ROUNDS,
        stall_stop_rounds: DEF_STALL_STOP_ROUNDS,
        stall_dry_rounds: DEF_STALL_DRY_ROUNDS,
        stall_min_rounds: DEF_STALL_MIN_ROUNDS,
        stall_grace_rounds: DEF_STALL_GRACE_ROUNDS,
    }
}

/// 启动期：env 提供默认（非法值回落到 `DEF_*` 常量，不 fail-fast 中断启动）。
pub fn init_tuning_from_env() {
    let t = tuning();
    let env_u32 = |key: &str, def: u32, lo: u32, hi: u32| -> u32 {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n: &u32| *n >= lo && *n <= hi)
            .unwrap_or(def)
    };
    let env_usize = |key: &str, def: usize| -> usize {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(def)
    };
    t.max_rounds
        .store(env_u32("REAL_MAX_ROUNDS", DEF_MAX_ROUNDS, 1, 200), Ordering::Relaxed);
    t.stall_warn_rounds.store(
        env_u32("REAL_STALL_WARN_ROUNDS", DEF_STALL_WARN_ROUNDS, 1, 50),
        Ordering::Relaxed,
    );
    t.stall_narrow_rounds.store(
        env_u32("REAL_STALL_NARROW_ROUNDS", DEF_STALL_NARROW_ROUNDS, 1, 50),
        Ordering::Relaxed,
    );
    t.stall_stop_rounds.store(
        env_u32("REAL_STALL_STOP_ROUNDS", DEF_STALL_STOP_ROUNDS, 1, 100),
        Ordering::Relaxed,
    );
    t.stall_dry_rounds.store(
        env_u32("REAL_STALL_DRY_ROUNDS", DEF_STALL_DRY_ROUNDS, 5, 200),
        Ordering::Relaxed,
    );
    t.stall_min_rounds.store(
        env_u32("REAL_STALL_MIN_ROUNDS", DEF_STALL_MIN_ROUNDS, 1, 200),
        Ordering::Relaxed,
    );
    t.stall_grace_rounds.store(
        env_u32("REAL_STALL_GRACE_ROUNDS", DEF_STALL_GRACE_ROUNDS, 0, 20),
        Ordering::Relaxed,
    );
    // 保留 env 能力（面板已撤）：它仍是 L3 硬顶的 token 路线。
    let budget = std::env::var("REAL_TOKEN_BUDGET")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n >= 10_000)
        .unwrap_or(DEF_TOKEN_BUDGET);
    t.token_budget.store(budget, Ordering::Relaxed);
    t.compact_trigger
        .store(env_usize("REAL_COMPACT_TRIGGER", DEF_COMPACT_TRIGGER), Ordering::Relaxed);
    t.compact_keep_raw
        .store(env_usize("REAL_COMPACT_KEEP_RAW", DEF_COMPACT_KEEP_RAW), Ordering::Relaxed);
    t.compact_pressure_chars.store(
        env_usize("REAL_COMPACT_PRESSURE_CHARS", DEF_COMPACT_PRESSURE_CHARS),
        Ordering::Relaxed,
    );
    t.task_keep_outputs
        .store(env_usize("REAL_TASK_KEEP_OUTPUTS", DEF_TASK_KEEP_OUTPUTS), Ordering::Relaxed);
    t.archive_keep_recent
        .store(env_usize("REAL_ARCHIVE_KEEP_RECENT", DEF_ARCHIVE_KEEP_RECENT), Ordering::Relaxed);
    t.body_archive_floor_bytes.store(
        env_usize("REAL_BODY_ARCHIVE_FLOOR_BYTES", DEF_BODY_ARCHIVE_FLOOR_BYTES),
        Ordering::Relaxed,
    );
    t.max_output_tokens.store(
        env_u32("REAL_MAX_OUTPUT_TOKENS", DEF_MAX_OUTPUT_TOKENS, 1_024, 1_000_000),
        Ordering::Relaxed,
    );
    t.task_stub_min_bytes.store(
        env_usize("REAL_TASK_STUB_MIN_BYTES", DEF_TASK_STUB_MIN_BYTES),
        Ordering::Relaxed,
    );
    t.read_spill_threshold_chars.store(
        env_usize("REAL_READ_SPILL_THRESHOLD_CHARS", DEF_READ_SPILL_THRESHOLD_CHARS),
        Ordering::Relaxed,
    );
    t.read_lines_spill_threshold_chars.store(
        env_usize(
            "REAL_READ_LINES_SPILL_THRESHOLD_CHARS",
            DEF_READ_LINES_SPILL_THRESHOLD_CHARS,
        ),
        Ordering::Relaxed,
    );
    t.spill_preview_head_chars.store(
        env_usize("REAL_SPILL_PREVIEW_HEAD_CHARS", DEF_SPILL_PREVIEW_HEAD_CHARS),
        Ordering::Relaxed,
    );
    t.spill_preview_tail_chars.store(
        env_usize("REAL_SPILL_PREVIEW_TAIL_CHARS", DEF_SPILL_PREVIEW_TAIL_CHARS),
        Ordering::Relaxed,
    );
    t.spill_max_file_bytes.store(
        env_usize("REAL_SPILL_MAX_FILE_BYTES", DEF_SPILL_MAX_FILE_BYTES),
        Ordering::Relaxed,
    );
    // 会话退场两档：温档 ≥60 秒（防误设成 0 导致刚建会话就被摘要），冷档必须 ≥ 温档
    let env_i64 = |key: &str, def: i64, lo: i64| -> i64 {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n: &i64| *n >= lo)
            .unwrap_or(def)
    };
    let warm = env_i64("REAL_RETIRE_WARM_SECS", DEF_RETIRE_WARM_SECS, 60);
    let cold = env_i64("REAL_RETIRE_COLD_SECS", DEF_RETIRE_COLD_SECS, 60).max(warm);
    t.retire_warm_secs.store(warm, Ordering::Relaxed);
    t.retire_cold_secs.store(cold, Ordering::Relaxed);
}

/// 启动期：DB settings 表覆盖 env 默认（非法值告警并保持默认——与 apply_overrides 同语义）。
pub fn apply_tuning_overrides(db_values: &[(String, String)]) {
    const TUNING_KEYS: [&str; 12] = [
        "max_rounds",
        "read_spill_threshold_chars",
        "read_lines_spill_threshold_chars",
        "spill_preview_head_chars",
        "spill_preview_tail_chars",
        "spill_max_file_bytes",
        // 无进展止损五档（面板可调；须与 routes/settings.rs::tuning_fields 保持同步）
        "stall_warn_rounds",
        "stall_narrow_rounds",
        "stall_stop_rounds",
        "stall_dry_rounds",
        "stall_min_rounds",
        "stall_grace_rounds",
    ];
    for (k, v) in db_values {
        if !TUNING_KEYS.contains(&k.as_str()) {
            continue;
        }
        match v.trim().parse::<u64>() {
            Ok(n) => {
                if let Err(reason) = set_tuning(k, n) {
                    tracing::warn!(key = %k, value = %v, "tuning 覆盖被拒：{reason}");
                }
            }
            Err(_) => {
                tracing::warn!(key = %k, value = %v, "tuning 设置值非法（需为数字），保持默认");
            }
        }
    }
}

/// 运行期 / 启动期共用：校验 + 更新原子。返回 Err = 带原因拒绝（HTTP 400 直传）。
pub fn set_tuning(key: &str, value: u64) -> Result<(), String> {
    let t = tuning();
    let reject = |msg: String| -> Result<(), String> {
        tracing::warn!(key = %key, value, "tuning 校验拒绝");
        Err(msg)
    };
    match key {
        "max_rounds" => {
            if !(1..=200).contains(&value) {
                return reject("最大轮数须在 1–200 之间".into());
            }
            t.max_rounds.store(value as u32, Ordering::Relaxed);
        }
        "token_budget" => {
            if value < 10_000 {
                return reject("token 预算至少 10_000（低于该值不设防）".into());
            }
            t.token_budget.store(value, Ordering::Relaxed);
        }
        "compact_trigger" => {
            if value < 2 {
                return reject("记忆压缩触发条数至少为 2".into());
            }
            t.compact_trigger.store(value as usize, Ordering::Relaxed);
        }
        "compact_keep_raw" => {
            if value < 1 {
                return reject("记忆压缩保留条数至少为 1".into());
            }
            t.compact_keep_raw.store(value as usize, Ordering::Relaxed);
        }
        "compact_pressure_chars" => {
            if value < 1_000 {
                return reject("记忆压力阈值至少 1_000 字符".into());
            }
            t.compact_pressure_chars.store(value as usize, Ordering::Relaxed);
        }
        "task_keep_outputs" => {
            if !(1..=50).contains(&value) {
                return reject("任务内保留工具输出条数须在 1–50 之间".into());
            }
            t.task_keep_outputs.store(value as usize, Ordering::Relaxed);
        }
        "archive_keep_recent" => {
            if !(2..=200).contains(&value) {
                return reject("正文保留回合数须在 2–200 之间".into());
            }
            t.archive_keep_recent.store(value as usize, Ordering::Relaxed);
        }
        "max_output_tokens" => {
            if !(1_024..=1_000_000).contains(&value) {
                return reject("单次输出 token 上限须在 1024–1000000 之间".into());
            }
            t.max_output_tokens.store(value as u32, Ordering::Relaxed);
        }
        "task_stub_min_bytes" => {
            if !(128..=4_000_000).contains(&value) {
                return reject("瘦身起切线须在 128–4000000 字节之间".into());
            }
            t.task_stub_min_bytes.store(value as usize, Ordering::Relaxed);
        }
        "read_spill_threshold_chars" => {
            if !(1_000..=1_000_000).contains(&value) {
                return reject("read 全文落盘线须在 1000–1000000 字符之间".into());
            }
            t.read_spill_threshold_chars
                .store(value as usize, Ordering::Relaxed);
        }
        "read_lines_spill_threshold_chars" => {
            if !(2_000..=2_000_000).contains(&value) {
                return reject("read 精读落盘线须在 2000–2000000 字符之间".into());
            }
            t.read_lines_spill_threshold_chars
                .store(value as usize, Ordering::Relaxed);
        }
        "spill_preview_head_chars" => {
            if value > 10_000 {
                return reject("落盘预览头部须 ≤10000 字符".into());
            }
            t.spill_preview_head_chars
                .store(value as usize, Ordering::Relaxed);
        }
        "spill_preview_tail_chars" => {
            if value > 10_000 {
                return reject("落盘预览尾部须 ≤10000 字符".into());
            }
            t.spill_preview_tail_chars
                .store(value as usize, Ordering::Relaxed);
        }
        "spill_max_file_bytes" => {
            if !(1_048_576..=536_870_912).contains(&value) {
                return reject("spill 单文件上限须在 1–512 MB 之间".into());
            }
            t.spill_max_file_bytes
                .store(value as usize, Ordering::Relaxed);
        }
        // ---- 无进展止损五档（`REAL_STALL_*` 的等价运行期入口）----
        // 档位语义：warn(提示) ≤ narrow(只读收窄·禁写) ≤ stop(收口停机)；dry 是纯诊断兜底；
        // min 是总门限（轮数不足不判停机）。这里只做**单键范围**校验，跨键次序不拦
        // —— 排布不合理（如 narrow > stop）不会崩，只是某档永不触发。
        "stall_warn_rounds" => {
            if !(1..=50).contains(&value) {
                return reject("停滞唤醒档须在 1–50 之间".into());
            }
            t.stall_warn_rounds.store(value as u32, Ordering::Relaxed);
        }
        "stall_narrow_rounds" => {
            if !(1..=50).contains(&value) {
                return reject("只读收窄档须在 1–50 之间".into());
            }
            t.stall_narrow_rounds.store(value as u32, Ordering::Relaxed);
        }
        "stall_stop_rounds" => {
            if !(1..=100).contains(&value) {
                return reject("收口停机档须在 1–100 之间".into());
            }
            t.stall_stop_rounds.store(value as u32, Ordering::Relaxed);
        }
        "stall_dry_rounds" => {
            if !(5..=200).contains(&value) {
                return reject("纯诊断兜底档须在 5–200 之间".into());
            }
            t.stall_dry_rounds.store(value as u32, Ordering::Relaxed);
        }
        "stall_min_rounds" => {
            if !(1..=200).contains(&value) {
                return reject("止损总门限须在 1–200 之间".into());
            }
            t.stall_min_rounds.store(value as u32, Ordering::Relaxed);
        }
        "stall_grace_rounds" => {
            if value > 20 {
                return reject("交账宽限须在 0–20 之间（0=触底即停）".into());
            }
            t.stall_grace_rounds.store(value as u32, Ordering::Relaxed);
        }
        _ => return Err(format!("未知调优项: {key}")),
    }
    Ok(())
}

// 思考档位决策（`thinking_mode` × 任务复杂度 → 本次请求的 reasoning_effort）

/// 判"这一轮是不是复杂任务"（**纯函数**）。
pub fn is_complex_input(input: &str) -> bool {
    if input.chars().count() > 200 {
        return true;
    }
    let lower = input.to_lowercase();
    // 代码 / 改动 / 排障类意图词。命中任一即视为复杂。
    const CODE_HINTS: [&str; 14] = [
        "修复", "重构", "实现", "编写", "修改", "添加", "排查", "定位",
        "bug", "error", "cargo", "npm", "git", "编译",
    ];
    CODE_HINTS.iter().any(|h| lower.contains(h))
}

/// 思考档位决策（**纯函数**）—— `thinking_mode` × 任务复杂度 → `reasoning_effort`。
pub fn resolve_effort(mode: &str, configured: &str, user_input: &str) -> String {
    match mode {
        // off / on 是**开关语义**（用户显式要求优先于启发式），不看复杂度。
        "off" => "none".into(),
        "on" => "max".into(),
        // auto：先看用户是否给了显式档位，给了就用它；只有 auto/非法值才启用启发式。
        _ => match configured {
            "low" | "high" | "max" => configured.into(),
            _ => {
                if is_complex_input(user_input) {
                    "high".into()
                } else {
                    "low".into()
                }
            }
        },
    }
}

/// 掩码展示 API Key（绝不返回明文）
pub fn mask_key(key: Option<&str>) -> Option<String> {
    let k = key?;
    if k.is_empty() {
        return None;
    }
    if k.len() <= 8 {
        return Some("sk-****".into());
    }
    Some(format!("{}****{}", &k[..5], &k[k.len() - 4..]))
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
