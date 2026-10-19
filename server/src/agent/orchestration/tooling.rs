//! 工具信封参数域：单次工具调用的超时预算与自动验证开关。
//!
//! 从 `workflow.rs` 归位而来。这些是「策略参数」，不是循环状态——
//! 内核不认识具体域，只负责遍历注册表询问域自己的超时地板。

/// 工具默认超时（秒，REAL_TOOL_TIMEOUT_SECS 可覆盖）
pub const TOOL_TIMEOUT_DEFAULT: u64 = 60;

/// 信封超时（秒）：域地板上浮 + 参数 timeout+15s 余量——工具自身超时先触发，信封只兜死锁。
pub fn verify_auto_enabled() -> bool {
    match std::env::var("REAL_VERIFY_AUTO") {
        Ok(v) => !matches!(v.trim().to_lowercase().as_str(), "off" | "0" | "false" | "no"),
        Err(_) => true,
    }
}

pub fn tool_envelope_secs(env_default: u64, tool: &str, args: &serde_json::Value) -> u64 {
    let mut t = env_default;
    // 域工具超时地板：内核不认识具体域，遍历注册表询问——
    for d in crate::domains::registered() {
        if let Some(floor) = d.tool_timeout_floor(tool) {
            t = t.max(floor);
        }
    }
    if let Some(n) = args.get("timeout").and_then(|v| v.as_i64()) {
        if n > 0 {
            t = t.max(n as u64 + 15);
        }
    }
    t
}
