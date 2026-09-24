
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 跳过目录（依赖/产物/版本库/隐藏目录）
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", ".git", ".next", "__pycache__", ".venv", "venv",
    ".idea", ".vscode",
];

/// 计入"被写过"的后缀（源码 + 配置 + 文档）
const SOURCE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "py", "go", "java", "kt", "cs",
    "cpp", "cc", "c", "h", "hpp", "php", "rb", "sh", "ps1", "sql", "json", "toml", "yml", "yaml",
    "md", "css", "scss", "html",
];

/// 单次扫描允许访问的条目上限——超了视为判不出（防在大仓库上长时间占用）
const MAX_VISIT: usize = 20_000;
/// 返回条数上限（只给"最近写过的几个"，不搬全量）
const MAX_LIST: usize = 12;

fn is_source(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| SOURCE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 扫描 `root` 下 **mtime >= since** 的源码类文件，按新→旧返回（最多 `MAX_LIST` 条，
pub fn written_since(root: &Path, since: SystemTime) -> Option<Vec<String>> {
    if !root.is_dir() {
        return None;
    }
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut visited = 0usize;
    let mut hits: Vec<(SystemTime, PathBuf)> = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            visited += 1;
            if visited > MAX_VISIT {
                return None;
            }
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with('.') || SKIP_DIRS.contains(&name) {
                    continue;
                }
                stack.push(p);
            } else if md.file_type().is_file() && is_source(&p) {
                if let Ok(mt) = md.modified() {
                    if mt >= since {
                        hits.push((mt, p));
                    }
                }
            }
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0));
    Some(
        hits.into_iter()
            .take(MAX_LIST)
            .map(|(_, p)| {
                p.strip_prefix(root)
                    .unwrap_or(&p)
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "wt_tests.rs"]
mod wt_tests;
