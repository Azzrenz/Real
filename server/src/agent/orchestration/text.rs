//! 文本骨架域：把整份文件压成「符号骨架」，供阅读账（ReadMemo）注入。
//!
//! 从 `workflow.rs` 归位而来。三件套职责单一：
//! - `is_skeleton_line` —— 哪些行算骨架
//! - `extract_skeleton` —— 抽出骨架 + 总行数
//! - `clip_utf8` —— UTF-8 安全截断（`floor_char_boundary` 未稳定，手写）

/// 符号行启发式（Rust / TS / Python / Markdown 通用）：骨架只认这些前缀，别贪。
pub fn is_skeleton_line(t: &str) -> bool {
    if t.len() > 300 {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "pub fn ", "pub async fn ", "fn ", "async fn ", "pub struct ", "struct ", "pub enum ",
        "enum ", "impl ", "pub trait ", "trait ", "pub mod ", "mod ", "pub const ", "const ",
        "pub static ", "static ", "class ", "def ", "interface ", "export ", "function ",
        "# ", "## ", "### ",
    ];
    PREFIXES.iter().any(|p| t.starts_with(p))
}

/// 从文件全文提取符号骨架：`(总行数, "行号: 符号行" 串)`；cap 60 条 / 6KB（预算硬顶）。
pub fn extract_skeleton(content: &str) -> (u64, String) {
    const MAX_ITEMS: usize = 60;
    const MAX_BYTES: usize = 6 * 1024;
    let mut sk = String::new();
    for (i, l) in content.lines().enumerate() {
        let t = l.trim_start();
        if is_skeleton_line(t) {
            use std::fmt::Write as _;
            let _ = writeln!(sk, "{}: {}", i + 1, t);
            if sk.lines().count() >= MAX_ITEMS || sk.len() >= MAX_BYTES {
                break;
            }
        }
    }
    (content.lines().count() as u64, sk.trim_end().to_string())
}

/// UTF-8 安全截断（floor_char_boundary 未稳定，手写）：回填注入的体积硬顶。
pub fn clip_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}
