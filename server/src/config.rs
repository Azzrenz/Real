
//! 集中配置：全部来自环境变量，启动时 fail-fast 校验（工程纪律 #4）
pub mod settings;

use serde::Deserialize;
use std::env;
use std::sync::Arc;

#[derive(serde::Serialize, Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// http 传输自定义 header（鉴权/路由用；值可引用 settings 凭据键名）
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LlmMode {
    Mock,
    Real,
}

/// 默认模型名（单一规格名）。
pub const DEFAULT_MODEL: &str = "deepseek-flash";

/// 旧模型名归一化（**唯一入口**，防"能跑但随时会停"的隐性依赖）。
pub fn normalize_model_name(raw: &str) -> String {
    let m = raw.trim();
    if m.to_ascii_lowercase().starts_with("deepseek-v4-flash") {
        tracing::warn!(
            from = %m,
            to = DEFAULT_MODEL,
            "模型名已被官方退役（V4 Flash / V4 Flash Vision Exp → V4.1 Flash），自动改用新名"
        );
        return DEFAULT_MODEL.to_string();
    }
    m.to_string()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub deepseek_api_key: Option<String>,
    pub deepseek_base_url: String,
    pub llm_model: String,
    pub llm_mode: LlmMode,
    pub mcp_servers: Vec<McpServerConfig>,
    // 单次 LLM 输出 token 上限**不在这里**——它是可热改的调优项，
    pub thinking_mode: String,
    /// 思考强度（E7，env REAL_THINKING_EFFORT: low/high/max，默认 low）。
    pub thinking_effort: String,
    /// SQLite 连接池上限（env REAL_DB_POOL_MAX，默认 64，支撑高并发会话）
    pub db_pool_max: u32,
    pub result_truncate_chars: usize,
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Result<Arc<Self>, String> {
        dotenvy::dotenv().ok();

        let api_key = env::var("DEEPSEEK_API_KEY")
            .ok()
            .filter(|s| !s.is_empty() && !s.starts_with("sk-your"));

        // 模式判定（生产安全默认，坑位 I8）
        let mode = match env::var("REAL_LLM_MODE") {
            Ok(m) => match m.to_ascii_lowercase().as_str() {
                "real" => LlmMode::Real,
                "mock" => LlmMode::Mock,
                other => {
                    return Err(format!(
                        "REAL_LLM_MODE 非法值: {other}（应为 real 或 mock）"
                    ))
                }
            },
            Err(_) => {
                if api_key.is_some() {
                    tracing::info!("REAL_LLM_MODE 未设置，检测到 DEEPSEEK_API_KEY → 自动 real");
                    LlmMode::Real
                } else {
                    LlmMode::Mock
                }
            }
        };

        if mode == LlmMode::Real && api_key.is_none() {
            return Err("REAL_LLM_MODE=real 但缺少 DEEPSEEK_API_KEY".into());
        }

        // 模型可用性校验：**查厂商档案**，不按厂商名前缀硬编码。
        let llm_model = normalize_model_name(&env_or("REAL_LLM_MODEL", DEFAULT_MODEL));
        if mode == LlmMode::Real && !crate::model::catalog::is_known_model(&llm_model) {
            return Err(format!(
                "REAL_LLM_MODEL={llm_model} 不在任何厂商档案中。\
                 要接入新模型/新公司：在 {} 放一个 <厂商>.json 档案（模板见 model/catalog.rs 末尾注释）\
                 并重启，无需改代码；也可先把 REAL_LLM_MODEL 改成已收录的型号。",
                crate::model::catalog::providers_dir_hint()
            ));
        }

        // MCP 配置宽松解析：JSON 错误不阻塞启动（MCP server 是可选能力，坑位 I9）
        let mcp_servers: Vec<McpServerConfig> = match env::var("REAL_MCP_SERVERS") {
            Ok(s) if !s.trim().is_empty() => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, raw = %s, "REAL_MCP_SERVERS JSON 解析失败，使用空配置");
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };

        Ok(Arc::new(Self {
            host: env_or("REAL_HOST", "127.0.0.1"),
            port: env_parse("REAL_PORT", 8943u16),
            database_url: env_or(
                "REAL_DATABASE_URL",
                &format!("sqlite://{}", crate::path::data_root::db_path().display()),
            ),
            deepseek_api_key: api_key,
            deepseek_base_url: env_or("DEEPSEEK_BASE_URL", "https://api.deepseek.com"),
            llm_model: llm_model.clone(),
            llm_mode: mode,
            mcp_servers,
            thinking_mode: env_or("REAL_THINKING_MODE", "auto").to_ascii_lowercase(),
            // GLM「始终思考」模型契约只接受 low/high/max（medium 会被 API 400 拒绝）
            thinking_effort: env_or("REAL_THINKING_EFFORT", "low").to_ascii_lowercase(),
            db_pool_max: env_parse("REAL_DB_POOL_MAX", 64u32),
            // 普通工具结果喂模型的上限（设计决定：后端不自动截断，模型自己分段读——
            result_truncate_chars: env_parse("REAL_RESULT_TRUNCATE_CHARS", 500_000usize),
        }))
    }
}

/// 判断输入是否属于"复杂任务"（启发式：输入较长 / 含代码/路径/操作意图）

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
