//! LLM 契约类型出口

pub mod catalog;
pub mod glm_adapter;
pub mod llm;
pub mod types;

/// 请求 debug dump（死循环排查）：env REAL_DEBUG_REQ=1 时把每次 LLM 请求的
pub async fn debug_dump_req(req: &crate::model::types::ResponsesRequest, resp: &reqwest::Response) {
    if std::env::var("REAL_DEBUG_REQ").as_deref() != Ok("1") {
        return;
    }
    let status = resp.status().as_u16();
    let json = serde_json::to_string(req).unwrap_or_default();
    let n_input = req.input.len();
    // 特征扫描：空 assistant / 悬挂 function_call
    let mut empty_assistant = 0usize;
    let mut fc_no_output = 0usize;
    use crate::model::types::InputItem;
    for it in &req.input {
        if let InputItem::Message { role, content, tool_calls } = it {
            let empty = match content {
                serde_json::Value::Array(a) => a.is_empty(),
                serde_json::Value::String(s) => s.trim().is_empty(),
                _ => true,
            };
            if role == "assistant" && empty && tool_calls.is_none() {
                empty_assistant += 1;
            }
        }
        if let InputItem::FunctionCall { .. } = it {
            fc_no_output += 1;
        }
    }
    let line = format!(
        "[{}] model={} status={} input_items={} bytes={} empty_assistant={} function_calls={} first={} last={}",
        chrono::Utc::now().format("%H:%M:%S"),
        req.model,
        status,
        n_input,
        json.len(),
        empty_assistant,
        fc_no_output,
        &json.chars().take(200).collect::<String>(),
        &json.chars().rev().take(150).collect::<String>().chars().rev().collect::<String>(),
    );
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("req_debug.log")
    {
        let _ = writeln!(f, "{line}");
    }
}
