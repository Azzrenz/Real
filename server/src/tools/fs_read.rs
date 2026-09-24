//! 文件只读域：read / search / find_files / list（含路径守卫 resolve_guarded）

use crate::tools::contract::{err_text, validate, ToolError};
use crate::tools::fs_common::{BINARY_PROBE_BYTES, SKIP_DIRS, SKIP_FILES};
use crate::tools::fs_common::extract_structure;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::mcp::envelope::ContentPart;
use crate::mcp::registry::BuiltinTool;
use async_trait::async_trait;
use tokio::fs;

#[allow(dead_code)]
fn parent_list_hint(start: &std::path::Path) -> String {
    let Some(par) = start.parent() else { return String::new() };
    if !par.is_dir() {
        return String::new();
    }
    let mut entries: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(par) {
        for en in rd.flatten().take(200) {
            let nm = en.file_name().to_string_lossy().to_string();
            let mark = if en.path().is_dir() { "/" } else { "" };
            entries.push(format!("{nm}{mark}"));
        }
    }
    entries.sort();
    entries.truncate(12);
    if entries.is_empty() {
        return format!("（{} 目录为空）", par.display());
    }
    format!("\n目标路径的父目录 {} 下真实条目（挑正确的文件名直接改用绝对路径）：\n  {}",
        par.display(), entries.join("\n  "))
}

/// 项目根目录（可由 REAL_PROJECT_ROOT 注入；否则自动推断）
pub fn project_root() -> PathBuf {
    if let Ok(root) = std::env::var("REAL_PROJECT_ROOT") {
        let r = root.trim();
        // 判定统一走事实部门（facts）：变量不存在或值指向幽灵目录 → Stale → 忽略并告警。
        if !r.is_empty() {
            if crate::facts::liveness(r) == crate::facts::Liveness::Valid {
                return PathBuf::from(r);
            }
            static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
            if WARNED.set(()).is_ok() {
                tracing::warn!(
                    var = %r,
                    "REAL_PROJECT_ROOT 判定为失效（变量不存在或其值指向不存在的目录），已忽略（本进程仅提示一次）"
                );
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let looks_like_server_dir = cwd.join("Cargo.toml").exists()
        && cwd
            .parent()
            .map(|p| p.join("client").exists())
            .unwrap_or(false);
    if looks_like_server_dir {
        cwd.parent().unwrap().to_path_buf()
    } else {
        cwd
    }
}

/// base64 标准解码（无外部依赖；附件 data_url 用，兼容 data: 前缀剥离后的纯 base64）
pub(crate) fn resolve_guarded(path: &str) -> Result<PathBuf, ToolError> {
    crate::path::WorkspacePath::new(path).map(|w| w.into_inner())
}

pub(crate) const SUMMARY_PREVIEW_CHARS: usize = 5_000;
/// full 模式单文件上限：1MB（≈25 万 token 内），超限截断带行级定位提示，模型可用 lines 续读。
pub(crate) const FULL_MAX_CHARS: usize = 1_000_000;
/// 超过任一阈值 → 默认 auto 预览（不整灌上下文）。
pub(crate) const AUTO_DOWNGRADE_LINES: usize = 1200;
pub(crate) const AUTO_DOWNGRADE_BYTES: u64 = 96 * 1024;
/// **单行爆长判据（新增）**：平均行长 > 1200 字节且文件 > 4KB ⇒ 判为"无结构内容"
pub(crate) const DENSE_AVG_LINE_BYTES: u64 = 1_200;
pub(crate) const DENSE_MIN_BYTES: u64 = 4 * 1024;

/// 数文件行数（读 '\n' 计数，O(n)，用于默认 mode 自动降级探测）。
async fn count_lines(path: &std::path::Path) -> std::io::Result<usize> {
    use tokio::io::AsyncBufReadExt;
    let f = tokio::fs::File::open(path).await?;
    let mut reader = tokio::io::BufReader::new(f);
    let mut n = 0usize;
    let mut buf = Vec::with_capacity(4096);
    loop {
        buf.clear();
        let read = reader.read_until(b'\n', &mut buf).await?;
        if read == 0 {
            break;
        }
        n += 1;
    }
    Ok(n)
}

// read

pub struct ReadTool;

#[async_trait]
impl BuiltinTool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }

    /// full 模式：envelope 层跳过截断，全量直通模型（修复二次截断问题）
    fn full_text_allowed(&self, args: &Value) -> bool {
        // full**（run 内 P0-2 自动降级），此时结果实际是全文，也必须直通——否则降级后的
        let mode = args.get("mode").and_then(|v| v.as_str());
        // 此处同样按 lines 直通（否则行区间内容仍被 envelope 当普通 text 截断，预览依旧只有 ~50 行）。
        if mode.is_none() {
            let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0);
            let end = args.get("end_line").and_then(|v| v.as_u64()).unwrap_or(0);
            if start > 0 && end >= start {
                return true;
            }
            //（小文件 full / 大文件 auto 降级）。full 是"全文直通"语义（截断会丢中段，
            return true;
        }
        matches!(mode, Some("full") | Some("lines"))
    }
    fn description(&self) -> &'static str {
        concat!(
            include_str!("../../prompts/tools/read.md"),
            include_str!("../../prompts/tools/spill_note.md")
        )
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "paths":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":20,"description":"[必须] 文件绝对路径列表（正反斜杠均可，**建议正斜杠**；可用 #En 引用前序结果；**批量：多个文件一次传入，后端并发读取**）"},
                "mode":{"type":"string","enum":["auto","full","lines"],"description":"[自动] lines=行区间(须带 start_line/end_line)；auto=摘要预览(5000字)；full=全文(≤1MB/文件)。**默认不传 = 后端按文件特征自动选档**：满足任一即降级为 auto 预览 —— (a) 行数 >1200；(b) 文件 >96KB；(c) 文件 >4KB 且平均行长 >1200 字节（单行 JSON、压缩产物、单行日志这类无行级结构的内容）。要精读被降档的文件，显式传 mode=full（知代价）或 mode=lines 带 start_line/end_line。不传 mode 但带合法 start_line/end_line 时按 lines 处理"},
                "start_line":{"type":"integer","minimum":1,"description":"[必须]（mode=lines 时）起始行（≥ 1）"},
                "end_line":{"type":"integer","minimum":1,"description":"[必须]（mode=lines 时）结束行（含；≥ 1）"},
                "numbered":{"type":"boolean","default":true,"description":"[自动] 后端强制 true（每行加行号前缀），模型无需传"},
                "fresh":{"type":"boolean","default":false,"description":"[可选] true=强制绕过缓存重新读取最新内容（本会话已读过该文件全文时也放行）。默认 false（命中缓存/已读拦截）。确需重读最新状态（如文件可能已变化、审计收尾核对）时传 true"}
            },
            "required":["paths"],
            "additionalProperties":true
        })
    }

    fn output_schema(&self) -> Option<Value> {
        // 模型传参宽容（洗前），工具返回严格（洗后）——多一个字段/字段类型错都进不了模型。
        Some(json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "ok":{"type":"boolean"},
                "kind":{"type":"string"},
                "data":{"type":"object","additionalProperties":false,"properties":{
                    "files":{"type":"array","items":{"type":"object","additionalProperties":true,"properties":{
                        "path":{"type":"string"},"total_lines":{"type":"integer"},
                        "mode":{"type":"string"},"content":{"type":"string"},
                        "error":{},"note":{"type":"string"},"language":{"type":"string"},
                        "explanation":{"type":"string"},"truncated":{"type":"boolean"},
                        "structure":{"type":"array"},"deps":{"type":"array"}
                    }}},
                    "truncated":{"type":"boolean"}
                }},
                "warnings":{"type":"array"},
                "error":{}
            }
        }))
    }
    fn annotations(&self) -> Value {
        json!({"read_only": true, "destructive": false, "idempotent": true})
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args).map_err(|e| err_text(&e))?;
        let paths: Vec<String> = args["paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let explicit_mode = args.get("mode").and_then(|v| v.as_str());
        let mut mode = explicit_mode.unwrap_or("full").to_string();
        let start_line = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let end_line = args.get("end_line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let numbered = args
            .get("numbered")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if explicit_mode.is_none() && start_line > 0 && end_line >= start_line {
            mode = "lines".to_string();
        }
        // 模型反复重试同样残缺参数，后端却只给建议不代修。降级后模型直接拿到内容，任务继续。
        let mut degraded = false;
        let mut mode = mode;
        if mode == "lines" && (start_line == 0 || end_line == 0 || start_line > end_line) {
            mode = "full".to_string();
            degraded = true;
        }

        // 并行读取
        let mut futures = Vec::new();
        for p in &paths {
            let p = p.clone();
            let mode = mode.clone();
            // 后端按文件大小自主选档——大文件默认 auto 预览，不整灌上下文。
            let implicit_mode = explicit_mode.is_none() && start_line == 0 && end_line == 0;
            futures.push(async move {
                let file = resolve_guarded(&p)?;
                if crate::path::is_sensitive(&file) {
                    return Err(ToolError::domain(
                        "SENSITIVE_FILE",
                        format!("拒绝读取敏感文件（含凭据/配置）: {}", file.display()),
                        Some("该文件禁止工具读取，请通过其他方式查看"),
                    ));
                }
                let mut eff_mode = mode.clone();
                if implicit_mode {
                    if let Ok(meta) = std::fs::metadata(&file) {
                        let len = meta.len();
                        if len > AUTO_DOWNGRADE_BYTES {
                            eff_mode = "auto".to_string();
                        } else if let Ok(n) = count_lines(&file).await {
                            // 三条降档判据（任一命中）：行数多 / 平均行长爆长。
                            let dense = n > 0
                                && len > DENSE_MIN_BYTES
                                && len / (n as u64) > DENSE_AVG_LINE_BYTES;
                            if n > AUTO_DOWNGRADE_LINES || dense {
                                eff_mode = "auto".to_string();
                            }
                        }
                    }
                }
                if let Some(mime) = sniff_image(&file) {
                    return Ok(json!({
                        "path": p,
                        "kind": "image",
                        "mime": mime,
                        "mode": "image",
                        "total_lines": 0,
                        "content": format!(
                            "〔图片附件 {mime}〕已按视觉通道返回——你现在能**直接看到**这张图，不要再去读它的字节。"
                        ),
                    }));
                }
                read_one(&file, &eff_mode, start_line, end_line, numbered).await
            });
        }
        let results: Vec<Result<Value, ToolError>> = futures_util::future::join_all(futures).await;

        let mut files = Vec::new();
        let mut failed_paths: Vec<String> = Vec::new();
        let mut first_err: Option<ToolError> = None;
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Ok(v) => files.push(v),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e.clone());
                    }
                    let p = paths.get(i).cloned().unwrap_or_default();
                    failed_paths.push(p.clone());
                    files.push(json!({"path": p, "error": err_text(&e)}));
                }
            }
        }

        let succeeded = files.len().saturating_sub(failed_paths.len());
        if succeeded == 0 {
            if let Some(e) = first_err {
                return Err(err_text(&e));
            }
        }
        let mut warnings: Vec<String> = Vec::new();
        if degraded {
            warnings.push("mode=lines 缺少 start_line/end_line，已自动降级为全文读取（mode=full）。如需指定行区间，请同时传 start_line/end_line".to_string());
        }
        if !failed_paths.is_empty() {
            warnings.push(format!(
                "{}/{} 个路径读取失败，其余 {succeeded} 个**已成功返回**（见 data.files）；失败项在 files 里带 error 字段，路径：{}",
                failed_paths.len(),
                files.len(),
                failed_paths.join("、")
            ));
        }
        Ok(json!({
            "ok": true, "kind": "read_result",
            "data": {"files": files, "truncated": false},
            "warnings": warnings,
            "error": null
        }).to_string())
    }

    /// **非文本产出通道**：读到的是图片时，整包换成 `image_ref` 视觉附件。
    fn content_parts(&self, _args: &Value, output: &str) -> Option<Vec<ContentPart>> {
        let v: Value = serde_json::from_str(output).ok()?;
        let files = v.pointer("/data/files")?.as_array()?;
        if files.is_empty() {
            return None;
        }
        let mut parts: Vec<ContentPart> = Vec::new();
        for f in files {
            if f.get("kind").and_then(|k| k.as_str())? != "image" {
                return None;
            }
            let p = f.get("path").and_then(|x| x.as_str())?;
            let mime = f.get("mime").and_then(|x| x.as_str())?;
            let bytes = std::fs::read(resolve_guarded(p).ok()?).ok()?;
            let data_url = format!(
                "data:{mime};base64,{}",
                crate::tools::base::base64_encode(&bytes)
            );
            parts.push(ContentPart::text(format!(
                "📎 {p}（{mime}）：视觉内容已按附件回灌，直接看上一轮上下文里的附件；\
需要再看时 read 该路径即可重新取回"
            )));
            parts.push(ContentPart::image_ref(data_url, mime));
        }
        Some(parts)
    }

    /// （行号 + 问题类型 + 修复方向）。轻量：每文件 ≤15 条，随 envelope 进模型上下文，
    fn structured_content(&self, _args: &Value, output: &str) -> Option<Value> {
        let v: Value = serde_json::from_str(output).ok()?;
        let files = v.pointer("/data/files").and_then(|f| f.as_array())?;
        let mut out_files = Vec::new();
        for f in files.iter().take(20) {
            let path = f.get("path").and_then(|p| p.as_str()).unwrap_or("?");
            if path == "?" {
                continue;
            }
            let content = f.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let total = f.get("total_lines").and_then(|t| t.as_u64()).unwrap_or(0);
            let issues = scan_issues(content);
            if issues.is_empty() {
                continue;
            }
            out_files.push(json!({
                "path": path,
                "total_lines": total,
                "issues": issues,
            }));
        }
        if out_files.is_empty() {
            None
        } else {
            Some(json!({"kind": "read_issues", "files": out_files}))
        }
    }
}

/// T2T 协议 v1：规则引擎问题扫描（保守高信号，零误报优先）——按行定位常见代码问题，
pub(crate) fn contains_in_code(line: &str, needle: &str) -> bool {
    let b = line.as_bytes();
    let n = b.len();
    let mut code = String::with_capacity(n);
    let mut idx = 0;
    while idx < n {
        // 行注释：// 之后全部忽略
        if idx + 1 < n && b[idx] == b'/' && b[idx + 1] == b'/' {
            break;
        }
        // 字符串字面量："..."（含转义）整体跳过
        if b[idx] == b'"' {
            idx += 1;
            while idx < n {
                if b[idx] == b'\\' {
                    idx += 1;
                } else if b[idx] == b'"' {
                    idx += 1;
                    break;
                }
                idx += 1;
            }
            continue;
        }
        code.push(b[idx] as char);
        idx += 1;
    }
    code.contains(needle)
}

pub(crate) fn scan_issues(content: &str) -> Vec<Value> {
    const MAX_ISSUES_PER_FILE: usize = 15;
    let mut issues: Vec<Value> = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let n = (i + 1) as u64;
        let _trimmed = line.trim_start();
        // ① 未完成/待办标记（注释）
        if line.contains("TODO")
            || line.contains("FIXME")
            || line.contains("HACK")
            || line.contains("XXX")
        {
            issues.push(
                json!({"line": n, "kind": "TODO", "hint": "未完成/待办标记，需补实现或清理"}),
            );
        }
        // ② 可能 panic：unwrap/expect（Rust）——仅真实代码上下文（跳过注释/字符串）
        if contains_in_code(line, ".unwrap()") || contains_in_code(line, ".expect(") {
            issues.push(json!({"line": n, "kind": "PANIC_RISK", "hint": "unwrap/expect 可能 panic，建议改为错误传播或提供默认值"}));
        }
        // ③ 显式 panic（Rust）——仅真实代码上下文（跳过注释/字符串，避免规则行/夹具字符串自误报）
        if contains_in_code(line, "panic!(") {
            issues.push(json!({"line": n, "kind": "PANIC", "hint": "显式 panic，触发即崩溃，建议用 Result 传播"}));
        }
        // ④ 调试残留（前端/JS/TS）
        if line.contains("console.log(") || line.contains("debugger;") {
            issues.push(json!({"line": n, "kind": "DEBUG_RESIDUE", "hint": "调试输出/断点残留，上线前应移除"}));
        }
        // ⑤ 超长行（可读性/可能是压缩产物）
        if line.chars().count() > 200 {
            issues.push(json!({"line": n, "kind": "LONG_LINE", "hint": "行超 200 字符，可读性差，建议拆分"}));
        }
        // ⑥ 吞异常：catch 块为空或仅注释（JS/TS 简配：catch 后同行 {})
        if line.contains("catch") && line.contains("{}") {
            issues.push(json!({"line": n, "kind": "SWALLOWED_ERROR", "hint": "空 catch 吞异常，建议至少记录日志"}));
        }
        if issues.len() >= MAX_ISSUES_PER_FILE {
            break;
        }
    }
    issues
}

/// 给文本每行加行号前缀（如 "   12\tcode"），便于模型引用具体行号作为证据。
fn number_lines(text: &str, start: usize) -> String {
    text.lines()
        .enumerate()
        .map(|(i, l)| format!("{:>5}\t{}", start + i, l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从文件路径推断语言（Tool Result Grounding：read 结果说明的精细标注，轻量映射）
fn language_hint(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "jsx" => "react/jsx",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        "sh" | "bash" => "shell",
        "bat" | "cmd" => "batch",
        "ps1" => "powershell",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "c" => "c",
        "h" => "c-header",
        "cpp" | "cc" | "cxx" => "cpp",
        "hpp" => "cpp-header",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "xml" => "xml",
        "ini" => "ini",
        "txt" => "text",
        "csv" => "csv",
        _ => "text",
    }
}

/// read 结果的精细说明（Tool Result Grounding：模式/行数/截断续读指引/引用方式）
fn read_explanation(mode: &str, total_lines: usize, note: &str) -> String {
    match mode {
        "full" if note.is_empty() => {
            format!("文件共 {total_lines} 行，全量返回。每行带行号前缀（形如 \"   12\\tcode\"）仅供精确定位；改它请用 modify 的 {{\"mode\":\"line\",\"line\":N,\"replace\":\"...\"}}（old 由后端自动代取）或 #En 引用，**勿把带行号的行复制进 find/replace**。")
        }
        "full" => format!("文件共 {total_lines} 行，{note}"),
        "lines" => format!("行区间片段（文件共 {total_lines} 行，{note}）"),
        _ => format!("auto 预览模式（文件共 {total_lines} 行，{note}）"),
    }
}

/// 按内容嗅探图片（魔数），返回 mime。**判据是内容，与扩展名无关**——
fn sniff_image(path: &Path) -> Option<&'static str> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 32];
    let n = std::io::BufReader::new(f).read(&mut head).ok()?;
    image::guess_format(&head[..n]).ok().map(|fmt| fmt.to_mime_type())
}

/// 二进制文件被 read 时的拒绝 + 正确路径指引。
fn binary_file_error(file: &Path, len: usize) -> ToolError {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let (kind, hint) = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" => (
            "图片",
            "read 是文本读取器，**看不了图片**——它只会把像素字节按文本解码成乱码。要查看这张图，**请把它作为附件发送**（系统会把它注入模型的视觉输入）；若只想确认它的存在/大小/尺寸，用 run 调相应命令。",
        ),
        "pdf" | "docx" | "xlsx" | "pptx" | "doc" | "xls" | "ppt" => (
            "文档",
            "文档请用 **doc 工具**读取（read 读不了它内部的二进制结构）。",
        ),
        "zip" | "rar" | "7z" | "gz" | "tar" | "xz" | "bz2" => (
            "压缩包",
            "压缩包需先解压：用 run 解压到临时目录后，再 read 里面的文本文件。",
        ),
        "exe" | "dll" | "so" | "dylib" | "bin" | "lib" | "pdb" | "obj" | "class" => (
            "可执行/库文件",
            "可执行/库文件不由 read 读取。要查符号或内嵌字符串，用 run 调 strings / dumpbin / nm。",
        ),
        "mp3" | "mp4" | "wav" | "avi" | "mkv" | "mov" | "webm" | "flac" => {
            ("音视频", "音视频是二进制媒体，read 读不了。")
        }
        "gguf" | "safetensors" | "onnx" | "pt" | "ckpt" | "h5" | "pb" => (
            "模型权重",
            "模型权重是二进制大件，read 读不了，也**不要**读（会撑爆上下文）。",
        ),
        _ => (
            "二进制文件",
            "这是二进制文件（内容含 NUL 字节，按内容判定，与扩展名无关），read 是文本读取器，读不了。要提取其中可读字符串，用 run 调 strings。",
        ),
    };
    ToolError::domain(
        "BINARY_FILE",
        format!("{} 是{kind}（{len} 字节，二进制），read 无法作为文本读取", file.display()),
        Some(hint),
    )
}

pub(crate) async fn read_one(
    file: &Path,
    mode: &str,
    start_line: usize,
    end_line: usize,
    numbered: bool,
) -> Result<Value, ToolError> {
    if file.is_dir() {
        // IS_DIRECTORY 直接带回目录清单（确定性，后端自己就能列）——
        let mut listing = String::new();
        if let Ok(mut entries) = tokio::fs::read_dir(file).await {
            let mut names: Vec<String> = Vec::new();
            while let Ok(Some(e)) = entries.next_entry().await {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = e.path().is_dir();
                names.push(if is_dir { format!("{name}/") } else { name });
                if names.len() >= 20 {
                    break;
                }
            }
            names.sort();
            listing = names.join("\n  ");
        }
        let suggestion = if listing.is_empty() {
            "请用 list 查看目录内容，或指定具体文件".to_string()
        } else {
            format!(
                "该目录内容（直接 read 里面的具体文件即可，勿再读目录本身）：\n  {listing}"
            )
        };
        return Err(ToolError::domain(
            "IS_DIRECTORY",
            format!("{} 是目录", file.display()),
            Some(suggestion.as_str()),
        ));
    }
    let raw = fs::read(file).await.map_err(|e| {
        let not_found = e.kind() == std::io::ErrorKind::NotFound;
        let code = if not_found { "NOT_FOUND" } else { "READ_FAILED" };
        let hint = if not_found {
            match file.parent() {
                Some(p) if p.is_dir() => "目录存在、文件不存在 —— 这通常是**本目录里第一次创建它**（日志/记忆/新产物都这么来）。**直接 write 创建即可**，不必再 list、也不要换路径。".to_string(),
                Some(p) => format!(
                    "父目录也不存在：{} —— 这才是路径问题：先 list 确认项目当前结构，或用 write 先建目录。",
                    p.display()
                ),
                None => "路径解析不出父目录 —— 请传文件的绝对路径。".to_string(),
            }
        } else {
            "读取失败（不是\"不存在\"）。若反复失败，改用 audit 看目录结构，不要反复重读同一路径。".to_string()
        };
        ToolError::domain(code, format!("读取 {} 失败: {e}", file.display()), Some(hint.as_str()))
    })?;
    if raw[..raw.len().min(BINARY_PROBE_BYTES)].contains(&0) {
        return Err(binary_file_error(file, raw.len()));
    }
    let (content, non_utf8) = super::fs_common::decode_text(&raw);
    let enc_note = if non_utf8 { super::fs_common::encoding_note() } else { "" };
    let total_lines = content.lines().count();
    let name = file.display().to_string();

    let (text, kind_note, truncated) = match mode {
        "full" => {
            let chars = content.chars().count();
            if chars > FULL_MAX_CHARS {
                // 截断时给出行级定位（read_lines 表示已读到第几行），模型可用 lines 模式续读
                let t: String = content.chars().take(FULL_MAX_CHARS).collect();
                let read_lines = t.lines().count();
                (
                    t,
                    format!(
                        "（全文已截断：前 {FULL_MAX_CHARS} 字符，读到第 {read_lines}/{total_lines} 行；如需剩余内容请用 mode=lines 读取第 {}+ 行）",
                        read_lines + 1
                    ),
                    true,
                )
            } else {
                (content.clone(), String::new(), false)
            }
        }
        "lines" => {
            let sel: Vec<&str> = content
                .lines()
                .enumerate()
                .filter(|(i, _)| i + 1 >= start_line && i + 1 <= end_line)
                .map(|(_, l)| l)
                .collect();
            (
                sel.join("\n"),
                format!("（第 {start_line}-{end_line} 行）"),
                false,
            )
        }
        _ => {
            // 结构摘要（函数/类/方法定义+行号）由下方 extract_structure 生成后
            let preview = if total_lines <= 100 {
                content.clone()
            } else if total_lines <= 500 {
                content.chars().take(SUMMARY_PREVIEW_CHARS * 2).collect()
            } else if total_lines <= 2000 {
                // 大文件：函数列表 + 每个定义前 20 行关键段 + 开头 100 行
                let structure = extract_structure(&content);
                let mut seg = String::new();
                seg.push_str(&content.lines().take(100).collect::<Vec<_>>().join("\n"));
                for (ln, kind, nm) in structure.iter().take(30) {
                    let start = *ln;
                    let slice: Vec<&str> = content.lines().skip(start).take(20).collect();
                    if !slice.is_empty() {
                        seg.push_str(&format!(
                            "\n…\n// {kind} {nm} @L{start}\n{}",
                            slice.join("\n")
                        ));
                    }
                }
                seg.chars().take(SUMMARY_PREVIEW_CHARS * 2).collect()
            } else {
                // >2000 行超大病：只给开头 + 结构由下方 extract_structure 附加，提示续读
                content.chars().take(SUMMARY_PREVIEW_CHARS).collect()
            };
            let truncated = preview.chars().count() < content.chars().count();
            // 否则模型不知道读到哪、反复 read（缺口 #2）。依据当前行的内容行数推算已展示到的行号
            let preview_lines = preview.lines().count();
            let note = if truncated {
                if preview_lines < total_lines {
                    format!("（预览已截断：当前展示约前 {preview_lines}/{total_lines} 行；需看剩余内容请用 mode=lines 传 start_line={} 或针对性读结构摘要中的行号）", preview_lines + 1)
                } else {
                    "（已看完全文）".to_string()
                }
            } else {
                "（已看完全文）".to_string()
            };
            (preview, note, truncated)
        }
    };

    // numbered：给返回内容每行加行号前缀，便于引用具体行号作为证据
    let number_start = if mode == "lines" { start_line } else { 1 };
    let text = if numbered {
        number_lines(&text, number_start)
    } else {
        text
    };

    // 全局认知、按行号精准续读（渐进式披露）。上限 60 条防超长。
    let structure = extract_structure(&content);
    let mut expl = read_explanation(mode, total_lines, &kind_note);
    // 非 UTF-8 降级**必须留痕**：否则模型会把按 GBK 解出的中文当作原样事实。
    if non_utf8 {
        expl = format!("{enc_note}\n{expl}");
    }
    if !structure.is_empty() {
        let list: Vec<String> = structure
            .iter()
            .take(60)
            .map(|(ln, kind, nm)| format!("L{ln} {kind} {nm}"))
            .collect();
        expl.push_str(&format!(
            "\n【结构摘要 · {} 个定义{}】\n{}",
            structure.len(),
            if structure.len() > 60 {
                "（前 60 条）"
            } else {
                ""
            },
            list.join("\n"),
        ));
    }

    let mut out = json!({
        "path": name, "total_lines": total_lines, "mode": mode, "note": kind_note, "truncated": truncated,
        "language": language_hint(&name),
        // 明确编码（`utf-8` / `gbk`）：模型据此判断中文是否可靠 —— 非 UTF-8 时内容是按 GBK 解出的。
        "encoding": if non_utf8 { "gbk" } else { "utf-8" },
        "explanation": expl,
        "content": text,
        // 模型读文件即知依赖（免 grep/免读全部 import 文件判断关系）
        "imports": crate::backbone::info_extractor::InfoExtractor::extract_imports(&content),
    });
    // 分段读取（lines 模式）带区间——需求"这次读多少到多少行"全程可见
    if mode == "lines" {
        out["start_line"] = json!(number_start);
        out["end_line"] = json!(number_start + content.lines().count().saturating_sub(1));
    }
    if !structure.is_empty() {
        out["structure"] = json!(structure
            .iter()
            .take(60)
            .map(|(ln, kind, nm)| json!({"line": ln, "kind": kind, "name": nm}))
            .collect::<Vec<_>>());
    }
    Ok(out)
}

// search（正则递归搜索）

pub fn looks_like_tool_envelope(content: &str) -> bool {
    let t = content.trim_start();
    if !t.starts_with('{') {
        return false;
    }
    let markers = [
        "\"kind\":\"read_result\"",
        "\"kind\":\"search_result\"",
        "\"kind\":\"find_files_result\"",
        "\"kind\":\"list_result\"",
        "\"kind\":\"write_result\"",
        "\"kind\":\"run_result\"",
        "\"tool_call_id\"",
    ];
    markers.iter().any(|m| t.contains(m))
}

// 列出目录内容（文件+子目录+大小+行数），支持递归；替代"run dir"的笨重方式。

// 工具面裁剪保留（未注册）：列目录并入 run（dir /b、Get-ChildItem）
#[allow(dead_code)]
pub struct ListTool;

#[async_trait]
impl BuiltinTool for ListTool {
    fn name(&self) -> &'static str {
        "list"
    }
    fn description(&self) -> &'static str {
        // （已裁剪出工具面，：列目录并入 run——dir /b、Get-ChildItem。
        "列出目录内容（已并入 run 工具；本工具当前未注册，仅保留实现）"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "path":{"type":"string","default":".","description":"目录路径（绝对或相对）"},
                "recursive":{"type":"boolean","default":true,"description":"是否递归列出子树（默认 true：一次给出完整目录树，避免一层一层调模型）"},
                "max_depth":{"type":"integer","default":8,"minimum":1,"maximum":12,"description":"recursive=true 时的最大递归深度（默认 8：审查/梳理项目看到深层结构；超大仓库可传 12）"},
                "max_entries":{"type":"integer","default":5000,"minimum":1,"maximum":50000,"description":"返回条目上限（1–50000；默认 5000，正常项目一轮给全，仅极端仓库触发截断）"},
                "detail":{"type":"boolean","default":false,"description":"是否附带每个文件的 size/lines（默认 false：仅列名字，不读文件内容、省 IO；需要行数时显式传 true）"}
            },
            "additionalProperties":true
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": true, "destructive": false, "idempotent": true})
    }

    fn output_schema(&self) -> Option<Value> {
        Some(json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "ok":{"type":"boolean"},
                "kind":{"type":"string"},
                "data":{"type":"object","additionalProperties":false,"properties":{
                    "path":{"type":"string"},
                    "truncated":{"type":"boolean"},
                    "hint":{"type":"string"},
                    "dirs":{"type":"integer"},
                    "files":{"type":"integer"},
                    "entries":{"type":"array","items":{"type":"object","additionalProperties":true,"properties":{
                        "name":{"type":"string"},"type":{"type":"string"},
                        "path":{"type":"string"},"size":{},"lines":{},"depth":{"type":"integer"}
                    }}}
                }},
                "warnings":{"type":"array"},
                "error":{}
            }
        }))
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args).map_err(|e| err_text(&e))?;
        let start =
            resolve_guarded(args["path"].as_str().unwrap_or(".")).map_err(|e| err_text(&e))?;
        if !start.exists() {
            return Err(err_text(&ToolError::domain(
                "PATH_NOT_FOUND",
                format!("路径不存在: {}{}", start.display(), parent_list_hint(&start)),
                Some("请确认 path 指向真实目录；父目录条目见上方"),
            )));
        }
        if !start.is_dir() {
            return Err(err_text(&ToolError::domain(
                "NOT_DIRECTORY",
                format!("{} 不是目录", start.display()),
                Some("列目录请传目录路径；读文件请用 read"),
            )));
        }
        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
        let max_entries = args
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000) as usize;
        let detail = args.get("detail").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut entries: Vec<Value> = Vec::new();
        let mut dirs = 0usize;
        let mut files = 0usize;
        let mut scanned = 0usize;
        let mut truncated = false;

        // 递归 DFS（携带深度）；非递归只扫一层。默认递归整树，一轮给全（避免一层一层调模型）。
        let mut stack: Vec<(PathBuf, usize)> = vec![(start.clone(), 0)];
        let mut unreadable_dirs: usize = 0;
        'walk: while let Some((dir, depth)) = stack.pop() {
            if entries.len() >= max_entries {
                truncated = true;
                break 'walk;
            }
            let read = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => {
                    unreadable_dirs += 1;
                    continue;
                }
            };
            for entry in read.flatten() {
                scanned += 1;
                if scanned > 100_000 {
                    truncated = true;
                    break 'walk;
                }
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    dirs += 1;
                    if recursive && depth + 1 <= max_depth {
                        // 子目录本身先入条目，再入栈展开
                        if !SKIP_DIRS.contains(&name.as_str()) {
                            entries.push(json!({"name": name, "type": "dir", "path": path.display().to_string(), "depth": depth}));
                            stack.push((path, depth + 1));
                        }
                    } else if depth == 0 {
                        entries.push(json!({"name": name, "type": "dir", "path": path.display().to_string(), "depth": 0}));
                    }
                } else {
                    files += 1;
                    if depth == 0 || recursive {
                        let mut e = json!({
                            "name": name, "type": "file",
                            "path": path.display().to_string(), "depth": depth
                        });
                        if detail {
                            // detail=true 才读文件算行数（读失败 → null，不伪装成 0）
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            let lines: Value = match std::fs::read_to_string(&path) {
                                Ok(c) => json!(c.lines().count() as u64),
                                Err(_) => Value::Null,
                            };
                            e["size"] = json!(size);
                            e["lines"] = lines;
                        }
                        entries.push(e);
                    }
                }
            }
        }

        let mut warnings = Vec::new();
        if unreadable_dirs > 0 {
            warnings.push(format!(
                "{unreadable_dirs} 个目录不可读已跳过（权限或 IO 错误），结果可能不完整"
            ));
        }

        let hint = if truncated {
            "结果已达 max_entries 上限（或被扫描上限打断）；请缩小 path、传 recursive=false 聚焦，或用 find_files 按 pattern 收窄"
        } else { "" };
        Ok(json!({
            "ok": true, "kind": "list_result",
            "data": {
                "path": start.display().to_string(),
                "recursive": recursive,
                "truncated": truncated,
                "hint": hint,
                "total": entries.len(), "dirs": dirs, "files": files,
                "entries": entries
            },
            "warnings": warnings, "error": null
        })
        .to_string())
    }
}

// search（正则递归搜索）

#[allow(dead_code)]
pub struct SearchTool;

#[allow(dead_code)]
pub(crate) const MAX_MATCHES: usize = 100;
/// 模型看到命中行前后文当场确定，不再 read 确认；显式传 context=0 关闭）。
#[allow(dead_code)]
pub(crate) const DEFAULT_SEARCH_CONTEXT: usize = 5;
/// 默认窗口只附给前 N 处命中（防 100 命中 × 窗口 上下文爆炸）。
#[allow(dead_code)]
pub(crate) const CONTEXT_WINDOW_CAP: usize = 15;
#[allow(dead_code)]
pub(crate) const MAX_FILES_SCANNED: usize = 2000;
                                                  // 中等项目应一次扫完；超大项目（>2000）靠 scan_offset/next_offset 续扫（已实现）。
#[allow(dead_code)]
pub(crate) const MAX_SEARCH_DEPTH: usize = 6;

/// 单个文件超过它**不读、不搜**（只记跳过，不进内存）。
#[allow(dead_code)]
pub(crate) const MAX_SEARCH_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// 返回该文件命中数；超过 MAX_MATCHES 时置 truncated
#[allow(dead_code)]
pub(crate) async fn scan_file_content(
    path: &std::path::Path,
    regex: &regex::Regex,
    output_mode: &str,
    context: usize,
    matches: &mut Vec<Value>,
    files_hit: &mut Vec<String>,
    truncated: &mut bool,
    skipped_big: &mut Vec<String>,
    skipped_binary: &mut std::collections::BTreeMap<String, usize>,
) -> usize {
    // ① 体积闸（**先于读取**）：用 metadata 判大小，绝不把大件读进内存。
    if let Ok(meta) = fs::metadata(path).await {
        if meta.len() > MAX_SEARCH_FILE_BYTES {
            skipped_big.push(format!(
                "{}（{:.1} MB）",
                path.display(),
                meta.len() as f64 / 1_048_576.0
            ));
            return 0;
        }
    }
    let bytes = match fs::read(path).await {
        Ok(b) => b,
        Err(_) => return 0,
    };
    // ② **二进制判据：按内容判，不按名字判**（`file(1)` 同款，格式无关）。
    if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
        let tag = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_else(|| "(无扩展名)".to_string());
        *skipped_binary.entry(tag).or_insert(0usize) += 1;
        return 0;
    }
    let content = super::fs_common::decode_text(&bytes).0;
    let lines: Vec<&str> = content.lines().collect();
    let mut file_count = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if !regex.is_match(line) {
            continue;
        }
        file_count += 1;
        match output_mode {
            "files" => {
                let p = path.display().to_string();
                if !files_hit.contains(&p) {
                    files_hit.push(p);
                }
            }
            "count" => {}
            _ => {
                let mut entry =
                    json!({"file": path.display().to_string(), "line": i + 1, "text": line});
                // 窗口只给前 CONTEXT_WINDOW_CAP 处命中（防上下文爆炸）；其余只给命中行，
                if context > 0
                    && matches
                        .iter()
                        .filter(|m| m.get("context").is_some())
                        .count()
                        < CONTEXT_WINDOW_CAP
                {
                    let lo = i.saturating_sub(context);
                    let hi = (i + context).min(lines.len() - 1);
                    let ctx: Vec<Value> = (lo..=hi)
                        .map(|j| json!({"line": j + 1, "text": lines[j], "match": j == i}))
                        .collect();
                    entry["context"] = Value::Array(ctx);
                }
                matches.push(entry);
            }
        }
        if matches.len() + files_hit.len() >= MAX_MATCHES {
            *truncated = true;
            break;
        }
    }
    if output_mode == "count" && file_count > 0 {
        matches.push(json!({"file": path.display().to_string(), "count": file_count}));
    }
    file_count
}

#[async_trait]
impl BuiltinTool for SearchTool {
    fn name(&self) -> &'static str {
        "search"
    }
    fn description(&self) -> &'static str {
        concat!(
            "在项目中按正则搜索文本（类似 grep）——**找代码/文本位置 → 用 search**。默认返回匹配的文件路径、行号与行内容，**并自动附带命中行前后 5 行上下文（每行带行号，命中行标 ▶）——看到即确定，通常无需再 read 确认**；output_mode=files 只列命中文件、count 只给每文件命中数；context=N 自定义窗口（0 关闭、最大 10），默认窗口只附前 15 处命中（命中更多时其余给命中行+提示，细节用 read 指定行号）。**正则区分大小写；忽略大小写请在 pattern 前加 (?i)（如 (?i)serializer）**。最多 100 条；自动跳过 node_modules/target/dist/.git，**并跳过二进制文件**（按内容判定：文件头含 NUL 字节即非文本，与扩展名无关）——它们**不读进内存、不计入结果**，跳过项在 warnings 里列出：**零命中不等于不存在，先看 warnings**；确实要搜二进制内容请用 run 走 findstr/Select-String。**续扫**：结果含 next_offset 时传 scan_offset=next_offset 从断点继续。\n返回 JSON 结构：{\"kind\":\"search_result\",\"data\":{\"total\":N,\"matches\":[{\"file\":\"绝对路径\",\"line\":N,\"text\":\"行内容\",\"context\":[{\"line\":N,\"text\":\"前后文行\",\"match\":bool}]}]}}——引用结果作为后续 read/edit 的路径参数用 #En 占位符。",
            include_str!("../../prompts/tools/spill_note.md")
        )
    }
    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "pattern":{"type":"string","minLength":1,"description":"正则表达式"},
                "path":{"type":"string","default":".","description":"搜索起始路径（相对项目根或绝对）"},
                "file_pattern":{"type":"string","description":"可选：按扩展名过滤文件（只支持 *.rs 形态=仅 .rs 文件；其他 glob 形态如 **/*.rs 不支持，会报错）"},
                "output_mode":{"type":"string","enum":["content","files","count"],"default":"content","description":"content=行内容(默认), files=仅文件列表, count=每文件命中数"},
                "context":{"type":"integer","minimum":0,"maximum":10,"default":5,"description":"匹配行前后附带的上下文行数(类 grep -C)；**0–10**，默认 5（自动附窗口，命中行带行号+▶，看到即确定免 read 确认）；传 0 关闭"},
                "scan_offset":{"type":"integer","minimum":0,"default":0,"description":"[续扫] 跳过前 N 个文件（上次返回 next_offset 的值）——单次上限 500 文件，大项目分多次续扫全扫完"},
                
            },
            "required":["pattern"],
            "additionalProperties":true
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": true, "destructive": false, "idempotent": true})
    }

    fn output_schema(&self) -> Option<Value> {
        Some(json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "ok":{"type":"boolean"},
                "kind":{"type":"string"},
                "data":{"type":"object","additionalProperties":false,"properties":{
                    "total":{"type":"integer"},
                    "matches":{"type":"array","items":{"type":"object","additionalProperties":true,"properties":{
                        "file":{"type":"string"},"line":{"type":"integer"},"text":{"type":"string"}
                    }}},
                    "has_more":{"type":"boolean"},"files_scanned":{"type":"integer"},
                    "next_offset":{"type":"integer","description":"续扫位置：>0 时传 scan_offset=next_offset 继续；0 = 已扫完无续扫点"},"truncated_reason":{},"explanation":{"type":"string"}
                }},
                "warnings":{"type":"array"},
                "error":{}
            }
        }))
    }

    /// 字符级截断，模型引用结果提取路径失败。此处把 matches[].file 去重清单走
    fn structured_content(&self, _args: &Value, output: &str) -> Option<Value> {
        let v: Value = serde_json::from_str(output).ok()?;
        let matches = v.pointer("/data/matches").and_then(|m| m.as_array())?;
        let mut files: Vec<Value> = Vec::new();
        for m in matches {
            if let Some(f) = m.get("file").and_then(|f| f.as_str()) {
                let item = json!(f);
                if !files.contains(&item) {
                    files.push(item);
                }
            }
        }
        if files.is_empty() {
            None
        } else {
            Some(json!({"kind": "search_files", "files": files}))
        }
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args).map_err(|e| err_text(&e))?;
        let pattern_str = args["pattern"].as_str().unwrap();
        let start =
            resolve_guarded(args["path"].as_str().unwrap_or(".")).map_err(|e| err_text(&e))?;
        let file_pattern = args.get("file_pattern").and_then(|v| v.as_str());
        let output_mode = args
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("content");
        let context = match args.get("context") {
            Some(v) => v.as_u64().unwrap_or(0) as usize,
            // 未传 context → 默认带窗口（智能投喂：命中自带前后文，模型免 read 确认）
            None => DEFAULT_SEARCH_CONTEXT,
        };
        // 单次上限 500 文件，大项目分多次续扫全扫完（不再每次从头扫 500 截断）。
        let scan_offset = args
            .get("scan_offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let regex = regex::Regex::new(pattern_str).map_err(|e| {
            err_text(&ToolError::domain(
                "INVALID_REGEX",
                format!("正则语法错误: {e}"),
                Some("请检查 pattern"),
            ))
        })?;

        if !start.exists() {
            return Err(err_text(&ToolError::domain(
                "PATH_NOT_FOUND",
                format!("路径不存在: {}{}", start.display(), parent_list_hint(&start)),
                Some("请确认 path；父目录真实条目见上方（跨项目记错文件名时尤其先看它）"),
            )));
        }

        let mut matches = Vec::new();
        let mut files_hit: Vec<String> = Vec::new();
        let mut truncated = false;
        let mut files_scanned: usize = 0;
        let mut skipped: usize = 0;
        // 因体积过大被跳过的文件（**必须回报给模型**，见 MAX_SEARCH_FILE_BYTES 的"留痕"要求）
        let mut skipped_big: Vec<String> = Vec::new();
        // 按扩展名跳过的二进制资产（**按类型聚合** —— 逐个列会刷屏，而它动辄上百个）
        let mut skipped_bin: std::collections::BTreeMap<String, usize> = Default::default();
        let trunc_reason: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);

        // files_scanned=0 假空结果。文件路径必须直接搜该文件，不进入目录遍历；
        if start.is_file() {
            // 预检只为"确认这个路径可读"（实际扫描在 `scan_file_content`，它自带宽容解码）。
            if let Err(e) = fs::read(&start).await {
                let code = if e.kind() == std::io::ErrorKind::NotFound {
                    "NOT_FOUND"
                } else {
                    "READ_FAILED"
                };
                return Err(err_text(&ToolError::domain(
                    code,
                    format!("读取 {} 失败: {e}", start.display()),
                    Some("请确认路径；若该文件确实不存在，先 list/audit 确认项目当前结构"),
                )));
            }
            scan_file_content(
                &start,
                &regex,
                output_mode,
                context,
                &mut matches,
                &mut files_hit,
                &mut truncated,
                &mut skipped_big,
                &mut skipped_bin,
            )
            .await;
            files_scanned = 1;
        } else {
            let mut stack: Vec<(PathBuf, usize)> = vec![(start.clone(), 0)];
            while let Some((dir, depth)) = stack.pop() {
                if depth > MAX_SEARCH_DEPTH {
                    continue;
                }
                if matches.len() >= MAX_MATCHES || files_scanned >= MAX_FILES_SCANNED {
                    truncated = true;
                    break;
                }
                let mut entries = match fs::read_dir(&dir).await {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
                    let path = entry.path();
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if path.is_dir() {
                        if !SKIP_DIRS.contains(&fname.as_str()) {
                            stack.push((path, depth + 1));
                        }
                    } else if path.is_file() {
                        // "token/exec/unsafe" 等搜索（comma-separated-tokens 之类）
                        if SKIP_FILES.contains(&fname.as_str()) {
                            continue;
                        }
                        // file_pattern 过滤：扩展名精确匹配（*.rs → 仅 .rs）
                        if let Some(fp) = file_pattern {
                            let want = fp.trim_start_matches("*.").trim();
                            let got = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if want.is_empty() || got != want {
                                continue;
                            }
                        }
                        // 二进制判定**不在这层做** —— 它要看内容（NUL 判据），
                        if scan_offset > 0 && skipped < scan_offset {
                            skipped += 1;
                            continue;
                        }
                        if matches.len() >= MAX_MATCHES {
                            truncated = true;
                            break;
                        }
                        if files_scanned >= MAX_FILES_SCANNED {
                            truncated = true;
                            *trunc_reason.borrow_mut() =
                                Some(format!("扫描文件超限（>{MAX_FILES_SCANNED}）"));
                            break;
                        }
                        files_scanned += 1;
                        scan_file_content(
                            &path,
                            &regex,
                            output_mode,
                            context,
                            &mut matches,
                            &mut files_hit,
                            &mut truncated,
                            &mut skipped_big,
                            &mut skipped_bin,
                        )
                        .await;
                        if matches.len() >= MAX_MATCHES {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
        }

        // files 模式：matches 为空、files_hit 收集了命中文件列表 → 统一输出
        let (total, out_matches) = if output_mode == "files" && !files_hit.is_empty() {
            let arr: Vec<Value> = files_hit.iter().map(|f| json!({"file": f})).collect();
            (arr.len(), arr)
        } else {
            (matches.len(), matches)
        };

        // （如 data/ 缓存目录）——静默返回空 matches 会让模型误以为"没代码"。
        if files_scanned == 0 && out_matches.is_empty() {
            return Err(err_text(&ToolError::domain(
                "SEARCH_NO_FILES",
                format!("在 {} 中未扫描到任何文件（files_scanned=0）——路径可能指向噪音/缓存目录或空目录", start.display()),
                Some("改用项目根目录或 src/ 等源码目录，或用 find_files 先确认目标路径"),
            )));
        }

        // 模型下次带 scan_offset=next_offset 从断点继续，不再从头扫。未截断 = 0（无续扫点）。
        let global_scanned = scan_offset + files_scanned;
        let next_offset = if truncated && files_scanned >= MAX_FILES_SCANNED {
            global_scanned
        } else {
            0
        };

        // 超大文件跳过必须**显式回报**（静默漏比慢更坏：模型会把"漏"当成"不存在"）
        let mut warnings: Vec<String> = Vec::new();
        if !skipped_big.is_empty() {
            warnings.push(format!(
                "{} 个文件因超过 {} MB 未被搜索（**未读进内存**）：{}。它们可能是二进制大件（模型权重 / dll / 媒体）；确实要搜请指定精确路径单独搜，或用 run 走 findstr / Select-String。",
                skipped_big.len(),
                MAX_SEARCH_FILE_BYTES / 1_048_576,
                skipped_big.iter().take(5).cloned().collect::<Vec<_>>().join("、"),
            ));
        }
        // 二进制：**告诉模型"跳过了什么"**，否则它会把零命中读成"不存在"
        if !skipped_bin.is_empty() {
            let total: usize = skipped_bin.values().sum();
            let kinds: Vec<String> = skipped_bin
                .iter()
                .take(8)
                .map(|(e, n)| {
                    if e.starts_with('(') {
                        format!("{e} ×{n}")
                    } else {
                        format!(".{e} ×{n}")
                    }
                })
                .collect();
            warnings.push(format!(
                "已跳过 {total} 个二进制文件（**按内容判定**：文件头含 NUL 字节，不是文本；{}{}）—— 未读进内存、未参与匹配。**零命中不等于不存在**；若确实要搜二进制内容，请用 run 走 findstr / Select-String 指名路径。",
                kinds.join("、"),
                if skipped_bin.len() > 8 { " 等" } else { "" }
            ));
        }

        Ok(json!({
            "ok": true, "kind": "search_result",
            "data": {
                "total": total, "matches": out_matches, "has_more": truncated, "files_scanned": files_scanned,
                "next_offset": next_offset,
                "truncated_reason": trunc_reason.borrow().clone(),
                "explanation": format!(
                    "扫描 {} 个文件（全局已扫 {}），命中 {} 条{}{}{}",
                    files_scanned,
                    global_scanned,
                    total,
                    if truncated { "（已达上限截断，可用 scan_offset 续扫或缩小 pattern）" } else { "" },
                    if skipped_big.is_empty() {
                        String::new()
                    } else {
                        format!("；另有 {} 个文件超过 {} MB 未搜", skipped_big.len(), MAX_SEARCH_FILE_BYTES / 1_048_576)
                    },
                    if skipped_bin.is_empty() {
                        String::new()
                    } else {
                        format!("；另有 {} 个二进制文件按内容判定跳过（未读）", skipped_bin.values().sum::<usize>())
                    }
                ),
                "skipped_oversize": skipped_big,
                "skipped_binary": skipped_bin,
            },
            "warnings": warnings, "error": null
        }).to_string())
    }
}

#[cfg(test)]
#[path = "fs_read_tests.rs"]
mod fs_read_tests;
