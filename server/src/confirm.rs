//! 确认门 ConfirmGate v2（重建）

use crate::path::{PathPolicy, PathVerdict};
use crate::sse;
use crate::state::AppState;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::sleep;

/// 确认请求没有任何超时：一直挂到用户答复（批准 / 拒绝）或会话被取消为止。

/// 一条挂起的确认请求（request_id → 槽位；用户响应由 chat.rs confirm 写入 approved）
pub struct PendingConfirm {
    pub session_id: String,
    pub approved: Option<bool>,
    /// 用户选定的那个值：select_workspace 是路径，ask 是选中项的 label（chat.rs 回传写入）
    pub selected: Option<String>,
    /// 用户自己补的一句话（ask）：选项之外的主张。可为空——空表示他只选了选项。
    pub note: Option<String>,
}

#[derive(Default)]
pub struct ConfirmGate {
    pending: Mutex<HashMap<String, PendingConfirm>>,
}

impl ConfirmGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// 用户响应回传（chat.rs confirm 调用）：写 approved + 选定值 + 补充说明。
    pub fn resolve(
        &self,
        request_id: &str,
        session_id: &str,
        approved: bool,
        selected: Option<String>,
        note: Option<String>,
    ) -> bool {
        let mut map = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(request_id) {
            Some(p) => {
                if p.session_id != session_id {
                    return false;
                }
                p.approved = Some(approved);
                p.selected = selected;
                p.note = note;
                true
            }
            None => false,
        }
    }
}

/// 一个可选项（只用于模型主动提问 `ask`；护栏类确认没有选项）。
#[derive(Debug, Clone)]
pub struct ConfirmOption {
    /// 选项短标题——界面上就是它，写清"做什么"，不写"方案一"这类空名。
    pub label: String,
    /// 选项说明：选它会怎样、代价是什么。**必填**——没说明的选项等于让用户猜。
    pub detail: String,
    /// 模型主张这一项。每问只允许一项。
    pub recommended: bool,
}

/// 一次确认请求的内容（前端弹窗展示用）
pub struct ConfirmSpec {
    pub action: String,
    pub target: String,
    pub impact: String,
    pub risk_label: String,
    /// 模型主动提问时的候选项（护栏类确认为空）。
    pub options: Vec<ConfirmOption>,
}

impl ConfirmSpec {
    /// 护栏类确认（删除 / 敏感写入 / 选工作区）：没有选项。
    pub fn guarded(action: &str, target: String, impact: String, risk_label: &str) -> Self {
        Self {
            action: action.into(),
            target,
            impact,
            risk_label: risk_label.into(),
            options: Vec::new(),
        }
    }
}

/// 判定一次工具调用是否需用户确认（执行前调用；不命中返回 None 直接放行）。
pub fn needs_confirm(session_id: &str, tool: &str, args: &Value) -> Option<ConfirmSpec> {
    match tool {
        "run" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if cmd.is_empty() {
                return None;
            }
            if let Some(target) = delete_command_target(cmd) {
                return Some(ConfirmSpec::guarded(
                    "delete",
                    target,
                    "删除操作不可恢复。确认这是模型当前任务需要的删除吗？".into(),
                    "danger",
                ));
            }
            let cwd = args
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let segs = split_segments(cmd);
            // 重定向写敏感区是更具体的分类，先于"不可静态分析"总闸判定
            for target in redirect_write_targets(&segs) {
                let Some(abs) = absolutize_target(&target, cwd) else {
                    continue;
                };
                if let PathVerdict::RequireConfirm(reason) =
                    PathPolicy::new().check_write_scoped(Some(session_id), &abs)
                {
                    return Some(ConfirmSpec::guarded("sensitive_write", abs, reason, "warn"));
                }
            }
            // 不能静态证明安全的构造 → 升级到危险命令类别，交用户裁决
            if let Some(reason) = unanalyzable_reason(cmd, &segs) {
                return Some(ConfirmSpec::guarded(
                    "system_command",
                    cmd.chars().take(120).collect(),
                    reason,
                    "danger",
                ));
            }
            // ③ cwd 无效（模型编造路径/工作区已迁移/未绑工作区）→ 弹窗让用户选真实工作区。
            let cwd = cwd.unwrap_or("");
            if !cwd.is_empty() && !cwd_is_usable(cwd) {
                return Some(ConfirmSpec::guarded(
                    "select_workspace",
                    cwd.to_string(),
                    format!(
                        "模型要执行命令，但工作区路径无效：{cwd}\n\
                         （它可能编造了路径，或会话工作区已失效）。\n\
                         请选择一个真实存在的目录作为本会话工作区，命令将用它重跑。"
                    ),
                    "info",
                ));
            }
            None
        }
        "write" | "edit" | "modify" => {
            let file = args
                .get("file")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("path").and_then(|v| v.as_str()))
                .unwrap_or("");
            if file.is_empty() {
                return None;
            }
            if let PathVerdict::RequireConfirm(reason) =
                PathPolicy::new().check_write_scoped(Some(session_id), file)
            {
                return Some(ConfirmSpec::guarded("sensitive_write", file.to_string(), reason, "warn"));
            }
            None
        }
        _ => None,
    }
}

/// cwd 是否可用（与 cmd_tools::resolve_workdir 同源判定）
fn cwd_is_usable(cwd: &str) -> bool {
    let p = std::path::Path::new(cwd);
    let raw = if p.is_absolute() {
        p.to_path_buf()
    } else {
        crate::tools::project_root().join(p)
    };
    raw.is_dir() || raw.is_file()
}

/// 按 shell 分隔符把命令切成"段"：引号（单/双/反引号）内部不切，`&&`/`||` 折叠为一个分隔符。
/// 元组首项为该段的前导分隔符（首段为 None），次项为 trim 后的段文本（空段丢弃）。
fn split_segments(cmd: &str) -> Vec<(Option<char>, String)> {
    let mut segs: Vec<(Option<char>, String)> = Vec::new();
    let mut cur = String::new();
    let mut pending: Option<char> = None;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' => {
                if quote == Some(c) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(c);
                }
                cur.push(c);
            }
            '&' | '|' | ';' | '\r' | '\n' if quote.is_none() => {
                // `&&` / `||` 折叠：吃掉紧随的同类字符
                if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                    chars.next();
                }
                let seg = cur.trim().to_string();
                if !seg.is_empty() {
                    segs.push((pending.take(), seg));
                }
                pending = Some(c);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let seg = cur.trim().to_string();
    if !seg.is_empty() {
        segs.push((pending.take(), seg));
    }
    segs
}

/// 段首词：小写、去引号、去 `.exe` 后缀。
fn seg_head(seg: &str) -> String {
    seg.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(['"', '\'', '`'])
        .trim_end_matches(".exe")
        .to_lowercase()
}

/// 段内 token：按空白切，逐项去引号并小写。
fn seg_tokens(seg: &str) -> Vec<String> {
    seg.split_whitespace()
        .map(|t| t.trim_matches(['"', '\'', '`']).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// 删除意图检测：逐段判定——只读豁免只在段内生效，段内再按 `&` 等分隔符取谓词词。
fn delete_command_target(cmd: &str) -> Option<String> {
    const DEL_PREDICATES: [&str; 8] = [
        "remove-item", "rm", "del", "erase", "rd", "rmdir", "unlink", "ri",
    ];
    // 段首词为搜索/列举/读取类时，该段后面的词是**被搜的内容**而非动作 → 该段只读
    const READ_HEADS: [&str; 16] = [
        "get-childitem", "dir", "ls", "grep", "findstr", "find", "search", "cat", "type",
        "head", "tail", "rg", "select-string", "more", "echo", "where",
    ];
    // 脚本内联代码里的删除 API —— 命令词判据认不出这一类
    const SCRIPT_DELETE: [&str; 12] = [
        "shutil.rmtree(", "os.remove(", "os.unlink(", "os.rmdir(",
        "rmsync(", ".rm(", "unlinksync(", ".unlink(", "rmdirsync(", ".rmdir(",
        "rimraf", ".remove(",
    ];
    let lower = cmd.to_lowercase();
    let mut has_writable_seg = false;
    for (_sep, seg) in split_segments(cmd) {
        if READ_HEADS.contains(&seg_head(&seg).as_str()) {
            continue;
        }
        has_writable_seg = true;
        for part in seg.split([' ', '\t', ';', '|', '&', '\r', '\n']) {
            // 剥离引号与尾随通配符/参数尾巴，取谓词
            let w = part
                .trim()
                .trim_matches(['"', '\'', '`'])
                .trim_end_matches(['*', '\\', '/'])
                .to_lowercase();
            if DEL_PREDICATES.contains(&w.as_str()) {
                return Some(cmd.chars().take(120).collect::<String>());
            }
        }
    }
    // 脚本内联删除 API 在整串上扫；全段只读时不扫，避免 `grep "os.remove("` 这类读取误伤
    if has_writable_seg && SCRIPT_DELETE.iter().any(|p| lower.contains(p)) {
        return Some(cmd.chars().take(120).collect::<String>());
    }
    None
}

/// 无法静态证明安全的构造 → 返回中文原因（用于弹窗 impact），交用户裁决。
fn unanalyzable_reason(cmd: &str, segs: &[(Option<char>, String)]) -> Option<String> {
    // 能内联执行脚本体的解释器（body 可编码/混淆，命令词判据看不穿）
    const INTERPRETERS: [&str; 18] = [
        "python", "python3", "py", "node", "nodejs", "deno", "bun", "perl", "ruby", "php",
        "powershell", "pwsh", "cmd", "bash", "sh", "wsl", "cscript", "wscript",
    ];
    // 解释器的"内联脚本体"开关
    const INLINE_FLAGS: [&str; 10] = [
        "-c", "/c", "-e", "-command", "-encodedcommand", "-enc", "-r", "eval", "-k", "/k",
    ];
    // 编码 / 间接执行特征（整串子串）
    const ENCODED: [&str; 5] = [
        "-encodedcommand", "frombase64string", "invoke-expression", "downloadstring",
        "invoke-webrequest",
    ];
    // ① 解释器内联代码：解释器 + 内联开关 + 后面确实跟着脚本体
    for (_sep, seg) in segs {
        let toks = seg_tokens(seg);
        if toks.first().map(|h| INTERPRETERS.contains(&h.as_str())) != Some(true) {
            continue;
        }
        for (i, t) in toks.iter().enumerate() {
            if INLINE_FLAGS.contains(&t.as_str()) && i + 1 < toks.len() {
                return Some("解释器内联代码".to_string());
            }
        }
    }
    // ② 编码或间接执行
    let lower = cmd.to_lowercase();
    if ENCODED.iter().any(|p| lower.contains(p)) {
        return Some("编码或间接执行".to_string());
    }
    if segs
        .iter()
        .any(|(_s, seg)| seg_tokens(seg).iter().any(|t| t == "iex"))
    {
        return Some("编码或间接执行".to_string());
    }
    // ③ 命令替换 / 子 shell
    if cmd.contains('`') || cmd.contains("$(") {
        return Some("命令替换或子 shell".to_string());
    }
    // ④ 变量展开间接执行：token 形如 `%名称%`（中间不含 `%` 与空白）
    for (_sep, seg) in segs {
        for t in seg_tokens(seg) {
            if let Some(inner) = t.strip_prefix('%').and_then(|r| r.strip_suffix('%')) {
                if !inner.is_empty()
                    && !inner.contains('%')
                    && !inner.chars().any(char::is_whitespace)
                {
                    return Some("变量展开间接执行".to_string());
                }
            }
        }
    }
    // ⑤ 管道进解释器：段由 `|` 引出、段首是解释器、且没给脚本文件（等于从 stdin 吃程序）
    for (sep, seg) in segs {
        if *sep != Some('|') {
            continue;
        }
        let toks = seg_tokens(seg);
        if let Some(head) = toks.first() {
            if INTERPRETERS.contains(&head.as_str()) && toks.len() <= 2 {
                return Some("管道进解释器".to_string());
            }
        }
    }
    None
}

/// 重定向写入目标：段内识别 `>` / `>>`（含 `a>b` 粘连写法）与 PowerShell 写文件 cmdlet。
fn redirect_write_targets(segs: &[(Option<char>, String)]) -> Vec<String> {
    const CMDLETS: [&str; 4] = ["out-file", "set-content", "add-content", "new-item"];
    let mut out: Vec<String> = Vec::new();
    for (_sep, seg) in segs {
        // `>` / `>>`：句柄重定向 `>&1` / `>&2` 直接跳过
        let chars: Vec<char> = seg.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '>' {
                if i + 1 < chars.len() && chars[i + 1] == '&' {
                    i += 2;
                    continue;
                }
                let mut j = i + 1;
                while j < chars.len() && chars[j] == '>' {
                    j += 1;
                }
                let rest: String = chars[j..].iter().collect();
                let target = rest.split_whitespace().next().unwrap_or("");
                if let Some(t) = clean_redirect_target(target) {
                    out.push(t);
                }
                i = j;
                continue;
            }
            i += 1;
        }
        // PowerShell 写文件 cmdlet：其后第一个非 flag token 是目标
        let toks: Vec<&str> = seg.split_whitespace().collect();
        for (idx, t) in toks.iter().enumerate() {
            let name = t.trim_matches(['"', '\'', '`']).to_lowercase();
            if CMDLETS.contains(&name.as_str()) {
                if let Some(next) = toks[idx + 1..].iter().find(|x| !x.starts_with('-')) {
                    if let Some(c) = clean_redirect_target(next) {
                        out.push(c);
                    }
                }
            }
        }
    }
    out
}

/// 过滤句柄/黑洞重定向目标（这些不是真实写入对象），其余返回清洗后的原始目标。
fn clean_redirect_target(raw: &str) -> Option<String> {
    let t = raw
        .trim()
        .trim_matches(['"', '\'', '`'])
        .trim_end_matches([';', ','])
        .trim();
    if t.is_empty() {
        return None;
    }
    let low = t.to_lowercase();
    // 句柄重定向 / 黑洞：nul、/dev/null、$null、&1、&2、1、2 不是写入目标
    const SKIP: [&str; 5] = ["nul", "/dev/null", "$null", "&1", "&2"];
    if SKIP.contains(&low.as_str()) || low == "1" || low == "2" {
        return None;
    }
    Some(t.to_string())
}

/// 把重定向目标解析为绝对路径：相对路径用有效 cwd，否则项目根；解析不出则返回 None（不判、不误伤）。
fn absolutize_target(target: &str, cwd: Option<&str>) -> Option<String> {
    let cleaned = crate::path::normalize(target);
    if cleaned.is_empty() {
        return None;
    }
    let abs = if std::path::Path::new(&cleaned).is_absolute() {
        cleaned
    } else {
        let base = match cwd {
            Some(c) if cwd_is_usable(c) => std::path::PathBuf::from(c),
            _ => crate::tools::project_root(),
        };
        crate::path::normalize(&base.join(&cleaned).to_string_lossy())
    };
    if !std::path::Path::new(&abs).is_absolute() {
        return None;
    }
    Some(abs)
}

/// 确认结果（request_confirmation 返回）：批准时按 action 携带不同 payload。
pub struct ConfirmResult {
    /// 用户是否批准。
    pub approved: bool,
    /// delete / sensitive_write 批准签发的一次性自修改令牌（注入 args 由 fs_write 消费）。
    pub token: Option<String>,
    /// 用户在弹窗里**选定的那个值**：select_workspace 是路径，ask 是选中的选项 label。
    pub selected: Option<String>,
    /// ask 专用：用户自己写的一句话（选项之外的主张）。空表示他没写。
    pub note: Option<String>,
}

/// 请求一次确认并等待用户响应（挂起当前工具执行）。
pub async fn request_confirmation(
    ctx: &AppState,
    session_id: &str,
    spec: &ConfirmSpec,
) -> ConfirmResult {
    // 确认请求 id：`cf-` + UUID（分隔符按统一规范用 `-`，见
    let request_id = format!("cf-{}", uuid::Uuid::new_v4().simple());
    // 批准即签发一次性 token（与 fs_write/fs_edit 消费端同源）；
    let token = if spec.action == "select_workspace" || spec.action == "ask" {
        None
    } else {
        Some(crate::tools::fs_write::issue_self_edit_token())
    };
    {
        let mut map = ctx.confirm_gate.pending.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(
            request_id.clone(),
            PendingConfirm {
                session_id: session_id.to_string(),
                approved: None,
                selected: None,
                note: None,
            },
        );
    }
    // select_workspace：payload 带候选工作区（default_workspace + 最近工作区，供前端点选）
    let mut payload = serde_json::json!({
        "request_id": request_id,
        "action": spec.action,
        "target": spec.target,
        "impact": spec.impact,
        "risk_label": spec.risk_label,
        // 模型主动提问的候选项：连同说明与推荐一起下发（护栏类确认为空数组）
        "options": spec.options.iter().map(|o| serde_json::json!({
            "label": o.label,
            "detail": o.detail,
            "recommended": o.recommended,
        })).collect::<Vec<_>>(),
    });
    if spec.action == "select_workspace" {
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(Some(dw)) = crate::db::repos::get_setting(&ctx.pool, "default_workspace").await {
            if !dw.trim().is_empty() && !candidates.contains(&dw) {
                candidates.push(dw.trim().to_string());
            }
        }
        if let Ok(rw) = crate::db::repos::get_recent_workspace(&ctx.pool).await {
            if !rw.trim().is_empty() && !candidates.contains(&rw) {
                candidates.push(rw.trim().to_string());
            }
        }
        if candidates.is_empty() {
            candidates.push(crate::tools::project_root().to_string_lossy().into_owned());
        }
        payload["workspace_candidates"] = serde_json::to_value(&candidates).unwrap_or_default();
    }
    let _ = sse::emit(ctx, session_id, "confirm.request", payload).await;

    // 轮询等待：用户回传（resolve 写 approved）/ 会话取消。**不设超时**。
    let outcome: Option<(bool, Option<String>, Option<String>)> = loop {
        if let Some(run) = ctx.cancels.lock().unwrap_or_else(|e| e.into_inner()).get(session_id) {
            if run.cancel.is_cancelled() {
                break None;
            }
        }
        let state = ctx
            .confirm_gate
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&request_id)
            .map(|p| (p.approved, p.selected.clone(), p.note.clone()));
        if let Some((Some(a), selected, note)) = state {
            break Some((a, selected, note));
        }
        if let Some((None, _, _)) = state {
            // 仍挂起，继续等
        }
        sleep(Duration::from_millis(300)).await;
    };

    // 收尾：清挂起槽。resolved 事件由 chat.rs confirm 统一发（响应瞬间即广播，轮询
    ctx.confirm_gate
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id);
    match outcome {
        Some((true, selected, note)) => {
            if spec.action == "select_workspace" {
                // 工作区选择：只取路径，不 grant 写授权、不签发 token
                ConfirmResult {
                    approved: true,
                    token: None,
                    selected: selected.filter(|p| !p.trim().is_empty()),
                    note: None,
                }
            } else if spec.action == "ask" {
                // 模型主动提问：选中的 label 与用户自己补的那句话都原样回传，
                ConfirmResult {
                    approved: true,
                    token: None,
                    selected: selected.filter(|s| !s.trim().is_empty()),
                    note: note.filter(|s| !s.trim().is_empty()),
                }
            } else {
                // 仅敏感写入按目标目录记住授权；删除是命令级判定，不按命令文本授予写授权。
                if spec.action == "sensitive_write" {
                    crate::path::grant_write_scope(session_id, &spec.target);
                }
                ConfirmResult {
                    approved: true,
                    token,
                    selected: None,
                    note: None,
                }
            }
        }
        Some((false, _, _)) => {
            ConfirmResult { approved: false, token: None, selected: None, note: None }
        }
        None => {
            // None 现在只有取消一个来源（不再有超时分支）。
            let _ = sse::emit(
                ctx,
                session_id,
                "confirm.cancelled",
                serde_json::json!({ "request_id": request_id }),
            )
            .await;
            ConfirmResult { approved: false, token: None, selected: None, note: None }
        }
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod confirm_tests;
