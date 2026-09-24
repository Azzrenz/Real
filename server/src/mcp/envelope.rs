//! ToolEnvelope：工具结果统一信封（status/is_error/truncated/meta/next_action/source），喂模型前归一

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Success,
    Error,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            type_: "text".into(),
            text: Some(text.into()),
            uri: None,
            mime_type: None,
        }
    }
    /// 全量文本：with_truncate 跳过（read full 模式专用，防 envelope 二次截断）
    pub fn full_text(text: impl Into<String>) -> Self {
        Self {
            type_: "full_text".into(),
            text: Some(text.into()),
            uri: None,
            mime_type: None,
        }
    }
    pub fn image_ref(uri: impl Into<String>, mime: impl Into<String>) -> Self {
        Self {
            type_: "image_ref".into(),
            text: None,
            uri: Some(uri.into()),
            mime_type: Some(mime.into()),
        }
    }
    pub fn resource(uri: impl Into<String>, text: Option<String>, mime: Option<String>) -> Self {
        Self {
            type_: "resource".into(),
            text,
            uri: Some(uri.into()),
            mime_type: mime,
        }
    }

    /// 这张图能否**直接回灌给模型**（内联 `data_url` / 可拉取的 `http(s)`）——返回其 uri。
    pub fn reachable_image_uri(&self) -> Option<&str> {
        if self.type_ != "image_ref" {
            return None;
        }
        let uri = self.uri.as_deref()?;
        let ok = uri.starts_with("data:")
            || uri.starts_with("http://")
            || uri.starts_with("https://");
        ok.then_some(uri)
    }

    /// 把内联图片的 `data_url` 换成一句说明，返回被剥掉的字节数（不是内联图则返回 0）。
    pub fn strip_inline_image_body(&mut self) -> usize {
        if self.type_ != "image_ref" {
            return 0;
        }
        let Some(uri) = self.uri.as_deref() else {
            return 0;
        };
        if !uri.starts_with("data:") {
            return 0;
        }
        let n = uri.len();
        self.uri = Some(format!(
            "（内联图片，{n} 字节 base64 未随历史留存 —— 本轮已按附件回灌；\
要再看时 read 它的原文件路径）"
        ));
        n
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEnvelope {
    pub tool_call_id: String,
    pub name: String,
    pub status: ToolStatus,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub duration_ms: u64,
    pub content: Vec<ContentPart>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    /// MCP 2025-11 结构化输出（outputSchema 对齐）：工具声明了输出 schema 时，
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// 业界 ToolResult.nextAction：continue / retry / ask_user / verify
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    /// 业界 ToolSource 追溯：结果来源（read→文件路径 / run→命令 / search→命中文）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_full: Option<String>,
    /// （失败可见性）：run 工具命令退出码——负数（如 -1）=
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
}

impl ToolEnvelope {
    /// 见 `ContentPart::strip_inline_image_body`：一次剥掉整份信封里的内联图。
    pub fn strip_inline_image_bodies(&mut self) -> usize {
        self.content
            .iter_mut()
            .map(|p| p.strip_inline_image_body())
            .sum()
    }

    pub fn success(
        tool_call_id: &str,
        name: &str,
        content: Vec<ContentPart>,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            status: ToolStatus::Success,
            is_error: false,
            duration_ms,
            content,
            truncated: false,
            meta: None,
            structured_content: None,
            next_action: Some("continue".into()),
            source: None,
            render_full: None,
            exit_code: None,
        }
    }

    pub fn error(
        tool_call_id: &str,
        name: &str,
        message: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            status: ToolStatus::Error,
            is_error: true,
            duration_ms,
            content: vec![ContentPart::text(message)],
            truncated: false,
            meta: None,
            structured_content: None,
            // 错误默认 ask_user（不可盲目重试）；retryable 的错误由调用方改成 retry
            next_action: Some("ask_user".into()),
            source: None,
            render_full: None,
            exit_code: None,
        }
    }

    pub fn timeout(tool_call_id: &str, name: &str, duration_ms: u64) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            status: ToolStatus::Timeout,
            is_error: true,
            duration_ms,
            content: vec![ContentPart::text(format!(
                "工具 {name} 调用超时（>{duration_ms}ms）"
            ))],
            truncated: false,
            meta: None,
            structured_content: None,
            // 超时通常可重试（瞬时资源问题）
            next_action: Some("retry".into()),
            source: None,
            render_full: None,
            exit_code: None,
        }
    }

    /// 截断内容，防上下文爆炸：证据表存全量，喂模型只喂截断版
    pub fn with_truncate(mut self, max_chars: usize) -> Self {
        let full_text_limit = 200_000usize.max(max_chars);
        let mut total: usize = 0;
        for part in &mut self.content {
            if part.type_ == "full_text" {
                // 绝对上限：超过强制截断并标记（任何工具/MCP 返回巨量文本都不允许直通）
                if let Some(t) = &part.text {
                    let n = t.chars().count();
                    if n > full_text_limit {
                        let keep: String = t.chars().take(full_text_limit).collect();
                        part.text = Some(format!("{keep}…[已截断，共 {n} 字符]"));
                        self.truncated = true;
                    }
                }
                continue;
            }
            if let Some(t) = &part.text {
                total += t.chars().count();
                if total > max_chars {
                    let keep: usize = t.chars().take(max_chars).map(|c| c.len_utf8()).sum();
                    part.text = Some(format!("{}…[已截断，共 {} 字符]", &t[..keep], total));
                    self.truncated = true;
                    // 只保留当前部分，丢弃其余
                    break;
                }
            }
        }
        self
    }

    /// 文本化摘要（非 text 内容降级为引用说明）
    pub fn to_plain_text(&self) -> String {
        let parts: Vec<String> = self
            .content
            .iter()
            .map(
                |c| match (c.type_.as_str(), c.text.as_deref(), c.uri.as_deref()) {
                    ("text", Some(t), _) | ("full_text", Some(t), _) => t.to_string(),
                    ("image_ref", _, Some(uri)) => format!("[图片引用: {uri}]"),
                    ("resource", t, Some(uri)) => format!("[资源 {uri}]: {}", t.unwrap_or("")),
                    (t, Some(x), _) => format!("[{t}]: {x}"),
                    (t, None, Some(u)) => format!("[{t} 引用: {u}]"),
                    (t, None, None) => format!("[{t}]"),
                },
            )
            .collect();
        parts.join("\n")
    }

}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod envelope_tests;
