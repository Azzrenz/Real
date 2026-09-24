//! Unix shell 通道：Windows 上给模型一个 Git Bash（类 Unix）的执行环境。

use std::path::PathBuf;
use std::sync::OnceLock;

/// Git Bash 候选位置（`REAL_GIT_BASH` 可覆盖；都没有再回退按 git 反推）。
const CANDIDATES: &[&str] = &[
    r"C:\Program Files\Git\bin\bash.exe",
    r"C:\Program Files (x86)\Git\bin\bash.exe",
    r"C:\Program Files\Git\usr\bin\bash.exe",
];

/// cmd 里**没有**、或语义完全不同的 Unix 命令名 —— 命中即说明这是 Unix 母语，整条交 Git Bash。
const UNIX_ONLY: &[&str] = &[
    // 查找 / 文本处理
    "grep", "sed", "awk", "find", "xargs", "head", "tail", "wc", "nl", "tr", "cut", "paste",
    "tee", "uniq", "diff", "seq", "stat", "readlink", "realpath",
    // 目录 / 文件
    "ls", "cat", "rm", "cp", "mv", "mkdir", "rmdir", "pwd", "touch", "chmod", "basename",
    "dirname",
    // 环境 / 容量
    "which", "du", "df",
];

/// cmd 专有语法特征 —— 在 PowerShell 壳里**必然出错**的写法。
const CMD_ONLY_SYNTAX: &[&str] = &[
    "2>nul", "2> nul", ">nul", "> nul", "dir /b", "dir /s", "dir /a", "dir /o",
];

/// 检测**混血命令**：首词属 PowerShell 壳（`ps_cmdlets` 命中），命令体却写了 cmd 专有语法。
pub fn detect_mixed_shell(command: &str, ps_cmdlets: &[&str]) -> Option<String> {
    let first = command.trim_start().split_whitespace().next().unwrap_or("");
    if first.is_empty() || !ps_cmdlets.iter().any(|c| c.eq_ignore_ascii_case(first)) {
        return None;
    }
    if crate::tools::cmd_translate::has_top_level_amp_or_paren(command) {
        return Some("& 顺次执行（PowerShell 里 `&` 是调用运算符）".to_string());
    }
    let lower = command.to_ascii_lowercase();
    CMD_ONLY_SYNTAX
        .iter()
        .find(|f| lower.contains(**f))
        .map(|f| format!("`{f}`（cmd 语法）"))
}

/// 命令首 token 是否为 Unix 专属命令（base 名，去掉路径与扩展名）。
fn head_name(seg: &str) -> String {
    let head = seg.trim_start().split_whitespace().next().unwrap_or("");
    std::path::Path::new(head)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| head.to_ascii_lowercase())
}

/// cmd 内建（**先做等价映射再送 bash** 的那些）——首段是它们时不在本函数直通，
const CMD_BUILTINS: &[&str] = &["type", "findstr", "copy", "move", "del", "erase"];

/// 是否应交给 Git Bash 执行。
pub fn wants_unix_shell(command: &str) -> bool {
    let first = head_name(command);
    if first.is_empty() {
        return false;
    }
    let lower = command.to_ascii_lowercase();
    if CMD_ONLY_SYNTAX.iter().any(|f| lower.contains(*f)) {
        return false;
    }
    // ⚠️ 两条"Unix 母语写法"的强信号 —— 它们比命令名更早、更可靠地说明该走哪个壳
    if crate::tools::cmd_translate::has_escaped_quote(command)
        || crate::tools::cmd_translate::has_posix_drive_path(command)
    {
        return true;
    }
    if UNIX_ONLY.contains(&first.as_str()) {
        return true;
    }
    // ② 管道段（跳过首段 —— 首段已在上面单独判过）
    let pipe_has_unix = crate::tools::cmd_translate::split_pipes_respecting_quotes(command)
        .iter()
        .skip(1)
        .any(|seg| !seg.trim().is_empty() && UNIX_ONLY.contains(&head_name(seg).as_str()));
    let pipe_unix =
        pipe_has_unix && crate::tools::cmd_translate::translate_unix_pipeline(command).is_err();
    // ③ 语句首（`;` / `&` / `&&` / `||` 分隔的语句 —— `|` 由 ② 管，
    let stmt_unix = crate::tools::cmd_translate::split_statements(command)
        .iter()
        .any(|seg| !seg.trim().is_empty() && UNIX_ONLY.contains(&head_name(seg).as_str()));
    if CMD_BUILTINS.contains(&first.as_str()) {
        return stmt_unix;
    }
    pipe_unix || stmt_unix
}

/// 定位 Git Bash 的可执行文件；找不到返回 `None`（调用方回落到原有 cmd 通道，行为不变）。
pub fn locate() -> Option<PathBuf> {
    CACHED.get_or_init(probe).clone()
}

// ── 第二类路由：cmd 内建命令 → Unix 等价（改写后同样走 Git Bash）──────────────────

/// 把一条 cmd 风格命令改写成 Unix 等价，好让 Git Bash 执行（规范引号 + UTF-8）。
pub fn rewrite_cmd_builtin(command: &str) -> Option<String> {
    if command.contains('&') {
        return None;
    }
    let segs = crate::tools::cmd_translate::split_pipes_respecting_quotes(command);
    let mut out: Vec<String> = Vec::with_capacity(segs.len());
    for seg in &segs {
        out.push(rewrite_one(seg.trim())?);
    }
    Some(out.join(" | "))
}

// ── 第三类路由：命令最终归哪个壳，由执行层按「实际执行壳」判定 ─────────────────────

/// 单段映射：认不出来返回 `None`（含"本来就是普通外部程序"的情况 —— 那种不归这几张表管）。
fn rewrite_one(seg: &str) -> Option<String> {
    let (head, rest) = split_head(seg);
    let low = head.to_ascii_lowercase();
    match low.as_str() {
        // type f → cat f；**无参不映射**（cmd 的 `type` 无参显示"命令类型"，与 cat 不等价）
        "type" if !rest.is_empty() => Some(format!("cat {rest}")),
        "findstr" => map_findstr(rest),
        // 文件操作类内建（copy / move / del / md）→ Unix 等价（flag 只认表内的）
        h if BUILTIN_TABLE.iter().any(|(k, _, _)| *k == h) => BUILTIN_TABLE
            .iter()
            .find(|(k, _, _)| *k == h)
            .and_then(|(_, unix, flags)| map_builtin(rest, unix, flags)),
        // 本来就是 Unix 命令：原样（与 `wants_unix_shell` 共用同一张表，不另立一份）
        h if UNIX_ONLY.contains(&h) => Some(seg.to_string()),
        _ => None,
    }
}

/// cmd 文件操作类内建 → Unix 等价：`(cmd 名, Unix 名, 认得并映射的 flag)`。
const BUILTIN_TABLE: &[(&str, &str, &[(&str, &str)])] = &[
    ("copy", "cp", &[("/y", "-f")]),
    ("move", "mv", &[("/y", "-f")]),
    ("del", "rm", &[("/f", "-f"), ("/q", "")]),
    ("erase", "rm", &[("/f", "-f"), ("/q", "")]),
    ("md", "mkdir", &[]),
];

/// 按表映射：flag 在表内才输出，裸 token 全是参数；**表外 flag 或没有参数 → `None`**。
fn map_builtin(rest: &str, unix: &str, flags: &[(&str, &str)]) -> Option<String> {
    if rest.is_empty() {
        return None;
    }
    let mut out_flags: Vec<&str> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    for tok in tokenize(rest) {
        let low = tok.to_ascii_lowercase();
        if low.starts_with('/') {
            match flags.iter().find(|(k, _)| *k == low.as_str()) {
                Some((_, v)) => {
                    if !v.is_empty() {
                        out_flags.push(v);
                    }
                }
                None => return None,
            }
        } else {
            args.push(tok);
        }
    }
    if args.is_empty() {
        return None;
    }
    let mut s = String::from(unix);
    for f in out_flags {
        s.push(' ');
        s.push_str(f);
    }
    for a in args {
        s.push(' ');
        s.push_str(&a);
    }
    Some(s)
}

/// `findstr` → `grep`。**只认这张 flag 表**，表外一律 `None`（不猜）
fn map_findstr(rest: &str) -> Option<String> {
    if rest.is_empty() {
        return None;
    }
    let mut flags: Vec<&str> = Vec::new();
    let mut literals: Vec<String> = Vec::new();
    let mut pattern: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    for tok in tokenize(rest) {
        let low = tok.to_ascii_lowercase();
        match low.as_str() {
            "/i" => flags.push("-i"),
            "/n" => flags.push("-n"),
            "/v" => flags.push("-v"),
            "/s" => flags.push("-r"),
            "/r" => {}
            _ if low.starts_with("/c:") => {
                // `/C:"字面串"`：跳过 `/c:` 三个字符，剥掉外层引号，**保留原始大小写**
                let v: String = tok.chars().skip(3).collect();
                literals.push(v.trim_matches('"').to_string());
            }
            _ if tok.starts_with('/') => return None,
            _ => {
                // 已经有 `/C:` 字面串时，pattern 由字面串承担 ⇒ 裸 token 一律是文件名
                if pattern.is_none() && literals.is_empty() {
                    pattern = Some(tok.clone());
                } else {
                    files.push(tok.clone());
                }
            }
        }
    }
    let mut out = String::from("grep");
    for f in &flags {
        out.push(' ');
        out.push_str(f);
    }
    if !literals.is_empty() {
        out.push_str(" -F");
        for l in &literals {
            out.push_str(" -e \"");
            out.push_str(l);
            out.push('"');
        }
    } else if let Some(p) = &pattern {
        out.push(' ');
        out.push_str(p);
    } else {
        return None;
    }
    for f in &files {
        out.push(' ');
        out.push_str(f);
    }
    Some(out)
}

/// 取首词与其余部分。
fn split_head(seg: &str) -> (&str, &str) {
    let s = seg.trim_start();
    match s.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (s, ""),
    }
}

/// 引号感知的空格分词：`/C:"test result"` 必须是一个 token（否则带空格的模式会被拆断）。
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match c {
            '"' | '\'' => {
                if quote == Some(c) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(c);
                }
                cur.push(c);
            }
            ' ' | '\t' if quote.is_none() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

static CACHED: OnceLock<Option<PathBuf>> = OnceLock::new();

fn probe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("REAL_GIT_BASH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    for c in CANDIDATES {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    derive_from_git()
}

/// 按 `where git` 反推：`…\Git\cmd\git.exe` → `…\Git\bin\bash.exe`。覆盖自定义安装盘。
fn derive_from_git() -> Option<PathBuf> {
    let mut c = std::process::Command::new("where");
    c.arg("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let out = c.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let git = PathBuf::from(line.trim());
        // …/Git/cmd/git.exe → …/Git
        let root = git.parent().and_then(|p| p.parent())?;
        let bash = root.join("bin").join("bash.exe");
        if bash.is_file() {
            return Some(bash);
        }
    }
    None
}

#[cfg(test)]
#[path = "cmd_bash_tests.rs"]
mod cmd_bash_tests;
