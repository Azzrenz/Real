//! 源码文件判定（审计工具删除后仅存 file_index 依赖的 is_src_file）

use std::path::Path;

const SRC_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "c", "h", "cpp", "hpp",
];

/// 是否源码文件（按扩展名；file_index 索引过滤用）
pub(crate) fn is_src_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| SRC_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}
