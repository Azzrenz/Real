//! 文件写入域：write（备份+读回验证）/ edit（块级 find-replace + 单行 line，冲突检测 + Python 语法/导入校验）

use crate::mcp::registry::BuiltinTool;
use crate::path::{PathPolicy, PathVerdict};
use crate::tools::contract::{err_text, validate, ToolError};
use crate::tools::fs_read::{looks_like_tool_envelope, resolve_guarded};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use tokio::fs;
use uuid::Uuid;

/// 一次性自修改授权令牌（self_heal 放开自改后从 security 域挪入；最小版删安全模块，
static SELF_EDIT_TOKENS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// 签发一次性自修改令牌（确认门批准后由 executor 注入 write/edit/modify 的 args）
pub fn issue_self_edit_token() -> String {
    // 令牌前缀按统一规范用 `-`（见 docs/20260915-数据落盘与命名规范.md §四）
    let t = format!("set-{}", Uuid::new_v4().simple());
    if let Ok(mut s) = SELF_EDIT_TOKENS.lock() {
        s.insert(t.clone());
    }
    t
}

pub fn consume_self_edit_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    SELF_EDIT_TOKENS
        .lock()
        .map(|mut s| s.remove(token))
        .unwrap_or(false)
}

/// 带重试的文件写入：Windows 下 pytest/import 校验子进程持有文件句柄
fn normalize_script_line_endings<'a>(
    path: &std::path::Path,
    content: &'a str,
) -> (std::borrow::Cow<'a, str>, bool) {
    use std::borrow::Cow;
    let is_script = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "bat" | "cmd"))
        .unwrap_or(false);
    if !is_script || !content.contains('\n') {
        return (Cow::Borrowed(content), false);
    }
    // 纯 CRLF（无裸 LF、无裸 CR）→ 原样返回，不制造无谓 diff
    let crlf = content.matches("\r\n").count();
    if content.matches('\n').count() == crlf && content.matches('\r').count() == crlf {
        return (Cow::Borrowed(content), false);
    }
    // 先把 CRLF 折成 LF，再把所有裸 LF 统一成 CRLF —— 幂等，混合行尾也能修好
    let lf_only = content.replace("\r\n", "\n").replace('\r', "\n");
    (Cow::Owned(lf_only.replace('\n', "\r\n")), true)
}

async fn write_with_retry(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let mut last_err = None;
    for attempt in 0..3u32 {
        match fs::write(path, content).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let is_lock =
                    e.raw_os_error() == Some(32) || format!("{e}").contains("os error 32");
                if !is_lock || attempt == 2 {
                    return Err(e);
                }
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(150 * (attempt + 1) as u64))
                    .await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("写入失败")))
}

async fn copy_with_retry(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let mut last_err = None;
    for attempt in 0..3u32 {
        match fs::copy(src, dst).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                let is_lock =
                    e.raw_os_error() == Some(32) || format!("{e}").contains("os error 32");
                if !is_lock || attempt == 2 {
                    return Err(e);
                }
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(150 * (attempt + 1) as u64))
                    .await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("备份失败")))
}

// 写后语法检查（验证靠代码不靠 prompt）
fn syntax_check_result(path: &std::path::Path, content: &str) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => match serde_json::from_str::<serde_json::Value>(content) {
            Ok(_) => Some("JSON_SYNTAX_OK".to_string()),
            Err(e) => Some(format!("JSON_SYNTAX_ERROR: {e}")),
        },
        "js" | "mjs" | "cjs" => node_check_script(content, "JS"),
        "py" => python_check_script(content),
        "html" | "htm" => html_script_check(content),
        _ => None,
    }
}

/// node --check 语法检查（写临时文件避免命令行转义地狱；node 缺失 → None 跳过）
fn node_check_script(script: &str, tag: &str) -> Option<String> {
    let tmp = std::env::temp_dir().join(format!(
        "real_syntax_{}_{}.js",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis()
    ));
    std::fs::write(&tmp, script).ok()?;
    let out = std::process::Command::new("node")
        .arg("--check")
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    match out {
        Ok(o) if o.status.success() => Some(format!("{tag}_SYNTAX_OK")),
        Ok(o) => Some(format!(
            "{tag}_SYNTAX_ERROR: {}",
            String::from_utf8_lossy(&o.stderr)
                .trim()
                .chars()
                .take(300)
                .collect::<String>()
        )),
        Err(_) => None,
    }
}

/// python 语法检查（ast 解析，无 __pycache__ 副作用；python 缺失 → None 跳过）
fn python_check_script(code: &str) -> Option<String> {
    let tmp = std::env::temp_dir().join(format!(
        "real_syntax_{}_{}.py",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis()
    ));
    std::fs::write(&tmp, code).ok()?;
    let out = std::process::Command::new("python")
        .args([
            "-c",
            "import ast,sys; ast.parse(open(sys.argv[1],encoding='utf-8').read())",
        ])
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    match out {
        Ok(o) if o.status.success() => Some("PY_SYNTAX_OK".to_string()),
        Ok(o) => Some(format!(
            "PY_SYNTAX_ERROR: {}",
            String::from_utf8_lossy(&o.stderr)
                .trim()
                .lines()
                .last()
                .unwrap_or("")
                .chars()
                .take(300)
                .collect::<String>()
        )),
        Err(_) => None,
    }
}

/// HTML 内嵌脚本检查：提取无 src 的 <script> 块逐个 node --check；无内嵌脚本 → None
fn html_script_check(html: &str) -> Option<String> {
    static SCRIPT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // regex crate 不支持 look-around：先提所有 <script> 块，再过滤含 src= 的外部脚本
    let re = SCRIPT_RE
        .get_or_init(|| regex::Regex::new(r"(?is)<script[^>]*>([\s\S]*?)</script>").unwrap());
    let mut results: Vec<String> = Vec::new();
    for cap in re.captures_iter(html) {
        let full = cap.get(0).map(|m| m.as_str()).unwrap_or("");
        if full.contains("src=") {
            continue;
        }
        let code = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if code.trim().is_empty() {
            continue;
        }
        match node_check_script(code, "HTML_JS") {
            Some(r) => results.push(r),
            None => return None,
        }
    }
    if results.is_empty() {
        return None;
    }
    let bad: Vec<&String> = results
        .iter()
        .filter(|r| r.contains("_SYNTAX_ERROR"))
        .collect();
    if bad.is_empty() {
        Some("HTML_JS_SYNTAX_OK".to_string())
    } else {
        Some(format!(
            "HTML_JS_SYNTAX_ERROR: {}",
            bad.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        ))
    }
}

// write

pub struct WriteTool;

#[async_trait]
impl BuiltinTool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        include_str!("../../prompts/tools/write.md")
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "path":{"type":"string","description":"[必须] 文件路径（可用 #En）"},
                "content":{"type":"string","minLength":1,"description":"[必须] 完整最终内容（先 read 目标，写保留原功能的新全文）"}
            },
            "required":["path","content"],
            "additionalProperties":true
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": false, "destructive": true, "idempotent": false})
    }

    fn output_schema(&self) -> Option<Value> {
        // （契约·输入宽容/输出严格）：write 输出严格锁定
        Some(json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "ok":{"type":"boolean"},
                "kind":{"type":"string"},
                "data":{"type":"object","additionalProperties":false,"properties":{
                    "path":{"type":"string"},
                    "bytes_written":{"type":"integer"},
                    "backup_path":{}
                }},
                "warnings":{"type":"array"},
                "error":{}
            }
        }))
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args).map_err(|e| err_text(&e))?;
        let path = resolve_guarded(args["path"].as_str().unwrap()).map_err(|e| err_text(&e))?;
        if crate::path::is_sensitive(&path) {
            return Err(err_text(&ToolError::domain(
                "SENSITIVE_FILE",
                format!("拒绝写入敏感文件（含凭据/配置）: {}", path.display()),
                Some("该文件禁止工具修改"),
            )));
        }
        // 护栏：自身目录/系统敏感区/盘符根/验证门禁脚本写入 →
        if let PathVerdict::RequireConfirm(reason) =
            PathPolicy::new().check_write(&path.display().to_string())
        {
            let authorized = raw_args
                .get("__real_self_edit_token")
                .and_then(|v| v.as_str())
                .map(consume_self_edit_token)
                .unwrap_or(false)
                // 会话授权范围（用户在本会话已弹窗批准过该目录）同样放行——与确认门
                || raw_args
                    .get("__session")
                    .and_then(|v| v.as_str())
                    .map(|sid| crate::path::write_granted(sid, &path.display().to_string()))
                    .unwrap_or(false);
            if !authorized {
                return Err(err_text(&ToolError::domain(
                    "WRITE_REQUIRES_CONFIRM",
                    format!("写入被护栏拦截：{reason}（需人工确认后由后端签发自修改令牌）"),
                    Some("该路径需用户显式确认；请通过人工确认流程授权后重试"),
                )));
            }
        }
        let content = args["content"].as_str().unwrap();

        if looks_like_tool_envelope(content) {
            return Err(err_text(&ToolError::domain(
                "CONTENT_IS_ENVELOPE",
                "write 的 content 参数疑似工具返回信封 JSON（含 kind/tool_call_id 特征）——content 必须是纯文本文件内容，禁止复制工具返回的 JSON。",
                Some("从 read 返回 JSON 的 content 字段提取值（那是文件内容），只复制该值，不要复制整个返回 JSON（含 kind/data/warnings/error 的都是信封）"),
            )));
        }

        // 备份已有文件
        let mut backup_path: Option<String> = None;
        if path.exists() {
            let bak = backup_path_for(&path);
            copy_with_retry(&path, &bak).await.map_err(|e| {
                err_text(&ToolError::domain(
                    "BACKUP_FAILED",
                    format!("备份失败: {e}"),
                    Some("请检查文件权限（可能被其他程序占用，稍后重试）"),
                ))
            })?;
            backup_path = Some(bak.display().to_string());
        }

        // 创建父目录
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    err_text(&ToolError::domain(
                        "PARENT_NOT_FOUND",
                        format!("无法创建父目录 {}: {e}", parent.display()),
                        Some("请检查路径层级"),
                    ))
                })?;
            }
        }

        // 写入（带锁重试：Windows 子进程句柄未释放时 os error 32，短延迟自愈）
        let (content, crlf_normalized) = normalize_script_line_endings(&path, content);
        write_with_retry(&path, &content).await.map_err(|e| {
            err_text(&ToolError::domain(
                "PERMISSION_DENIED",
                format!("写入失败: {e}"),
                Some("请检查文件权限"),
            ))
        })?;

        // 写后验证（坑位：读回失败单独报错，不伪装成"内容不一致"）
        let written = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return Err(err_text(&ToolError::domain(
                    "WRITE_VERIFY_READ_FAILED",
                    format!("写入成功但读回验证失败: {e}（文件已写入，请用 read 确认实际内容）"),
                    None,
                )));
            }
        };
        if written != content.as_ref() {
            return Err(err_text(&ToolError::domain(
                "WRITE_VERIFY_FAILED",
                "写入后读回内容不一致",
                Some("请重试"),
            )));
        }

        // 行尾：.bat/.cmd 已在落盘前由 normalize_script_line_endings 确定性归一（见上），此处仅告知
        let mut warnings: Vec<Value> = Vec::new();
        let ext_lower = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if crlf_normalized && matches!(ext_lower.as_str(), "bat" | "cmd") {
            warnings.push(json!(
                "ℹ️ 已自动把 content 的行尾统一为 CRLF —— .bat/.cmd 在 cmd.exe 下必须 CRLF\
                 （裸 LF 会让解析器吃掉下一行开头 1~2 个字符）；这是落盘前的确定性兜底，\
                 你**无需重写**，文件已是可执行版本。"
            ));
        }

        // 写入成功 = 模型手里的 content 就是文件最新态：登记指纹，后续 edit 直接放行

        Ok(json!({
            "ok": true, "kind": "write_result",
            "data": {
                "path": path.display().to_string(),
                "bytes_written": content.len(),
                "backup_path": backup_path,
                // （后端兜底·验证靠代码不靠 prompt）：写后确定性语法检查。
                "syntax_check": syntax_check_result(&path, &written),
            },
            "warnings": warnings, "error": null
        })
        .to_string())
    }
}

// edit（行替换 + 冲突检测 + 备份 + 读回验证）

pub struct EditTool;

#[async_trait]
impl BuiltinTool for EditTool {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &'static str {
        include_str!("../../prompts/tools/edit.md")
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "file":{"type":"string","description":"文件绝对路径（正反斜杠均可，**建议正斜杠**避免转义问题；Windows 如 D:/proj/src/lib.rs；可用 #En 引用前序 read/find_files/search 结果）"},
                "replacements":{"type":"array","maxItems":10,"items":{
                    "type":"object",
                    "properties":{
                        "find":{"type":"string","description":"[块级模式·推荐] 要被替换掉的完整原文片段（可多行，复制文件中真实存在的文本，不要凭记忆；多处报 FIND_AMBIGUOUS，0 匹配报 FIND_NOT_FOUND）。**它与 replace 必须不同**：`find` = 改之前，`replace` = 改之后。"},
                        "replace":{"type":"string","description":"[块级模式] find 命中后你要把它**变成**的完整文本（可多行）；[单行模式] 改后行内容。**必须与 find 不同**——写成原文照抄 = 空替换，契约层直接拒绝（EMPTY_REPLACEMENT，本次不会落盘）。只是想核对原文就别调用 edit，用 read。"},
                        "line":{"type":"integer","minimum":1,"description":"[单行模式] 目标行号（从 1 开始），与 find 模式二选一"},
                        "old":{"type":"string","description":"[单行模式] 第 line 行原文（可选，省略时后端自动取该行原文；可用 #En 引用同文件前序 read 步骤，后端自动按 line 切出精确原文——**严禁凭记忆编造 old**，必 OLD_TEXT_MISMATCH）"},
                        "new":{"type":"string","description":"[单行模式] 改后的行内容（与 replace 等价，兼容旧调用）。**必须与第 line 行原文不同**——照抄原文 = 空替换，直接拒绝（EMPTY_REPLACEMENT）。"}
                    },
                    "additionalProperties":true
                }}
            },
            "required":["file"],
            "additionalProperties":true
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": false, "destructive": true, "idempotent": false})
    }

    fn output_schema(&self) -> Option<Value> {
        // （契约·输入宽容/输出严格）：edit 输出严格锁定
        Some(json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "ok":{"type":"boolean"},
                "kind":{"type":"string"},
                "data":{"type":"object","additionalProperties":false,"properties":{
                    "file":{"type":"string"},
                    "changes":{"type":"array"},
                    "changed":{"type":"integer"},
                    "backup_path":{}
                }},
                "warnings":{"type":"array"},
                "error":{}
            }
        }))
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args).map_err(|e| err_text(&e))?;
        let file = resolve_guarded(args["file"].as_str().unwrap()).map_err(|e| err_text(&e))?;
        if crate::path::is_sensitive(&file) {
            return Err(err_text(&ToolError::domain(
                "SENSITIVE_FILE",
                format!("拒绝编辑敏感文件（含凭据/配置）: {}", file.display()),
                Some("该文件禁止工具修改"),
            )));
        }
        // 护栏：系统敏感区/盘符根/自身目录编辑 → 需一次性自修改 token。
        if let PathVerdict::RequireConfirm(reason) =
            PathPolicy::new().check_write(&file.display().to_string())
        {
            let authorized = raw_args
                .get("__real_self_edit_token")
                .and_then(|v| v.as_str())
                .map(consume_self_edit_token)
                .unwrap_or(false);
            if !authorized {
                return Err(err_text(&ToolError::domain(
                    "EDIT_REQUIRES_CONFIRM",
                    format!("编辑被护栏拦截：{reason}（需人工确认后由后端签发自修改令牌）"),
                    Some("该路径需用户显式确认；请通过人工确认流程授权后重试"),
                )));
            }
        }
        let replacements = args
            .get("replacements")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if replacements.is_empty() {
            // 坑位 J22 根治：REWOO 中 Planner 规划 edit 时看不到文件内容 → replacements 空。
            let original = fs::read_to_string(&file).await.map_err(|e| {
                err_text(&ToolError::domain(
                    "READ_FAILED",
                    format!("读取失败: {e}"),
                    None,
                ))
            })?;
            let all_lines: Vec<&str> = original.lines().collect();
            let shown = all_lines.len().min(90);
            let mut numbered = String::new();
            for (i, l) in all_lines.iter().take(shown).enumerate() {
                numbered.push_str(&format!("L{}: {}\n", i + 1, l));
            }
            if all_lines.len() > shown {
                numbered.push_str(&format!(
                    "…（共 {} 行，此处示前 {shown} 行；编辑点靠后请用 read 行区间或 search 定位）\n",
                    all_lines.len()
                ));
            }
            return Err(err_text(&ToolError::domain(
                "REPLACEMENTS_REQUIRED",
                format!(
                    "replacements 为空：请按下方【真实内容+行号】直接构造——数组每项 {{ \"line\": <行号>, \"old\": \"<该行原文>\", \"new\": \"<改后内容>\" }}。\n目标文件 {} 真实内容（带行号，可直接引用）：\n{numbered}",
                    file.display(),
                ),
                Some("照抄模板：replacements: [{ \"line\": 15, \"old\": \"    原行原文（含缩进，从上方 L15 行逐字复制）\", \"new\": \"    改后的新行\" }]。逐字复制 L 行原文到 old、只改 new；多行改动把整段行一起放入 old/new（行数对齐）；无法精确对齐就整文件 write。")
            )));
        }

        if !file.exists() {
            return Err(err_text(&ToolError::domain(
                "NOT_FOUND",
                // ── 第 1 类错因：文件**真的不在** —— 这类才该引导去查文件系统 ──
                format!(
                    "文件不存在: {}{}\n\
                     ⚠️ 这是**文件系统问题**（路径错 / 文件还没建），\
                     **不是参数没填实** —— 别去改 change_spec 的内容。",
                    file.display(),
                    match file.parent() {
                        Some(dir) if dir.is_dir() => {
                            let mut names: Vec<String> = std::fs::read_dir(dir)
                                .map(|rd| {
                                    rd.filter_map(|e| e.ok())
                                        .map(|e| {
                                            let n = e.file_name().to_string_lossy().to_string();
                                            if e.path().is_dir() {
                                                format!("{n}/")
                                            } else {
                                                n
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            names.sort();
                            let total = names.len();
                            names.truncate(20);
                            format!(
                                "\n父目录 {} 确实存在，其中 {total} 项{}：{}",
                                dir.display(),
                                if total > 20 { "（只列前 20）" } else { "" },
                                names.join("  ")
                            )
                        }
                        Some(dir) => format!(
                            "\n父目录 {} **也不存在** —— 是整条路径写错了，不是文件名写错。",
                            dir.display()
                        ),
                        None => String::new(),
                    }
                ),
                Some("照上面列出的实际内容改文件名；若本就该新建，改用 write 先创建"),
            )));
        }
        let original = fs::read_to_string(&file).await.map_err(|e| {
            err_text(&ToolError::domain(
                "READ_FAILED",
                format!("读取失败: {e}"),
                None,
            ))
        })?;
        let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
        let total = lines.len();

        // 冲突检测：同号多次替换 or 行号越界
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut changes = Vec::new();
        for r in &replacements {
            // ===== 块级模式（find/replace，优先，支持多行结构化改写）=====
            if let Some(find) = r.get("find").and_then(|v| v.as_str()) {
                if !find.is_empty() {
                    let replace = r
                        .get("replace")
                        .and_then(|v| v.as_str())
                        .or_else(|| r.get("new").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    if replace.is_empty() {
                        changes.push(json!({"find": find, "replace": replace,
                            "note": format!("省略 replace：按删除处理（find 唯一命中时删除该块；若本意是替换，请补 replace）")}));
                    } else if find == replace {
                        let n = find.chars().count();
                        return Err(err_text(&ToolError::domain(
                            "EMPTY_REPLACEMENT",
                            format!("find 与 replace 完全相同（{n} 字符，空替换，未产生实际变更）"),
                            Some("replace 是你希望它**变成**的样子，不是原文照抄——把要改的部分写进 replace（其余保持原文）。若你想要的正是原文，本次修改无需提交"),
                        )));
                    }
                    // 行尾归一化：Windows 文件常为 \\r\\n，模型基于 read 输出构造的 find/replace
                    let find_n = find.replace("\r\n", "\n").replace("\r", "\n");
                    let replace_n = replace.replace("\r\n", "\n").replace("\r", "\n");
                    let content = lines.join("\n");
                    // ——（FIND_NOT_FOUND 根治·空白容错）——模型凭记忆拼 find 时最常见
                    if content.matches(&find_n).count() == 0 {
                        let f_lines: Vec<&str> = find_n.lines().collect();
                        let trim_eq = |a: &str, b: &str| a.trim() == b.trim();
                        let c_lines: Vec<&str> = content.lines().collect();
                        // 用 find 首行（非空）扫描候选起点
                        let first_f = f_lines.iter().find(|l| !l.trim().is_empty());
                        if let (Some(first_f), false) = (first_f, f_lines.is_empty()) {
                            let mut starts: Vec<usize> = Vec::new();
                            for (i, cl) in c_lines.iter().enumerate() {
                                if trim_eq(cl, first_f) {
                                    starts.push(i);
                                }
                            }
                            if starts.len() == 1 {
                                let s0 = starts[0];
                                if s0 + f_lines.len() <= c_lines.len() {
                                    let window = &c_lines[s0..s0 + f_lines.len()];
                                    if window.iter().zip(f_lines.iter()).all(|(c, f)| trim_eq(c, f)) {
                                        // 以文件真实文本为锚构造替换：replace 的每行 trim 后
                                        let mut out: Vec<String> = c_lines[..s0].iter().map(|s| s.to_string()).collect();
                                        let file_indent = window[0].len() - window[0].trim_start().len();
                                        for (k, fl) in f_lines.iter().enumerate() {
                                            let cur = window[k];
                                            if trim_eq(cur, fl) {
                                                // 该行未被 replace 改动 → 保留文件原行（缩进原样）
                                                out.push(cur.to_string());
                                            } else {
                                                // 被替换的行：用 replace 行内容 + 文件缩进
                                                let rn = replace_n.lines().nth(k.min(replace_n.lines().count().saturating_sub(1))).unwrap_or("");
                                                let r_trim = rn.trim();
                                                if r_trim.is_empty() {
                                                    out.push(String::new());
                                                } else {
                                                    out.push(format!("{}{}", " ".repeat(file_indent), r_trim));
                                                }
                                            }
                                        }
                                        out.extend(c_lines[s0 + f_lines.len()..].iter().map(|s| s.to_string()));
                                        lines = out;
                                        changes.push(json!({
                                            "find": find, "replace": replace,
                                            "note": "空白容错匹配命中（模型缩进/行尾空白与文件不一致，已按文件真实文本自动对齐执行）"
                                        }));
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    let count = content.matches(&find_n).count();
                    match count {
                        0 => {
                            // 纠偏变体第三层：空白容错之后，JSON 转义污染/
                            if let Some((fv, rv, why)) = find_fuzzy_variants(&content, &find_n, &replace_n) {
                                let c2 = content.matches(&fv).count();
                                if c2 == 1 {
                                    let nc = content.replacen(&fv, &rv, 1);
                                    lines = nc.lines().map(|l| l.to_string()).collect();
                                    changes.push(json!({
                                        "find": find, "replace": replace,
                                        "note": format!("find 自动纠偏命中（{why}）：已按纠偏后文本执行")
                                    }));
                                    continue;
                                }
                            }
                            // （FIND_NOT_FOUND 根治）：报错时回灌真实内容——
                            let hint = find_not_found_hint(&content, &find_n);
                            return Err(err_text(&ToolError::domain(
                                "FIND_NOT_FOUND",
                                format!("find 文本在文件中未找到：{find}\n{hint}"),
                                Some("请基于下方【文件真实内容】逐字构造 find（含空白/缩进/行尾），或改用 line 模式指定行号"),
                            )));
                        }
                        1 => {
                            let nc = content.replacen(&find_n, &replace_n, 1);
                            if nc.trim().is_empty() && !content.trim().is_empty() && !replace.is_empty() {
                                // replace 非空仍删光全文 = 明显失配，拒（防整文件被误吞）
                                return Err(err_text(&ToolError::domain(
                                    "REPLACEMENT_LOSES_FILE",
                                    "本次替换将清空整个文件（find 只匹配到开头，剩余全被 replace 吞掉）——疑似 find/replace 失配。",
                                    Some("把 find 扩到完整收尾（含最后一行/最后一个 }），或改用 line 模式/整文件 write"),
                                )));
                            }
                            if nc.trim().is_empty() && content.trim().is_empty() {
                                // 原文件本就空：删除语义下保持空，不拦
                            }
                            lines = nc.lines().map(|l| l.to_string()).collect();
                            changes.push(json!({"find": find, "replace": replace}));
                        }
                        _ => {
                            return Err(err_text(&ToolError::domain(
                                "FIND_AMBIGUOUS",
                                format!("find 在文件中出现 {count} 处，无法唯一定位"),
                                // 回灌匹配行号 + 第一个匹配附近真实内容（不再是空泛建议）
                                Some(&format!(
                                    "扩展 find 上下文（多含几行）使其唯一，或使用更精确的子串。{}",
                                    find_ambiguous_hint(&content, &find_n, count)
                                )),
                            )));
                        }
                    }
                    continue;
                }
            }
            // ===== 单行模式（line/old/new，兼容旧调用）=====
            let line_no =
                match r.get("line").and_then(|v| v.as_u64()) {
                    Some(n) => n as usize,
                    None => return Err(err_text(&ToolError::domain(
                        "REPLACEMENTS_REQUIRED",
                        "每项替换需提供 find+replace（块级）或 line+new（单行）；当前项两者皆缺。",
                        Some("块级：{find, replace}；单行：{line, new}"),
                    ))),
                };
            // （契约漏洞修复）：`r["new"]` 裸索引 + unwrap 在 new 缺失或
            let new = r.get("new").and_then(|v| v.as_str()).ok_or_else(|| {
                err_text(&ToolError::domain(
                    "INVALID_PARAM",
                    "单行模式 new 字段缺失或不是字符串（当前值非文本）。",
                    Some("单行：{line, new}，new 必须是要写入该行的字符串；或改块级：{find, replace}"),
                ))
            })?;
            // （old 可选）：old 缺省 → 后端自动取该行原文（模型零记忆负担，
            let old = r.get("old").and_then(|v| v.as_str()).unwrap_or("");
            if old.is_empty() {
                // old 未提供：行号必须有效（用它直接定位），取该行原文作 old
                if line_no < 1 || line_no > total {
                    return Err(err_text(&ToolError::domain(
                        "LINE_NOT_FOUND",
                        format!("第 {} 行不存在（共 {total} 行）", line_no),
                        Some("请确认行号"),
                    )));
                }
                // 行号直接定位：old 就是该行原文（无需匹配/校正）
                let idx = line_no - 1;
                if lines[idx] == new {
                    return Err(err_text(&ToolError::domain(
                        "EMPTY_REPLACEMENT",
                        format!("第 {line_no} 行 old 与 new 完全相同（空替换，未产生实际变更）\n内容: {new}\n请提供真正的新内容（若目标是修改某处逻辑，new 必须是与 old 不同的完整新行）"),
                        Some("核对目标行：new 必须与 old 不同"),
                    )));
                }
                if !seen.insert(line_no) {
                    return Err(err_text(&ToolError::domain(
                        "EDIT_CONFLICT",
                        format!("第 {line_no} 行被多次替换，互相覆盖"),
                        Some("请拆分替换"),
                    )));
                }
                changes.push(json!({"line": line_no, "old": lines[idx], "new": new}));
                lines[idx] = new.to_string();
                continue;
            }
            if line_no < 1 || line_no > total {
                return Err(err_text(&ToolError::domain(
                    "LINE_NOT_FOUND",
                    format!("第 {} 行不存在（共 {total} 行）", line_no),
                    Some("请确认行号"),
                )));
            }
            let idx = line_no - 1;
            if old.contains('\n') {
                if replacements.len() > 1 {
                    return Err(err_text(&ToolError::domain(
                        "MULTI_LINE_REPLACE_SINGLE",
                        "多行区间替换（line+多行 old）请单独一次 edit 调用，不要与其他替换混用（行号会互相偏移）；或用 modify 的 find/replace 一次完成。",
                        Some("拆成多次 edit，或改用 modify {find, replace}"),
                    )));
                }
                let old_lines: Vec<&str> = old.lines().collect();
                let end = line_no + old_lines.len() - 1;
                if end > total {
                    return Err(err_text(&ToolError::domain(
                        "LINE_NOT_FOUND",
                        format!("第 {line_no}-{end} 行区间超出文件范围（共 {total} 行）"),
                        Some("请核对行号"),
                    )));
                }
                let ok = old_lines
                    .iter()
                    .enumerate()
                    .all(|(i, ol)| lines[line_no - 1 + i] == *ol);
                if ok {
                    if old == new {
                        return Err(err_text(&ToolError::domain(
                            "EMPTY_REPLACEMENT",
                            format!("第 {line_no} 行起的多行 old 与 new 完全相同（空替换，未产生实际变更）\n内容: {new}\n请提供真正的新内容（与 old 不同的完整新文本）"),
                            Some("核对目标区间：new 必须与 old 不同"),
                        )));
                    }
                    if !seen.insert(line_no) {
                        return Err(err_text(&ToolError::domain(
                            "EDIT_CONFLICT",
                            format!("第 {line_no} 行被多次替换，互相覆盖"),
                            Some("请拆分替换"),
                        )));
                    }
                    let new_lines: Vec<&str> = new.lines().collect();
                    changes.push(
                        json!({"line": line_no, "lines": old_lines.len(), "old": old, "new": new}),
                    );
                    lines.splice(line_no - 1..end, new_lines.iter().map(|s| s.to_string()));
                    continue;
                }
                // 多行不匹配 → 报错并列出实际行（带行号引导，模型据此修正）
                let actual: Vec<String> = (line_no..=end)
                    .map(|n| format!("{n}: {}", lines[n - 1]))
                    .collect();
                return Err(err_text(&ToolError::domain(
                    "OLD_TEXT_MISMATCH",
                    format!("第 {line_no} 行起的多行 old 与实际内容不匹配，且全文未找到\n期望:\n{old}\n实际:\n{}", actual.join("\n")),
                    Some("请先用 read 读取文件核对行区间与原文"),
                )));
            }
            let mut real_line = line_no;
            let mut corrected = false;
            // （度量衡统一②）：模型 old 尾带 \r（JSON 显式 \r\n 的单行形态）时，
            let old_norm = old.replace("\r\n", "\n").replace('\r', "\n");
            let old_norm = old_norm.trim_end_matches('\n');
            if lines[idx] != old && lines[idx] != old_norm {
                let old_for_match = old_norm;
                // 坑位 J57：行号只是提示——模型数行易偏 1（如 a1 任务 unwrap 实际 44 行、给 43）。
                let matches: Vec<usize> = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| *l == old_for_match)
                    .map(|(i, _)| i + 1)
                    .collect();
                match matches.len() {
                    0 => {
                        return Err(err_text(&ToolError::domain(
                            "OLD_TEXT_MISMATCH",
                            format!("第 {line_no} 行内容与 old 不匹配，且全文未找到 old\n期望: {old}\n第 {line_no} 行实际: {}", lines[idx]),
                            Some("请先用 read 重新读取文件核对行号与原文"),
                        )));
                    }
                    1 => {
                        // 唯一匹配 → 校正行号
                        real_line = matches[0];
                        corrected = true;
                    }
                    _ => {
                        return Err(err_text(&ToolError::domain(
                            "OLD_TEXT_MISMATCH",
                            format!("old 在文件中出现 {len} 处（第 {lines_str} 行），无法自动定位，请提供精确行号\nold: {old}",
                                len = matches.len(),
                                lines_str = matches.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")),
                            Some("请核对行号，或用唯一的 old 文本"),
                        )));
                    }
                }
            }
            // 坑位 J53：old == new 是"空替换"——模型想改却没给新内容（如只替换了注释行），
            if old == new {
                return Err(err_text(&ToolError::domain(
                    "EMPTY_REPLACEMENT",
                    format!("第 {real_line} 行 old 与 new 完全相同（空替换，未产生实际变更）\n内容: {old}\n请提供真正的新内容（若目标是修改某处逻辑，new 必须是与 old 不同的完整新行）"),
                    Some("核对目标行：new 必须与 old 不同"),
                )));
            }
            if !seen.insert(real_line) {
                return Err(err_text(&ToolError::domain(
                    "EDIT_CONFLICT",
                    format!("第 {real_line} 行被多次替换，互相覆盖"),
                    Some("请拆分替换"),
                )));
            }
            lines[real_line - 1] = new.to_string();
            if corrected {
                changes.push(json!({"line": real_line, "old": old, "new": new, "corrected": true}));
            } else {
                changes.push(json!({"line": real_line, "old": old, "new": new}));
            }
        }

        // 备份（with_file_name：无扩展名文件不再误加 .txt）
        let bak = backup_path_for(&file);
        copy_with_retry(&file, &bak).await.map_err(|e| {
            err_text(&ToolError::domain(
                "BACKUP_FAILED",
                format!("备份失败: {e}"),
                None,
            ))
        })?;

        // 落盘（带锁重试：Windows 子进程句柄未释放时 os error 32，短延迟自愈）
        let eol: &str = if original.contains("\r\n") { "\r\n" } else { "\n" };
        let new_content = lines.join(eol)
            + if original.ends_with('\n') { eol } else { "" };
        write_with_retry(&file, &new_content).await.map_err(|e| {
            err_text(&ToolError::domain(
                "PERMISSION_DENIED",
                format!("写入失败: {e}"),
                None,
            ))
        })?;

        let read_back = match fs::read_to_string(&file).await {
            Ok(c) => c,
            Err(e) => {
                return Err(err_text(&ToolError::domain(
                    "POST_EDIT_VERIFY_READ_FAILED",
                    format!(
                        "写入成功但读回验证失败: {e}（文件已写入，未回滚，请用 read 确认实际内容）"
                    ),
                    None,
                )));
            }
        };
        // （机制修复）：比对前统一换行符（\r\n → \n）——Windows 上 IDE/编辑器
        let read_back_norm = read_back.replace("\r\n", "\n").replace('\r', "\n");
        let new_content_norm = new_content.replace("\r\n", "\n").replace('\r', "\n");
        if read_back_norm != new_content_norm {
            return Err(err_text(&ToolError::domain(
                "POST_EDIT_INVALID",
                "写入后读回内容不一致（文件已写入，未回滚）",
                None,
            )));
        }

        // 语法护栏（坑位：行替换若产生缩进/语法错误，文件已落盘，但往往要到 pytest 才暴露，
        if let Some(ext) = file.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("py") || ext.eq_ignore_ascii_case("pyw") {
                if let Err(syn_err) = check_python_syntax(&file).await {
                    let _ = fs::copy(&bak, &file).await;
                    return Err(err_text(&ToolError::domain(
                        "SYNTAX_ERROR",
                        format!("编辑后 Python 语法校验失败，已回滚到修改前备份（{}）。\n{syn_err}", bak.display()),
                        Some("请修正缩进/语法后重新提交 edit：new 必须是合法 Python，try/except 等代码块须正确缩进其函数体"),
                    )));
                }
            }
        }

        // 导入有效性校验（三修：拦截回滚 → 提示不拦 → 分级）
        let mut import_warn = String::new();
        if let Some(ext) = file.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("py") || ext.eq_ignore_ascii_case("pyw") {
                match check_python_imports(&file).await {
                    Ok(None) => {}
                    Ok(Some(warn)) => {
                        import_warn = format!("[IMPORT_WARNING] 导入校验提示（编辑已生效，未回滚；若报错因环境缺依赖属误报，可忽略）：\n{warn}");
                    }
                    Err(imp_err) => {
                        // 硬问题：符号缺失（模块可导入但符号不存在）= 模型拼错/改错，与环境无关，
                        let _ = fs::copy(&bak, &file).await;
                        return Err(err_text(&ToolError::domain(
                            "IMPORT_ERROR",
                            format!("编辑后导入符号校验失败，已回滚到修改前备份（{}）。\n{imp_err}", bak.display()),
                            Some("目标符号在模块中不存在（模块本身可导入）——请核对符号名拼写与模块可用导出，或改用 write 重写整个文件"),
                        )));
                    }
                }
            }
        }

        let mut warnings: Vec<String> = Vec::new();
        if !import_warn.is_empty() {
            warnings.push(import_warn);
        }
        // 编辑成功 = written 内容即文件最新态：登记指纹（后续 edit 连续编辑免重 read）

        Ok(json!({
            "ok": true, "kind": "edit_result",
            "data": {"file": file.display().to_string(), "changes": changes, "changed": changes.len(), "backup_path": bak.display().to_string()},
            "warnings": warnings, "error": null
        }).to_string())
    }
}

/// 备份落到工作区**之外**的全局目录（坑位 G9）：write/edit 默认在同目录生成 .bak，
fn backup_path_for(orig: &std::path::Path) -> std::path::PathBuf {
    // 路径编码：分隔符与盘符冒号统一成 `-`（kebab），剔除 Windows 非法字符，其余原样保留。
    let safe: String = orig
        .display()
        .to_string()
        .chars()
        .map(|c| if c == '\\' || c == '/' || c == ':' { '-' } else { c })
        .filter(|c| !"<>\"|?*".contains(*c))
        .collect();
    let dir = std::env::temp_dir().join("real-backups");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}-{safe}", crate::path::data_root::stamp_ms()))
}

/// FIND_NOT_FOUND 回灌提示：find 找不到 = 模型凭记忆编造（没先 read）。
fn find_ambiguous_hint(content: &str, find: &str, count: usize) -> String {
    let find_n = find.replace("\r\n", "\n").replace('\r', "\n");
    // 收集所有匹配行号（最多 12 个）
    let mut line_nos: Vec<usize> = Vec::new();
    let mut first_pos: Option<usize> = None;
    let mut search_from = 0usize;
    while let Some(rel) = content[search_from..].find(&find_n) {
        let pos = search_from + rel;
        if first_pos.is_none() {
            first_pos = Some(pos);
        }
        let line_no = content[..pos].matches('\n').count() + 1;
        if !line_nos.contains(&line_no) {
            line_nos.push(line_no);
        }
        if line_nos.len() >= 12 {
            break;
        }
        search_from = pos + find_n.len().max(1);
    }
    let mut hint = format!(
        "\n你的 find 在文件中出现 **{count} 处**（行号：{}）。\n\
         find 太短/太泛导致无法唯一定位。",
        if line_nos.is_empty() {
            "?（无法定位）".to_string()
        } else {
            line_nos
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("、")
        }
    );
    // 回灌第一个匹配位置附近（前后 12 行）的真实内容
    if let Some(pos) = first_pos {
        let start = content[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let mut s = start;
        for _ in 0..10 {
            if s == 0 {
                break;
            }
            match content[..s].rfind('\n') {
                Some(p) => s = p + 1,
                None => {
                    s = 0;
                    break;
                }
            }
        }
        let mut e = pos + find_n.len();
        for _ in 0..12 {
            match content[e..].find('\n') {
                Some(rel) => e = e + rel + 1,
                None => {
                    e = content.len();
                    break;
                }
            }
            if e >= content.len() {
                e = content.len();
                break;
            }
        }
        let window: String = content[s..e.min(content.len())]
            .chars()
            .take(2500)
            .collect();
        hint.push_str(&format!(
            "\n【第一个匹配位置附近的真实内容】\n```\n{window}\n```\n\
             请基于上面【真实内容】扩展 find（多含几行、带上下文）使其唯一，\
             或改用 line 模式指定目标行号（若目标是 {} 行附近）。",
            line_nos.first().copied().unwrap_or(0)
        ));
    }
    hint
}

fn find_not_found_hint(content: &str, find: &str) -> String {
    // 提取 find 里的"标识符/特征词"（第一段缩进后的真实单词）
    let first_line = find.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let trimmed = first_line.trim();
    let keyword = trimmed
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .find(|w| !w.is_empty() && w.len() >= 2)
        .unwrap_or(trimmed);
    // 在文件里定位关键词首次出现
    let mut hint = String::new();
    if let Some(pos) = content.find(keyword) {
        // 取关键词前后 25 行
        let prefix = &content[..pos];
        let line_no = prefix.matches('\n').count() + 1;
        let start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
        // 往前扩展 8 行（向后窗口由下方 window_end 计算，无需提前算 end）
        let mut s = start;
        for _ in 0..8 {
            if s == 0 {
                break;
            }
            let prev = content[..s].rfind('\n');
            match prev {
                Some(p) => s = p + 1,
                None => {
                    s = 0;
                    break;
                }
            }
        }
        let window_end = content[s..]
            .match_indices('\n')
            .nth(15)
            .map(|(i, _)| s + i)
            .unwrap_or(content.len());
        let window: String = content[s..window_end].chars().take(2000).collect();
        hint.push_str(&format!(
            "\n【文件真实内容 · 关键词「{keyword}」在第 {line_no} 行附近】\n```\n{window}\n```\n\
             你的 find 中「{trimmed}」与文件真实内容不一致（缩进/空白/上下文差异，或该代码已不存在）。\
             请基于上面【真实内容】逐字复制 find，或改用 line 模式指定行号。"
        ));
    } else {
        // 关键词也不在文件里 → 该代码可能完全不存在（模型记错了），给结构摘要
        let struct_lines: Vec<&str> = content.lines().take(30).collect();
        hint.push_str(&format!(
            "\n【文件前 30 行真实内容】\n```\n{}\n```\n\
             关键词「{keyword}」在文件中不存在——这段代码可能已不存在或从未存在（模型凭记忆编造）。\
             请先 read 目标文件确认真实代码，再构造 find。",
            struct_lines.join("\n")
        ));
    }
    hint
}

/// 写后语法校验（仅 Python 文件）：broken edit（缩进/语法错误）立即暴露，避免落盘后
async fn check_python_syntax(path: &std::path::Path) -> Result<(), String> {
    let p = path.to_path_buf();
    // 用 spawn_blocking 跑同步的 std::process（不依赖 tokio process feature）
    let res = tokio::task::spawn_blocking(move || {
        std::process::Command::new(resolve_python_exe())
            .args(["-m", "py_compile", &p.display().to_string()])
            .output()
    })
    .await;
    match res {
        Ok(Ok(o)) => {
            if o.status.success() {
                Ok(())
            } else {
                let msg = String::from_utf8_lossy(&o.stderr).to_string();
                // py_compile 在 stderr 末尾打印 "SyntaxError: ..."，取最后非空行作为精炼错误
                let last = msg
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .last()
                    .unwrap_or("语法校验失败")
                    .to_string();
                Err(last)
            }
        }
        _ => Ok(()),
    }
}

/// 解析用于写后校验的 Python 解释器
fn resolve_python_exe() -> String {
    if let Ok(env_py) = std::env::var("REAL_PYTEST_PYTHON") {
        if !env_py.is_empty() && std::path::Path::new(&env_py).exists() {
            return env_py;
        }
    }
    "python".to_string()
}

/// 写后"导入有效性"校验（仅 Python 文件，增量2）
async fn check_python_imports(path: &std::path::Path) -> Result<Option<String>, String> {
    let p = path.to_path_buf();
    let py = resolve_python_exe();
    let res = tokio::task::spawn_blocking(move || -> Result<std::process::Output, std::io::Error> {
        let checker = r#"
import ast, importlib, importlib.util, os, sys

target = sys.argv[1]
# workspace 模块误伤：校验子进程的 sys.path 不含 workspace，
# 编辑 workspace 内模块（如 django 源码）的 import 会被误判"模块不存在"→ 大面积回滚。
# 从目标文件向上找 workspace 根（含 .git 或 pyproject.toml），插入 sys.path。
def find_ws_root(p):
    d = os.path.dirname(os.path.abspath(p))
    while True:
        if os.path.isdir(os.path.join(d, '.git')) or os.path.isfile(os.path.join(d, 'pyproject.toml')):
            return d
        nd = os.path.dirname(d)
        if nd == d:
            return None
        d = nd

_ws = find_ws_root(target)
if _ws:
    sys.path.insert(0, _ws)
    _src = os.path.join(_ws, 'src')
    if os.path.isdir(_src):
        sys.path.insert(0, _src)

def module_exists(mod):
    # PathFinder.find_spec 不覆盖 builtin/frozen 模块
    # （sys/os/asyncio 等标准库全被误判"不存在"→ 大面积误伤）。改用
    # importlib.util.find_spec：按 sys.modules → builtin → frozen → PathFinder 全链查找。
    # 依赖缺失等异常保守放行（pytest 兜底）。
    try:
        if mod in sys.modules:
            return True
        return importlib.util.find_spec(mod) is not None
    except Exception:
        return True

try:
    with open(target, encoding='utf-8') as f:
        tree = ast.parse(f.read())
except Exception as e:
    print('REAL_IMPORT_CHECK_FAILED')
    print('  - 无法解析目标文件: %s' % e)
    sys.exit(1)

problems = []
hard_problems = []
for node in ast.walk(tree):
    if isinstance(node, ast.ImportFrom):
        # 相对导入（from ._compat import ... / from ..pkg import ...）跳过——
        # 独立进程无包上下文无法解析（importlib.import_module('.xxx') 必失败），
        # 误判会回滚掉正确编辑（itsdangerous 实测 38 次误伤）。
        # 相对导入的正确性由包级 pytest 验证兜底，不属于本校验职责。
        if node.level > 0:
            continue
        mod = node.module or ''
        for alias in node.names:
            name = alias.name
            if not module_exists(mod):
                # 模块缺失 → 软问题：可能是环境未装依赖（三修决策，
                # dictofitems/dotenv/django 实测均属环境误伤），交运行时 pytest 兜底
                problems.append('from %s import %s: 模块 %s 不存在（可能为环境未安装依赖）' % (mod, name, mod))
                continue
            try:
                m = importlib.import_module(mod)
            except Exception:
                # 模块文件存在但依赖缺失/运行期异常：不判定为编辑引入的错误，pytest 兜底
                continue
            if not hasattr(m, name):
                # 子模块式包（PIL/torch 等）：from X import Y 的真实语义=先试属性、
                # 再回退 X.Y 子模块导入；顶层包对象上常无该属性（实测
                # PIL.ImageGrab 被误判 2 次回滚——运行时能导入，校验器判"不存在"）。
                try:
                    if importlib.util.find_spec(mod + '.' + name) is not None:
                        continue  # 子模块存在 = from 可导入，符号合法
                except (ImportError, AttributeError, ValueError, TypeError):
                    pass
                avail = [n for n in dir(m) if not n.startswith('_')]
                if not avail:
                    # 空导出 = 导入到的是命名空间/空壳模块——校验器进程没有脚本
                    # 运行时的 sys.path 定制（blib2to3 pygram 实测 2 次误伤回滚：
                    # 脚本自行 sys.path.insert(0,'src') 后 from blib2to3 import pygram
                    # 运行时有效，校验器却导入到空模块判"符号不存在"）。判定不可信 →
                    # 降级软问题交运行时 pytest 兜底，不回滚。
                    problems.append('from %s import %s: 模块 %s 无可判定导出（空/命名空间模块——校验环境与脚本运行环境可能不一致，不做硬判）' % (mod, name, mod))
                    continue
                # 符号缺失 → 硬问题：模块能导入且有导出但缺目标符号 = 模型拼错/改错，
                # 确定性错误，必须回滚给 Replanner 精确信号（fs_write.rs 同步处理）
                hard_problems.append('from %s import %s: 符号 %s 在模块 %s 中不存在。模块可用导出(前20): %s'
                                     % (mod, name, name, mod, avail[:20]))
    elif isinstance(node, ast.Import):
        for alias in node.names:
            mod = alias.name
            if not module_exists(mod):
                problems.append('import %s: 模块 %s 不存在（可能为环境未安装依赖）' % (mod, mod))
                continue
            try:
                importlib.import_module(mod)
            except Exception:
                continue

# 硬问题（符号缺失）优先：确定性错误，调用方据此回滚
if hard_problems:
    print('REAL_IMPORT_SYMBOL_ERROR')
    for pr in hard_problems:
        print('  - ' + pr)
    sys.exit(2)
if problems:
    print('REAL_IMPORT_CHECK_FAILED')
    for pr in problems:
        print('  - ' + pr)
    sys.exit(1)
"#;
        let dir = std::env::temp_dir().join("real_import_check");
        let _ = std::fs::create_dir_all(&dir);
        let ck = dir.join("check_imports.py");
        if std::fs::write(&ck, checker).is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "write checker script failed"));
        }
        std::process::Command::new(&py)
            .arg(&ck)
            .arg(&p.display().to_string())
            .output()
    }).await;
    match res {
        Ok(Ok(o)) => {
            if o.status.success() {
                return Ok(None);
            }
            let out = String::from_utf8_lossy(&o.stdout).to_string();
            // 硬问题（符号缺失）：确定性错误，调用方回滚
            if out.contains("REAL_IMPORT_SYMBOL_ERROR") {
                return Err(out.trim().to_string());
            }
            // 软问题（模块缺失，可能环境未装依赖）：仅附警告
            if out.contains("REAL_IMPORT_CHECK_FAILED") {
                return Ok(Some(out.trim().to_string()));
            }
            // 校验脚本自身异常（非导入错误），best-effort 跳过，不阻断正常编辑
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn find_fuzzy_variants(content: &str, find_n: &str, replace_n: &str) -> Option<(String, String, &'static str)> {
    let mut variants: Vec<(String, String, &'static str)> = Vec::new();

    // ①a JSON 转义引号解转义：\" → "、\\n → 换行、\\t → 制表
    if find_n.contains("\\\\") {
        let fv = find_n
            .replace("\\\\\"", "\"")
            .replace("\\\\n", "\n")
            .replace("\\\\t", "\t");
        let rv = replace_n
            .replace("\\\\\"", "\"")
            .replace("\\\\n", "\n")
            .replace("\\\\t", "\t");
        variants.push((fv, rv, "解 JSON 转义"));
    }

    // ①b 弯引号 → 直引号
    let curly = ["\u{201c}", "\u{201d}", "\u{2018}", "\u{2019}"];
    if curly.iter().any(|c| find_n.contains(c)) {
        let straighten = |s: &str| {
            s.replace("\u{201c}", "\"")
                .replace("\u{201d}", "\"")
                .replace("\u{2018}", "'")
                .replace("\u{2019}", "'")
        };
        variants.push((straighten(find_n), straighten(replace_n), "弯引号转直引号"));
    }

    // ② 行首前缀补全：find 首行在文件里找不到，但 find 的**末行**（或首行去掉引号前缀后）
    let f_lines: Vec<&str> = find_n.lines().collect();
    if f_lines.len() >= 2 {
        let tail = f_lines[1..].join("\n");
        if !tail.is_empty() {
            if let Some(pos) = content.find(&tail) {
                // tail（=f_lines[1..]）定位到的是 f_lines[1] 所在行首；要取整段需从 f_lines[0] 行首起。
                let line_start = content[..pos]
                    .rfind('\n')
                    .and_then(|p| content[..p].rfind('\n'))
                    .map(|q| q + 1)
                    .unwrap_or(0);
                // 真实整段 = 从 line_start 起 f_lines.len() 行
                let real: String = content[line_start..]
                    .split_inclusive('\n')
                    .take(f_lines.len())
                    .collect();
                let real_trimmed = real.trim_end_matches('\n');
                // 模型 find 首行与真实首行的差异必须是「前缀性」的（真实行首 ⊃ 或 ≠ 但尾部一致）
                let f_first = f_lines[0];
                let r_first = real_trimmed.lines().next().unwrap_or("");
                let r_trimmed = r_first.trim();
                let f_trimmed = f_first.trim();
                let both_end_same = !r_trimmed.is_empty()
                    && !f_trimmed.is_empty()
                    && (f_first.ends_with(r_first.trim_start())
                        || r_first.ends_with(f_first.trim_start()));
                if both_end_same && real_trimmed != find_n {
                    // replace 侧同步补：replace 的行数通常与 find 对应行结构相同，首行同样补真实前缀
                    let r_lines: Vec<&str> = replace_n.lines().collect();
                    let rv = if r_lines.len() == f_lines.len() {
                        let real_prefix = &r_first[..r_first.len() - f_first.trim_start().len()];
                        let mut new_first = real_prefix.to_string();
                        new_first.push_str(r_lines[0].trim_start());
                        let mut out: Vec<String> = vec![new_first];
                        out.extend(r_lines[1..].iter().map(|s| s.to_string()));
                        out.join("\n")
                    } else {
                        replace_n.to_string()
                    };
                    variants.push((real_trimmed.to_string(), rv, "行首前缀补全"));
                }
            }
        }
    }

    variants.into_iter().next()
}

#[cfg(test)]
#[path = "fs_write_tests.rs"]
mod fs_write_tests;
