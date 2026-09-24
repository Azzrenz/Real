//! GLM（智谱 chat/completions 兼容协议）适配层

use crate::error::{AppError, AppResult};
use crate::model::catalog::ThinkingStyle;
use crate::model::types::*;
use futures_util::StreamExt;
use serde_json::{json, Value};

/// 本模块服务"chat/completions 协议"的**全部**厂商（GLM 是第一个，不是唯一）。
pub fn uses_chat_protocol(model: &str) -> bool {
    crate::model::catalog::protocol_of(model) == crate::model::catalog::Protocol::ChatCompletions
}

// 请求翻译：ResponsesRequest → chat/completions body

/// 思考参数按**档案声明的怪癖**注入：GLM 恒开 thinking + reasoning_effort；
fn inject_thinking(body: &mut Value, req: &ResponsesRequest, style: ThinkingStyle) {
    if style != ThinkingStyle::GlmAlwaysOn {
        return;
    }
    // GLM 5.3-flash 思考不可关：恒开 thinking。clear_thinking 默认 **true**（每轮独立
    let clear_thinking = std::env::var("REAL_GLM_CLEAR_THINKING")
        .map(|v| v != "false")
        .unwrap_or(true);
    body["thinking"] = json!({"type": "enabled", "clear_thinking": clear_thinking});

    if let Some(effort) = req.reasoning.as_ref().map(|r| r.effort.as_str()) {
        if effort == "none" {
            // 思考关不掉（官方：thinking.type 仅支持 enabled，2026-09 核实 Z.ai/BigModel 文档），
            body["reasoning_effort"] = json!("low");
        } else {
            body["reasoning_effort"] = json!(effort);
        }
    }
}

pub fn translate_request(req: &ResponsesRequest) -> Value {
    // 厂商怪癖从档案读（不按型号名硬编码）——新公司档案不声明 thinking 即走通用形态。
    let thinking = crate::model::catalog::provider_of(&req.model)
        .map(|p| p.thinking)
        .unwrap_or_default();
    let mut body = json!({
        "model": req.model,
        "stream": req.stream,
    });
    inject_thinking(&mut body, req, thinking);

    if let Some(inst) = &req.instructions {
        body["messages"] = json!([{"role": "system", "content": inst}]);
        append_items(&mut body, &req.input);
    } else {
        append_items(&mut body, &req.input);
    }

    if let Some(tools) = &req.tools {
        // Responses 扁平 ToolDef → chat/completions 嵌套 {type:function, function:{...}}
        let converted: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                })
            })
            .collect();
        body["tools"] = json!(converted);
    }
    if let Some(choice) = &req.tool_choice {
        body["tool_choice"] = json!(match choice {
            ToolChoice::None => "none",
            ToolChoice::Auto => "auto",
            ToolChoice::Required => "required",
        });
    }
    // GLM 流式工具调用增量需要 tool_stream；usage 随末尾 chunk 回传
    if req.stream {
        body["tool_stream"] = json!(true);
        body["stream_options"] = json!({"include_usage": true});
    }
    if let Some(t) = req.temperature {
        // GLM temperature 契约：最多 2 位小数（官方 1210 错误）。f32 直接序列化会带
        body["temperature"] = json!(((t as f64) * 100.0).round() / 100.0);
    }
    if let Some(n) = req.max_output_tokens {
        body["max_tokens"] = json!(n);
    }
    if let Some(fmt) = &req.text {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {"name": fmt.name, "strict": fmt.strict, "schema": fmt.schema},
        });
    }
    body
}

/// 把 Responses input items 追加为 chat messages。
fn append_items(body: &mut Value, items: &[InputItem]) {
    let wire = to_wire_items(items);
    let msgs = body["messages"].as_array().cloned().unwrap_or_default();
    let mut out = msgs;

    // 待发布的 assistant 消息（正文可能为空、tool_calls 随后合并进来）
    let mut pending_assistant: Option<Value> = None;
    // 无 assistant 可挂时的兜底分组（用户消息后直接跟 fc 的退化序列）
    let mut orphan_calls: Vec<Value> = Vec::new();
    fn flush(
        out: &mut Vec<Value>,
        pending: &mut Option<Value>,
        orphan: &mut Vec<Value>,
    ) {
        if let Some(pa) = pending.take() {
            out.push(pa);
        }
        if !orphan.is_empty() {
            out.push(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": orphan,
            }));
            orphan.clear();
        }
    }

    for item in wire {
        match item {
            InputItem::Message {
                role,
                content,
                tool_calls,
            } => {
                flush(&mut out, &mut pending_assistant, &mut orphan_calls);
                let role = if role == "developer" { "system" } else { &role };
                let mut m = json!({"role": role});
                m["content"] = translate_content(&content);
                if let Some(tcs) = tool_calls {
                    let converted: Vec<Value> = tcs
                        .iter()
                        .filter_map(|tc| {
                            let (id, name, args) = (
                                tc.get("call_id").and_then(|v| v.as_str()),
                                tc.get("name").and_then(|v| v.as_str()),
                                tc.get("arguments").and_then(|v| v.as_str()),
                            );
                            match (id, name) {
                                (Some(id), Some(name)) => Some(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": args.unwrap_or("{}"),
                                    },
                                })),
                                _ => None,
                            }
                        })
                        .collect();
                    if !converted.is_empty() {
                        m["tool_calls"] = json!(converted);
                    }
                }
                if role == "assistant" && m.get("tool_calls").is_none() {
                    // assistant 正文先挂起，等后续同轮 FunctionCall 并入
                    pending_assistant = Some(m);
                } else {
                    out.push(m);
                }
            }
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                let tc = json!({
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                });
                if let Some(pa) = pending_assistant.as_mut() {
                    // 并入挂起的 assistant 消息（同轮 text + tool_calls 同体）
                    let arr = pa.as_object_mut().unwrap().entry("tool_calls").or_insert(json!([]));
                    arr.as_array_mut().unwrap().push(tc);
                } else {
                    orphan_calls.push(tc);
                }
            }
            InputItem::FunctionCallOutput { call_id, output } => {
                flush(&mut out, &mut pending_assistant, &mut orphan_calls);
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
            InputItem::Reasoning { .. } | InputItem::Raw(_) => {}
        }
    }
    flush(&mut out, &mut pending_assistant, &mut orphan_calls);
    body["messages"] = json!(out);
}

/// Responses 内容块（input_text/output_text/input_image）→ GLM content。
fn translate_content(content: &Value) -> Value {
    match content {
        Value::String(s) => json!(s),
        Value::Array(parts) => {
            let mut texts: Vec<String> = Vec::new();
            let mut blocks: Vec<Value> = Vec::new();
            for p in parts {
                let t = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if t.contains("text") {
                    if let Some(text) = p.get("text").and_then(|v| v.as_str()) {
                        texts.push(text.to_string());
                        blocks.push(json!({"type": "text", "text": text}));
                    }
                } else if t == "input_image" {
                    // Real 块：{type:input_image, image_url:"data:..."} → GLM 块
                    let url = match p.get("image_url") {
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(Value::Object(o)) => o.get("url").and_then(|v| v.as_str()).map(String::from),
                        _ => None,
                    };
                    if let Some(u) = url {
                        blocks.push(json!({"type": "image_url", "image_url": {"url": u}}));
                    }
                }
            }
            if blocks.iter().any(|b| b["type"] == "image_url") {
                json!(blocks)
            } else {
                json!(texts.join(""))
            }
        }
        other => other.clone(),
    }
}

/// chat/completions usage → Real Usage（缓存命中从 prompt_tokens_details.cached_tokens 取）
fn translate_usage(u: &Value) -> Usage {
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // 思考 token：智谱流式 usage 可能放 completion_tokens_details.reasoning_tokens
    let reasoning = u
        .get("completion_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_u64())
        .or_else(|| u.get("reasoning_tokens").and_then(|v| v.as_u64()));
    Usage {
        input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        input_tokens_details: Some(json!({"cached_tokens": cached})),
        output_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens_details: None,
        reasoning_tokens: reasoning,
        total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()),
    }
}

// 流式解析：chat/completions chunk SSE → StreamResult

#[derive(Default)]
struct CallAcc {
    call_id: String,
    name: String,
    arguments: String,
}

/// 流式调用（可带思考/文本增量回调）。重试策略与 DeepSeek 路径一致
pub async fn stream_chat<'a>(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    req: &ResponsesRequest,
    mut on_reasoning: Option<&'a mut (dyn for<'b> FnMut(&'b str) + Send)>,
    mut on_text: Option<&'a mut (dyn for<'b> FnMut(&'b str) + Send)>,
) -> AppResult<StreamResult> {
    let body = translate_request(req);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let resp = match http
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if attempt <= 3 {
                    let backoff = std::time::Duration::from_millis(500 * (1 << attempt));
                    tracing::warn!(error = %e, attempt, "GLM 流式网络请求失败，退避重试");
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                return Err(AppError::Llm(format!("GLM 流式网络请求失败: {e}")));
            }
        };
        crate::model::debug_dump_req(req, &resp).await;
        let status = resp.status();
        // （限流自愈）429 = "稍后再来"的临时信号，不是终态错误：退避重试——
        if status.as_u16() == 429 && attempt <= 5 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(std::time::Duration::from_secs);
            let backoff = retry_after.unwrap_or_else(|| {
                std::time::Duration::from_millis((2000u64 << (attempt - 1)).min(15_000))
            });
            tracing::warn!(attempt, backoff_ms = backoff.as_millis() as u64, "GLM 限流（429），退避重试");
            tokio::time::sleep(backoff).await;
            continue;
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!(
                "GLM 流式 HTTP {}: {}",
                status,
                crate::model::llm::truncate_pub(&text, 300)
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut output_text = String::new();
        let mut reasoning = String::new();
        // index → 增量累积（GLM 流式工具调用按 index 分片：首片带 id/name，后续片 append arguments）
        let mut calls: std::collections::BTreeMap<u64, CallAcc> = Default::default();
        let mut usage: Option<Usage> = None;
        let mut finish: Option<String> = None;
        let error: Option<String> = None;
        let mut produced = false;

        let mut buf: Vec<u8> = Vec::new();
        let mut read_failed: Option<AppError> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    read_failed = Some(AppError::Llm(format!("GLM SSE 读取失败: {e}")));
                    break;
                }
            };
            buf.extend_from_slice(&chunk);
            while let Some(pos) = crate::model::llm::find_frame_pub(&buf) {
                let frame = buf.drain(..pos).collect::<Vec<_>>();
                let frame_str = String::from_utf8_lossy(&frame);
                let mut data: Option<&str> = None;
                for line in frame_str.lines() {
                    if let Some(rest) = line.strip_prefix("data:") {
                        data = Some(rest.trim());
                    }
                }
                let Some(data) = data else { continue };
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let Ok(chunk) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
                    usage = Some(translate_usage(u));
                }
                let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
                    continue;
                };
                if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                    finish = Some(f.to_string());
                }
                let Some(delta) = choice.get("delta") else { continue };
                if let Some(d) = delta.get("reasoning_content").and_then(|r| r.as_str()) {
                    if !d.is_empty() {
                        produced = true;
                        reasoning.push_str(d);
                        if let Some(cb) = on_reasoning.as_mut() {
                            cb(d);
                        }
                    }
                }
                if let Some(d) = delta.get("content").and_then(|c| c.as_str()) {
                    if !d.is_empty() {
                        produced = true;
                        output_text.push_str(d);
                        if let Some(cb) = on_text.as_mut() {
                            cb(d);
                        }
                    }
                }
                if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tcs {
                        let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        let acc = calls.entry(idx).or_default();
                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                            if !id.is_empty() {
                                acc.call_id = id.to_string();
                            }
                        }
                        if let Some(func) = tc.get("function") {
                            if let Some(n) = func.get("name").and_then(|v| v.as_str()) {
                                if !n.is_empty() {
                                    acc.name.push_str(n);
                                }
                            }
                            if let Some(a) = func.get("arguments").and_then(|v| v.as_str()) {
                                acc.arguments.push_str(a);
                            }
                        }
                        produced = true;
                    }
                }
            }
        }

        if let Some(err) = read_failed {
            if attempt <= 3 && !produced {
                let backoff = std::time::Duration::from_millis(500 * (1 << attempt));
                tracing::warn!(error = %err, attempt, "GLM SSE 冷启动断流，退避整体重试");
                tokio::time::sleep(backoff).await;
                output_text.clear();
                reasoning.clear();
                calls.clear();
                buf.clear();
                on_reasoning = None;
                on_text = None;
                continue;
            }
            return Err(err);
        }

        let function_calls: Vec<FunctionCall> = calls
            .into_values()
            .enumerate()
            .map(|(i, acc)| FunctionCall {
                call_id: if acc.call_id.is_empty() {
                    format!("call_glm_stream_{i}")
                } else {
                    acc.call_id
                },
                name: acc.name,
                arguments: if acc.arguments.trim().is_empty() {
                    "{}".into()
                } else {
                    acc.arguments
                },
            })
            .collect();

        let final_status = match finish.as_deref() {
            Some("length") => StreamStatus::Incomplete,
            Some("stop") | Some("tool_calls") | Some("function_call") | None => {
                StreamStatus::Completed
            }
            Some(other) => {
                tracing::warn!(finish = %other, "GLM 流式未知 finish_reason，按完成处理");
                StreamStatus::Completed
            }
        };
        if final_status == StreamStatus::Incomplete {
            tracing::warn!(
                sent_max_tokens = req.max_output_tokens.unwrap_or(0),
                out_tokens = usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
                reasoning_chars = reasoning.chars().count(),
                tool_calls = function_calls.len(),
                "GLM 响应被截断（max_tokens 或上下文限制）——比对 out_tokens 与 sent_max_tokens 定位侧别"
            );
        }

        // 流异常早停检测（彻底修复）：GLM 5.3 thinking 恒开，正常轮必有
        if stream_stalled(
            function_calls.is_empty(),
            usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
            reasoning.chars().count(),
            final_status == StreamStatus::Incomplete,
        ) {
            return Err(AppError::Llm(format!(
                "GLM 流异常早停：上游产出前断流（finish={}，completion_tokens={}，思考 {} 字）——已判异常而非正常空轮",
                finish.as_deref().unwrap_or("None"),
                usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
                reasoning.chars().count(),
            )));
        }

        return Ok(StreamResult {
            output_text,
            reasoning,
            function_calls,
            usage,
            status: final_status,
            error,
        });
    }
}

/// 流异常早停判定（修复 1-token 空转）：无工具调用 且 产出 ≤2 token 且
fn stream_stalled(no_fc: bool, out_tokens: u64, reasoning_chars: usize, incomplete: bool) -> bool {
    no_fc && out_tokens <= 2 && (reasoning_chars < 10 || incomplete)
}

#[cfg(test)]
#[path = "glm_adapter_tests.rs"]
mod glm_adapter_tests;
