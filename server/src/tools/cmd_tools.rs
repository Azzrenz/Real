//! 命令执行域：run（白名单 + 危险拦截 + 超时）/ verify（语言自适应验证）/ env

use crate::mcp::registry::BuiltinTool;
use crate::tools::contract::{validate, ToolError};
use crate::tools::{err_text, project_root};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::process::Command;

/// PowerShell cmdlet 集合：base 在此列表内时自动用 powershell.exe 执行

/// PowerShell cmdlet 集合（执行映射，非拦截）：base 在此列表内时自动用 powershell.exe 执行——
const PS_CMDLETS: [&str; 20] = [
    "Get-ChildItem",
    "Get-Content",
    "Select-String",
    "Get-Item",
    "Get-Process",
    "Get-Service",
    "Get-Date",
    "Get-Location",
    "Measure-Object",
    "Where-Object",
    "Get-Command",
    "Get-FileHash",
    "Remove-Item",
    "Invoke-WebRequest",
    "Invoke-RestMethod",
    "Get-NetTCPConnection",
    "Test-NetConnection",
    "Stop-Process",
    "New-Item",
    "Start-Process",
];

// 单条命令输出进入上下文的字符上限；超出即走 `truncate_with_spill`——
const MAX_OUTPUT_CHARS: usize = 60_000;
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// `REAL_MAX_OUTPUT_CHARS` 可覆盖上限。超限走 `truncate_with_spill`
pub fn max_output_chars() -> usize {
    std::env::var("REAL_MAX_OUTPUT_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_OUTPUT_CHARS)
}

/// cargo build/npm install 冷编译常超 30s；测试版可设 120~300）
pub fn default_timeout_secs() -> u64 {
    std::env::var("REAL_DEFAULT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// 错误摘要通常在输出**尾部**，前截断会砍掉结论导致模型重跑；头保留命令上下文）。
pub fn truncate_output(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let head_len = (max / 3).max(200);
    let tail_len = max.saturating_sub(head_len);
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s.chars().skip(n.saturating_sub(tail_len)).collect();
    format!("{head}\n…（中间省略，共 {n} 字符）\n{tail}")
}

/// 输出超限时，截断文本给模型（带"共 N 字符"自解释），**全文写 spill 文件**并返回路径——

/// 从命令里取出 **stdout 重定向目标**（`>` / `>>`），给"超时且零输出"的回执指路用。
pub fn redirect_target(cmd: &str) -> Option<String> {
    let b: Vec<char> = cmd.chars().collect();
    let mut quote: Option<char> = None;
    let mut best: Option<String> = None;
    let mut i = 0usize;
    while i < b.len() {
        let ch = b[i];
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        if ch == '>' && b.get(i + 1) != Some(&'&') {
            let mut j = i + 1;
            if b.get(j) == Some(&'>') {
                j += 1;
            }
            while b.get(j).map_or(false, |x| x.is_whitespace()) {
                j += 1;
            }
            let mut t = String::new();
            if let Some(&q) = b.get(j) {
                if q == '"' || q == '\'' {
                    j += 1;
                    while let Some(&x) = b.get(j) {
                        if x == q {
                            break;
                        }
                        t.push(x);
                        j += 1;
                    }
                } else {
                    while let Some(&x) = b.get(j) {
                        if x.is_whitespace() || x == '&' || x == '|' {
                            break;
                        }
                        t.push(x);
                        j += 1;
                    }
                }
            }
            if !t.is_empty() {
                best = Some(t);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    best
}

/// 转轨临时文件名的**进程内自增序号** —— 与 `stamp_ms()` 合用保证唯一。
static INLINE_SCRIPT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn defuse_inline_script(command: &str) -> Option<(String, String)> {
    let lower = command.to_lowercase();
    let (interp, ext) = if lower.starts_with("python") || lower.starts_with("py ") {
        ("python", "py")
    } else if lower.starts_with("node") {
        ("node", "mjs")
    } else {
        return None;
    };
    let ci = command.find("-c ")?;
    let rest = command[ci + 3..].trim_start();
    let first = rest.chars().next()?;
    if first != '"' && first != '\'' {
        return None;
    }
    // ── 配对收尾引号：从第 1 个字符起**第一个**同类引号 ──────────────────────
    let mut close: Option<usize> = None;
    for (i, c) in rest.char_indices().skip(1) {
        if c == first {
            close = Some(i);
            break;
        }
    }
    let last = close?;
    if last == 0 {
        return None;
    }
    let code = &rest[1..last];
    let risky = code.contains('\n')
        || code.contains('$')
        || code.contains('%')
        || code.contains('\\')
        || code.contains(first);
    if !risky {
        return None;
    }
    let dir = crate::path::data_root::tmp_dir();
    // 命名走统一时间戳出口（`data_root::stamp_ms`）——见 docs/20260915-数据落盘与命名规范.md
    let seq = INLINE_SCRIPT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!(
        "{}-{}-inline.{ext}",
        crate::path::data_root::stamp_ms(),
        seq
    ));
    std::fs::write(&path, code).ok()?;
    let trailer = rest[last + 1..].trim();
    let new_cmd = if trailer.is_empty() {
        format!("{interp} \"{}\"", path.display())
    } else {
        format!("{interp} \"{}\" {trailer}", path.display())
    };
    Some((new_cmd, path.display().to_string()))
}

pub fn truncate_with_spill(s: &str, max: usize, _cwd: &Path, tag: &str) -> (String, Option<String>) {
    if s.chars().count() <= max {
        return (s.to_string(), None);
    }
    // 落盘全文（写失败不影响主结果——只是丢 spill 出口，截断文本仍带"共 N 字符"）
    let spill_dir = crate::path::data_root::spill_root();
    // 文件名走**唯一入口**（`data_root::spill_file_name`，含统一时间戳）——
    let path = spill_dir.join(crate::path::data_root::spill_file_name(tag));
    let spill = std::fs::create_dir_all(&spill_dir)
        .and_then(|_| std::fs::write(&path, s))
        .map(|_| path.display().to_string())
        .ok();
    let n = s.chars().count();
    let head_len = (max / 3).max(200);
    let tail_len = max.saturating_sub(head_len);
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s.chars().skip(n.saturating_sub(tail_len)).collect();
    let trunc = match &spill {
        Some(p) => format!("{head}\n…（中间省略，共 {n} 字符；全文已落盘: {p}）\n{tail}"),
        None => format!("{head}\n…（中间省略，共 {n} 字符；spill 落盘失败，仅保留首尾）\n{tail}"),
    };
    (trunc, spill)
}

///（如 "找不到路径…因为该路径不存在"），只能猜 → 归因错误（把"已删除"猜成"被拦截"）。
pub fn decode_output(bytes: &[u8]) -> String {
    super::fs_common::decode_text(bytes).0
}

/// 剥离末尾冗余的 `2>&1`。后端已用 `Stdio::piped()` 原生分别抓取 stdout/stderr，
fn strip_terminal_redirects(cmd: &str) -> String {
    let t = cmd.trim_end();
    if let Some(cleaned) = strip_one_terminal_2and1(t) {
        return cleaned;
    }
    // 形态②只对 PowerShell 承载的命令生效：`RemoteException` 是 PS 5.1 对「原生命令写 stderr
    if !outer_is_powershell(t) {
        return cmd.to_string();
    }
    // 形态②：末尾是引号且引号内以 `2>&1` 结尾 → 剥引号内的，引号与外部原样保留
    let Some(quote) = t.chars().last().filter(|c| *c == '"' || *c == '\'') else {
        return cmd.to_string();
    };
    let head = &t[..t.len() - quote.len_utf8()];
    let Some(open) = head.rfind(quote) else {
        return cmd.to_string();
    };
    match strip_one_terminal_2and1(&head[open + quote.len_utf8()..]) {
        Some(cleaned) => format!("{}{quote}{cleaned}{quote}", &head[..open]),
        None => cmd.to_string(),
    }
}

/// 命令的第一个词是否是 PowerShell（兼容 `powershell.exe` 与全路径写法）。
fn outer_is_powershell(cmd: &str) -> bool {
    cmd.split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".exe")
        == "powershell"
}

/// 单层剥离：返回 None = 不该剥。两条护栏在此收口。
fn strip_one_terminal_2and1(s: &str) -> Option<String> {
    let body = s.trim_end().strip_suffix("2>&1")?;
    // 护栏一：`2>&1` 前一个字符必须非字母数字（避免误剥 `std2>&1out` 这类词内子串）
    if body.chars().last()?.is_alphanumeric() {
        return None;
    }
    // 护栏二：所在命令段含 `>` ⇒ 属 `> file 2>&1` 重定向搭配，剥掉会丢 stderr
    let seg_start = body
        .rfind(|c| c == ';' || c == '|' || c == '&')
        .map(|i| i + 1)
        .unwrap_or(0);
    if body[seg_start..].contains('>') {
        return None;
    }
    // 剥掉 `2>&1` 及其前导空白/分隔符，避免残留 `; ` 空段
    Some(
        body.trim_end()
            .trim_end_matches(|c| c == ';' || c == '&')
            .to_string(),
    )
}

/// 类失败签名。Windows 上 `cargo build` 删/写刚生成的 exe 报 `os error 5`（拒绝访问）
fn locked_file_hint(combined: &str) -> Option<&'static str> {
    let c = combined.to_lowercase();
    let sig = c.contains("os error 5")
        || c.contains("拒绝访问")
        || c.contains("access is denied")
        || (c.contains("failed to remove file") || c.contains("failed to rename"))
        || c.contains("(os error 32)");
    if sig {
        Some(
            "【文件被占用·非模型错误】目标文件被占用导致命令失败（典型 os error 5/32：拒绝访问）。常见真因：① 杀软（如 Windows Defender）实时扫描刚生成的 exe；② 上一次运行的进程尚未退出仍持锁；③ 另一 cargo/rustc 实例在跑。建议：等待 2–3 秒重试（杀软通常很快释放）；或用 run({stop_process:<pid>}) 终止占用进程；必要时换临时 target 目录。不要反复换命令姿势——这是环境锁，不是命令写错。",
        )
    } else {
        None
    }
}

/// 检测命令/文本中是否含 #En 步骤引用（如 "#E2"、"运行 #E3"）。
fn has_step_ref(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'#' && (bytes[i + 1] == b'E' || bytes[i + 1] == b'e') {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 2 {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// 模型常把 cwd 传成**目标文件本身**（删除/修改文件时 cwd=文件路径）——旧代码
pub(crate) fn resolve_workdir(cwd: &str, base: &std::path::Path) -> std::path::PathBuf {
    let p = std::path::Path::new(cwd);
    let raw = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    let raw: std::path::PathBuf = raw.components().map(|c| c.as_os_str()).collect();
    if raw.is_dir() {
        raw
    } else if raw.is_file() {
        match raw.parent().map(|d| d.to_path_buf()).filter(|d| d.is_dir()) {
            Some(d) => {
                tracing::info!(cwd = %raw.display(), fallback = %d.display(), "cwd 指向文件，自动用其父目录");
                d
            }
            None => {
                tracing::warn!(cwd = %raw.display(), "cwd 指向文件且无有效父目录，回退项目根");
                base.to_path_buf()
            }
        }
    } else {
        tracing::warn!(cwd = %raw.display(), "cwd 不存在，回退项目根");
        base.to_path_buf()
    }
}

pub fn effective_workdir(raw: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cleaned = std::path::PathBuf::new();
    for c in raw.components() {
        match c {
            std::path::Component::Normal(seg) => {
                let seg_str = seg.to_string_lossy();
                let trimmed = seg_str.trim_end_matches([' ', '.']);
                if trimmed.is_empty() {
                    continue;
                }
                cleaned.push(trimmed);
            }
            other => cleaned.push(other.as_os_str()),
        }
    }
    if cleaned.is_dir() {
        return Some(cleaned);
    }
    // 失效 → 沿祖先链找最近存在目录（"跑在最近有效目录"优于报错）
    for anc in cleaned.ancestors().skip(1) {
        if anc.is_dir() {
            return Some(anc.to_path_buf());
        }
    }
    None
}

/// 命令预览（错误消息用，截断超长命令）
async fn kill_process_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    {
        if let Some(pid) = child.id() {
            let _ = tokio::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    let _ = child.kill().await;
}

/// 撞锁 → 演一遍 kill-wait-retry）。参数化于 workdir（工具已知）+ cargo 命令前缀识别，
fn is_cargo_rebuild(command: &str) -> bool {
    let mut it = command.trim_start().split_whitespace();
    let first = it.next().unwrap_or("");
    let first_base = std::path::Path::new(first)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let second = it.next().unwrap_or("");
    first_base == "cargo" && matches!(second, "build" | "run" | "test")
}

async fn stop_stale_cargo_server(workdir: &Path) {
    // 排除 agent 自身运行时：Real server 的 exe 也在 <workdir>/target 下，杀它等于自杀。
    let own = std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if own.is_empty() {
        return;
    }
    // 仅匹配本项目 target 下的可执行文件（参数化于 workdir），不动其它进程/端口。
    let pattern = format!("{}//target//*", workdir.display());
    let ps = format!(
        "$own='{own_q}'; Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -like '{pat}' -and $_.ExecutablePath -ne $own }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
        own_q = own.replace('\\', "\\\\").replace('\'', "''"),
        pat = pattern.replace('\\', "\\\\").replace('\'', "''"),
    );
    let _ = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        // CREATE_NO_WINDOW：预步不弹 CMD 窗口（解析异常时 -NonInteractive 直接退出不挂住）
        .creation_flags(0x0800_0000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// 就绪探测：wait_port（端口监听）/ wait_http（HTTP 200）/ wait_process（进程出现）
async fn wait_for_ready(
    port: Option<u16>,
    http: Option<String>,
    process: Option<String>,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = std::time::Instant::now() + timeout;
    let mut last_probe = String::new();
    loop {
        if let Some(p) = port {
            if TcpStream::connect(("127.0.0.1", p)).await.is_ok() {
                return Ok(format!("端口 {p} 已监听"));
            }
            last_probe = format!("端口 {p} 未监听");
        }
        if let Some(u) = &http {
            match reqwest::get(u).await {
                Ok(resp) if resp.status().is_success() => {
                    return Ok(format!("HTTP {u} 返回 {}", resp.status()));
                }
                Ok(resp) => last_probe = format!("HTTP {u} 返回 {}", resp.status()),
                Err(_) => last_probe = format!("HTTP {u} 不可达"),
            }
        }
        if let Some(proc) = &process {
            if let Ok(o) = Command::new("tasklist")
                .args(["/FI", &format!("IMAGENAME eq {proc}"), "/NH"])
                .output()
                .await
            {
                let text = String::from_utf8_lossy(&o.stdout);
                if !text.contains("No tasks") && text.contains(proc.as_str()) {
                    return Ok(format!("进程 {proc} 已出现"));
                }
                last_probe = format!("进程 {proc} 未出现");
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("等待服务就绪超时（{timeout:?}）：{last_probe}"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// run

pub struct RunTool;

#[async_trait]
impl BuiltinTool for RunTool {
    fn name(&self) -> &'static str {
        "run"
    }
    fn description(&self) -> &'static str {
        concat!(
            include_str!("../../prompts/tools/run.md"),
            include_str!("../../prompts/tools/spill_note.md")
        )
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "description":"command 与 script 二选一必给其一。修饰字段（timeout / wait_port / wait_http / wait_process / background / stop_process / wait_exit / sample_seconds / env）必须与 command 或 script 同用，单独传无效。",
            "properties":{
                "command":{"type":"string","minLength":1,"description":"完整命令，如 cargo check。"},
                "script":{"type":"object","properties":{
                    "code":{"type":"string","minLength":1,"description":"脚本代码原文（JSON 字符串）"},
                    "lang":{"type":"string","enum":["python","node"],"default":"python","description":"解释器"}
                },"required":["code"],"description":"内联脚本（推荐）：代码落临时文件执行，绕开 cmd 引号剥离"},
                "cwd":{"type":"string","default":".","description":"工作目录（相对项目根；显式传非空值即保留，缺省时后端注入本会话工作区）"},
                "target":{"type":"string","description":"cwd 的别名；同时传时以 target 为准"},
                "timeout":{"type":"integer","default":30,"minimum":5,"maximum":600,"description":"超时秒数（5–600）"},
                "wait_port":{"type":"integer","minimum":1,"maximum":65535,"description":"等此端口开始监听（1–65535，合法端口区间）"},
                "wait_http":{"type":"string","description":"等此 URL 返回 200"},
                "wait_process":{"type":"string","description":"等此进程名出现"},
                "background":{"type":"boolean","default":false,"description":"后台运行，立即返回 {background,pid,log_file}"},
                "stop_process":{"type":"integer","minimum":1,"description":"终止指定 PID（≥1，正整数进程号）"},
                "wait_exit":{"type":"boolean","default":false,"description":"等进程退出（长命令的完成信号）"},
                "sample_seconds":{"type":"integer","minimum":1,"maximum":600,"description":"限时采样 N 秒后自动 kill 并返回已捕获输出（1–600）"},
                "env":{"type":"object","additionalProperties":true,"description":"子进程环境变量"}
            },
            "required":[],
            "anyOf":[{"required":["command"]},{"required":["script"]}],
            "additionalProperties":true
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": false, "destructive": true, "idempotent": false})
    }

    fn output_schema(&self) -> Option<Value> {
        // registry 强制校验——run 返回 {kind/run_result,data{...}} 锁定结构。
        Some(json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "ok": {"type": "boolean"},
                "kind": {"type": "string"},
                "data": {"type": "object", "additionalProperties": false, "properties": {
                    "exit_code": {"type": "integer"},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"},
                    "command": {"type": "string", "description": "实际执行的命令（恒在——命令翻译链可能改写过它，这是核对实际执行的唯一窗口）"},
                    "command_asked": {"type": "string", "description": "仅当命令翻译链改写了命令时出现：模型原本请求的命令"},
                    "cwd": {"type": "string", "description": "仅后台/终止/就绪/超时分支带（模型需据此拼日志路径）；主路径不带（会话级固定，冗余）"},
                    "termination": {"type": "object", "description": "仅特殊终止（ready / timed_out / 后台）时带"},
                    "background": {"type": "boolean"},
                    "pid": {"type": "integer"},
                    "log_file": {"type": "string"},
                    "spilled": {"type": "boolean", "description": "仅当 stdout 被截断落盘时出现"},
                    "stderr_spilled": {"type": "boolean", "description": "仅当 stderr 被截断落盘时出现"},
                    "spill_path": {"type": ["string", "null"]},
                    "stderr_spill_path": {"type": ["string", "null"]},
                    "marker": {"type": "string"}
                }},
                "warnings": {"type": "array"},
                "error": {}
            }
        }))
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args).map_err(|e| err_text(&e))?;
        // command / script 二选一：script 直通道（代码落临时文件执行，零 shell 转义）
        let command: String = if let Some(sc) = args.get("script").and_then(|v| v.as_object()) {
            let code = sc.get("code").and_then(|v| v.as_str()).unwrap_or("");
            if code.trim().is_empty() {
                return Err("script.code 不能为空".into());
            }
            let lang = sc.get("lang").and_then(|v| v.as_str()).unwrap_or("python");
            let (interp, ext) = match lang {
                "node" => ("node", "mjs"),
                _ => ("python", "py"),
            };
            let path = crate::path::data_root::tmp_dir().join(format!(
                "{}-run.{ext}",
                crate::path::data_root::stamp_ms()
            ));
            std::fs::write(&path, code).map_err(|e| format!("临时脚本落盘失败: {e}"))?;
            format!("{interp} \"{}\"", path.display())
        } else if let Some(c) = args["command"].as_str() {
            c.trim().to_string()
        } else {
            // 兜底（schema 的 `anyOf` 已在上游 `validate` 拦截；走到这里说明契约有洞）。
            let keys: Vec<String> = args
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            return Err(format!(
                "run 需要 command 或 script，本次两个都没给（收到的字段：{}）。\n\
                 · command：{{\"command\": \"cargo check\", \"cwd\": \"D:/x\"}}\n\
                 · script ：{{\"script\": {{\"code\": \"print(1)\", \"lang\": \"python\"}}}}\n\
                 wait_port / wait_http / wait_process / stop_process / sample_seconds / timeout / \
                 background / wait_exit / env 都是【修饰字段】，必须和 command 或 script 一起传，\
                 单独使用无效。",
                if keys.is_empty() {
                    "（空）".to_string()
                } else {
                    keys.join(", ")
                }
            ));
        };
        let command = command.as_str();
        // 接纳 target 作为 cwd 别名（契约对齐）：模型习惯传 target 时直接并入 cwd，
        let cwd = args["target"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| args["cwd"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or(".");
        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(default_timeout_secs);
        // 程序/长任务不需要模型拼 PowerShell（$p = Start-Process 撞白名单）或管道挂死。
        let wait_exit = args
            .get("wait_exit")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let background = args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let stop_process = args
            .get("stop_process")
            .and_then(|v| v.as_u64())
            .map(|p| p as u32);
        // N 秒、实时捕获输出、到点（或进程提前退出）自动 kill 并返回已捕获输出。验证常驻
        let sample_seconds = args
            .get("sample_seconds")
            .and_then(|v| v.as_u64());
        let is_sampling = sample_seconds.is_some();
        // 静默失效（wait_port 被当一次性命令等退出码 → 常驻服务永不退出 → 30s TIMEOUT 死循环；
        let wait_port = args
            .get("wait_port")
            .and_then(|v| v.as_u64())
            .map(|p| p as u16);
        let wait_http = args
            .get("wait_http")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        let wait_process = args
            .get("wait_process")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        let env_overrides: Vec<(String, String)> = args
            .get("env")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // pytest-of-<user>，目录过多会弹出交互式确认（SAFE_DELETE_BULK_CONFIRM_REQUIRED），
        let lower_cmd = command.to_lowercase();
        let mut effective_command = command.to_string();
        // `--basetemp=... --continue-on-collection-errors`（v9.5/增量5），字符串追加会

        // 模型习惯写 `cargo test | tail -40`、`grep xxx file | head` 这类 Unix 管道，
        let unix_command: Option<String> = if cfg!(windows) {
            if crate::tools::cmd_bash::wants_unix_shell(command) {
                Some(command.to_string())
            } else {
                crate::tools::cmd_bash::rewrite_cmd_builtin(command)
            }
        } else {
            None
        };
        let unix_shell: Option<std::path::PathBuf> = unix_command
            .as_ref()
            .and_then(|_| crate::tools::cmd_bash::locate());
        let use_unix_shell = unix_shell.is_some();
        let mut slash_normalized = false;
        if use_unix_shell {
            // 用路由阶段确定的命令：Unix 原样，或 cmd 内建的 Unix 等价改写
            if let Some(uc) = unix_command.clone() {
                effective_command = uc;
            }
        } else {
            // 混血命令检测：首词属 PS 壳、命令体却是 cmd 语法 —— 这种命令在任何单一壳里都跑不通，
            if let Some(feature) = crate::tools::cmd_bash::detect_mixed_shell(command, &PS_CMDLETS) {
                return Err(err_text(&ToolError::domain(
                    "MIXED_SHELL_SYNTAX",
                    &format!(
                        "这条命令混用了两种壳的语法：首词是 PowerShell 命令，而命令体里的 {feature} —— 二者不能写在同一行。"
                    ),
                    Some("整条改用 Git Bash 那套（一条命令只用一个壳）：删除用 `rm -f '路径'`、列文件用 `ls` / `find`、多条串联用 `;`（不是 `&`）——例：`rm -f 'D:\\x\\a.rs' ; ls 'D:\\x'`。"),
                )));
            }
            effective_command =
                crate::tools::cmd_translate::translate_unix_pipeline(&effective_command)?;
            // 走 cmd/PowerShell：盘符正斜杠归一为反斜杠（cmd 内建 type/dir/findstr 只认反斜杠）
            let fixed = crate::tools::cmd_translate::normalize_drive_slash(&effective_command);
            if fixed != effective_command {
                slash_normalized = true;
                effective_command = fixed;
            }
        }

        //（如 "#E2 的命令"/"运行 #E3"）——#En 是证据引用，解析层会把整体替换成工具
        if use_unix_shell {
            let fixed =
                crate::tools::cmd_translate::normalize_backslash_paths_for_bash(&effective_command);
            if fixed != effective_command {
                slash_normalized = true;
                effective_command = fixed;
            }
        }
        if has_step_ref(&effective_command) {
            return Err(err_text(&ToolError::domain(
                "COMMAND_PLACEHOLDER",
                "命令含 #En 步骤引用（如 \"#E2\"）——#En 是证据引用，不是命令。",
                Some("把要执行的**完整命令**直接写出来（如 cargo check / python script.py），不要引用步骤编号；若命令来自某次工具结果，直接用结果里的真实命令文本。"),
            )));
        }

        // 只做 token 级等价名映射 + 参数原样传执行层——不解析/不改写/不 contains 替换。
        let first = command.split_whitespace().next().unwrap_or("");
        let base = Path::new(first)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| first.to_string())
            .trim_end_matches(".cmd")
            .trim_end_matches(".bat")
            .to_string();
        let base = if base.is_empty() {
            first
        } else {
            base.as_str()
        };

        // ═══ 命令翻译链（token 级等价映射，参数原样不动）═══
        let (mut effective_command, base) = if use_unix_shell {
            (effective_command.clone(), base)
        } else {
            (|| -> Result<(String, &str), String> {
            // ① token 级等价名映射表（命令首 token → Windows 等价；参数一律原样不动）
            let rest = effective_command
                .split_once(' ')
                .map(|(_, r)| r.trim())
                .unwrap_or("");
            // cmd 内置命令（dir/type/cd/where/findstr/echo/call）**只认反斜杠路径**——
            let fix_cmd_slashes = |s: &str, is_ls: bool| -> String {
                if is_ls
                    || matches!(
                        base,
                        "dir" | "type" | "cd" | "where" | "findstr" | "echo" | "call" | "tree"
                    )
                {
                    s.split_whitespace()
                        .map(|tok| {
                            let t = tok.trim_matches('"');
                            if t.len() >= 3
                                && t.as_bytes()[0].is_ascii_alphabetic()
                                && t.as_bytes()[1] == b':'
                                && t.contains('/')
                            {
                                tok.replace('/', "\\")
                            } else {
                                tok.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    s.to_string()
                }
            };
            match base {
                "python3" => Ok((
                    format!("python{}{}", if rest.is_empty() { "" } else { " " }, rest),
                    base,
                )),
                "ls" => Ok((format!("dir {}", fix_cmd_slashes(rest, true)), "dir")),
                "mkdir" => {
                    // mkdir -p 的 -p 剥掉（Windows 不需要），参数原样传 PowerShell
                    let r2 = rest
                        .trim_start_matches("-p ")
                        .trim_start_matches("-p")
                        .trim()
                        .trim_matches('"');
                    if r2.is_empty() {
                        return Err(err_text(&ToolError::domain(
                            "ARGS_REQUIRED",
                            "mkdir 需要目标目录路径（如 mkdir D:/new-folder）",
                            Some("直接给路径即可，Windows 自动创建多级目录"),
                        )));
                    }
                    Ok((format!("powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path '{}' | Out-Null\"", r2.replace('\'', "''")), "mkdir"))
                }
                "cd" => {
                    // cmd cd 不带 /d 不跨盘符 → 自动补（模型无需知道 Windows 细节）；
                    let r2 = rest.trim_matches('"').trim();
                    let is_drive = r2.len() >= 3
                        && r2.as_bytes()[0].is_ascii_alphabetic()
                        && r2.as_bytes()[1] == b':'
                        && (r2.as_bytes()[2] == b'/' || r2.as_bytes()[2] == b'\\');
                    if is_drive && !r2.starts_with("/d") && !r2.starts_with("/D") {
                        Ok((
                            format!(
                                "cd /d {}",
                                fix_cmd_slashes(
                                    effective_command
                                        .split_once(' ')
                                        .map(|(_, r)| r)
                                        .unwrap_or("")
                                        .trim_start(),
                                    false
                                )
                            ),
                            base,
                        ))
                    } else {
                        Ok((fix_cmd_slashes(&effective_command, false), base))
                    }
                }
                _ => Ok((fix_cmd_slashes(&effective_command, false), base)),
            }
            })()?
        };

        // grep→findstr、sed/awk→拒绝+教学）是"语义等价"（读文件=读文件），不是"猜意图
        if !use_unix_shell && !effective_command.contains('|') {
            match crate::tools::cmd_translate::translate_single_unix_command(&effective_command) {
                Ok(Some(translated)) => {
                    tracing::debug!(original = %effective_command, translated = %translated, "run 单命令 Unix→Windows 等价翻译");
                    effective_command = translated;
                }
                Err(why) => {
                    // 这次拒绝发生在**构造命令阶段**（进程还没起）⇒ 整条命令零副作用。
                    return Err(err_text(&ToolError::domain(
                        "UNIX_CMD_NOT_TRANSLATABLE",
                        format!(
                            "{why}\n（**整条命令未执行**：本次拒绝发生在构造命令阶段、进程尚未启动；\
                             `&`/`&&` 链上任一段被拒即整链拒绝——没有任何副作用，不必猜哪一段跑过、\
                             也不必再确认一次。改掉不合规的那一段，整条重发即可。）"
                        ),
                        None,
                    )));
                }
                Ok(None) => {}
            }
        }

        // 内联脚本断路：高危 -c 自动落临时文件改写执行（信封 command 字段会显示转轨后命令）
        if let Some((rewritten, tmp_path)) = defuse_inline_script(&effective_command) {
            tracing::info!(original = %effective_command, rewritten = %rewritten, tmp = %tmp_path, "run 内联脚本自动转轨（避 cmd 引号剥离）");
            effective_command = rewritten;
        }

        let workdir = resolve_workdir(cwd, &project_root());

        // 撞锁→演 kill-wait-retry）。参数化于 workdir + cargo 命令前缀，排除 agent 自身运行时。
        if is_cargo_rebuild(command) {
            stop_stale_cargo_server(&workdir).await;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        }

        // 环境变量未刷新），模型写 `set PATH=...` 会被拒、写完整路径又冗长。这里把常见
        let mut extra_paths: Vec<String> = Vec::new();
        if let Some(home) = std::env::var_os("USERPROFILE") {
            extra_paths.push(
                Path::new(&home)
                    .join(".cargo\\bin")
                    .to_string_lossy()
                    .to_string(),
            );
            extra_paths.push(
                Path::new(&home)
                    .join(".rustup\\toolchains\\stable-x86_64-pc-windows-msvc\\bin")
                    .to_string_lossy()
                    .to_string(),
            );
            extra_paths.push(
                Path::new(&home)
                    .join("AppData\\Roaming\\npm")
                    .to_string_lossy()
                    .to_string(),
            );
        }
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            let pf = Path::new(&pf);
            for p in [pf.join("Git\\cmd"), pf.join("Git\\bin"), pf.join("CMake\\bin")] {
                if p.exists() {
                    extra_paths.push(p.to_string_lossy().to_string());
                }
            }
        }
        // 不再需要 CWD_NOT_FOUND 报错分支——模型传错 cwd 由后端兜底，不白费一轮。

        // 末尾 2>&1 在 powershell -Command 里触发解析错，执行前剥离（保留管道的 2>&1 | xxx）。
        let effective_command = if use_unix_shell {
            effective_command
        } else {
            strip_terminal_redirects(&effective_command)
        };

        // Windows 下执行分派：PowerShell cmdlet（Get-ChildItem/Select-String 等）
        let mut cmd_builder = if cfg!(windows) {
            if let Some(bash) = &unix_shell {
                // Unix 母语直通：原样交 Git Bash（grep/find/sed/awk 由它自带，无需注入 PATH）
                let mut c = Command::new(bash);
                c.arg("-c").arg(&effective_command);
                c
            } else if PS_CMDLETS.contains(&base) {
                // `command`（原始），effective_command 的管道翻译（translate_unix_pipeline）
                let mut c = Command::new("powershell");
                c.args(["-NoProfile", "-NonInteractive", "-Command", &effective_command]);
                c
            } else if matches!(base, "python" | "python3" | "py") && effective_command.contains(" -c ") {
            // python -c 经 cmd /C 引号被剥 → 原生 CreateProcess（CommandLineToArgvW 正确解析引号）。
                let code =
                    crate::tools::cmd_translate::extract_inline_code(&effective_command, " -c ");
                // 跨行内联代码：原生不带管道、cmd /C 不支持跨行引号 → 整条交 PowerShell 承载
                let carried = if code.contains('\n') {
                    crate::tools::cmd_translate::to_ps_pipeline(command).map(|ps| {
                        let mut c = Command::new("powershell");
                        c.args(["-NoProfile", "-NonInteractive", "-Command", &ps]);
                        c
                    })
                } else {
                    None
                };
                match carried {
                    Some(c) => c,
                    None => {
                        let mut c = Command::new(if base == "python3" { "python" } else { base });
                        // 不加"空则 pass"兜底：抽取异常时让解释器报真实错（Argument expected for the -c option）。
                        c.arg("-c").arg(code);
                        c
                    }
                }
            } else if base == "node" && effective_command.contains(" -e ") {
                // node -e 同 python -c：原生 CreateProcess，cmd 不介入；跨行代码同样交 PS 承载
                let code =
                    crate::tools::cmd_translate::extract_inline_code(&effective_command, " -e ");
                let carried = if code.contains('\n') {
                    crate::tools::cmd_translate::to_ps_pipeline(command).map(|ps| {
                        let mut c = Command::new("powershell");
                        c.args(["-NoProfile", "-NonInteractive", "-Command", &ps]);
                        c
                    })
                } else {
                    None
                };
                match carried {
                    Some(c) => c,
                    None => {
                        let mut c = Command::new("node");
                        c.arg("-e").arg(code);
                        c
                    }
                }
            } else if matches!(base, "curl" | "curl.exe" | "wget") {
                // curl 引号参数经 cmd /C 被剥 → 原生 CreateProcess；含 && 多命令串联则走 cmd /C
                let has_multi = effective_command.contains("&&") || effective_command.contains(" & ");
                if has_multi {
                    let mut c = Command::new("cmd");
                    c.args(["/C", &effective_command]);
                    c
                } else {
                    let mut c = Command::new(if base == "curl.exe" { "curl" } else { base });
                    // effective_command 含 "curl" 自身前缀，raw_arg 整行传给 curl.exe 会把
                    let rest = effective_command
                        .split_once(' ')
                        .map(|(_, r)| r.trim())
                        .unwrap_or("");
                    #[cfg(windows)]
                    c.raw_arg(rest);
                    #[cfg(not(windows))]
                    c.arg(rest);
                    c
                }
            } else if lower_cmd.contains("| select-object")
                || lower_cmd.contains("| sort-object")
                || lower_cmd.contains("| format-list")
                || lower_cmd.contains("| select-string")
            {
                // 含 PowerShell 管道命令（Select-Object 等）时整条走 powershell（base 是 cargo 等但管道段需要 PS）
                let mut c = Command::new("powershell");
                c.args(["-NoProfile", "-NonInteractive", "-Command", &effective_command]);
                c
            } else {
                let mut c = Command::new("cmd");
                c.arg("/C");
                // 时 command 含引号（如 call "D:\x\build-run.bat"）会被 Rust 自动转义成 \"，
                #[cfg(windows)]
                c.raw_arg(&effective_command);
                #[cfg(not(windows))]
                c.arg(&effective_command);
                c
            }
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        if !extra_paths.is_empty() {
            let mut path = std::env::var_os("PATH").unwrap_or_default();
            for p in &extra_paths {
                if std::path::Path::new(p).exists() && !path.to_string_lossy().contains(p.as_str())
                {
                    path.push(";");
                    path.push(p);
                }
            }
            cmd_builder.env("PATH", path);
        }
        for (k, val) in &env_overrides {
            cmd_builder.env(k, val);
        }
        // cargo/npm 等默认彩色输出 + 分页器（less/more）会让模型看到 ANSI 转义乱码、或
        cmd_builder.env("NO_COLOR", "1");
        cmd_builder.env("TERM", "dumb");
        cmd_builder.env("PAGER", "cat");
        cmd_builder.env("GIT_PAGER", "cat");
        // pytest 运行：关闭缓存 provider 与临时目录自动清理（避免 SAFE_DELETE_BULK_CONFIRM_REQUIRED
        if lower_cmd.contains("pytest") {
            let existing = std::env::var("PYTEST_ADDOPTS").unwrap_or_default();
            let mut addopts = if existing.is_empty() {
                String::new()
            } else {
                format!("{existing} ")
            };
            // -p no:cacheprovider：关缓存（防 .pytest_cache 共享锁/串扰）
            let basetemp = std::env::temp_dir().join(format!(
                "{}-pytest-{}",
                crate::path::data_root::stamp_ms(),
                std::process::id()
            ));
            let _ = std::fs::create_dir_all(&basetemp);
            addopts.push_str(&format!(
                "-p no:cacheprovider -o tmp_path_retention_policy=all --basetemp={} --continue-on-collection-errors",
                basetemp.display()
            ));
            cmd_builder.env("PYTEST_ADDOPTS", addopts);
        }
        // Python editable 安装会在全局 site-packages 写 .pth 指向**某次 run** 的 workspace
        if lower_cmd.contains("python") {
            let src_dir = workdir.join("src");
            if src_dir.is_dir() {
                let existing = std::env::var("PYTHONPATH").unwrap_or_default();
                let mut pp = src_dir.to_string_lossy().to_string();
                if !existing.is_empty() {
                    pp.push(';');
                    pp.push_str(&existing);
                }
                cmd_builder.env("PYTHONPATH", pp);
            }
        }
        // background=true：spawn 后**立即返回**，不等待退出码——stdout/stderr 落日志文件，
        let background_log: Option<std::path::PathBuf> = if background {
            let log_dir = crate::path::data_root::background_dir();
            let _ = std::fs::create_dir_all(&log_dir);
            Some(log_dir.join(format!(
                "{}-bg-{}.log",
                crate::path::data_root::stamp_ms(),
                std::process::id()
            )))
        } else {
            None
        };
        // 有效 cwd 兜底：267 不再漏给模型（见 effective_workdir）
        let eff_cwd = effective_workdir(&workdir);
        // 源码结构护栏（**执行前快照**）：命令可能绕过编辑工具直接改 `.rs`——
        let rs_guards = crate::tools::fs_common::snapshot_rs_targets(&effective_command, &workdir);
        let mut child = match &background_log {
            Some(log_path) => {
                let file_out = std::fs::File::create(log_path).map_err(|e| {
                    err_text(&ToolError::domain(
                        "EXEC_FAILED",
                        format!("创建后台日志文件失败: {e}"),
                        None,
                    ))
                })?;
                let file_err = file_out.try_clone().map_err(|e| {
                    err_text(&ToolError::domain(
                        "EXEC_FAILED",
                        format!("克隆后台日志文件失败: {e}"),
                        None,
                    ))
                })?;
                let mut b = cmd_builder.stdout(Stdio::from(file_out)).stderr(Stdio::from(file_err));
                if let Some(d) = &eff_cwd {
                    b = b.current_dir(d);
                }
                b.spawn()
            }
            None => {
                let mut b = cmd_builder.stdout(Stdio::piped()).stderr(Stdio::piped());
                if let Some(d) = &eff_cwd {
                    b = b.current_dir(d);
                }
                b.spawn()
            }
        }
        .map_err(|e| {
            err_text(&ToolError::domain(
                "EXEC_FAILED",
                format!("进程启动失败: {e}"),
                Some("请检查命令"),
            ))
        })?;

        // background=true：立即返回（不等待退出码），输出在日志文件
        if let Some(log_path) = &background_log {
            let pid = child.id().unwrap_or(0);
            // wait_exit=true：**等进程退出**（设计决定："判断进程而非靠时间等）——
            if wait_exit {
                let mut waited = 0u64;
                let exit_code = loop {
                    match child.try_wait() {
                        Ok(Some(st)) => break st.code().unwrap_or(-1),
                        Ok(None) => {
                            if waited >= 600 {
                                let result_json = json!({
                                    "ok": true, "kind": "run_result",
                                    "data": {"exit_code": -1, "stdout": format!("等待 600s 进程仍未退出（已转后台继续跑）：pid={pid}
日志：{}
后续用 run({{\"command\":\"type 日志路径\"}}) 查进度，或 stop_process 终止。", log_path.display()), "stderr": "", "cwd": workdir.display().to_string(), "command": effective_command, "background": true, "pid": pid, "log_file": log_path.display().to_string()},
                                    "warnings": ["wait_exit 到硬上限：进程未退出，已转后台语义"],
                                    "error": null
                                });
                                return Ok(result_json.to_string());
                            }
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            waited += 1;
                        }
                        Err(e) => return Err(format!("进程状态查询失败: {e}")),
                    }
                };
                let log_tail: String = std::fs::read(log_path)
                    .map(|b| {
                        let c = super::fs_common::decode_text(&b).0;
                        let chars: Vec<char> = c.chars().collect();
                        let start = chars.len().saturating_sub(8000);
                        chars[start..].iter().collect()
                    })
                    .unwrap_or_default();
                let result_json = json!({
                    "ok": true, "kind": "run_result",
                    "data": {"exit_code": exit_code, "stdout": format!("[进程已退出，exit={exit_code}] 日志尾部：
{log_tail}"), "stderr": "", "cwd": workdir.display().to_string(), "command": effective_command, "background": true, "pid": pid, "log_file": log_path.display().to_string()},
                    "warnings": [],
                    "error": null
                });
                return Ok(result_json.to_string());
            }
            let result_json = json!({
                "ok": true, "kind": "run_result",
                "data": {"exit_code": -1, "stdout": format!("后台进程已启动（pid={pid}），stdout/stderr 已重定向到日志文件：\n{}\n用 run({{\"command\":\"type 日志路径\"}}) 读取输出；用 run({{\"command\":\"<任意>\", \"stop_process\": {pid}}}) 终止该进程。", log_path.display()), "stderr": "", "cwd": workdir.display().to_string(), "command": effective_command, "background": true, "pid": pid, "log_file": log_path.display().to_string()},
                "warnings": ["后台运行：命令未等待退出（常驻语义），exit_code=-1 不代表失败；日志文件随进程持续写入"],
                "error": null
            });
            return Ok(result_json.to_string());
        }
        // stop_process=<pid>：终止指定后台进程（配合 background 启动的进程）
        if let Some(pid) = stop_process {
            let killed = {
                #[cfg(windows)]
                {
                    let out = tokio::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .await;
                    out.map(|s| s.success()).unwrap_or(false)
                }
                #[cfg(not(windows))]
                {
                    let out = tokio::process::Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .await;
                    out.map(|s| s.success()).unwrap_or(false)
                }
            };
            let result_json = json!({
                "ok": true, "kind": "run_result",
                "data": {"exit_code": 0, "stdout": format!("后台进程 {pid} 已终止（killed={killed}）。如已退出则幂等达成。"), "stderr": "", "cwd": workdir.display().to_string(), "command": format!("stop_process {pid}")},
                "warnings": if killed { Vec::<String>::new() } else { vec!["进程可能已自行退出（幂等：终止不存在的进程不算失败）".to_string()] },
                "error": null
            });
            return Ok(result_json.to_string());
        }

        // sample_seconds 优先作为运行时长（限时采样）；否则用 timeout 死锁防护。
        let run_secs = sample_seconds.unwrap_or(timeout_secs);
        let timeout = Duration::from_secs(run_secs);
        // （npm run dev / cargo run / python app.py 等不退出）不等待退出码，轮询
        if wait_port.is_some() || wait_http.is_some() || wait_process.is_some() {
            match wait_for_ready(wait_port, wait_http, wait_process, timeout).await {
                Ok(cond) => {
                    let result_json = json!({
                        "ok": true, "kind": "run_result",
                        "data": {"exit_code": -1, "stdout": format!("（常驻服务已就绪，命令仍在后台运行）\n就绪条件: {cond}"), "stderr": "", "cwd": workdir.display().to_string(), "command": effective_command, "termination": json!({"kind": "ready"})},
                        "warnings": ["wait_* 就绪探测：命令未退出（常驻服务在后台继续运行），exit_code=-1 不代表失败"],
                        "error": null
                    });
                    return Ok(result_json.to_string());
                }
                Err(why) => {
                    kill_process_tree(&mut child).await;
                    return Err(err_text(&ToolError::domain(
                        "TIMEOUT",
                        why,
                        Some("请检查服务是否正常启动（端口占用/启动报错），或提高 timeout"),
                    )));
                }
            }
        }
        // 持续读管道（read_to_end 字节级）：避免 read_to_string 遇 GBK 输出
        let mut stdout_buf: Vec<u8> = Vec::new();
        let mut stderr_buf: Vec<u8> = Vec::new();
        let mut so = child.stdout.take();
        let mut se = child.stderr.take();
        let status = tokio::time::timeout(timeout, async {
            let read_out = async {
                if let Some(r) = so.as_mut() {
                    let _ = r.read_to_end(&mut stdout_buf).await;
                }
            };
            let read_err = async {
                if let Some(r) = se.as_mut() {
                    let _ = r.read_to_end(&mut stderr_buf).await;
                }
            };
            tokio::join!(read_out, read_err, child.wait()).2
        })
        .await;
        let exit_code = match status {
            Ok(Ok(s)) => s.code().unwrap_or(-1),
            Ok(Err(e)) => {
                return Err(err_text(&ToolError::domain(
                    "EXEC_FAILED",
                    format!("等待失败: {e}"),
                    None,
                )))
            }
            Err(_) => {
                kill_process_tree(&mut child).await;
                // TIMEOUT 错误，部分已输出的内容被丢弃 → 模型看到空白、无法归因 → 卡死/重跑。
                let partial_out = decode_output(&stdout_buf);
                let partial_err = decode_output(&stderr_buf);
                let (termination, marker) = if is_sampling {
                    (
                        json!({"kind": "sampled", "seconds": run_secs}),
                        format!("[sampled after {run_secs}s]"),
                    )
                } else {
                    (
                        json!({"kind": "timed_out", "seconds": run_secs}),
                        format!("[timed out after {run_secs}s]"),
                    )
                };
                let note = if is_sampling {
                    format!("已采样 {run_secs}s，进程已终止并返回截至此刻的输出")
                } else if partial_out.trim().is_empty() && partial_err.trim().is_empty() {
                    // **"超时 + 零输出"必须给出归因，否则模型只会重跑同一形态的命令。**
                    match redirect_target(&effective_command) {
                        Some(target) => format!(
                            "命令超时（{timeout_secs}s），并且 **Real 未捕获到任何输出** —— \
                             本命令把 stdout 重定向到了 `{target}`，那部分内容不经本工具的管道，我看不见。\n\
                             先查那个文件本身：① 它在增长吗（不增长 ⇒ 卡住，或输出被缓冲）；\
                             ② 若它恒为 0 字节，多半是 Python 的**块缓冲**（重定向时默认开启），\
                             加 `-u`（`python -u …`）让它实时落盘；\n\
                             长任务改用 `background:true` 起，再轮询该文件，别用前台等它跑完。"
                        ),
                        None => format!(
                            "命令超时（{timeout_secs}s），并且**全程零输出**。常见成因：\
                             ① 命令在等输入（读到 EOF 才继续）；\
                             ② 数据量远超预估、卡在超线性算法上（先拿小样本验证，并给命令加进度打印）；\
                             ③ 输出被写进了别的文件/日志（本工具只能看到管道里的内容）。"
                        ),
                    }
                } else {
                    format!("命令超时（{timeout_secs}s），已返回截至超时前的部分输出（可能不完整）")
                };
                let result_json = json!({
                    "ok": true, "kind": "run_result",
                    "data": {
                        "exit_code": -1,
                        "stdout": truncate_output(&partial_out, max_output_chars()),
                        "stderr": truncate_output(&partial_err, max_output_chars()),
                        "cwd": workdir.display().to_string(),
                        "command": effective_command,
                        // 终止原因契约（大厂 C1/C2）：sampled=预期采样到点 / timed_out=超时被杀，
                        "termination": termination,
                        "marker": marker
                    },
                    "warnings": [note],
                    "error": null
                });
                return Ok(result_json.to_string());
            }
        };
        // Windows 中文系统 cmd 输出 GBK：decode_output 先严格 UTF-8、失败再 GBK 解码——
        let raw_out = decode_output(&stdout_buf);
        let raw_err = decode_output(&stderr_buf);
        let max_chars = max_output_chars();
        let (stdout, spill_path) = truncate_with_spill(&raw_out, max_chars, &workdir, &first);
        let (stderr, stderr_spill_path) =
            truncate_with_spill(&raw_err, max_chars, &workdir, &format!("{first}-err"));
        // 确定性截断标志：模型读 data 顶层即可确定"stdout/stderr 是否被截断、全文在哪"，
        let stdout_truncated = spill_path.is_some();
        let stderr_truncated = stderr_spill_path.is_some();

        // 返回 Success 信封 + 完整 run_result（data.exit_code/stdout/stderr 全保留，
        let cmd_lower = effective_command.to_lowercase();
        let findstr_no_match = exit_code == 1
            && (cmd_lower.contains("findstr") || cmd_lower.contains("select-string"))
            && stdout.is_empty()
            && stderr.is_empty();
        // 中文锚点 + findstr 零命中 → 编码静默漏高发（GBK/UTF-8 代码页），
        let has_cjk = effective_command
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
        let cjk_findstr_suspect = findstr_no_match
            && cmd_lower.contains("findstr")
            && !cmd_lower.contains("select-string")
            && has_cjk;
        // taskkill / del / rmdir 等）删除**不存在的目标**时 PowerShell/cmd 报 exit 1 +
        let stderr_lower = stderr.to_lowercase();
        let idempotent_cleanup = exit_code != 0
            && (cmd_lower.contains("remove-item")
                || cmd_lower.contains("taskkill")
                || cmd_lower.contains("del ")
                || cmd_lower.contains("rmdir")
                || cmd_lower.contains("erase "))
            && (stderr_lower.contains("不存在")
                || stderr_lower.contains("找不到")
                || stderr_lower.contains("cannot find")
                || stderr_lower.contains("not found"));
        let effective_exit = if findstr_no_match || idempotent_cleanup {
            0
        } else {
            exit_code
        };
        // 源码结构护栏（**执行后校验 + 回滚**）：只在前台同步路径做——
        let src_guard_warnings = crate::tools::fs_common::guard_rs_targets(&rs_guards);

        let cmd_rewritten = effective_command.as_str() != command;
        let mut data = serde_json::Map::new();
        data.insert("stdout".into(), json!(stdout.clone()));
        data.insert("stderr".into(), json!(stderr.clone()));
        data.insert("exit_code".into(), json!(effective_exit));
        data.insert("command".into(), json!(effective_command));
        if cmd_rewritten {
            data.insert("command_asked".into(), json!(command));
        }
        if stdout_truncated {
            data.insert("spilled".into(), json!(true));
        }
        if stderr_truncated {
            data.insert("stderr_spilled".into(), json!(true));
        }
        // 路径类字段：仅在存在时插入（旧实现恒带 null 占位，794 次里绝大多数是 null）。
        if let Some(p) = spill_path {
            data.insert("spill_path".into(), json!(p));
        }
        if let Some(p) = stderr_spill_path {
            data.insert("stderr_spill_path".into(), json!(p));
        }

        let result_json = json!({
            "ok": effective_exit == 0,
            "kind": "run_result",
            "data": Value::Object(data),
            "warnings": if !src_guard_warnings.is_empty() {
                // 护栏最高优先：它意味着"这次命令改坏了源码、已被回滚"，比编码/空输出更该先看到
                src_guard_warnings.clone()
            } else if cjk_findstr_suspect {
                vec!["⚠️ 中文锚点 + findstr 零命中：findstr 对中文（GBK/UTF-8）编码不稳，零命中很可能是编码静默漏而非不存在。复核命令：run powershell -Command \"Get-ChildItem -Recurse <目录> -Include *.rs | Select-String -Pattern '同一中文'\" ——复核也零命中后才可下'不存在'结论（实测：中文锚点 3 轮 findstr 全空，换 python 即命中）".to_string()]
            } else if findstr_no_match {
                vec!["findstr/Select-String 无匹配（退出码 1 属正常，表示没找到该关键词）".to_string()]
            } else if idempotent_cleanup {
                vec!["删除/终止类命令目标不存在（幂等达成：目标已不在，exit 1 是 cmd 语义，非失败）。stderr 原文保留供核对".to_string()]
            } else if slash_normalized {
                vec![format!(
                    "命令里的 Windows 路径斜杠已按执行壳归一（{}）——原命令见 command_asked。\
                     原因：Git Bash 把未加引号的反斜杠当转义符（`D:\\proj\\src` 进 bash 会变成 `D:Realsrc`），\
                     故走 bash 一律转正斜杠；cmd/PowerShell 相反、只认反斜杠。",
                    if use_unix_shell {
                        "Git Bash：反斜杠 → 正斜杠"
                    } else {
                        "cmd/PowerShell：正斜杠 → 反斜杠"
                    }
                )]
            } else if stdout.is_empty() && stderr.is_empty() {
                vec!["命令成功但无输出（EMPTY_OUTPUT）".to_string()]
            } else { Vec::<String>::new() },
            "error": if effective_exit == 0 {
                Value::Null
            } else {
                // 退出码是**可信的结构化信号**；首个非空输出行**原样透传、不做任何解读**。
                let sig = stdout
                    .lines()
                    .chain(stderr.lines())
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("（无输出）");
                let sig: String = sig.chars().take(160).collect();
                json!({"code": "RUN_FAILED", "message": format!("命令退出码 {exit_code}（非零）：{sig}")})
            }
        });

        if effective_exit == 0 {
            Ok(result_json.to_string())
        } else {
            let mut suggestion = format!("【黄·结果待判断，非工具故障】命令退出码 {exit_code}（非零）。先核对返回结果里的 `cwd` 字段确认真实工作目录；检查路径/权限/参数后可 adjust 重试，同一原因连续失败可 skip 或换工具（swap）。");
            let lower = format!("{} {}", stderr.to_lowercase(), stdout.to_lowercase());
            if lower.contains("already exists") || lower.contains("destination path") {
                suggestion.push_str(" 检测到目标已存在（如 git clone 目录已存在且非空）：请勿同目录重试——adjust 改用全新绝对路径（如 D:/proj/_tmp/xxx），或 insert 一个 run 步骤先清理残留目录（rm -rf）再重试。");
            }
            if lower.contains("not a git repository")
                || (lower.contains("fatal:") && lower.contains("path"))
            {
                suggestion.push_str(" 疑似在错误 cwd 执行：先 insert run(`pwd`) 核对真实工作目录，再 adjust 到正确绝对路径。");
            }
            // 锁文件类失败归类——给出确定性提示，模型不再猜"残留进程"去追（证据推翻仍不罢休）。
            if let Some(hint) = locked_file_hint(&lower) {
                suggestion.push_str(&format!(" {}", hint));
            }
            let mut v = result_json;
            v["error"]["suggestion"] = json!(suggestion);
            Ok(v.to_string())
        }
    }
}

/// 模型不确定性。**不自动改代码**（自动修复可能引入新失败模式），检测 + 给精确

#[cfg(windows)]
#[tokio::test]
async fn ls_alias_maps_to_dir() {
    let tool = RunTool;
    // 工作目录用编译期注入的 crate 根 —— 硬编码盘符路径只保证作者机器能过
    let ws = env!("CARGO_MANIFEST_DIR");
    let out = tool
        .run(json!({
            "command": format!("ls {ws}"),
            "cwd": ws,
        }))
        .await
        .expect("ls 必须映射 dir 执行成功");
    let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
    assert_eq!(v["data"]["exit_code"], 0, "ls 映射 dir 应成功: {out}");
    let stdout = v["data"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("Cargo.toml"),
        "dir 输出应含 Cargo.toml: {stdout}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn cd_drive_prefixed_and_chain_preserved() {
    let tool = RunTool;
    // 只验证 cd 不报错且能跑（cd 到 crate 根后跑 python 打印 cwd）
    let ws = env!("CARGO_MANIFEST_DIR");
    let out = tool
        .run(json!({
            "command": format!("cd /d {ws} && python -c \"import os; print(os.getcwd())\""),
            "cwd": ws,
        }))
        .await
        .expect("cd /d + && 链应执行成功");
    let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
    let stdout = v["data"]["stdout"].as_str().unwrap_or("");
    let back = ws.replace('/', "\\");
    assert!(
        stdout.contains(ws) || stdout.contains(&back),
        "应在 crate 根执行，实际: {stdout}"
    );
}

#[cfg(test)]
#[path = "cmd_tools_tests.rs"]
mod cmd_tools_tests;
