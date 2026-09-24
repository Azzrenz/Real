//! 统一路径管线：清洗 → 归一 → 分类 → 注入消毒；WorkspacePath 守卫 + PathPolicy 写保护

pub mod data_root;

use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use crate::tools::contract::ToolError;
use serde_json::Value;

/// 会话级写授权记忆（确认一遍即放行）
static GRANTED_WRITES: LazyLock<Mutex<HashMap<String, HashSet<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 登记授权：用户确认通过后调用（确认门/前端批准后由后端签发）
pub fn grant_write_scope(session_id: &str, path: &str) {
    let dir = parent_dir_key(path);
    if dir.is_empty() {
        return;
    }
    if let Ok(mut g) = GRANTED_WRITES.lock() {
        g.entry(session_id.to_string()).or_default().insert(dir);
    }
}

/// 查询：该路径所在目录本会话是否已获授权
pub fn write_granted(session_id: &str, path: &str) -> bool {
    let dir = parent_dir_key(path);
    if dir.is_empty() {
        return false;
    }
    if let Ok(g) = GRANTED_WRITES.lock() {
        return g.get(session_id).map(|s| s.contains(&dir)).unwrap_or(false);
    }
    false
}

/// 目录键（规范化：正斜杠 + 小写 + 去尾斜杠）——授权按**目录**记，不按单文件
fn parent_dir_key(path: &str) -> String {
    let norm = path.replace('\\', "/").to_lowercase();
    let trimmed = norm.trim_end_matches('/').to_string();
    match trimmed.rfind('/') {
        // 盘符根（"c:"）没有父目录 → 以自身为授权键（否则根盘写入永远记不住授权）
        Some(i) if i > 0 => trimmed[..i].to_string(),
        _ => trimmed,
    }
}

/// 清空某会话的授权记忆（会话删除/重置时调用）
pub fn clear_write_grants(session_id: &str) {
    if let Ok(mut g) = GRANTED_WRITES.lock() {
        g.remove(session_id);
    }
}

/// 路径字符串清洗（原 path_guard::normalize_input_path）
pub fn normalize(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // 去首尾引号/括号（按 char 处理，中文安全）
    let trim_chars: &[char] = &['"', '\'', '(', ')', '（', '）', '[', ']', '`'];
    s = s
        .trim_matches(|c: char| trim_chars.contains(&c))
        .to_string();
    // 去尾部中文/英文标点（逗号、句号、分号、引号、空白）
    let re = regex::Regex::new(r#"[，。；、,.;'"\s]+$"#).unwrap();
    s = re.replace(&s, "").to_string();
    // 统一为反斜杠（展示层习惯；内部比较用 to_internal 正斜杠）
    s = s.replace('/', "\\");
    // 清理 `\.\` 段（如 D:\proj\.\README.md → D:\proj\README.md）
    while s.contains("\\.\\") {
        s = s.replace("\\.\\", "\\");
    }
    // 折叠连续反斜杠（D:/a//b 或 D:\a\\b → D:\a\b）
    while s.contains("\\\\") {
        s = s.replace("\\\\", "\\");
    }
    // 去尾部 `\.` 与尾随反斜杠（盘符根 D:\ 除外）
    if s.ends_with("\\.") {
        s.truncate(s.len() - 2);
    }
    if s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

/// 敏感文件匹配：按文件名/扩展名拒绝（防 API Key / 凭据泄露，原 path_guard::is_sensitive）
pub fn is_sensitive(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let low = name.to_lowercase();
    if low == ".env" || low.starts_with("real.db") {
        return true;
    }
    if let Some(ext) = path
        .extension()
        .map(|e| e.to_string_lossy().to_string().to_lowercase())
    {
        if matches!(
            ext.as_str(),
            "pem" | "key" | "secret" | "p12" | "pfx" | "token"
        ) {
            return true;
        }
    }
    false
}

/// workspace 归一化（原 memory::normalize_workspace）
pub fn normalize_workspace(raw: &str) -> String {
    if raw.is_empty() {
        return raw.to_string();
    }
    let p = Path::new(raw);
    // 去掉尾随的文件名（dev.bat → 目录）
    let dir = if p.extension().is_some() {
        p.parent().unwrap_or(p)
    } else {
        p
    };
    let s = dir.display().to_string();
    // 常见项目内深度子目录 → 上溯到项目根（marker 可配置）
    for marker in workspace_markers() {
        if let Some(idx) = s.find(&marker) {
            return s[..idx].trim_end_matches(['\\', '/']).to_string();
        }
    }
    s.trim_end_matches(['\\', '/']).to_string()
}

/// workspace 上溯 marker 列表（env REAL_WORKSPACE_MARKERS 逗号分隔可扩展，默认内置常见项）
fn workspace_markers() -> Vec<String> {
    let defaults = vec![
        "\\src-tauri\\src\\",
        "\\src-tauri\\",
        "\\src\\agent",
        "\\src\\",
        "\\node_modules\\",
        "\\target\\",
    ];
    match std::env::var("REAL_WORKSPACE_MARKERS") {
        Ok(v) if !v.trim().is_empty() => {
            let mut list: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            // 自定义不覆盖内置（约定：内置 + 扩展）
            for d in defaults {
                let ds = d.to_string();
                if !list.contains(&ds) {
                    list.push(ds);
                }
            }
            list
        }
        _ => defaults.into_iter().map(|s| s.to_string()).collect(),
    }
}

// WorkspacePath 路径守卫

/// 经过校验的绝对路径（项目白名单内）
#[derive(Debug, Clone)]
pub struct WorkspacePath(PathBuf);

/// 路径字符串规范化（原 path_guard::normalize_input_path，薄封装）
pub fn normalize_input_path(raw: &str) -> String {
    normalize(raw)
}

impl WorkspacePath {
    /// 构造：绝对路径 → 校验；"." / "./" / 空 → 项目根本身。
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let raw = normalize_input_path(&path.into().to_string_lossy());
        let pb = if raw == "." || raw == "./" || raw.is_empty() {
            crate::tools::project_root()
        } else if raw == "\\" || raw == "/" {
            return Err(ToolError::domain(
                "INVALID_PATH",
                "路径不合法（仅分隔符）",
                None,
            ));
        } else {
            let p = PathBuf::from(&raw);
            if p.is_relative() {
                return Err(ToolError::domain(
                    "RELATIVE_PATH",
                    format!(
                        "路径必须是绝对路径（含盘符，如 D:\\proj\\file）：{}",
                        p.display()
                    ),
                    Some("请提供完整绝对路径，或用 #En 引用前序工具结果（后端自动提取绝对路径）"),
                ));
            }
            p
        };
        validate(&pb)?;
        Ok(Self(pb))
    }

    /// 只读访问底层路径
    pub fn into_inner(self) -> PathBuf {
        self.0
    }
}

/// 核心校验：非空、绝对路径（策略：用户给出的路径均可访问，不做工作区白名单限制）
fn validate(pb: &Path) -> Result<(), ToolError> {
    if pb.as_os_str().is_empty() {
        return Err(ToolError::domain("INVALID_PATH", "路径为空", None));
    }
    if !pb.is_absolute() {
        return Err(ToolError::domain(
            "INVALID_PATH",
            format!("路径不是绝对路径: {}", pb.display()),
            None,
        ));
    }
    Ok(())
}

impl std::fmt::Display for WorkspacePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl AsRef<Path> for WorkspacePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

// PathPolicy 路径准入策略

/// 路径准入检查结果
#[derive(Debug, Clone)]
pub enum PathVerdict {
    /// 允许操作（普通路径，用户指令=圣旨，直接执行）
    Allow,
    /// 需要用户确认，附原因
    RequireConfirm(String),
}

pub struct PathPolicy {
    /// Real 自身项目根（如 D:\proj\），小写
    real_root: String,
    /// 系统敏感区（前缀, 说明）
    sensitive: Vec<(String, &'static str)>,
}

impl PathPolicy {
    pub fn new() -> Self {
        let real_root = Self::detect_root().to_lowercase();
        let user = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Default".into());
        let user_lower = user.to_lowercase();
        Self {
            real_root,
            sensitive: vec![
                // ── 系统命脉（不可放行，务必确认）──
                ("c:\\windows\\".into(), "系统目录 (Windows)"),
                ("c:\\program files\\".into(), "应用目录 (Program Files)"),
                (
                    "c:\\program files (x86)\\".into(),
                    "应用目录 (Program Files x86)",
                ),
                (format!("{}\\.ssh\\", user_lower), "SSH 密钥目录"),
                // AppData 只保留**系统/全局相关**子目录为敏感
                (
                    format!("{}\\appdata\\local\\microsoft\\", user_lower),
                    "系统配置目录 (AppData\\Local\\Microsoft)",
                ),
                (
                    format!("{}\\appdata\\roaming\\microsoft\\", user_lower),
                    "系统配置目录 (AppData\\Roaming\\Microsoft)",
                ),
            ],
        }
    }

    /// 检测 Real 自身根目录（从 exe 路径向上找 Cargo.toml / package.json）
    fn detect_root() -> String {
        if let Ok(exe) = std::env::current_exe() {
            let mut p = exe.parent();
            while let Some(dir) = p {
                if dir.join("Cargo.toml").exists() || dir.join("package.json").exists() {
                    return dir.to_string_lossy().to_string();
                }
                p = dir.parent();
            }
        }
        std::env::current_dir()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|_| "d:\\proj".into())
    }

    /// 写操作（会话级授权版）
    pub fn check_write_scoped(&self, session_id: Option<&str>, path: &str) -> PathVerdict {
        // 已授权过的目录（用户本会话已确认过）→ 直接放行，不再反复弹
        if let Some(sid) = session_id {
            if write_granted(sid, path) {
                return PathVerdict::Allow;
            }
        }
        self.check_write(path)
    }

    /// 写操作：盘符根 / 自身目录 / 系统敏感区 → 确认，其余放行
    pub fn check_write(&self, path: &str) -> PathVerdict {
        let p = Path::new(path);
        let path_str = p.to_string_lossy().to_lowercase();
        let in_temp = path_str.contains("\\temp\\") || path_str.contains("\\tmp\\");

        // 临时目录放行（\temp\ / \tmp\ 是正常中间产物，pytest 等子进程写临时文件）——
        if in_temp && !path_str.starts_with("c:\\windows\\") {
            return PathVerdict::Allow;
        }

        if self.is_drive_root(&path_str) {
            return PathVerdict::RequireConfirm(format!(
                "⚠️ {} 是盘符根目录，确定要写入吗？",
                path
            ));
        }
        if path_str.starts_with(&self.real_root) {
            return PathVerdict::RequireConfirm(format!(
                "⚠️ 目标路径在 Real 自身目录内: {}\n确定要修改 Real 的文件吗？",
                path
            ));
        }
        for (prefix, label) in &self.sensitive {
            if path_str.starts_with(prefix) {
                return PathVerdict::RequireConfirm(format!(
                    "⚠️ 目标路径为 {}: {}\n确定要写入吗？",
                    label, path
                ));
            }
        }
        // verifier.py / run_verify.py 是验证门禁脚本——模型改写它可让"自我验证"假通过
        let fname = p
            .file_name()
            .map(|f| f.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if fname == "verifier.py" || fname == "run_verify.py" {
            return PathVerdict::RequireConfirm(format!(
                "⚠️ {} 是验证门禁脚本（Verification Gate 执行它判定任务成败），模型不得擅自修改。\n确认要写入吗？",
                path
            ));
        }
        PathVerdict::Allow
    }

    fn is_drive_root(&self, p: &str) -> bool {
        let trimmed = p.trim_end_matches('\\').trim_end_matches('/');
        trimmed.len() == 2 && trimmed.chars().nth(1) == Some(':')
    }
}

/// 从用户输入提取路径锚定（Windows 盘符 `X:/` 与 `X:\` 均支持；容忍空格，
pub fn extract_anchor_path(input: &str) -> Option<String> {
    // 收集**所有**盘符候选，不取第一个——正文里的示例路径
    let candidates = extract_anchor_paths(input);
    if candidates.is_empty() {
        return None;
    }
    // ① 真实存在的目录（优先）
    if let Some(p) = candidates.iter().find(|c| std::path::Path::new(c).is_dir()) {
        return Some(p.clone());
    }
    // ② 真实存在的文件
    if let Some(p) = candidates
        .iter()
        .find(|c| std::path::Path::new(c).is_file())
    {
        return Some(p.clone());
    }
    // ③ 都不存在 → 取最后一个（任务指令通常在消息末尾）
    candidates.last().cloned()
}

/// 提取输入中的**全部**盘符路径候选（单路径返回 1 项；"对比 A 和 B"返回 2 项）——
pub fn extract_anchor_paths(input: &str) -> Vec<String> {
    let Some(re) = regex::Regex::new(r#"[A-Za-z]:[\\/][^"'\n\x00-\x1f]+"#).ok() else {
        return Vec::new();
    };
    let mut candidates: Vec<String> = Vec::new();
    for m in re.find_iter(input) {
        let raw = m.as_str();
        // 原始匹配可能吞入后续路径（贪婪），按"下一个盘符（X:，X 为字母）"切分——
        let bytes = raw.as_bytes();
        let mut start = 0usize;
        let mut i = 1usize;
        while i + 1 < bytes.len() {
            if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' {
                if start < i {
                    candidates.push(cut_anchor(&raw[start..i]));
                }
                start = i;
                i += 2;
                continue;
            }
            i += 1;
        }
        candidates.push(cut_anchor(&raw[start..]));
    }
    candidates.retain(|s| !s.is_empty());
    // URL/协议串排除（实报脏值：用户消息里的 `https://…` 被当成盘符候选 →
    candidates.retain(|s| !s.contains("://"));
    // 去重保序（同一路径出现多次只留一次）
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|s| seen.insert(normalize(s)));
    candidates
}

/// 截断锚定候选：遇 CJK/引号/行尾停，trim 尾随分隔符
fn cut_anchor(s: &str) -> String {
    let cut = s
        .find(|c: char| ('\u{4e00}'..='\u{9fff}').contains(&c))
        .unwrap_or(s.len());
    s[..cut]
        .trim()
        .trim_end_matches(['\\', '/'])
        .trim_end()
        .to_string()
}

/// 检测"意图描述占位符"：模型把计划阶段描述（"待定位的解压源码文件"、"<源文件>"、
pub fn looks_like_placeholder(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // ① 占位意图词（描述性，非真实内容）
    const WORDS: &[&str] = &[
        "待定位",
        "待补充",
        "待确认",
        "待查",
        "待定",
        "待填",
        "目标文件",
        "等价验证",
        "或等价",
        "对应文件",
        "相关文件",
        "相应文件",
        "占位",
        "待写入",
        "待修改",
    ];
    if WORDS.iter().any(|w| s.contains(w)) {
        return true;
    }
    // ② 尖括号占位：真实路径不会含 `<...>`
    if s.contains('<') && s.contains('>') {
        return true;
    }
    // ③ 中文描述文本且不像路径（无盘符、无分隔符、无扩展名）
    let cjk = s
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    if cjk >= 2 {
        let has_sep = s.contains('/') || s.contains('\\');
        let has_drive = s.as_bytes().get(1) == Some(&b':');
        let has_ext = s
            .rsplit('.')
            .next()
            .map(|e| e.len() <= 6 && !e.is_empty())
            .unwrap_or(false)
            && s.contains('.');
        if !has_sep && !has_drive && !(has_ext && !s.ends_with('.')) {
            return true;
        }
    }
    false
}

/// 把 shell 命令里的"字面量"挖掉，只留结构骨架，供占位判定使用。
fn shell_skeleton(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            // 单/双引号内的内容整段丢弃（含 `awk '{print $5}'` 这类脚本字面量）
            '\'' | '"' => {
                let q = c;
                while let Some(n) = it.next() {
                    if q == '"' && n == '\\' {
                        it.next();
                        continue;
                    }
                    if n == q {
                        break;
                    }
                }
            }
            // 词首 `#` 注释丢到行尾；`echo "#E6"` 那种在引号里，上一步已吃掉。
            '#' => {
                let is_comment = {
                    let head = out.trim_end();
                    head.is_empty()
                        || head.ends_with([';', '|', '&', '('])
                        || out.ends_with(char::is_whitespace)
                };
                if is_comment {
                    for n in it.by_ref() {
                        if n == '\n' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// 命令字段的占位符判定
pub fn looks_like_placeholder_cmd(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    const WORDS: &[&str] = &[
        "待定位",
        "待补充",
        "待确认",
        "待查",
        "待定",
        "待填",
        "目标文件",
        "等价验证",
        "或等价",
        "对应文件",
        "相关文件",
        "相应文件",
        "占位",
        "待写入",
        "待修改",
    ];
    // ① 词表：只在**骨架**上匹配（引号内字面量与注释已挖掉）。
    let skel = shell_skeleton(s);
    if WORDS.iter().any(|w| skel.contains(w)) {
        // ③ 结构豁免：骨架里有真正的**复合命令结构**（管道/串联/重定向/变量引用）
        return !skel.contains(['|', ';', '$', '`']);
    }
    // ② 尖括号占位：<...> 内**含中文**才判占位（如 <目标命令>、<待写入的路径>）。
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        match rest[lt + 1..].find('>') {
            Some(gt) => {
                let inner = &rest[lt + 1..lt + 1 + gt];
                if inner.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
                    return true;
                }
                rest = &rest[lt + 1 + gt + 1..];
            }
            None => break,
        }
    }
    false
}

/// 路径纠正（后端静默纠正，不拦截让模型改）。
pub fn correct_path(target: &str, candidates: &[String]) -> Option<(String, String)> {
    let target_key = drift_key(target);
    let base = target.rsplit(['/', '\\']).next()?.to_lowercase();
    let base_key = drift_key(&base);
    let target_dir_key = dir_tail_key(target);
    let mut full_hit: Option<&String> = None;
    let mut base_hit: Option<&String> = None;
    for cand in candidates {
        if cand == target || !std::path::Path::new(cand).exists() {
            continue;
        }
        if full_hit.is_none() && drift_key(cand) == target_key {
            full_hit = Some(cand);
        }
        let cb = cand.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
        if base_hit.is_none()
            && drift_key(&cb) == base_key
            && drift_key(cand) != target_key
            && dir_tail_key(cand) == target_dir_key
        {
            base_hit = Some(cand);
        }
    }
    full_hit
        .or(base_hit)
        .map(|c| (c.clone(), target.to_string()))
}

/// 路径的父目录（无分隔符则空串）。
fn parent_dir(p: &str) -> &str {
    match p.rfind(['/', '\\']) {
        Some(i) => &p[..i],
        None => "",
    }
}

/// 父目录**末段**的漂移键 —— "目录可解释"的唯一机械判据。
fn dir_tail_key(p: &str) -> String {
    let d = parent_dir(p);
    let tail = d.rsplit(['/', '\\']).next().unwrap_or("");
    drift_key(tail)
}

/// 漂移归一键：小写 + 连字符/下划线统一。LLM 重生成路径的主要漂移形态是
fn drift_key(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c == '_' || c == '-' { '-' } else { c })
        .collect()
}

/// 工具参数路径静默纠正（归 path 部门）
pub fn path_exists_cached(p: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static CACHE: OnceLock<Mutex<HashMap<String, (bool, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = p.to_string();
    if let Ok(map) = cache.lock() {
        if let Some((v, t)) = map.get(&key) {
            if t.elapsed() < Duration::from_secs(30) {
                return *v;
            }
        }
    }
    let v = std::path::Path::new(p).exists();
    if let Ok(mut map) = cache.lock() {
        if map.len() > 512 {
            map.clear();
        }
        map.insert(key, (v, Instant::now()));
    }
    v
}

pub fn structured_paths_from_args(v: &serde_json::Value) -> Vec<String> {
    fn push_path(out: &mut Vec<String>, s: &str) {
        let t = s.trim();
        if t.len() < 4 {
            return;
        }
        let b = t.as_bytes();
        let is_abs = b.len() > 2
            && (b[0] as char).is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'/' || b[2] == b'\\');
        if is_abs && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    let mut out: Vec<String> = Vec::new();
    for key in ["cwd", "file", "path", "target"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            push_path(&mut out, s);
        }
    }
    if let Some(arr) = v.get("paths").and_then(|x| x.as_array()) {
        for s in arr.iter().filter_map(|x| x.as_str()) {
            push_path(&mut out, s);
        }
    }
    out
}

pub fn live_paths_block(paths: &[String], limit: usize) -> String {
    format_live_block(crate::facts::filter_for_injection(paths), limit)
}

/// 测试专用入口：允许注入判定闭包（确定性测试不依赖真实文件系统）
#[cfg(test)]
pub fn live_paths_block_with(
    paths: &[String],
    limit: usize,
    exists: &dyn Fn(&str) -> bool,
) -> String {
    let kept: Vec<String> = paths.iter().filter(|p| exists(p)).cloned().collect();
    format_live_block(kept, limit)
}

fn format_live_block(items: Vec<String>, limit: usize) -> String {
    let mut live: Vec<String> = Vec::new();
    for p in items {
        if !live.iter().any(|x| *x == p) {
            live.push(p);
        }
        if live.len() >= limit {
            break;
        }
    }
    if live.is_empty() {
        return String::new();
    }
    let mut out = String::from("【当前路径事实·近几轮】以下为最近轮次仍在使用的真实路径（其它旧路径已过期，不再列出）：\n");
    for p in &live {
        out.push_str(&format!("- {p}\n"));
    }
    out
}

pub fn correct_invalid_cwd(args: &serde_json::Value, session_ws: Option<&str>) -> Option<(String, String)> {
    let raw = args.get("cwd").and_then(|v| v.as_str())?.trim();
    if raw.is_empty() {
        return None;
    }
    match crate::facts::liveness(raw) {
        crate::facts::Liveness::Stale => {}
        _ => return None,
    }
    let used = session_ws
        .map(|w| w.trim())
        .filter(|w| !w.is_empty() && std::path::Path::new(w).is_dir())
        .map(|w| w.to_string())
        .unwrap_or_else(|| crate::tools::project_root().display().to_string());
    Some((raw.to_string(), used))
}

pub fn silent_correct_args(
    args: &mut Value,
    read_memo_files: &[String],
    changed_files: &[String],
    path_map: &[String],
    goal_paths: &[String],
) -> Option<(String, String)> {
    let fp_opt: Option<String> = args
        .get("file")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            args.get("paths")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    let fp = fp_opt?;
    if std::path::Path::new(&fp).exists() {
        return None;
    }
    let base = fp.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    let mut cands: Vec<String> = Vec::new();
    for k in read_memo_files.iter().chain(changed_files.iter()) {
        if *k != fp && k.rsplit(['/', '\\']).next().map(|x| x.to_lowercase()) == Some(base.clone()) {
            cands.push(k.clone());
        }
    }
    // 候选源补充：路径证据账 PathMap（list 已确认存在的目录与文件条目）——
    for k in path_map {
        if *k != fp && !cands.contains(k) {
            cands.push(k.clone());
        }
    }
    // 候选源补充：用户消息里声明的路径——首轮无成功轨迹时，读目标描述记错路径也能当场纠
    if cands.is_empty() {
        cands.extend(goal_paths.iter().cloned());
    }
    let (cand, _) = correct_path(&fp, &cands)?;
    // 直接改参数：file / path / paths[0] → 正确路径（分步借用避免冲突）
    if let Some(v) = args.get_mut("file") {
        *v = serde_json::json!(cand.clone());
    } else if let Some(v) = args.get_mut("path") {
        *v = serde_json::json!(cand.clone());
    } else if let Some(arr) = args.get_mut("paths").and_then(|v| v.as_array_mut()) {
        if let Some(first) = arr.first_mut() {
            *first = serde_json::json!(cand.clone());
        }
    }
    Some((cand.clone(), fp))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
