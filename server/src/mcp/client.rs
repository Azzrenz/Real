//! MCP 客户端适配层：官方 SDK（rmcp）之上封装 Real 的工具面契约

use crate::error::{AppError, AppResult};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation,
    PaginatedRequestParams, ProtocolVersion,
};
use rmcp::service::{ClientLifecycleMode, RoleClient, RunningService};
use rmcp::ClientServiceExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

/// 首选协议版本（实际以协商结果为准，见 McpClient::protocol）
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// 单次工具调用上限（外部服务器可能长跑；信封层另有兜底）
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 清单类请求上限
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
/// 列表缓存 TTL（服务器未给 ttlMs 时的默认值）
const LIST_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct McpServerConfigRef {
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    /// http 传输自定义 header（鉴权/路由）
    pub headers: std::collections::HashMap<String, String>,
}

/// 连接后的服务句柄（SDK 的 RunningService 不可 Clone，用 Arc 共享）
type Svc = RunningService<RoleClient, ClientInfo>;

/// MCP 客户端：可跨任务共享（&self 调用，SDK 内部按请求 id 路由）
pub struct McpClient {
    svc: Arc<Svc>,
    pub server_name: String,
    /// 实际协商的协议版本（供信封 meta 与状态展示）
    pub protocol: String,
}

impl McpClient {
    /// 建立连接：新规范优先（server/discover），探测失败回落传统握手（initialize）
    pub async fn connect(cfg: &McpServerConfigRef) -> AppResult<Self> {
        let svc = match Self::connect_with(cfg, ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        })
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    server = %cfg.name,
                    error = %e,
                    "server/discover 未成功，回落传统握手（initialize）"
                );
                Self::connect_with(cfg, ClientLifecycleMode::Initialize).await?
            }
        };

        let svc = Arc::new(svc);
        // 列表缓存：服务器未给 ttlMs 时用默认 TTL（SDK 默认 0 = 立即过期，等于没缓存）
        svc.set_response_cache_config(
            rmcp::ClientCacheConfig::default().with_default_ttl(LIST_CACHE_TTL),
        )
        .await;
        Ok(Self {
            svc,
            server_name: cfg.name.clone(),
            protocol: PROTOCOL_VERSION.to_string(),
        })
    }

    /// 按给定生命周期建连（传输类型决定走子进程还是 Streamable HTTP）
    async fn connect_with(
        cfg: &McpServerConfigRef,
        lifecycle: ClientLifecycleMode,
    ) -> AppResult<Svc> {
        let info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("Real", env!("CARGO_PKG_VERSION")),
        );
        let svc: Svc = match cfg.transport.as_str() {
            "stdio" => {
                let command = cfg.command.as_deref().ok_or_else(|| {
                    AppError::Config(format!("MCP server [{}] stdio 传输缺少 command", cfg.name))
                })?;
                let mut cmd = tokio::process::Command::new(command);
                cmd.args(&cfg.args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    // 子进程 stderr 若不接管会直写宿主控制台：MCP 服务器（如 Python mcp 库）
                    .stderr(Stdio::null())
                    // CREATE_NO_WINDOW：spawn npx.cmd / python 这类批处理时 Windows 会
                    .creation_flags(0x0800_0000);
                let transport = rmcp::transport::child_process::TokioChildProcess::new(cmd)
                    .map_err(|e| {
                        AppError::Config(format!(
                            "MCP server [{}] 启动子进程失败: {e}",
                            cfg.name
                        ))
                    })?;
                info.serve_with_lifecycle(transport, lifecycle)
                    .await
                    .map_err(|e| {
                        AppError::Config(format!("MCP server [{}] 连接失败: {e}", cfg.name))
                    })?
            }
            "http" | "https" | "streamable-http" => {
                let url = cfg.url.as_deref().ok_or_else(|| {
                    AppError::Config(format!("MCP server [{}] http 传输缺少 url", cfg.name))
                })?;
                // 注意：SDK 的 HTTP 客户端基于 reqwest 0.13（见 Cargo.toml 的 reqwest013 别名），
                let (auth, extra) = split_headers(&cfg.headers);
                let mut tcfg = rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(url);
                tcfg.auth_header = auth;
                tcfg.custom_headers = extra;
                let worker = rmcp::transport::streamable_http_client::StreamableHttpClientWorker::new(
                    reqwest013::Client::new(),
                    tcfg,
                );
                let transport = rmcp::transport::streamable_http_client::StreamableHttpClientTransport::<
                    reqwest013::Client,
                >::spawn(worker);
                info.serve_with_lifecycle(transport, lifecycle)
                    .await
                    .map_err(|e| {
                        AppError::Config(format!("MCP server [{}] 连接失败: {e}", cfg.name))
                    })?
            }
            other => {
                return Err(AppError::Config(format!(
                    "MCP server [{}] 不支持的传输类型: {other}（支持 stdio / http）",
                    cfg.name
                )))
            }
        };
        Ok(svc)
    }

    /// 工具清单（自动翻页，服务器不给 next_cursor 则单页结束）
    pub async fn list_tools(&self) -> AppResult<Vec<McpTool>> {
        let mut out: Vec<McpTool> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor.as_ref().map(|c| {
                let mut p = PaginatedRequestParams::default();
                p.cursor = Some(c.clone());
                p
            });
            let page = tokio::time::timeout(LIST_TIMEOUT, self.svc.list_tools(params))
                .await
                .map_err(|_| {
                    AppError::Tool(format!(
                        "MCP [{}] 工具清单超时（{}s）",
                        self.server_name,
                        LIST_TIMEOUT.as_secs()
                    ))
                })?
                .map_err(|e| {
                    AppError::Tool(format!("MCP [{}] 工具清单失败: {e}", self.server_name))
                })?;
            for t in &page.tools {
                out.push(convert_tool(t));
            }
            cursor = page.next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// 调用工具（SDK 自动处理 MRTR：服务器要求补参数/确认时驱动轮次）
    pub async fn call_tool(&self, name: &str, arguments: Value) -> AppResult<Value> {
        let arguments = match arguments {
            Value::Object(o) => Some(o),
            Value::Null => None,
            other => {
                return Err(AppError::Tool(format!(
                    "MCP [{}] 调用 {name} 参数必须是对象，实际: {other}",
                    self.server_name
                )))
            }
        };
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = arguments;
        let res = tokio::time::timeout(CALL_TIMEOUT, self.svc.call_tool(params))
            .await
            .map_err(|_| {
                AppError::Tool(format!(
                    "MCP [{}] 调用 {name} 超时（{}s）",
                    self.server_name,
                    CALL_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| AppError::Tool(format!("MCP [{}] 调用 {name} 失败: {e}", self.server_name)))?;

        // 结果归一化成信封消费的形状（content / isError / structuredContent）
        let content = serde_json::to_value(&res.content).unwrap_or(json!([]));
        let mut out = json!({
            "content": content,
            "isError": res.is_error.unwrap_or(false),
        });
        if let Some(sc) = res.structured_content {
            out["structuredContent"] = sc;
        }
        Ok(out)
    }
}

/// 拆 headers：Authorization 走 SDK 的 auth_header（专位），其余进 custom_headers
fn split_headers(
    headers: &std::collections::HashMap<String, String>,
) -> (
    Option<String>,
    std::collections::HashMap<reqwest013::header::HeaderName, reqwest013::header::HeaderValue>,
) {
    let mut auth = None;
    let mut extra = std::collections::HashMap::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("authorization") {
            auth = Some(v.clone());
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            reqwest013::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest013::header::HeaderValue::from_str(v),
        ) {
            extra.insert(name, val);
        }
    }
    (auth, extra)
}

/// SDK 工具定义 → Real 契约形状（字段名与协议一致）
fn convert_tool(t: &rmcp::model::Tool) -> McpTool {
    McpTool {
        name: t.name.to_string(),
        title: t.title.clone(),
        description: t.description.as_ref().map(|d| d.to_string()),
        input_schema: serde_json::to_value(&*t.input_schema).unwrap_or(json!({})),
        annotations: t
            .annotations
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok()),
    }
}

/// 按配置批量连接（供 Registry 启动时调用；单个失败跳过并告警）
pub async fn connect_servers(configs: &[McpServerConfigRef]) -> Vec<McpClient> {
    let mut clients = Vec::new();
    for cfg in configs {
        match McpClient::connect(cfg).await {
            Ok(c) => {
                tracing::info!(
                    server = %cfg.name,
                    transport = %cfg.transport,
                    protocol = %c.protocol,
                    "MCP server 已连接"
                );
                clients.push(c);
            }
            Err(e) => {
                tracing::error!(server = %cfg.name, error = %e, "MCP server 连接失败（跳过）")
            }
        }
    }
    clients
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;
