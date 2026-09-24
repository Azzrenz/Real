//! 工作区解析：开局恢复当前工作区（输入路径 → 会话持久化 → 历史记忆），工具 cwd 默认值

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// 工作区按 session 分桶（隔离：全局静态被多会话共享会串——会话 A resolve 后
static WORKSPACES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 已读文件表（防重复读全文 + 系统提示已读清单注入； read-before-edit
static KNOWN_FILES: LazyLock<Mutex<HashMap<String, HashSet<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 取某 session 已确认文件清单（resolve_known_path 修正猜错路径用）
pub fn known_files(session_id: &str) -> Vec<String> {
    if let Ok(g) = KNOWN_FILES.lock() {
        g.get(session_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// 获取某 session 当前工作区（工具 cwd 默认值读取——worker 注入，勿再读全局）
pub fn current(session_id: &str) -> Option<String> {
    WORKSPACES.lock().ok()?.get(session_id).cloned()
}

pub fn note_cwd(session_id: &str, cwd: &str) {
    let norm: std::path::PathBuf = std::path::Path::new(cwd)
        .components()
        .map(|c| c.as_os_str())
        .collect();
    if norm.is_dir() {
        if let Ok(mut g) = WORKSPACES.lock() {
            g.insert(session_id.to_string(), norm.to_string_lossy().to_string());
        }
    }
}

/// 会话结束/删除时清掉该 session 的工作区
pub fn clear_session(session_id: &str) {
    if let Ok(mut g) = WORKSPACES.lock() {
        g.remove(session_id);
    }
    if let Ok(mut g) = KNOWN_FILES.lock() {
        g.remove(session_id);
    }
}
