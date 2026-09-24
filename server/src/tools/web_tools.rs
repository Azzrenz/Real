//! 上网工具（web_fetch / web_search）：**两者当前都不在工具面** ——

use std::sync::{Arc, OnceLock, RwLock};

/// 搜索凭据在 `settings` 表里的键名 —— **唯一来源**，主程序回填与设置路由写入都引用它，
pub const SEARCH_KEY_SETTING: &str = "web_search_api_key";

/// 搜索凭据静态持有器（工具执行层拿不到 AppState，凭据经此单点流转）。
static SEARCH_KEY: OnceLock<Arc<RwLock<Option<String>>>> = OnceLock::new();

fn search_cell() -> &'static Arc<RwLock<Option<String>>> {
    SEARCH_KEY.get_or_init(|| Arc::new(RwLock::new(None)))
}

/// 写入搜索凭据（`None` = 清除）。设置面板保存后立即生效。
pub fn set_search_key(key: Option<String>) {
    *search_cell().write().unwrap() = key;
}

/// 当前是否已配搜索凭据 —— 供设置页回显与诊断用。
pub fn has_search_key() -> bool {
    search_cell()
        .read()
        .unwrap()
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false)
}

/// 取搜索凭据。
fn search_key() -> Option<String> {
    search_cell()
        .read()
        .unwrap()
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
}

/// 拉取正文上限（字符）——超过截断并自报（"截断必须自解释"不变量）
#[allow(dead_code)]
const FETCH_TEXT_CAP: usize = 20_000;

/// 去 HTML 标签取正文（确定性，零依赖）
#[allow(dead_code)]
fn strip_html(html: &str) -> String {
    let chars: Vec<char> = html.chars().collect();
    let lower: Vec<char> = html.to_lowercase().chars().collect();
    let n = chars.len();
    let starts_with = |i: usize, tag: &str| -> bool {
        let t: Vec<char> = tag.chars().collect();
        i + t.len() <= n && lower[i..i + t.len()] == t[..]
    };
    let mut body: Vec<char> = Vec::with_capacity(n / 2);
    let mut i = 0usize;
    while i < n {
        if chars[i] == '<' {
            // ① 整块剥除：<script>…</script> / <style>…</style> / <!--…-->
            let mut block: Option<(&str, &str)> = None;
            for (open, close) in [("<script", "</script>"), ("<style", "</style>"), ("<!--", "-->")] {
                if starts_with(i, open) {
                    block = Some((open, close));
                    break;
                }
            }
            if let Some((_, close)) = block {
                let close_chars: Vec<char> = close.chars().collect();
                let mut k = i;
                let mut found = None;
                while k + close_chars.len() <= n {
                    if lower[k..k + close_chars.len()] == close_chars[..] {
                        found = Some(k);
                        break;
                    }
                    k += 1;
                }
                i = match found {
                    Some(k) => k + close_chars.len(),
                    None => n,
                };
                continue;
            }
            // ② 普通标签：剥到 '>'
            if let Some(rel) = body_scanner_pos(&chars[i..]) {
                i += rel;
                continue;
            }
        }
        body.push(chars[i]);
        i += 1;
    }
    // ③④ 实体还原 + 空白压缩（下标法）
    let mut text = String::with_capacity(body.len());
    let mut last_ws = false;
    let mut i = 0usize;
    while i < body.len() {
        let c = body[i];
        if c == '&' {
            let rest: String = body[(i + 1)..body.len().min(i + 9)].iter().collect();
            let ent: Option<(&str, char)> = [
                ("amp;", '&'),
                ("lt;", '<'),
                ("gt;", '>'),
                ("quot;", '"'),
                ("#39;", '\''),
                ("nbsp;", ' '),
            ]
            .iter()
            .find(|(tag, _)| rest.starts_with(tag))
            .map(|(tag, ch)| (*tag, *ch));
            if let Some((tag, ch)) = ent {
                text.push(ch);
                last_ws = ch.is_whitespace();
                i += 1 + tag.chars().count();
                continue;
            }
            text.push('&');
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            if !last_ws {
                text.push(' ');
            }
            last_ws = true;
            i += 1;
            continue;
        }
        last_ws = false;
        text.push(c);
        i += 1;
    }
    text.trim().to_string()
}

/// 从 pos 找 '>' 的相对位置（含 1 步跨过 '>'）；找不到 → 剥到尾
#[allow(dead_code)]
fn body_scanner_pos(chars: &[char]) -> Option<usize> {
    chars.iter().position(|&c| c == '>').map(|p| p + 1)
}

/// BigModel 网页搜索（服务端搜索：POST /api/paas/v4/web_search）。
#[allow(dead_code)]
async fn bigmodel_search(query: &str, count: usize) -> Result<Vec<(String, String, String)>, String> {
    let key = search_key().ok_or_else(|| {
        "[WEB_SEARCH_NO_KEY] 未配置搜索凭据：请在设置面板填写「联网搜索 API Key」\
         （独立于模型 Key，配一次即对所有模型生效）；未配置前可用 run + curl 直接拉取已知 URL"
            .to_string()
    })?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
    let resp = client
        .post("https://open.bigmodel.cn/api/paas/v4/web_search")
        .bearer_auth(&key)
        .json(&serde_json::json!({
            "search_engine": "search_std",
            "search_query": query,
            "count": count,
        }))
        .send()
        .await
        .map_err(|e| format!("搜索请求失败: {e}"))?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("搜索响应解析失败: {e}"))?;
    if status != 200 {
        return Err(format!(
            "搜索服务返回 {status}: {}",
            body.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("未知错误")
        ));
    }
    let mut out = Vec::new();
    if let Some(items) = body.pointer("/search_result").and_then(|v| v.as_array()) {
        for it in items.iter().take(count) {
            let title = it.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = it
                .get("link")
                .or_else(|| it.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = it
                .get("content")
                .or_else(|| it.get("snippet"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(300)
                .collect::<String>();
            out.push((title, url, snippet));
        }
    }
    Ok(out)
}

// ── BuiltinTool 实现 ──

use crate::mcp::registry::BuiltinTool;
use serde_json::{json, Value};

// 工具面裁剪保留（未注册）：抓网页由 run+curl 覆盖
#[allow(dead_code)]
pub struct WebFetchTool;

#[async_trait::async_trait]
impl BuiltinTool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }
    fn description(&self) -> &'static str {
        concat!(
            "拉取网页并返回去标签正文（读文档页/API 参考/新闻正文的唯一正规途径）。\
         输入 {url, timeout?(秒,默认25)}；正文上限 20000 字符（超限截断并自报，可用原 URL 分段无意义——\
         需要特定段落时先拿到全文再由模型自寻）。\
         与 web_search 配合：search 找到 URL → fetch 读全文。\
         常见失败：404=URL 拼错（先 search 校正）；超时=重试一次；\
         反爬（返回 403/验证页）=该站需浏览器，改用 search 的摘要。\
         返回 {kind:\"web_result\", data:{url,status,content}}。",
            include_str!("../../prompts/tools/spill_note.md")
        )
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "完整 URL（http/https）"},
                "timeout": {"type": "number", "description": "超时秒数（默认 25）"}
            },
            "required": ["url"]
        })
    }
    async fn run(&self, args: Value) -> Result<String, String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("[WEB_URL_INVALID] url 必须以 http:// 或 https:// 开头——先 web_search 拿到准确 URL".into());
        }
        let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(25);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout.clamp(5, 120)))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Real/1.0")
            .build()
            .map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("[WEB_FETCH_FAIL] 拉取失败: {e}（404=URL 拼错先 search 校正；超时可重试一次）"))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
        let mut text = strip_html(&body);
        let mut truncated = false;
        if text.chars().count() > FETCH_TEXT_CAP {
            text = text.chars().take(FETCH_TEXT_CAP).collect();
            text.push_str("…（正文已截断：仅前 20000 字符）");
            truncated = true;
        }
        Ok(json!({
            "ok": true,
            "kind": "web_result",
            "data": {"url": url, "status": status, "content": text, "truncated": truncated}
        })
        .to_string())
    }
}

/// 联网关键词搜索（BigModel 搜索服务）。
#[allow(dead_code)]
pub struct WebSearchTool;

#[async_trait::async_trait]
impl BuiltinTool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }
    fn description(&self) -> &'static str {
        concat!(
            "联网搜索（返回结果列表，不含正文）。输入 {query, count?(默认5,上限10)}。\
         与模型无关——DeepSeek/GLM 均可调用，凭据是设置面板里独立的「联网搜索 API Key」。\
         拿到 url 后用 run + curl 读正文（web_fetch 已并入 run）。\
         返回 {kind:\"web_search_result\", data:{query, results:[{title,url,snippet}]}}。",
            include_str!("../../prompts/tools/spill_note.md")
        )
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "搜索词（具体名词优于整句）"},
                "count": {"type": "number", "description": "结果条数（默认 5，上限 10）"}
            },
            "required": ["query"]
        })
    }
    async fn run(&self, args: Value) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("[SEARCH_QUERY_EMPTY] query 不能为空：给具体搜索词（如 \"DeepSeek API reasoning guide\"）".into());
        }
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(5).clamp(1, 10) as usize;
        let results = bigmodel_search(&query, count).await?;
        let items: Vec<Value> = results
            .into_iter()
            .map(|(title, url, snippet)| json!({"title": title, "url": url, "snippet": snippet}))
            .collect();
        Ok(json!({
            "ok": true,
            "kind": "web_search_result",
            "data": {"query": query, "results": items}
        })
        .to_string())
    }
}

#[cfg(test)]
#[path = "web_tools_tests.rs"]
mod web_tools_tests;
