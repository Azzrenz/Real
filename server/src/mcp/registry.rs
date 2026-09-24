//! ToolRegistry：内建工具 + MCP 聚合、冲突消解、inputSchema 单一事实源、执行路由

use crate::error::AppResult;
use crate::mcp::client::{McpClient, McpServerConfigRef};
use crate::mcp::envelope::{ContentPart, ToolEnvelope};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const REASON_INSTRUCTION: &str = include_str!("../../prompts/tools/reason_instruction.md");

// 工具结果缓存（session 隔离 + 大内存策略）
const TOOL_CACHE_TTL_FILE: Duration = Duration::from_secs(72 * 3600);
const TOOL_CACHE_TTL_STATIC: Duration = Duration::from_secs(48 * 3600);
const RUN_FAIL_CACHE_TTL: Duration = Duration::from_secs(300);
const TOOL_CACHE_MAX_ENTRIES: usize = 4000;
static TOOL_CACHE: LazyLock<Mutex<HashMap<String, (Instant, Option<u64>, ToolEnvelope)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 按工具类型选 TTL（差异化）
fn cache_ttl_for(name: &str) -> Duration {
    if name == "run" {
        return RUN_FAIL_CACHE_TTL;
    }
    if matches!(name, "read" | "list" | "find_files" | "search") {
        TOOL_CACHE_TTL_FILE
    } else {
        TOOL_CACHE_TTL_STATIC
    }
}

fn cacheable_ro(name: &str) -> bool {
    matches!(
        name,
        "read" | "list" | "find_files" | "search" | "env" | "audit"
    )
}

/// 写工具：不缓存，但成功执行后失效该 session 全部缓存（文件/代码已变）
fn cache_invalidating(name: &str) -> bool {
    matches!(name, "write" | "modify")
}

/// 是否允许进入缓存（run 是特例：仅失败结果缓存，成功不缓存）
fn cache_allowed(name: &str) -> bool {
    cacheable_ro(name) || name == "run"
}

fn tool_cache_key(session_id: &str, name: &str, args: &Value) -> String {
    format!("{session_id}|{name}|{args}")
}

/// 失效某个 session 的全部工具缓存（写操作成功后调用——代码/文件变了，
fn tool_cache_invalidate(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let prefix = format!("{session_id}|");
    let mut g = TOOL_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let before = g.len();
    g.retain(|k, _| !k.starts_with(&prefix));
    let removed = before - g.len();
    if removed > 0 {
        tracing::debug!(
            session = session_id,
            removed,
            "tool_cache invalidated by write op"
        );
    }
}

/// LRU 条数上限：超限时删除最旧的 1/4（保证缓存有存在感但不会失控）
fn tool_cache_enforce_cap() {
    let mut g = TOOL_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() <= TOOL_CACHE_MAX_ENTRIES {
        return;
    }
    let mut entries: Vec<(String, Instant)> =
        g.iter().map(|(k, (t, _, _))| (k.clone(), *t)).collect();
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    let drop = g.len() - TOOL_CACHE_MAX_ENTRIES + TOOL_CACHE_MAX_ENTRIES / 4;
    for (k, _) in entries.into_iter().take(drop) {
        g.remove(&k);
    }
    tracing::debug!(drop, "tool_cache LRU evict");
}

/// run 黄信封检测（分级纠偏）：Success 信封 + 内层 exit_code≠0 =
fn run_envelope_is_yellow(env: &ToolEnvelope) -> bool {
    if env.name != "run" || env.is_error {
        return false;
    }
    if let Some(text) = env.content.first().and_then(|c| c.text.as_deref()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(ec) = v.pointer("/data/exit_code").and_then(|e| e.as_i64()) {
                return ec != 0;
            }
        }
    }
    false
}

fn tool_cache_get(session_id: &str, name: &str, args: &Value) -> Option<ToolEnvelope> {
    if session_id.is_empty() || !cache_allowed(name) {
        return None;
    }
    // （规则不得截胡语义）：fresh=true → 显式绕过缓存——模型/用户要"重新看"
    if args.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    let key = tool_cache_key(session_id, name, args);
    let mut g = TOOL_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let ttl = cache_ttl_for(name);
    let e = g.get(&key)?;
    if e.0.elapsed() > ttl {
        g.remove(&key);
        return None;
    }
    // run 只缓存失败结果（黄也算失败）
    if name == "run" && !e.2.is_error && !run_envelope_is_yellow(&e.2) {
        return None;
    }
    // （缓存命中优化 · 文件变更失效）：read 缓存带 mtime——若源文件被修改
    if name == "read" {
        if let Some(cached_mtime) = e.1 {
            if let Some(current_mtime) = read_target_mtime(args) {
                if current_mtime != cached_mtime {
                    g.remove(&key);
                    tracing::debug!(
                        session = session_id,
                        "read 缓存因文件变更失效（mtime 变化）"
                    );
                    return None;
                }
            }
        }
    }
    tracing::debug!(session = session_id, tool = name, "tool_cache hit");
    Some(e.2.clone())
}

/// 从 read 参数提取目标文件 mtime（mtime 变了 = 文件被改，缓存作废）
fn read_target_mtime(args: &Value) -> Option<u64> {
    let paths = args.get("paths").and_then(|v| v.as_array())?;
    let first = paths.first().and_then(|v| v.as_str())?;
    if first.contains('*') {
        return None;
    }
    let path = std::path::Path::new(first);
    path.metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

fn tool_cache_put(session_id: &str, name: &str, args: &Value, env: &ToolEnvelope) {
    if session_id.is_empty() || !cache_allowed(name) {
        return;
    }
    // run 只缓存失败（黄也算失败）
    if name == "run" && !env.is_error && !run_envelope_is_yellow(env) {
        return;
    }
    let key = tool_cache_key(session_id, name, args);
    // （缓存命中优化）：read 缓存记录源文件 mtime——文件变了缓存即失效
    let mtime = if name == "read" {
        read_target_mtime(args)
    } else {
        None
    };
    TOOL_CACHE
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), mtime, env.clone()));
    tool_cache_enforce_cap();
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDef {
    pub name: String,
    // 契约字段：描述由 BuiltinTool/MCP 工具声明，registry 测试（every_tool_description_*）
    #[serde(default)]
    #[allow(dead_code)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<Value>,
    /// 契约：输出 Schema（MCP outputSchema）——结构化输出契约
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

impl ToolDef {
    pub fn builtin(name: &str, description: &str, schema: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            annotations: None,
            output_schema: None,
        }
    }
}

/// 内建工具接口：无需 MCP server 即可演示
#[async_trait::async_trait]
pub trait BuiltinTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;

    /// 契约：输出 Schema（MCP outputSchema 对齐）——工具声明结构化输出契约，
    fn output_schema(&self) -> Option<Value> {
        None
    }

    /// T2T：结构化内容生产端——工具执行后生成供下游
    fn structured_content(&self, _args: &Value, _output: &str) -> Option<Value> {
        None
    }

    /// 契约：行为注解
    fn annotations(&self) -> Value {
        json!({"read_only": false, "destructive": false, "idempotent": false})
    }

    async fn run(&self, args: Value) -> Result<String, String>;

    /// 该次调用是否应直通全量文本（跳过 envelope 截断，如 read full 模式）
    fn full_text_allowed(&self, _args: &Value) -> bool {
        false
    }

    /// **非文本产出通道**（本地工具产图）：返回 Some 时，用它**整包替代**默认的
    fn content_parts(&self, _args: &Value, _output: &str) -> Option<Vec<ContentPart>> {
        None
    }
}

enum Target {
    Builtin(Arc<dyn BuiltinTool>),
    Mcp { server: String },
}

impl Clone for Target {
    fn clone(&self) -> Self {
        match self {
            Target::Builtin(b) => Target::Builtin(Arc::clone(b)),
            Target::Mcp { server } => Target::Mcp { server: server.clone() },
        }
    }
}

pub struct Registry {
    tools: std::sync::RwLock<Vec<ToolDef>>,
    targets: std::sync::RwLock<HashMap<String, Target>>,
    mcp_clients: tokio::sync::RwLock<Vec<McpClient>>,
    /// 连接失败记录（name → 原因；重连成功后清除）——状态接口的数据源
    mcp_failures: std::sync::Mutex<HashMap<String, String>>,
    result_truncate_chars: usize,
}

/// 自含证据工具（audit/diagnose）的结果截断上限（晚·第一层）
pub const AUDIT_RESULT_MAX_CHARS: usize = 300_000;

/// 单个 MCP 服务器的运行状态（状态接口返回体）
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub last_error: Option<String>,
}

impl Registry {
    pub async fn new(
        mcp_configs: &[McpServerConfigRef],
        result_truncate_chars: usize,
    ) -> AppResult<Self> {
        // 内建工具总线（tools/ 模块：契约驱动，见 tools/mod.rs）
        let builtins: Vec<Arc<dyn BuiltinTool>> = crate::tools::builtins();

        let mcp_clients = crate::mcp::client::connect_servers(mcp_configs).await;

        let mut tools: Vec<ToolDef> = Vec::new();
        let mut targets: HashMap<String, Target> = HashMap::new();

        // 内建工具
        for b in &builtins {
            let mut def = ToolDef::builtin(b.name(), b.description(), b.input_schema());
            def.output_schema = b.output_schema();
            def.annotations = Some(b.annotations());
            targets.insert(def.name.clone(), Target::Builtin(b.clone()));
            tools.push(def);
        }

        // MCP 工具聚合 + 冲突消解
        for client in &mcp_clients {
            match client.list_tools().await {
                Ok(list) => {
                    for t in list {
                        let def = ToolDef {
                            name: t.name.clone(),
                            description: t.description.clone().unwrap_or_default(),
                            input_schema: t.input_schema.clone(),
                            annotations: t.annotations.clone(),
                            output_schema: None,
                        };
                        let mut final_name = def.name.clone();
                        if targets.contains_key(&final_name) {
                            final_name = format!("{}_{}", client.server_name, final_name);
                            tracing::warn!(original = %def.name, renamed = %final_name, "工具名冲突，已加前缀消解");
                        }
                        targets.insert(
                            final_name.clone(),
                            Target::Mcp {
                                server: client.server_name.clone(),
                            },
                        );
                        tools.push(ToolDef {
                            name: final_name,
                            ..def
                        });
                    }
                }
                Err(e) => {
                    tracing::error!(server = %client.server_name, error = %e, "tools/list 失败")
                }
            }
        }

        // 确定性排序（利于 prompt cache 命中，MCP 规范建议）
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Self {
            tools: std::sync::RwLock::new(tools),
            targets: std::sync::RwLock::new(targets),
            mcp_clients: tokio::sync::RwLock::new(mcp_clients),
            mcp_failures: std::sync::Mutex::new(HashMap::new()),
            result_truncate_chars,
        })
    }

    /// MCP 服务器状态（状态接口数据源）：connected = client 存在；失败原因取自 failures 表
    pub fn mcp_server_status(&self) -> Vec<McpServerStatus> {
        let mut out: Vec<McpServerStatus> = Vec::new();
        if let Ok(clients) = self.mcp_clients.try_read() {
            for c in clients.iter() {
                out.push(McpServerStatus {
                    name: c.server_name.clone(),
                    connected: true,
                    tool_count: 0,
                    last_error: None,
                });
            }
        }
        {
            let f = self.mcp_failures.lock().unwrap_or_else(|e| e.into_inner());
            for (name, err) in f.iter() {
                out.push(McpServerStatus {
                    name: name.clone(),
                    connected: false,
                    tool_count: 0,
                    last_error: Some(err.clone()),
                });
            }
        }
        {
            let targets = self.targets.read().unwrap_or_else(|e| e.into_inner());
            for st in out.iter_mut() {
                st.tool_count = targets
                    .values()
                    .filter(|t| matches!(t, Target::Mcp { server } if server.as_str() == st.name.as_str()))
                    .count();
            }
        }
        out
    }

    /// 热同步单个 MCP 服务器（新增/改配置/重连统一入口）
    pub async fn upsert_mcp_server(&self, cfg: &McpServerConfigRef) -> Result<(), String> {
        self.remove_mcp_server(&cfg.name);
        match McpClient::connect(cfg).await {
            Ok(client) => {
                // 注册该 server 的工具（冲突消解：重名加 <server>_ 前缀，内建/域优先）
                let list = client.list_tools().await.map_err(|e| e.to_string())?;
                {
                    let mut targets = self.targets.write().unwrap_or_else(|e| e.into_inner());
                    let mut tools = self.tools.write().unwrap_or_else(|e| e.into_inner());
                    for t in list {
                        let mut def = ToolDef {
                            name: t.name.clone(),
                            description: t.description.clone().unwrap_or_default(),
                            input_schema: t.input_schema.clone(),
                            annotations: t.annotations.clone(),
                            output_schema: None,
                        };
                        let mut final_name = def.name.clone();
                        if targets.contains_key(&final_name) {
                            final_name = format!("{}_{}", cfg.name, final_name);
                            tracing::warn!(original = %def.name, renamed = %final_name, "工具名冲突，已加前缀消解");
                        }
                        targets.insert(
                            final_name.clone(),
                            Target::Mcp {
                                server: cfg.name.clone(),
                            },
                        );
                        def.name = final_name;
                        tools.push(def);
                    }
                    tools.sort_by(|a, b| a.name.cmp(&b.name));
                }
                self.mcp_clients.write().await.push(client);
                self.mcp_failures
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&cfg.name);
                tracing::info!(server = %cfg.name, "MCP server 已接入（热）");
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                self.mcp_failures
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(cfg.name.clone(), msg.clone());
                tracing::error!(server = %cfg.name, error = %msg, "MCP server 接入失败");
                Err(msg)
            }
        }
    }

    /// 摘除某 MCP 服务器：先按 targets 收集该 server 的工具名，再删 tools/targets/client
    pub fn remove_mcp_server(&self, name: &str) {
        let mut doomed: Vec<String> = Vec::new();
        {
            let mut targets = self.targets.write().unwrap_or_else(|e| e.into_inner());
            targets.retain(|tool_name, t| {
                if matches!(t, Target::Mcp { server } if server == name) {
                    doomed.push(tool_name.clone());
                    false
                } else {
                    true
                }
            });
        }
        if !doomed.is_empty() {
            let mut tools = self.tools.write().unwrap_or_else(|e| e.into_inner());
            tools.retain(|d| !doomed.contains(&d.name));
        }
        if let Ok(mut clients) = self.mcp_clients.try_write() {
            clients.retain(|c| c.server_name != name);
        }
    }

    /// （原子工具直暴露·砍 submit 计划板）：导出全部注册工具定义（内建 + MCP）
    pub fn tools(&self) -> Vec<crate::model::types::ToolDef> {
        self.tools
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|t| {
                let mut schema = t.input_schema.clone();
                // 给每个原子工具注入可选 reason 字段——让模型输出
                if let Value::Object(ref mut m) = schema {
                    if let Some(Value::Object(props)) = m.get_mut("properties") {
                        props.insert(
                            "reason".into(),
                            json!({ "type": "string", "description": REASON_INSTRUCTION }),
                        );
                    }
                }
                crate::model::types::ToolDef::function(t.name.clone(), t.description.clone(), schema)
            })
            .collect()
    }

    /// 执行工具（Worker 调用），统一产出 ToolEnvelope
    pub async fn call(
        &self,
        tool_call_id: &str,
        name: &str,
        args: Value,
        timeout: std::time::Duration,
        session_id: &str,
    ) -> ToolEnvelope {
        let start = Instant::now();
        // 工具结果缓存：同 session 同参 TTL 内重复调用直接返回缓存，
        if let Some(cached) = tool_cache_get(session_id, name, &args) {
            let mut env = cached;
            // （Duplicate tool output 防御）：缓存信封的 tool_call_id 是
            env.tool_call_id = tool_call_id.to_string();
            let meta = env.meta.get_or_insert_with(|| json!({}));
            meta["cached"] = json!(true);
            return env;
        }
        let Some(target) = self
            .targets
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
        else {
            return ToolEnvelope::error(
                tool_call_id,
                name,
                format!("未知工具: {name}（模型幻觉）"),
                start.elapsed().as_millis() as u64,
            );
        };

        // 执行前契约闸门：按工具名找 ToolDef（内建 + MCP 统一），校验 args 是否符合 input_schema
        let schema = self
            .tools
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.input_schema.clone());
        let cleaned_args = match schema {
            Some(schema) => match crate::tools::contract::validate(&schema, &args) {
                Ok(cleaned) => cleaned,
                Err(e) => {
                    // 契约：错误信封带 error_code + retryable + err_type + next_action
                    let mut env = ToolEnvelope::error(
                        tool_call_id,
                        name,
                        crate::tools::err_text(&e),
                        start.elapsed().as_millis() as u64,
                    );
                    env.meta = Some(json!({
                        "error_code": e.code,
                        "retryable": e.retryable,
                        "err_type": e.err_type,
                        "suggestion": e.suggestion,
                    }));
                    // （信号契约 v1）：契约错误与领域错误走**同一套分类表**——
                    let next_action = if crate::tools::contract::code_retryable_with_change(e.code) {
                        "retry"
                    } else {
                        crate::tools::contract::code_next_action(e.code)
                    };
                    env.next_action = Some(next_action.into());
                    return env;
                }
            },
            None => args.clone(),
        };

        let outcome = tokio::time::timeout(timeout, async {
            match target {
                Target::Builtin(b) => {
                    // 会话上下文注入（通用通道）：工具层拿不到 session_id，在这里统一塞进参数。
                    let mut run_args = cleaned_args.clone();
                    if !session_id.is_empty() {
                        run_args["__session"] = json!(session_id);
                    }
                    let out = b.run(run_args.clone()).await?;
                    // 闸门 2：输出契约（工具契约基础层：契约严丝合缝）
                    if let Err(e) = crate::tools::base::validate_output(name, &b.output_schema(), &out) {
                        let mut env = ToolEnvelope::error(
                            tool_call_id, name, crate::tools::err_text(&e),
                            start.elapsed().as_millis() as u64,
                        );
                        env.meta = Some(json!({
                            "error_code": e.code, "retryable": false,
                            "err_type": e.err_type, "suggestion": e.suggestion,
                        }));
                        return Ok::<ToolEnvelope, String>(env);
                    }
                    // T2T：先取结构化内容（借用 out），再 move 进 ContentPart
                    let sc = b.structured_content(&cleaned_args, &out);
                    // 非文本产出通道优先：本地工具产图（read 读图片）→ 整包用
                    if let Some(parts) = b.content_parts(&cleaned_args, &out) {
                        let mut env = ToolEnvelope::success(tool_call_id, name, parts, 0);
                        if let Some(sc) = sc {
                            env.structured_content = Some(sc);
                        }
                        return Ok::<ToolEnvelope, String>(env);
                    }
                    let part = if b.full_text_allowed(&cleaned_args) {
                        // 全文通道（read 专口）：把结果 JSON 摊平成**纯正文**再交出去 ——
                        ContentPart::full_text(
                            crate::tools::summary_render::read_full_text(&out).unwrap_or(out),
                        )
                    } else {
                        ContentPart::text(out)
                    };
                    let mut env = ToolEnvelope::success(tool_call_id, name, vec![part], 0);
                    if let Some(sc) = sc {
                        env.structured_content = Some(sc);
                    }
                    Ok::<ToolEnvelope, String>(env)
                }
                Target::Mcp { server } => {
                    let clients = self.mcp_clients.read().await;
                    let client = clients
                        .iter()
                        .find(|c| c.server_name == *server)
                        .ok_or_else(|| format!("MCP server [{server}] 已断开"))?;
                    let result = client.call_tool(name, cleaned_args.clone()).await
                        .map_err(|e| e.to_string())?;
                    // MCP 结果信封 → 统一 envelope
                    let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
                    let content = result.get("content").cloned().unwrap_or(json!([]));
                    let mut parts = Vec::new();
                    if let Some(arr) = content.as_array() {
                        for item in arr {
                            if let Some(t) = item.get("type").and_then(|v| v.as_str()) {
                                match t {
                                    "text" => parts.push(ContentPart::text(item.get("text").and_then(|v| v.as_str()).unwrap_or(""))),
                                    "resource" => parts.push(ContentPart::resource(
                                        item.get("uri").and_then(|v| v.as_str()).unwrap_or(""),
                                        item.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                        item.get("mimeType").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                    )),
                                    "image" | "audio" => {
                                        let mime = item.get("mimeType").and_then(|v| v.as_str()).unwrap_or("application/octet-stream");
                                        let uri = match item.get("data").and_then(|v| v.as_str()) {
                                            Some(d) if !d.is_empty() => format!("data:{mime};base64,{d}"),
                                            _ => format!("{}://mcp/{server}/{}", t, uuid::Uuid::new_v4()),
                                        };
                                        parts.push(ContentPart::image_ref(uri, mime));
                                    }
                                    other => parts.push(ContentPart::text(format!("[未处理内容类型: {other}]"))),
                                }
                            }
                        }
                    }
                    let mut env = if is_error {
                        let msg = parts.iter().filter_map(|p| p.text.clone()).collect::<Vec<_>>().join("\n");
                        ToolEnvelope::error(tool_call_id, name, if msg.is_empty() { "工具执行失败（isError=true）".to_string() } else { msg }, 0)
                    } else {
                        let mut env = ToolEnvelope::success(tool_call_id, name, if parts.is_empty() { vec![ContentPart::text("(空结果)")] } else { parts }, 0);
                        env.meta = Some(json!({"mcp_server": server, "protocol": crate::mcp::client::PROTOCOL_VERSION}));
                        env
                    };
                    env.duration_ms = start.elapsed().as_millis() as u64;
                    Ok(env)
                }
            }
        }).await;

        let env = match outcome {
            Ok(Ok(mut env)) => {
                env.duration_ms = start.elapsed().as_millis() as u64;
                // 自含证据工具（audit/diagnose）
                let cap = if matches!(name, "audit" | "diagnose") {
                    crate::mcp::registry::AUDIT_RESULT_MAX_CHARS
                } else {
                    self.result_truncate_chars
                };
                // （events 完整性）：截断**前**先渲染完整展示文本——
                let already_plain = env.content.iter().any(|c| c.type_ == "full_text")
                    && env
                        .content
                        .iter()
                        .filter_map(|c| c.text.as_deref())
                        .all(|t| serde_json::from_str::<serde_json::Value>(t).is_err());
                if env.render_full.is_none() && !already_plain {
                    let joined: String = env
                        .content
                        .iter()
                        .filter_map(|c| c.text.clone())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !joined.is_empty() {
                        env.render_full = Some(crate::tools::summary_render::clean(&name, &joined));
                    }
                }
                env = env.with_truncate(cap);
                // 契约：成功信封声明来源（read→路径 / run→命令 / edit→文件）——
                if env.source.is_none() {
                    let src = extract_source_from_args(name, &cleaned_args);
                    if !src.is_empty() {
                        env.source = Some(src);
                    }
                }
                env
            }
            Ok(Err(e)) => {
                // 契约错误（MISSING_PARAM/INVALID_PARAM/领域错误码）带 meta.error_code
                let meta = crate::tools::contract::extract_error_meta(&e);
                let mut env =
                    ToolEnvelope::error(tool_call_id, name, e, start.elapsed().as_millis() as u64);
                // + （信号契约 v1）：领域错误按**完整分类表**决定 next_action
                let code = meta
                    .as_ref()
                    .and_then(|m| m.get("error_code"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // 参数/路径/命令形态类错误（NOT_FOUND/OLD_TEXT_MISMATCH/COMMAND_NOT_ALLOWED…）
                let next_action = if crate::tools::contract::code_retryable_with_change(code) {
                    "retry"
                } else {
                    crate::tools::contract::code_next_action(code)
                };
                env.meta = meta;
                env.next_action = Some(next_action.into());
                env
            }
            Err(_) => ToolEnvelope::timeout(tool_call_id, name, start.elapsed().as_millis() as u64),
        };
        // 写缓存（只读 30min / run 失败 5min）
        tool_cache_put(session_id, name, &args, &env);
        // 写工具成功执行 → 该 session 缓存全部失效（代码/文件变了，旧读旧跑作废）
        if cache_invalidating(name) && !env.is_error {
            tool_cache_invalidate(session_id);
        }
        // （spill · 三层存储的全文层）：工具结果超大 → 全文落盘、只留预览。
        let env = {
            let mut env = env;
            let payload = serde_json::to_string(&env).unwrap_or_else(|_| env.to_plain_text());
            let read_mode = if name == "read" {
                cleaned_args.get("mode").and_then(|v| v.as_str())
            } else {
                None
            };
            let read_paths: Vec<String> = if name == "read" {
                cleaned_args
                    .get("paths")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Some((locator, preview)) =
                crate::mcp::spill::maybe_spill(name, session_id, &payload, read_mode, &read_paths)
                    .await
            {
                // 形态定义在 spill 那边（唯一事实源）：正文换预览 + 去掉重复副本 + 标记 locator。
                crate::mcp::spill::mark_spilled(&mut env, &locator, &preview);
            }
            env
        };
        env
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }
}

/// 从工具参数提取来源追溯（[{type, id, label}]）
fn extract_source_from_args(name: &str, args: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut push = |id: String, label: &str| {
        let t = match name {
            // "edit" 已不暴露——modify 是改文件唯一入口，来源归 file
            "read" | "modify" | "write" | "search" | "find_files" | "list" => "file",
            "run" | "verify" => "command",
            _ => "tool",
        };
        if !id.is_empty() {
            out.push(json!({"type": t, "id": id, "label": label}));
        }
    };
    if let Some(p) = args.get("file").and_then(|v| v.as_str()) {
        push(p.to_string(), "file");
    }
    if let Some(paths) = args.get("paths").and_then(|v| v.as_array()) {
        if let Some(p) = paths.first().and_then(|v| v.as_str()) {
            push(p.to_string(), "paths[0]");
        }
    }
    if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
        push(p.to_string(), "path");
    }
    if let Some(c) = args.get("command").and_then(|v| v.as_str()) {
        push(c.to_string(), "command");
    }
    if let Some(c) = args.get("target").and_then(|v| v.as_str()) {
        push(c.to_string(), "target");
    }
    out
}

mod tests {
    #[allow(unused_imports)]
    use super::*;

    // cargo fix 曾误删本行（bin 编译 unused，但测试模块通过 super::* 拿 Registry 等）——已恢复

    /// 三层测试 L1 扩展：工具契约自检——每个内建工具的 description
    #[tokio::test]
    async fn every_tool_description_declares_return_shape() {
        let registry = Registry::new(&[], 20_000).await.unwrap();
        for t in registry.tools.read().unwrap().iter() {
            let desc = &t.description;
            assert!(
                desc.contains("返回 JSON 结构") || desc.contains("返回") || desc.contains("kind"),
                "工具「{}」description 必须声明返回格式，实际: {}",
                t.name,
                &desc.chars().take(80).collect::<String>()
            );
        }
    }

    /// 契约价签自检（经验沉淀「模型不知道工具价签」）：关键工具的
    #[tokio::test]
    async fn every_tool_description_declares_price_tag() {
        let registry = Registry::new(&[], 20_000).await.unwrap();
        let get = |name: &str| {
            registry
                .tools
                .read()
                .unwrap()
                .iter()
                .find(|t| t.name == name)
                .map(|t| t.description.clone())
                .unwrap_or_default()
        };
        assert!(
            get("read").contains("大文件") && get("read").contains("自动"),
            "read 必须有「大文件自动降级」价签（后端按大小自动选档，\
             不靠模型自觉分段——默认不传 mode 大文件自动 auto 预览，显式 full 才全文）"
        );
        assert!(
            get("run").contains("Remove-Item"),
            "run 必须有「删除形态」价签（Remove-Item 单路径）"
        );
        assert!(
            get("run").contains("dir /b"),
            "run 必须有「目录列举并入」价签（工具面收敛：list 并入 run）"
        );
    }

    /// 契约自检（架构定调·宽容执行）：所有工具的 input_schema 必须**宽容**
    #[tokio::test]
    async fn every_tool_schema_is_lenient() {
        let registry = Registry::new(&[], 20_000).await.unwrap();
        for t in registry.tools.read().unwrap().iter() {
            // 宽容模式：additionalProperties 不得声明为 false（=拒绝未知字段的旧严格语义）
            let strict = t
                .input_schema
                .get("additionalProperties")
                .map(|v| v.as_bool() == Some(false))
                .unwrap_or(false);
            assert!(!strict, "工具「{}」input_schema 不得声明 additionalProperties=false（宽容执行取代严格模式）", t.name);
        }
    }

    /// 契约自检：声明了边界的参数，应把边界写进 description。
    #[test]
    fn bounded_params_state_bounds_in_description() {
        fn walk(path: &str, node: &serde_json::Value, bad: &mut Vec<String>) {
            if let Some(obj) = node.as_object() {
                let desc = obj.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let nums: Vec<String> = desc
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                for key in ["minimum", "maximum"] {
                    if let Some(b) = obj.get(key).and_then(|v| v.as_i64()) {
                        if key == "minimum" && b == 0 {
                            continue;
                        }
                        if !nums.iter().any(|n| n == &b.to_string()) {
                            bad.push(format!("{path} 的 {key}={b} 未写进 description"));
                        }
                    }
                }
            }
            match node {
                serde_json::Value::Object(m) => {
                    for (k, v) in m {
                        walk(&format!("{path}.{k}"), v, bad);
                    }
                }
                serde_json::Value::Array(a) => {
                    for (i, v) in a.iter().enumerate() {
                        walk(&format!("{path}[{i}]"), v, bad);
                    }
                }
                _ => {}
            }
        }

        let mut bad = Vec::new();
        for t in crate::tools::builtins() {
            if let Some(props) = t.input_schema().get("properties") {
                walk(&t.name(), props, &mut bad);
            }
        }
        assert!(
            bad.is_empty(),
            "边界只写在 schema 关键字里、没进 description（模型看不到，只能猜，猜错就白烧一轮）：\n  {}",
            bad.join("\n  ")
        );
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod registry_tests;
