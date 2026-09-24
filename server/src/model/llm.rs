//! DeepSeek Responses API 客户端：Llm trait（Real 真实调用 / Mock 内置假模型），SSE 手动分帧解析

use crate::config::{Config, LlmMode};
use crate::error::{AppError, AppResult};
use crate::model::glm_adapter;
use crate::config::settings::{RuntimeSettings, SettingsRef};
use crate::model::types::*;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

#[async_trait]
/// 非流式 complete 整条链路随兜底 LLM 总结删除（编排只走流式）。
pub trait Llm: Send + Sync {
    /// 流式响应（用户可见的流式输出 + 实时 function calling）
    async fn stream_complete(&self, req: &ResponsesRequest) -> AppResult<StreamResult>;

    /// 流式 + 思考增量实时回调（Planner"推演中"流式透传：reasoning delta 逐段回调）
    async fn stream_reasoning<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult>;

    /// 流式 + 思考增量 + **输出文本增量**实时回调（打字机）
    async fn stream_reasoning_with_text<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
        on_text: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        let _ = on_text;
        self.stream_reasoning(req, on_reasoning).await
    }
}

/// 构建 LLM：返回（可切换实现，运行时设置句柄）
pub fn build_llm(cfg: &Arc<Config>) -> (Arc<dyn Llm>, SettingsRef) {
    let settings: SettingsRef = Arc::new(RwLock::new(RuntimeSettings::from_config(cfg)));
    let real = RealLlm::new(settings.clone());
    let switchable = SwitchableLlm {
        settings: settings.clone(),
        real,
        mock: MockLlm,
    };
    (Arc::new(switchable), settings)
}

/// 可切换 LLM：按运行时 llm_mode 分发到 Real / Mock
pub struct SwitchableLlm {
    settings: SettingsRef,
    real: RealLlm,
    mock: MockLlm,
}

#[async_trait]
impl Llm for SwitchableLlm {
    async fn stream_complete(&self, req: &ResponsesRequest) -> AppResult<StreamResult> {
        match self.settings.read().await.llm_mode {
            LlmMode::Real => self.real.stream_complete(req).await,
            LlmMode::Mock => self.mock.stream_complete(req).await,
        }
    }

    async fn stream_reasoning<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        match self.settings.read().await.llm_mode {
            LlmMode::Real => self.real.stream_reasoning(req, on_reasoning).await,
            LlmMode::Mock => self.mock.stream_reasoning(req, on_reasoning).await,
        }
    }

    async fn stream_reasoning_with_text<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
        on_text: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        match self.settings.read().await.llm_mode {
            LlmMode::Real => {
                self.real
                    .stream_reasoning_with_text(req, on_reasoning, on_text)
                    .await
            }
            LlmMode::Mock => {
                self.mock
                    .stream_reasoning_with_text(req, on_reasoning, on_text)
                    .await
            }
        }
    }
}

// Real

pub struct RealLlm {
    http: reqwest::Client,
    settings: SettingsRef,
}

impl RealLlm {
    pub fn new(settings: SettingsRef) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                // 连接超时收紧：DeepSeek 不可达时快速失败，避免编排傻等（坑位 A10 补充）
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("构建 HTTP client 失败"),
            settings,
        }
    }

    /// 读取当前运行时凭据（每次调用实时读取，保证设置即时生效）。
    async fn snapshot_for(&self, model: &str) -> (String, String) {
        let s = self.settings.read().await;
        let (base_url, api_key) = s.credentials_for(model);
        (
            base_url.trim_end_matches('/').to_string(),
            api_key,
        )
    }

    fn endpoint(&self, base_url: &str) -> String {
        format!("{base_url}/responses")
    }

    /// 发送前清洗（坑位 F1 兜底防线）：DeepSeek thinking 模式（reasoning.effort != none）
    fn sanitize(req: &ResponsesRequest) -> Option<ResponsesRequest> {
        let thinking = req
            .reasoning
            .as_ref()
            .map(|r| r.effort.as_str() != "none")
            .unwrap_or(false);
        // thinking 模式**仅**与 tool_choice=required 冲突（坑位 F1，HTTP 400）。
        let required = matches!(req.tool_choice, Some(ToolChoice::Required));
        if req.tools.is_none() {
            let before = req.input.len();
            let mut clean = req.clone();
            clean.input.retain(|it| !matches!(it, InputItem::Reasoning { .. }));
            if clean.input.len() != before {
                return Some(clean);
            }
        }
        if thinking && required {
            tracing::warn!(
                effort = %req.reasoning.as_ref().map(|r| r.effort.as_str()).unwrap_or(""),
                "thinking 模式不支持 tool_choice，发送层降级关闭思考（保工具调用契约）"
            );
            let mut clean = req.clone();
            clean.reasoning = Some(ReasoningConfig::none());
            Some(clean)
        } else {
            None
        }
    }

    /// 流式主体（可带思考增量 + 输出文本增量实时回调——Planner 流式透传）
    async fn post_stream_inner<'a>(
        &self,
        req: &ResponsesRequest,
        mut on_reasoning: Option<&'a mut (dyn for<'b> FnMut(&'b str) + Send)>,
        mut on_text: Option<&'a mut (dyn for<'b> FnMut(&'b str) + Send)>,
    ) -> AppResult<StreamResult> {
        let owned;
        let req = if let Some(clean) = Self::sanitize(req) {
            owned = clean;
            &owned
        } else {
            req
        };
        let (base_url, api_key) = self.snapshot_for(&req.model).await;
        // 修复（用户反馈"SSE 读取失败: error decoding response body"）
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = match self
                .http
                .post(self.endpoint(&base_url))
                .bearer_auth(&api_key)
                .json(req)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if attempt <= 3 {
                        let backoff = std::time::Duration::from_millis(500 * (1 << attempt));
                        tracing::warn!(error = %e, attempt, "LLM 流式网络请求失败，退避重试");
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(AppError::Llm(format!("LLM 流式网络请求失败: {e}")));
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
                tracing::warn!(attempt, backoff_ms = backoff.as_millis() as u64, "LLM 限流（429），退避重试");
                tokio::time::sleep(backoff).await;
                continue;
            }
            let sc = status.as_u16();
            if (status.is_server_error() || sc == 408) && attempt <= 5 {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(std::time::Duration::from_secs);
                let backoff = retry_after.unwrap_or_else(|| {
                    std::time::Duration::from_millis((2000u64 << (attempt - 1)).min(15_000))
                });
                tracing::warn!(
                    attempt, status = sc, backoff_ms = backoff.as_millis() as u64,
                    "上游繁忙（HTTP {sc}），退避重试"
                );
                tokio::time::sleep(backoff).await;
                continue;
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                if status.as_u16() == 402 {
                    tracing::error!(
                        status = 402,
                        "DeepSeek 账户余额不足——后端本身正常，是账户没钱了。\
                         请到 platform.deepseek.com 充值后重试（无需重启 Real）"
                    );
                }
                // （400 定位辅助）：失败时把**实际发出的完整请求 input JSON** 落盘
                if let Ok(json) = serde_json::to_string(&req.input) {
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(crate::path::data_root::req_dump_path())
                    {
                        use std::io::Write;
                        let _ = writeln!(
                            f,
                            "\n===== 400 at {} =====\n{}",
                            chrono::Utc::now().to_rfc3339(),
                            json
                        );
                    }
                }
                return Err(AppError::Llm(format!(
                    "LLM 流式 HTTP {}: {}{}",
                    status,
                    truncate(&body, 300),
                    recharge_hint(status, &body)
                )));
            }

            let mut stream = resp.bytes_stream();

            let mut output_text = String::new();
            let mut reasoning = String::new();
            let mut function_calls: Vec<FunctionCall> = Vec::new();
            let mut usage: Option<Usage> = None;
            let mut final_status = StreamStatus::Completed;
            let mut error: Option<String> = None;
            // （并行 fc 空参数）：completed 事件的完整 response（含完整 arguments），
            let mut completed_resp: Option<ResponseObject> = None;

            let mut buf: Vec<u8> = Vec::new();
            let mut read_failed: Option<AppError> = None;
            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        read_failed = Some(AppError::Llm(format!("SSE 读取失败: {e}")));
                        break;
                    }
                };
                buf.extend_from_slice(&chunk);

                // 按空行分帧（SSE 帧分隔符为 \n\n）
                while let Some(pos) = find_frame(&buf) {
                    let frame = buf.drain(..pos).collect::<Vec<_>>();
                    let frame_str = String::from_utf8_lossy(&frame);

                    let mut data: Option<&str> = None;
                    for line in frame_str.lines() {
                        if let Some(rest) = line.strip_prefix("data:") {
                            data = Some(rest.trim());
                        }
                    }
                    let Some(data) = data else { continue };
                    if data.is_empty() {
                        continue;
                    }

                    let ev: StreamEvent = match serde_json::from_str(data) {
                        Ok(e) => e,
                        Err(_) => {
                            tracing::debug!(data = %truncate(data, 120), "跳过无法解析的事件帧");
                            continue;
                        }
                    };

                    match ev.type_.as_str() {
                        "response.output_text.delta" => {
                            if let Some(d) = ev.delta {
                                output_text.push_str(&d);
                                if let Some(cb) = on_text.as_mut() {
                                    cb(&d);
                                }
                            }
                        }
                        "response.reasoning_summary_text.delta"
                        | "response.reasoning_text.delta" => {
                            // 思考流式增量（summary 摘要 + 完整思维链都收集）：透传前端"思考"实时显示
                            if let Some(d) = ev.delta {
                                reasoning.push_str(&d);
                                if let Some(cb) = on_reasoning.as_mut() {
                                    cb(&d);
                                }
                            }
                        }
                        "response.output_item.done" => {
                            if let Some(OutputItem::FunctionCall {
                                call_id,
                                name,
                                arguments,
                                ..
                            }) = ev.item
                            {
                                function_calls.push(FunctionCall {
                                    call_id,
                                    name,
                                    arguments,
                                });
                            }
                        }
                        "response.completed" => {
                            final_status = StreamStatus::Completed;
                            if let Some(r) = &ev.response {
                                usage = r.usage.clone();
                                completed_resp = Some(r.clone());
                            }
                        }
                        "response.incomplete" => {
                            final_status = StreamStatus::Incomplete;
                            if let Some(r) = &ev.response {
                                usage = r.usage.clone();
                            }
                            tracing::warn!(
                                sent_max_tokens = req.max_output_tokens.unwrap_or(0),
                                out_tokens = usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
                                "LLM 响应被截断（max_output_tokens 或上下文限制）——比对 out_tokens 与 sent_max_tokens 定位侧别"
                            );
                        }
                        "response.failed" => {
                            final_status = StreamStatus::Failed;
                            error = ev.error.as_ref().map(|e| e.to_string());
                            tracing::error!(error = ?ev.error, "LLM 流式响应失败");
                        }
                        _ => {}
                    }
                }
            }

            if let Some(err) = read_failed {
                // 二次修复（用户反馈"SSE 读取失败: error decoding response body"仍出现）
                if attempt <= 3 {
                    let backoff = std::time::Duration::from_millis(500 * (1 << attempt));
                    tracing::warn!(error = %err, attempt, produced = output_text.len(), calls = function_calls.len(), "SSE 流中断，退避整体重试（丢弃半截增量）");
                    tokio::time::sleep(backoff).await;
                    output_text.clear();
                    reasoning.clear();
                    function_calls.clear();
                    buf.clear();
                    on_reasoning = None;
                    on_text = None;
                    continue;
                }
                return Err(err);
            }

            return Ok(StreamResult {
                output_text,
                reasoning,
                function_calls: resolve_function_calls(
                    function_calls,
                    &final_status,
                    completed_resp.as_ref(),
                ),
                usage,
                status: final_status,
                error,
            });
        }
    }
}

fn resolve_function_calls(
    streamed: Vec<FunctionCall>,
    status: &StreamStatus,
    completed: Option<&ResponseObject>,
) -> Vec<FunctionCall> {
    match (status, completed) {
        (StreamStatus::Completed, Some(r)) => {
            // completed 非空参数索引：call_id -> arguments（只收非空，空参不算数）
            let mut full_args: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for fc in r.function_calls() {
                if is_fc_args_nonempty(&fc.arguments) {
                    full_args
                        .entry(fc.call_id.clone())
                        .or_insert(fc.arguments.clone());
                }
            }
            if full_args.is_empty() {
                // completed 全空参（API 源头丢参）→ 保留 streamed 原样（含空参，交给教学）
                return streamed;
            }
            let mut merged: Vec<FunctionCall> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for fc in &streamed {
                if !is_fc_args_nonempty(&fc.arguments) {
                    if let Some(args) = full_args.get(&fc.call_id) {
                        // streamed 空参 → 用 completed 同 call_id 的非空参数补全
                        merged.push(FunctionCall {
                            arguments: args.clone(),
                            ..fc.clone()
                        });
                        seen.insert(fc.call_id.clone());
                        continue;
                    }
                }
                merged.push(fc.clone());
                seen.insert(fc.call_id.clone());
            }
            // completed 有但 streamed 缺的 fc（output_item.done 事件整体缺失）→ 补上
            for fc in r.function_calls() {
                if !seen.contains(&fc.call_id) && is_fc_args_nonempty(&fc.arguments) {
                    merged.push(fc.clone());
                    seen.insert(fc.call_id.clone());
                }
            }
            if merged.is_empty() {
                streamed.clone()
            } else {
                merged
            }
        }
        _ => streamed,
    }
}

/// arguments 是否"有效"：非空串且不是空对象 {}
fn is_fc_args_nonempty(arguments: &str) -> bool {
    let t = arguments.trim();
    !t.is_empty() && t != "{}"
}

/// 在缓冲区中找 SSE 帧边界（\n\n 或 \r\n\r\n），返回帧结束位置（含分隔符）
fn find_frame(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 2);
        }
        if buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf.get(i + 2) == Some(&b'\r')
            && buf.get(i + 3) == Some(&b'\n')
        {
            return Some(i + 4);
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// 把可辨识的上游错误码翻译成**可执行的下一步**（附加在原始报文之后，不改写报文）。
fn recharge_hint(status: reqwest::StatusCode, body: &str) -> &'static str {
    match status.as_u16() {
        402 => "  ← 账户余额不足，非代码故障。到 platform.deepseek.com 充值后直接重试，无需重启 Real",
        401 if body.contains("Authentication") || body.contains("invalid_api_key") => {
            "  ← API Key 无效或已撤销，到设置面板重新填入"
        }
        400 if body.contains("model") => {
            "  ← 模型名可能不被上游接受（API 名与版本名是两个口径，见 models.json 的 id/label）"
        }
        _ => "",
    }
}

/// glm_adapter 复用的帧解析/截断助手（保持本文件内私有实现单源）
pub(crate) fn find_frame_pub(buf: &[u8]) -> Option<usize> {
    find_frame(buf)
}

pub(crate) fn truncate_pub(s: &str, max: usize) -> String {
    truncate(s, max)
}

#[async_trait]
impl Llm for RealLlm {
    async fn stream_complete(&self, req: &ResponsesRequest) -> AppResult<StreamResult> {
        let req = &*crate::model::catalog::clamp_request(req);
        let mut req = req.clone();
        req.stream = true;
        if glm_adapter::uses_chat_protocol(&req.model) {
            return self.stream_chat_protocol(&req, None, None).await;
        }
        self.post_stream_inner(&req, None, None).await
    }

    async fn stream_reasoning<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        let req = &*crate::model::catalog::clamp_request(req);
        let mut req = req.clone();
        req.stream = true;
        if glm_adapter::uses_chat_protocol(&req.model) {
            return self.stream_chat_protocol(&req, Some(on_reasoning), None).await;
        }
        self.post_stream_inner(&req, Some(on_reasoning), None).await
    }

    async fn stream_reasoning_with_text<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
        on_text: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        let req = &*crate::model::catalog::clamp_request(req);
        let mut req = req.clone();
        req.stream = true;
        if glm_adapter::uses_chat_protocol(&req.model) {
            return self
                .stream_chat_protocol(&req, Some(on_reasoning), Some(on_text))
                .await;
        }
        self.post_stream_inner(&req, Some(on_reasoning), Some(on_text))
            .await
    }
}

impl RealLlm {
    fn chat_endpoint(&self, base_url: &str) -> String {
        format!("{base_url}/chat/completions")
    }

    /// chat/completions 协议通道（GLM 及一切 OpenAI 兼容厂商共用；厂商怪癖由档案声明）
    async fn stream_chat_protocol<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: Option<&'a mut (dyn for<'b> FnMut(&'b str) + Send)>,
        on_text: Option<&'a mut (dyn for<'b> FnMut(&'b str) + Send)>,
    ) -> AppResult<StreamResult> {
        let (base_url, api_key) = self.snapshot_for(&req.model).await;
        glm_adapter::stream_chat(
            &self.http,
            &self.chat_endpoint(&base_url),
            &api_key,
            req,
            on_reasoning,
            on_text,
        )
        .await
    }
}

// Mock（内置假模型）

pub struct MockLlm;

impl MockLlm {
    /// 从 input 中提取最后一条用户文本（供假模型"理解"任务）
    fn user_text(req: &ResponsesRequest) -> String {
        req.input
            .iter()
            .rev()
            .find_map(|item| {
                if let InputItem::Message { role, content, .. } = item {
                    if role == "user" {
                        if let Some(arr) = content.as_array() {
                            for part in arr {
                                if let (Some(t), Some(text)) = (
                                    part.get("type").and_then(|v| v.as_str()),
                                    part.get("text").and_then(|v| v.as_str()),
                                ) {
                                    if t.contains("text") {
                                        return Some(text.to_string());
                                    }
                                }
                            }
                        }
                        if let Some(s) = content.as_str() {
                            return Some(s.to_string());
                        }
                    }
                }
                None
            })
            .unwrap_or_default()
    }

    /// Mock 意图分流启发式（**仅测试替身用**——模拟"模型自己判断聊天/干活）。
    fn is_smalltalk(text: &str) -> bool {
        let t = if let Some(idx) = text.find("【用户任务】") {
            let rest = &text[idx + "【用户任务】".len()..];
            rest.split("\n\n").next().unwrap_or(rest).trim()
        } else {
            text.trim()
        };
        if t.is_empty() || t.chars().count() > 12 {
            return false;
        }
        const TASK_HINTS: &[&str] = &[
            "帮我", "看看", "检查", "修复", "分析", "查找", "生成", "写", "读", "打开", "安装",
            "编译", "构建", "测试", "运行", "删除", "移动", "整理", "对比", "优化", "重构", "更新",
            "总结", "计算", "查一",
        ];
        if TASK_HINTS.iter().any(|h| t.contains(h)) {
            return false;
        }
        const SMALL: &[&str] = &[
            "你好",
            "您好",
            "嗨",
            "哈喽",
            "hello",
            "hi",
            "hey",
            "在吗",
            "谢谢",
            "感谢",
            "心情",
            "吃",
            "天气",
            "你是谁",
            "能做什么",
            "干嘛",
            "无聊",
            "再见",
            "拜拜",
            "没事",
        ];
        SMALL.iter().any(|w| t.to_lowercase().contains(w))
    }

}

#[async_trait]
impl Llm for MockLlm {
    async fn stream_complete(&self, req: &ResponsesRequest) -> AppResult<StreamResult> {
        let text = Self::user_text(req);

        // Mock 意图分流（：planner 改 tool_choice=auto 后，Mock 也要模拟
        if Self::is_smalltalk(&text) {
            let mut out = String::from("（Mock 直答）收到：");
            out.push_str(&text);
            for _ in 0..3 {
                out.push_str("…");
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            }
            return Ok(StreamResult {
                output_text: out,
                reasoning: String::new(),
                function_calls: vec![],
                usage: None,
                status: StreamStatus::Completed,
                error: None,
            });
        }

        // 任务路径：调 submit_plan（Auto 下也调，模拟模型判断为任务——保 mock 完整流程演示）。
        if req
            .tools
            .as_ref()
            .map(|t| t.iter().any(|x| x.name == "submit_plan"))
            .unwrap_or(false)
        {
            let plan = json!({
                "objective": text,
                "steps": [
                    {
                        "step_id": "#E1",
                        "description": "回显用户输入（演示内建工具）",
                        "tool_name": "demo_echo",
                        "tool_args": {"message": text},
                        "depends_on": []
                    },
                    {
                        "step_id": "#E2",
                        "description": "获取当前 UTC 时间",
                        "tool_name": "demo_now",
                        "tool_args": {},
                        "depends_on": []
                    }
                ]
            });
            return Ok(StreamResult {
                output_text: String::new(),
                reasoning: String::new(),
                function_calls: vec![crate::model::types::FunctionCall {
                    call_id: "call_mock_plan".into(),
                    name: "submit_plan".into(),
                    arguments: plan.to_string(),
                }],
                usage: None,
                status: StreamStatus::Completed,
                error: None,
            });
        }

        // 无工具 / 普通对话
        let mut out = String::from("（Mock 流式回复）收到：");
        out.push_str(&text);
        for _ in 0..3 {
            out.push_str("…");
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
        Ok(StreamResult {
            output_text: out,
            reasoning: String::new(),
            function_calls: vec![],
            usage: None,
            status: StreamStatus::Completed,
            error: None,
        })
    }

    async fn stream_reasoning<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        // Mock：模拟思考增量（3 段），验证流式链路
        for chunk in ["正在分析任务…", "确定关键步骤…", "生成执行计划…"] {
            on_reasoning(chunk);
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
        self.stream_complete(req).await
    }

    async fn stream_reasoning_with_text<'a>(
        &self,
        req: &ResponsesRequest,
        on_reasoning: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
        on_text: &'a mut (dyn for<'b> FnMut(&'b str) + Send),
    ) -> AppResult<StreamResult> {
        // Mock：模拟思考增量（2 段，简短）
        for chunk in ["正在思考…", "组织回复…"] {
            on_reasoning(chunk);
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        let text = Self::user_text(req);
        // 闲聊 → 打字机输出文本增量（DirectAnswer 流式链路演示）
        if Self::is_smalltalk(&text) {
            let full = format!("（Mock 直答）收到：{text}");
            // 逐块输出（每次 3 字），模拟 token 流
            let chars: Vec<char> = full.chars().collect();
            let mut chunk = String::new();
            for (i, c) in chars.iter().enumerate() {
                chunk.push(*c);
                if (i + 1) % 3 == 0 {
                    on_text(&chunk);
                    chunk.clear();
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                }
            }
            if !chunk.is_empty() {
                on_text(&chunk);
            }
            return Ok(StreamResult {
                output_text: full,
                reasoning: String::new(),
                function_calls: vec![],
                usage: None,
                status: StreamStatus::Completed,
                error: None,
            });
        }
        // 任务 → 复用原逻辑（submit_plan 计划）
        self.stream_complete(req).await
    }
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod llm_tests;
