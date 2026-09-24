//! 在系统文件管理器中定位一个路径——工具卡上的文件路径点击后跳过去。

use axum::Json;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

/// 交给 explorer 的路径必须是「绝对 + 反斜杠」。
#[cfg(target_os = "windows")]
fn explorer_path(path: &std::path::Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let cow = abs.to_string_lossy();
    let trimmed = cow.strip_prefix(r"\\?\").unwrap_or(cow.as_ref());
    let mut full = trimmed.replace('/', "\\");
    if full.len() > 3 && full.ends_with('\\') {
        full.pop();
    } else if full.len() == 3 && full.ends_with(":\\") {
        full.truncate(2);
    }
    full
}

/// 打开路径所在的位置：是文件就选中它，是目录就直接打开。
#[cfg(target_os = "windows")]
fn reveal(path: &std::path::Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;

    // raw_arg, not arg: `arg` would quote the WHOLE "/select,C:\dir with space\a.rs" token, and
    let full = explorer_path(path);
    let target = std::path::Path::new(&full);

    let line = if target.is_file() {
        format!("/select,\"{full}\"")
    } else {
        format!("\"{full}\"")
    };
    std::process::Command::new("explorer")
        .raw_arg(line)
        .spawn()
        .map_err(|e| AppError::Internal(format!("打开文件夹失败: {e}")))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn reveal(path: &std::path::Path) -> AppResult<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map_err(|e| AppError::Internal(format!("打开文件夹失败: {e}")))?;
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn reveal(path: &std::path::Path) -> AppResult<()> {
    // xdg-open has no "select this file" capability: fall back to opening its directory.
    let dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map_err(|e| AppError::Internal(format!("打开文件夹失败: {e}")))?;
    Ok(())
}

/// POST /api/reveal —— body: `{ "path": "<绝对路径>" }`
pub async fn reveal_path(Json(req): Json<Value>) -> AppResult<Json<Value>> {
    let raw = req.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        return Err(AppError::Validation("路径为空".into()));
    }
    let path = std::path::PathBuf::from(raw);
    if !path.exists() {
        return Err(AppError::Validation(format!("路径不存在: {raw}")));
    }
    reveal(&path)?;
    Ok(Json(json!({ "ok": true, "path": raw })))
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::explorer_path;

    #[test]
    fn forward_slashes_become_absolute_backslashes() {
        // 模型给的参数常年是 `D:/proj/...` 这种正斜杠写法；不转换的话 explorer 会去开「文档」。
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let fwd = manifest.to_string_lossy().replace('\\', "/");
        let got = explorer_path(std::path::Path::new(&fwd));
        assert!(!got.contains('/'), "explorer 不认正斜杠，实际给的是 {got}");
        assert!(std::path::Path::new(&got).is_absolute(), "必须是绝对路径: {got}");
        assert!(std::path::Path::new(&got).is_file(), "要能选中目标本身: {got}");
    }

    #[test]
    fn trailing_backslash_is_stripped_so_quotes_stay_closed() {
        let dir = explorer_path(&std::env::temp_dir());
        assert!(!dir.ends_with('\\'), "尾反斜杠会吃掉右引号: {dir}");
        assert!(std::path::Path::new(&dir).is_dir(), "{dir}");
    }
}
