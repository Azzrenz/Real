//! 文件索引：文件名→路径模糊匹配（治路径幻觉）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// 全局文件索引注册表（按 workspace 惰性 build + 缓存）
static INDEX_CACHE: LazyLock<Mutex<HashMap<String, Arc<FileIndex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 取指定 workspace 的全局索引（首次调用全量扫描并缓存）
pub fn global_index(workspace: &str) -> Arc<FileIndex> {
    let mut g = INDEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(idx) = g.get(workspace) {
        return idx.clone();
    }
    let mut idx = FileIndex::new();
    idx.build(Path::new(workspace));
    let arc = Arc::new(idx);
    g.insert(workspace.to_string(), arc.clone());
    arc
}

/// 文件索引
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    /// 文件名(basename小写) → 完整路径列表（一个名字可能多处）
    name_to_path: HashMap<String, Vec<String>>,
}

impl FileIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// 全量扫描 workspace，建立索引
    pub fn build(&mut self, workspace: &Path) {
        self.name_to_path.clear();
        let mut files: Vec<PathBuf> = Vec::new();
        collect_source_files(workspace, 0, &mut files);
        for f in files {
            self.index_file(&f);
        }
    }

    /// 索引单个文件（只做文件名→路径映射）
    fn index_file(&mut self, f: &Path) {
        let path_str = f.to_string_lossy().to_string();
        if let Some(bn) = f.file_name().and_then(|n| n.to_str()) {
            self.name_to_path
                .entry(bn.to_ascii_lowercase())
                .or_default()
                .push(path_str);
        }
    }

    /// 文件名模糊匹配：完整路径直接命中；否则按 basename 匹配
    pub fn lookup(&self, input: &str) -> LookupResult {
        let p = Path::new(input);
        if p.is_absolute() && p.exists() {
            return LookupResult::Exact(input.to_string());
        }
        let bn = p.file_name().and_then(|n| n.to_str()).unwrap_or(input);
        let key = bn.to_ascii_lowercase();
        match self.name_to_path.get(&key) {
            None => LookupResult::NotFound,
            Some(v) if v.len() == 1 => LookupResult::Exact(v[0].clone()),
            Some(v) => LookupResult::Ambiguous(v.clone()),
        }
    }
}

/// 查找结果
#[derive(Debug, Clone, PartialEq)]
pub enum LookupResult {
    Exact(String),
    Ambiguous(Vec<String>),
    NotFound,
}

/// 递归收集源码文件（深度不限，SKIP_DIRS 黑名单）
fn collect_source_files(dir: &Path, cur_depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if crate::tools::fs_common::SKIP_DIRS_AUDIT.contains(&name) {
                continue;
            }
            collect_source_files(&p, cur_depth + 1, out);
        } else if crate::tools::audit::is_src_file(&p) {
            out.push(p);
        }
    }
}
