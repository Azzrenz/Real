//! 文件系统公共常量：SKIP_DIRS（基础/审计两集）+ SKIP_FILES + AUDIT_FULL_READ_THRESHOLD

/// **宽容读文本 —— 文件工具的唯一解码入口。**
pub fn decode_text(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), false),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(bytes);
            (cow.into_owned(), true)
        }
    }
}

/// 降级标注 —— 拼进工具返回值，让模型知道这份中文可能不准。
pub fn encoding_note() -> &'static str {
    "（注意：该文件不是 UTF-8，已按 GBK 解码 —— 中文可能有个别字失真，ASCII 部分可靠）"
}

/// 基础跳过目录（search/find_files/list 用）
#[allow(dead_code)]
pub const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", ".git"];

/// 基础跳过文件（search 递归扫描用——依赖/构建锁文件，扫描噪音）
#[allow(dead_code)]
pub const SKIP_FILES: &[&str] = &[
    "package-lock.json",
    "Cargo.lock",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "package-lock.json5",
];

/// 二进制探测深度（字节）—— **判据是内容，不是文件名**。
#[allow(dead_code)]
pub const BINARY_PROBE_BYTES: usize = 8 * 1024;

pub const SKIP_DIRS_AUDIT: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    "coverage",
    "out",
    ".cache",
    ".real",
    ".turbo",
    ".parcel-cache",
    ".eslintcache",
    ".pytest_cache",
    ".mypy_cache",
    ".gradle",
    ".cargo",
    ".rustup",
    "vendor",
    "third_party",
];

// 花括号结构护栏（**modify 与 run 共用同一套判据**）

/// 括号平衡检查（结构护栏）：轻量词法扫描（跳过字符串/字符/注释）
pub struct BraceScan {
    /// 全文扫描结束时的深度（0=平衡；>0 缺闭合；<0 多闭合）
    pub depth: i32,
    /// 首次出现"多余闭合"（深度归零后又遇到 `}`）的行号（1-based）
    pub first_negative_line: Option<usize>,
    /// 未闭合的开口行号（最早 3 个，供定位）
    pub unclosed_lines: Vec<usize>,
}

/// raw string 起点判定：`r"…"` / `r#"…"#` / `r##"…"##` / `br#"…"#`。
fn raw_string_start(i: usize, n: usize, bytes: &[u8]) -> Option<(usize, usize)> {
    let mut j = i;
    if bytes[j] == b'b' {
        j += 1;
        if j >= n || bytes[j] != b'r' {
            return None;
        }
    }
    if j >= n || bytes[j] != b'r' {
        return None;
    }
    j += 1;
    let mut hashes = 0usize;
    while j < n && bytes[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    if j < n && bytes[j] == b'"' {
        Some((j + 1, hashes))
    } else {
        None
    }
}

/// 括号扫描（误判根治）
pub fn brace_scan(content: &str) -> BraceScan {
    let bytes = content.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    let n = bytes.len();
    let mut line = 1usize;
    let mut first_negative_line: Option<usize> = None;
    let mut openers: Vec<usize> = Vec::new();

    let is_char_literal_start = |i: usize, n: usize, bytes: &[u8]| -> bool {
        // 'x' 形态：下一个字符后紧跟 '（含转义 '\x' 由下面按转义序列推进）
        if i + 2 < n {
            return bytes[i + 2] == b'\'' || bytes[i + 1] == b'\\';
        }
        false
    };

    while i < n {
        // raw string **优先于普通 `"` 分支**：`r##"{"objective":…}"##` 里的花括号是**内容**。
        if bytes[i] == b'r' || bytes[i] == b'b' {
            if let Some((start, hashes)) = raw_string_start(i, n, bytes) {
                let mut j = start;
                while j < n {
                    if bytes[j] == b'\n' {
                        line += 1;
                        j += 1;
                        continue;
                    }
                    if bytes[j] == b'"' {
                        // 收尾必须是 `"` + 同样数量的 `#`
                        let mut k = j + 1;
                        let mut h = 0usize;
                        while h < hashes && k < n && bytes[k] == b'#' {
                            h += 1;
                            k += 1;
                        }
                        if h == hashes {
                            j = k;
                            break;
                        }
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'"' => {
                i += 1;
                while i < n {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b'\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'\'' if is_char_literal_start(i, n, bytes) => {
                i += 1;
                while i < n {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'\'' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
            }
            b'{' => {
                depth += 1;
                openers.push(line);
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth < 0 && first_negative_line.is_none() {
                    first_negative_line = Some(line);
                }
                openers.pop();
                i += 1;
            }
            _ => i += 1,
        }
    }
    BraceScan {
        depth,
        first_negative_line,
        unclosed_lines: openers.into_iter().take(3).collect(),
    }
}

pub fn brace_balance_ok(content: &str) -> bool {
    let s = brace_scan(content);
    s.depth == 0 && s.first_negative_line.is_none()
}

/// 护栏是否该拦（防误伤）：只有"**这次改动把平衡改坏**"才拦——
pub fn brace_guard_should_block(before: &str, after: &str) -> bool {
    !brace_balance_ok(after) && brace_balance_ok(before)
}

/// 结构护栏判据：**这次执行把已存在的定义点成批删掉** → 返回消失清单（供诊断）；空 = 不拦。
pub fn structure_guard_should_block(before: &str, after: &str) -> Vec<String> {
    /// 净减几个定义点才拦。**取 3**：删 1–2 个可能是本意（单个废弃函数），
    const MIN_LOST: usize = 3;
    let names = |s: &str| -> std::collections::HashSet<String> {
        extract_structure(s)
            .into_iter()
            .map(|(_, kind, name)| format!("{kind} {name}"))
            .collect()
    };
    let (b, a) = (names(before), names(after));
    if b.is_empty() {
        return Vec::new();
    }
    let mut lost: Vec<String> = b.difference(&a).cloned().collect();
    if lost.len() < MIN_LOST {
        return Vec::new();
    }
    lost.sort();
    lost
}

/// 不平衡诊断文本（报错给第一手证据：差多少、第几行、什么内容）。
pub fn brace_diagnose(content: &str) -> String {
    let s = brace_scan(content);
    let lines: Vec<&str> = content.lines().collect();
    let snippet = |ln: usize| -> String {
        lines
            .get(ln.saturating_sub(1))
            .map(|l| l.trim().chars().take(60).collect::<String>())
            .unwrap_or_default()
    };
    let mut out = format!(
        "花括号净差 {}（{}）",
        s.depth.abs(),
        if s.depth > 0 {
            "缺闭合 }"
        } else if s.depth < 0 {
            "多出 }"
        } else {
            "闭合序错乱"
        }
    );
    if let Some(ln) = s.first_negative_line {
        out.push_str(&format!("；首个多余闭合在第 {ln} 行：`{}`", snippet(ln)));
    }
    if !s.unclosed_lines.is_empty() && s.depth > 0 {
        let lns: Vec<String> = s
            .unclosed_lines
            .iter()
            .map(|l| format!("第 {l} 行 `{}`", snippet(*l)))
            .collect();
        out.push_str(&format!("；未闭合开口最早在 {}", lns.join("、")));
    }
    out
}

// ── run 通道的源码护栏：执行前快照 / 执行后校验回滚 ──────────────────────────

/// 单条命令最多快照几个 `.rs`（防命令里出现几百个路径时全读一遍）
pub const RS_TARGET_CAP: usize = 20;

/// 从命令文本里捞出**工作区内已存在的 `.rs` 文件**并读入原文 → `(路径, 原文)`。
pub fn snapshot_rs_targets(command: &str, workdir: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    let Ok(wd) = workdir.canonicalize() else {
        return Vec::new();
    };
    let mut out: Vec<(std::path::PathBuf, String)> = Vec::new();
    // 命令里路径可能被引号/分隔符包裹 → 先按非路径字符切 token
    for raw in command.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '`' | ';' | '&' | '|' | '(' | ')' | ',' | '=' | '<' | '>' | '\n'
            )
    }) {
        let t = raw.trim_matches(|c: char| matches!(c, '"' | '\'' | '`'));
        if t.len() < 4 || !t.to_ascii_lowercase().ends_with(".rs") {
            continue;
        }
        let rel = t.replace('/', "\\");
        let cand = std::path::PathBuf::from(&rel);
        let p = if cand.is_absolute() { cand } else { wd.join(&rel) };
        let Ok(canon) = p.canonicalize() else { continue };
        if !canon.is_file() || !canon.starts_with(&wd) {
            continue;
        }
        if out.iter().any(|(q, _)| q == &canon) {
            continue;
        }
        if let Ok(txt) = std::fs::read_to_string(&canon) {
            out.push((canon, txt));
            if out.len() >= RS_TARGET_CAP {
                break;
            }
        }
    }
    out
}

/// 执行后校验：花括号**被这次执行改坏**的 → 按快照回滚原文，返回给模型的 warning（无则空）。
pub fn guard_rs_targets(guards: &[(std::path::PathBuf, String)]) -> Vec<String> {
    let mut warns = Vec::new();
    for (path, before) in guards {
        let Ok(after) = std::fs::read_to_string(path) else {
            continue;
        };
        // 判据一：花括号被这次执行改坏（语法层面确定错）
        if brace_guard_should_block(before, &after) {
            let diag = brace_diagnose(&after);
            let restored = std::fs::write(path, before).is_ok();
            warns.push(format!(
                "⛔ 源码结构护栏（run 通道）：{} 的花括号被这次命令改坏（{}）。{}",
                path.display(),
                diag,
                if restored {
                    "**已按执行前原文回滚**，文件恢复原状。这类改动请改用 modify 工具（同一套护栏 + 读回校验 + 编辑ID）。"
                } else {
                    "★ 回滚失败——请立刻用 git status / git diff 检查该文件！"
                }
            ));
            continue;
        }
        // 判据二：括号没坏，但**定义点成批消失**（位置删对了却删多了）——括号判据看不见这类
        let lost = structure_guard_should_block(before, &after);
        if !lost.is_empty() {
            let shown: Vec<String> = lost.iter().take(8).cloned().collect();
            let more = if lost.len() > 8 { "（仅列前 8 个）" } else { "" };
            let restored = std::fs::write(path, before).is_ok();
            warns.push(format!(
                "⛔ 源码结构护栏（run 通道）：{} 的定义点被这次命令删掉 {} 个（{}{}）。{}",
                path.display(),
                lost.len(),
                shown.join("、"),
                more,
                if restored {
                    "**已按执行前原文回滚**。若确实要删这些定义，请改用 modify 工具（有改前备份 + 读回校验）或分步删除。"
                } else {
                    "★ 回滚失败——请立刻用 git status / git diff 检查该文件！"
                }
            ));
        }
    }
    warns
}

// ── 结构提取（共享件：read 的结构摘要 / modify 的定位 / run 的护栏都用它）──

/// 按语言正则匹配；只取**行首**定义（缩进 ≤4 的顶层），避免方法/嵌套函数噪音爆炸。
pub(crate) fn extract_structure(content: &str) -> Vec<(usize, String, String)> {
    let mut out: Vec<(usize, String, String)> = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let ln = i + 1;
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if t.is_empty()
            || t.starts_with("//")
            || t.starts_with('#')
            || t.starts_with("/*")
            || t.starts_with('*')
        {
            continue;
        }
        // 顶层定义：缩进 ≤4（顶层 fn/class/struct/enum 通常顶格或 4 空格缩进内部）
        if indent > 4 {
            continue;
        }
        // 统一小写用于匹配关键字（名称保留原样）
        let lower = t.to_ascii_lowercase();
        let is_impl = lower.starts_with("impl ");
        let is_def = lower.starts_with("def ") || lower.starts_with("async def ");
        let is_class = lower.starts_with("class ")
            || lower.starts_with("trait ")
            || lower.starts_with("struct ")
            || lower.starts_with("enum ");
        let is_fn = lower.starts_with("fn ")
            || lower.starts_with("pub fn ")
            || lower.starts_with("function ")
            || lower.starts_with("func ");
        let is_method = lower.starts_with("public ")
            || lower.starts_with("private ")
            || lower.starts_with("protected ")
            || lower.starts_with("pub async fn ")
            || lower.starts_with("async fn ");
        let kind_name: Option<(&str, String)> = if is_impl {
            // impl X { 或 impl Trait for X {
            let body = t.trim_start_matches("impl").trim();
            let name = body
                .split_whitespace()
                .next()
                .map(|s| s.trim_end_matches('{').to_string());
            name.map(|n| ("impl", n))
        } else if is_def || is_class {
            let kw = if is_class {
                ["class", "trait", "struct", "enum"]
                    .iter()
                    .find(|k| lower.starts_with(**k))
                    .copied()
                    .unwrap_or("class")
            } else {
                "def"
            };
            // trim_end_matches 只能剥结尾字符，"main():" 结尾是 ')' 剥不掉 → 漏提取
            let body = t
                .split_whitespace()
                .skip_while(|w| *w != kw)
                .skip(1)
                .next()
                .map(|s| {
                    s.split('(')
                        .next()
                        .unwrap_or("")
                        .trim_end_matches(['{', ':'])
                        .to_string()
                });
            body.map(|n| (if is_class { "class" } else { "def" }, n))
        } else if is_fn {
            // 跳过 pub/async 修饰符，函数名在 '(' 处截断
            let body = t
                .split_whitespace()
                .filter(|w| !matches!(*w, "pub" | "async" | "fn" | "func" | "function" | "export"))
                .next()
                .map(|s| {
                    s.split('(')
                        .next()
                        .unwrap_or("")
                        .trim_end_matches(['{', ':'])
                        .to_string()
                });
            body.map(|n| ("fn", n))
        } else if is_method {
            // Java/C# 风格：public/private 修饰符后取方法名（含括号前 token）
            let tokens: Vec<&str> = t.split_whitespace().collect();
            let idx = tokens.iter().rposition(|w| w.contains('('));
            match idx {
                Some(j) => {
                    // rstrip('(') 剥不掉 → nm 含 '(' → 被下方 `nm.contains('(')` 过滤 → async fn
                    let nm = tokens[j].split('(').next().unwrap_or("").trim().to_string();
                    Some(("fn", nm))
                }
                None => {
                    // 修饰符+返回类型+名称( 的常见形态：取最后一个非修饰符 token
                    let nm = tokens
                        .iter()
                        .rev()
                        .find(|w| {
                            !matches!(
                                **w,
                                "public"
                                    | "private"
                                    | "protected"
                                    | "static"
                                    | "final"
                                    | "async"
                                    | "pub"
                                    | "fn"
                            )
                        })
                        .map(|s| s.to_string());
                    nm.map(|n| ("fn", n))
                }
            }
        } else {
            None
        };
        if let Some((kind, nm)) = kind_name {
            // （fs_read 摘要 take 60 / file_store 地图 take 120），内部硬编码 60 会让
            if !nm.is_empty() && nm != "{" && !nm.contains('(') {
                out.push((ln, kind.to_string(), nm));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "fs_common_tests.rs"]
mod fs_common_tests;
