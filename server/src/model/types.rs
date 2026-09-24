//! Responses API 契约类型：无状态（历史全量回传）、流式以 completed/incomplete/failed 收尾、arguments 二次解析

#![allow(dead_code)]
// API 契约类型：字段为 serde 反序列化所需（协议响应字段），编译器 dead_code 检查不适用

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// 请求侧

#[derive(Debug, Clone, Serialize)]
pub struct ReasoningConfig {
    /// none = 关闭思考（tool_choice 仅在关闭思考时可用，官方坑位 A6 补充）
    pub effort: String,
    /// thinking 模式带 tools 时，每个后续请求必须回传上一轮的 reasoning 原文
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl ReasoningConfig {
    pub fn none() -> Self {
        Self {
            effort: "none".into(),
            summary: None,
            content: None,
        }
    }
}

/// 工具定义（透传给 LLM 的 tools 参数）
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

impl ToolDef {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            type_: "function".into(),
            name: name.into(),
            description: description.into(),
            parameters,
            strict: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    None,
    Auto,
    Required,
}

/// text.format 结构化输出配置
#[derive(Debug, Clone, Serialize)]
pub struct JsonSchemaFormat {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
    pub schema: Value,
    #[serde(default = "default_true")]
    pub strict: bool,
}

fn default_true() -> bool {
    true
}

impl JsonSchemaFormat {
    pub fn new(name: &str, schema: Value) -> Self {
        Self {
            type_: "json_schema".into(),
            name: name.into(),
            schema,
            strict: true,
        }
    }
}

/// 输入 item：message / function_call / function_call_output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message {
        role: String,
        content: Value,
        /// （并行 fc 400 根治·assistant 聚合）：assistant 消息承载同轮 tool_calls
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<Value>>,
    },
    FunctionCall {
        #[serde(rename = "call_id")]
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        #[serde(rename = "call_id")]
        call_id: String,
        output: String,
    },
    /// 思考内容（reasoning_text）——thinking 模式要求下一步回传
    Reasoning {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<Value>>,
    },
    /// 原样回传（DeepSeek 思考模式：reasoning 可能含 content/summary 两种结构，
    Raw(Value),
}

impl InputItem {
    pub fn user_message(text: &str) -> Self {
        InputItem::Message {
            role: "user".into(),
            content: json!([{"type": "input_text", "text": text}]),
            tool_calls: None,
        }
    }

    /// 带内容块的 user 消息（**统一图片通道**用）：工具产出的图片按「用户附件」回灌，
    pub fn user_message_blocks(content: Value) -> Self {
        InputItem::Message {
            role: "user".into(),
            content,
            tool_calls: None,
        }
    }

    pub const IMAGE_ATTACHMENT_MARK: &'static str = "📎 工具 `";

    /// 这个输入项是不是"工具返回的图片附件块"（只该活一轮 → 轮首摘除）。
    pub fn is_ephemeral_image_attachment(&self) -> bool {
        let InputItem::Message { role, content, .. } = self else {
            return false;
        };
        if role != "user" {
            return false;
        }
        content
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.starts_with(Self::IMAGE_ATTACHMENT_MARK))
    }

    pub fn assistant_message(text: &str) -> Self {
        InputItem::Message {
            role: "assistant".into(),
            content: json!([{"type": "output_text", "text": text}]),
            tool_calls: None,
        }
    }

    /// 模型一轮响应（正文 + 同轮全部 tool_calls）聚合为**一条 assistant 消息**。content 仅含
    pub fn assistant_with_tools(_reasoning: &str, text: &str, tool_calls: &[FunctionCall]) -> Self {
        // （reasoning 不进 content 修复）：content 仅放 output_text。目标 API 只接受
        let mut content: Vec<Value> = Vec::new();
        if !text.trim().is_empty() {
            content.push(json!({"type": "output_text", "text": text}));
        }
        let tcs: Vec<Value> = tool_calls
            .iter()
            .map(|fc| {
                json!({
                    "type": "function_call",
                    "call_id": fc.call_id,
                    "name": fc.name,
                    "arguments": fc.arguments,
                })
            })
            .collect();
        InputItem::Message {
            role: "assistant".into(),
            content: json!(content),
            tool_calls: if tcs.is_empty() { None } else { Some(tcs) },
        }
    }

    /// 该 assistant 消息是否携带 tool_calls（工具轮判定/落库用）
    pub fn assistant_tool_call_ids(&self) -> Vec<String> {
        match self {
            InputItem::Message {
                role,
                tool_calls: Some(tcs),
                ..
            } if role == "assistant" => tcs
                .iter()
                .filter_map(|tc| tc.get("call_id").and_then(|c| c.as_str()).map(String::from))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn function_call(call_id: &str, name: &str, arguments: &str) -> Self {
        InputItem::FunctionCall {
            call_id: call_id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    /// DeepSeek thinking 带 tools 的多轮必须回传上一轮 reasoning 原文。
    pub fn reasoning_item(text: &str) -> Self {
        InputItem::Reasoning {
            content: Some(json!([{"type": "reasoning_text", "text": text}])),
            summary: None,
        }
    }

    pub fn function_call_output(call_id: &str, output: &str) -> Self {
        InputItem::FunctionCallOutput {
            call_id: call_id.into(),
            output: output.into(),
        }
    }

    /// 从模型响应的 OutputItem 构造 InputItem（含 reasoning，满足 DeepSeek 回传要求）
    pub fn from_output_item(item: &crate::model::types::OutputItem) -> Self {
        match item {
            crate::model::types::OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => InputItem::FunctionCall {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            },
            crate::model::types::OutputItem::Reasoning { content, summary } => {
                // DeepSeek 思考模式：reasoning 必须用 content 且为 reasoning_text 块数组回传
                match (content, summary) {
                    (Some(c), _) => InputItem::Reasoning {
                        content: Some(c.clone()),
                        summary: None,
                    },
                    (None, Some(s)) => InputItem::Reasoning {
                        content: None,
                        summary: Some(s.to_vec()),
                    },
                    (None, None) => InputItem::Reasoning {
                        content: Some(json!([])),
                        summary: None,
                    },
                }
            }
            crate::model::types::OutputItem::Message { content, .. } => {
                let c: Vec<Value> = content
                    .iter()
                    .filter_map(|c| match c.text() {
                        Some(t) => Some(json!({"type": "output_text", "text": t})),
                        None => None,
                    })
                    .collect();
                InputItem::Message {
                    role: "assistant".into(),
                    content: Value::Array(c),
                    tool_calls: None,
                }
            }
            _ => InputItem::Message {
                role: "assistant".into(),
                content: json!([]),
                tool_calls: None,
            },
        }
    }
}

/// 把存储/聚合态的 item 序列展开为 **API 线格式**。
pub fn normalize_raw_item(v: &serde_json::Value) -> Option<InputItem> {
    if v.get("type").and_then(|t| t.as_str()).is_none() {
        // 形状兜底（无 type 键时按字段形状判定，确定性，不猜语义）
        if v.get("call_id").is_some() && v.get("output").is_some() {
            return Some(InputItem::FunctionCallOutput {
                call_id: v.get("call_id")?.as_str()?.to_string(),
                output: v.get("output")?.as_str()?.to_string(),
            });
        }
        if v.get("call_id").is_some() && v.get("name").is_some() && v.get("arguments").is_some() {
            return Some(InputItem::FunctionCall { 
                call_id: v.get("call_id")?.as_str()?.to_string(),
                name: v.get("name")?.as_str()?.to_string(),
                arguments: v.get("arguments")?.as_str()?.to_string(),
            });
        }
        if v.get("role").is_some() && v.get("content").is_some() {
            return Some(InputItem::Message {
                role: v.get("role")?.as_str()?.to_string(),
                content: v.get("content").cloned().unwrap_or(serde_json::Value::Null),
                tool_calls: v.get("tool_calls").and_then(|t| t.as_array()).cloned(),
            });
        }
        return None;
    }
    let ty = v.get("type").and_then(|t| t.as_str())?;
    match ty {
        "function_call_output" => Some(InputItem::FunctionCallOutput {
            call_id: v.get("call_id")?.as_str()?.to_string(),
            output: v.get("output")?.as_str()?.to_string(),
        }),
        "function_call" => Some(InputItem::FunctionCall {
            call_id: v.get("call_id")?.as_str()?.to_string(),
            name: v.get("name")?.as_str()?.to_string(),
            arguments: v.get("arguments")?.as_str()?.to_string(),
        }),
        "message" => Some(InputItem::Message {
            role: v.get("role")?.as_str()?.to_string(),
            content: v.get("content").cloned().unwrap_or(serde_json::Value::Null),
            tool_calls: v.get("tool_calls").and_then(|t| t.as_array()).cloned(),
        }),
        "reasoning" => Some(InputItem::Reasoning {
            content: v.get("content").cloned(),
            summary: None,
        }),
        _ => None,
    }
}

pub fn to_wire_items(items: &[InputItem]) -> Vec<InputItem> {
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        match it {
            InputItem::Message {
                role,
                content,
                tool_calls: Some(tcs),
            } => {
                // 1) assistant 消息：剥离 tool_calls（已拆为独立条目），content 原样保留
                out.push(InputItem::Message {
                    role: role.clone(),
                    content: content.clone(),
                    tool_calls: None,
                });
                // 2) 每个聚合的 tool_call → 独立 function_call 条目（API 期望的顶层 item）
                for tc in tcs {
                    let (call_id, name, arguments) = (
                        tc.get("call_id").and_then(|v| v.as_str()),
                        tc.get("name").and_then(|v| v.as_str()),
                        tc.get("arguments").and_then(|v| v.as_str()),
                    );
                    if let (Some(call_id), Some(name), Some(arguments)) = (call_id, name, arguments) {
                        out.push(InputItem::function_call(call_id, name, arguments));
                    }
                }
            }
            InputItem::Raw(v) => {
                // 具名类型逐项还原（否则全局以 type:"raw" 上送 → 配对契约失败）
                match normalize_raw_item(v) {
                    Some(named) => out.push(named),
                    None => out.push(InputItem::Raw(v.clone())),
                }
            }
            other => out.push(other.clone()),
        }
    }
    keep_call_output_adjacent(out)
}

pub fn validate_wire_items(items: &[InputItem]) -> Option<String> {
    use std::collections::HashSet;
    let mut declared: Vec<String> = Vec::new();
    let mut answered: HashSet<String> = HashSet::new();
    for (i, it) in items.iter().enumerate() {
        match it {
            InputItem::Raw(_) => {
                return Some(format!("第 {i} 项仍是裸 Raw（类型未归一）"));
            }
            InputItem::FunctionCall { call_id, .. } => declared.push(call_id.clone()),
            InputItem::FunctionCallOutput { call_id, .. } => {
                answered.insert(call_id.clone());
            }
            InputItem::Message {
                role,
                tool_calls: Some(tcs),
                ..
            } => {
                if role == "user" && !declared.is_empty() {
                    if let Some(last) = declared.last() {
                        if !answered.contains(last) {
                            return Some(format!("输出未就绪时插入了 user 消息（调用 {last} 被打断）"));
                        }
                    }
                }
                for tc in tcs {
                    if let Some(c) = tc.get("call_id").and_then(|v| v.as_str()) {
                        declared.push(c.to_string());
                    }
                }
            }
            InputItem::Message { role, .. } => {
                if role == "user" && !declared.is_empty() {
                    if let Some(last) = declared.last() {
                        if !answered.contains(last) {
                            return Some(format!("输出未就绪时插入了 user 消息（调用 {last} 被打断）"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for c in &declared {
        if !answered.contains(c) {
            return Some(format!("调用 {c} 无配对输出"));
        }
    }
    None
}

/// 见 to_wire_items 注释：把夹在"未闭合调用"与其输出之间的 user 消息后移。
pub fn keep_call_output_adjacent(items: Vec<InputItem>) -> Vec<InputItem> {
    let mut out: Vec<InputItem> = Vec::with_capacity(items.len());
    let mut pending: Vec<String> = Vec::new();
    let mut deferred_users: Vec<InputItem> = Vec::new();
    for it in items {
        match &it {
            InputItem::FunctionCall { call_id, .. } => {
                pending.push(call_id.clone());
                out.push(it);
            }
            InputItem::Message {
                tool_calls: Some(tcs),
                ..
            } => {
                for tc in tcs {
                    if let Some(c) = tc.get("call_id").and_then(|v| v.as_str()) {
                        pending.push(c.to_string());
                    }
                }
                out.push(it);
            }
            InputItem::FunctionCallOutput { call_id, .. } => {
                pending.retain(|c| c != call_id);
                out.push(it);
                if pending.is_empty() {
                    out.append(&mut deferred_users);
                }
            }
            InputItem::Message { role, .. } if role == "user" && !pending.is_empty() => {
                deferred_users.push(it);
            }
            _ => out.push(it),
        }
    }
    out.append(&mut deferred_users);
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<InputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<JsonSchemaFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl ResponsesRequest {
    pub fn builder(model: impl Into<String>) -> ResponsesRequestBuilder {
        ResponsesRequestBuilder {
            req: ResponsesRequest {
                model: model.into(),
                instructions: None,
                input: vec![],
                tools: None,
                tool_choice: None,
                stream: false,
                temperature: None,
                max_output_tokens: None,
                text: None,
                reasoning: None,
                user: None,
            },
        }
    }
}

pub struct ResponsesRequestBuilder {
    req: ResponsesRequest,
}

impl ResponsesRequestBuilder {
    pub fn instructions(mut self, s: impl Into<String>) -> Self {
        self.req.instructions = Some(s.into());
        self
    }
    pub fn input(mut self, items: Vec<InputItem>) -> Self {
        self.req.input = items;
        self
    }
    pub fn tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.req.tools = Some(tools);
        self
    }
    pub fn tool_choice(mut self, c: ToolChoice) -> Self {
        self.req.tool_choice = Some(c);
        self
    }
    pub fn stream(mut self, s: bool) -> Self {
        self.req.stream = s;
        self
    }
    pub fn temperature(mut self, t: f32) -> Self {
        self.req.temperature = Some(t);
        self
    }
    pub fn max_output_tokens(mut self, n: u32) -> Self {
        self.req.max_output_tokens = Some(n);
        self
    }
    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.req.reasoning = Some(ReasoningConfig {
            effort: effort.into(),
            summary: None,
            content: None,
        });
        self
    }
    pub fn user(mut self, u: impl Into<String>) -> Self {
        self.req.user = Some(u.into());
        self
    }
    pub fn build(self) -> ResponsesRequest {
        self.req
    }
}

// 响应侧

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: Option<Value>,
    pub output_tokens: u64,
    #[serde(default)]
    pub output_tokens_details: Option<Value>,
    /// 思考 token 拆分（设计决定："思考输出要自我克制"——没有度量就没有克制；
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

/// 响应中的输出 item（内部 tag 枚举，未知类型兜底 Unknown）
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        content: Vec<OutputContent>,
        #[serde(default)]
        role: Option<String>,
    },
    FunctionCall {
        #[serde(default)]
        id: Option<String>,
        #[serde(rename = "call_id")]
        call_id: String,
        name: String,
        arguments: String,
        #[serde(default)]
        status: Option<String>,
    },
    Reasoning {
        // DeepSeek 思考模式返回 content 数组（[{type:text,text:..}]）或字符串——用 Value 容忍
        #[serde(default)]
        content: Option<Value>,
        #[serde(default)]
        summary: Option<Vec<Value>>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContent {
    OutputText {
        text: String,
    },
    InputText {
        text: String,
    },
    Refusal {
        text: String,
    },
    #[serde(other)]
    Other,
}

impl OutputContent {
    pub fn text(&self) -> Option<&str> {
        match self {
            OutputContent::OutputText { text } | OutputContent::InputText { text } => Some(text),
            _ => None,
        }
    }
}

/// 非流式完整响应对象
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseObject {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub output_text: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub error: Option<Value>,
}

impl ResponseObject {
    /// 提取全部 function_call（并行工具调用恒开，可能多个）
    pub fn function_calls(&self) -> Vec<FunctionCall> {
        self.output
            .iter()
            .filter_map(|o| match o {
                OutputItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => Some(FunctionCall {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    pub fn output_text(&self) -> String {
        self.output_text.clone().unwrap_or_else(|| {
            self.output
                .iter()
                .filter_map(|o| match o {
                    OutputItem::Message { content, .. } => Some(
                        content
                            .iter()
                            .filter_map(|c| c.text().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                            .join(""),
                    ),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
    }
}

/// 一次工具调用意图（模型出决策，后端执行）
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// 流式事件（SSE data 字段）
#[derive(Debug, Clone, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub sequence_number: Option<u64>,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub item: Option<OutputItem>,
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub response: Option<ResponseObject>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamStatus {
    Completed,
    Incomplete,
    Failed,
}

/// 流式调用的聚合结果
#[derive(Debug, Clone)]
pub struct StreamResult {
    pub output_text: String,
    /// 思考内容增量（DeepSeek 思考模式 `response.reasoning_summary_text.delta`，
    pub reasoning: String,
    pub function_calls: Vec<FunctionCall>,
    pub usage: Option<Usage>,
    pub status: StreamStatus,
    pub error: Option<String>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;
