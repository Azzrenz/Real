//! 数据根目录：一主三分类（logs / spill / tmp）+ db + archive。

use std::path::PathBuf;

/// 用户在设置面板选定的数据目录——落位为默认根下的标记文件 data_dir.txt。
const MARKER_FILE: &str = "data_dir.txt";

fn marker_path() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            return Some(PathBuf::from(appdata).join("real-agent").join(MARKER_FILE));
        }
    }
    // 无 APPDATA（极简环境）→ 项目根 .real 下
    Some(crate::tools::fs_read::project_root().join(".real").join(MARKER_FILE))
}

/// 读取设置面板选定的数据目录（未设置 = None）
pub fn get_data_dir_setting() -> Option<String> {
    let p = marker_path()?;
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 写入/清除设置面板的数据目录选择（None/空串 = 清除，回到默认根）
pub fn set_data_dir_setting(dir: Option<&str>) -> Result<(), String> {
    let p = marker_path().ok_or("无法定位标记文件位置（无 APPDATA）")?;
    match dir.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(d) => {
            let pb = PathBuf::from(d);
            std::fs::create_dir_all(&pb)
                .map_err(|e| format!("目录不可用（创建失败）：{d} —— {e}"))?;
            std::fs::write(&p, d).map_err(|e| format!("写入标记文件失败: {e}"))
        }
        None => {
            let _ = std::fs::remove_file(&p);
            Ok(())
        }
    }
}

/// 标记文件里用户选定的目录（未设置/不可用 = None）
fn configured_dir() -> Option<PathBuf> {
    get_data_dir_setting().map(PathBuf::from)
}

/// 数据根目录（环境变量 REAL_DATA_DIR > 设置面板标记 > %APPDATA%/real-agent > 项目根 .real）
pub fn data_root() -> PathBuf {
    if let Ok(dir) = std::env::var("REAL_DATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(dir) = configured_dir() {
        return dir;
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            return PathBuf::from(appdata).join("real-agent");
        }
    }
    crate::tools::fs_read::project_root().join(".real")
}

fn ensure(dir: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 日志根：logs/（backend / req_dump / background 分类放里面，按天滚动）
pub fn logs_dir() -> PathBuf {
    ensure(data_root().join("logs"))
}

/// 模型 400 诊断留痕：logs/req_dump.log
pub fn req_dump_path() -> PathBuf {
    logs_dir().join("req_dump.log")
}

/// 后台命令日志：logs/background/
pub fn background_dir() -> PathBuf {
    ensure(logs_dir().join("background"))
}

/// spill 全文层：spill/（会话子目录，7 天保留由 cleanup_expired 负责）
pub fn spill_root() -> PathBuf {
    ensure(data_root().join("spill"))
}

/// 统一时间戳（**秒级**）—— 全系统唯一出口，任何落盘点都不得自己 `format!` 时间。
pub fn stamp_secs() -> String {
    chrono::Local::now().format("%Y%m%dT%H%M%S%z").to_string()
}

/// 统一时间戳（**毫秒级**）—— 同秒内可能生成多个文件的场景用它。
pub fn stamp_ms() -> String {
    let now = chrono::Local::now();
    format!(
        "{}{:03}{}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_millis(),
        now.format("%z")
    )
}

/// spill 全文落盘的**文件名** —— 唯一入口，所有落盘点共用，**不得各自拼名字**。
pub fn spill_file_name(tag: &str) -> String {
    let clean: String = tag.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("{}-{clean}.txt", stamp_ms())
}

/// 过程脚本/中间文件：tmp/（**不承诺留存**，由启动期 `cleanup_tmp` 按天清理）
pub fn tmp_dir() -> PathBuf {
    ensure(data_root().join("tmp"))
}

/// 清理 `tmp/` 下超过保留期的过程脚本，返回删除数。
pub fn cleanup_tmp(retention_days: i64) -> usize {
    let dir = tmp_dir();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);
    let mut removed = 0usize;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let mtime: chrono::DateTime<chrono::Utc> = modified.into();
        if mtime < cutoff && std::fs::remove_file(e.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 数据库：db/real.db
pub fn db_path() -> PathBuf {
    ensure(data_root().join("db")).join("real.db")
}

/// 技能库根：skills/（用户技能 SKILL.md 落位；REAL_SKILLS_ROOT 仍可覆盖）
pub fn skills_dir() -> PathBuf {
    ensure(data_root().join("skills"))
}

/// 常用脚本库：scripts/（跨项目、跨会话共享的"高频脚本"落位）。
pub fn scripts_dir() -> PathBuf {
    ensure(data_root().join("scripts"))
}

/// 厂商档案库：providers/（**一家公司一个 JSON 文件**）。
pub fn providers_dir() -> PathBuf {
    ensure(data_root().join("providers"))
}

/// 厂商档案说明（内嵌，随程序分发；放**用户实际动手的目录**里）。
const PROVIDERS_README: &[(&str, &str)] = &[(
    "接入新公司-读我.md",
    include_str!("../../assets/providers/接入新公司-读我.md"),
)];

/// 播种厂商档案说明（幂等：缺失即写，已存在不覆盖用户改动）。
pub fn seed_providers_readme() {
    let dir = providers_dir();
    for (name, content) in PROVIDERS_README {
        let path = dir.join(name);
        if !path.exists() {
            match std::fs::write(&path, content) {
                Ok(_) => tracing::info!(
                    path = %path.display(),
                    "已播种厂商档案说明（怎么加新公司：丢一个 JSON 进这个目录）"
                ),
                Err(e) => tracing::warn!(error = %e, "厂商档案说明播种失败（不影响运行）"),
            }
        }
    }
}

/// 脚本库内容（**内嵌进二进制**，随程序分发；播种=缺失即写，已存在不覆盖）。
const TOOLBOX: &[(&str, &str)] = &[
    ("list.py", include_str!("../../assets/scripts/list.py")),
    ("ev.py", include_str!("../../assets/scripts/ev.py")),
    ("cost.py", include_str!("../../assets/scripts/cost.py")),
    ("logs.py", include_str!("../../assets/scripts/logs.py")),
    ("svc.py", include_str!("../../assets/scripts/svc.py")),
    ("scan.py", include_str!("../../assets/scripts/scan.py")),
    ("report.py", include_str!("../../assets/scripts/report.py")),
];

/// 脚本库**索引**（一行一支：`名字=用途`）——供每轮上下文注入。
pub fn scripts_index(max: usize) -> String {
    let dir = scripts_dir();
    let mut items: Vec<(String, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("py") {
                continue;
            }
            let stem = match p.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if stem == "list" {
                continue;
            }
            items.push((stem, first_docline(&p)));
        }
    }
    items.sort();
    let total = items.len();
    let shown: Vec<String> = items
        .into_iter()
        .take(max)
        .map(|(n, u)| if u.is_empty() { n } else { format!("{n}={u}") })
        .collect();
    if shown.is_empty() {
        return String::new();
    }
    let mut s = shown.join(" · ");
    if total > max {
        s.push_str(&format!(" …（共 {total} 支，余下用 list.py 看）"));
    }
    s
}

/// 取脚本首行 docstring 作为用途（约定：`名字 · 用途`），截断到 20 字符。
fn first_docline(path: &std::path::Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let head: String = text.chars().take(400).collect();
    let Some(line) = head
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("\"\"\"") || l.starts_with("'''"))
    else {
        return String::new();
    };
    let usage = line.trim_matches('"').trim_matches('\'').trim();
    let usage = usage.split('·').nth(1).unwrap_or(usage).trim();
    let usage = usage.split('（').next().unwrap_or(usage).trim();
    usage.chars().take(20).collect()
}

/// 播种常用脚本库（幂等）：只补缺失，**不覆盖已存在**——用户/模型改过的保留。
pub fn seed_scripts() {
    let dir = scripts_dir();
    let mut added = 0usize;
    for (name, body) in TOOLBOX {
        let p = dir.join(name);
        if !p.exists() && std::fs::write(&p, body).is_ok() {
            added += 1;
        }
    }
    if added > 0 {
        tracing::info!(dir = %dir.display(), added, "常用脚本库已播种（scripts/）");
    }
}

/// 会话附件暂存根：attachments/{session_id}/（聊天上传的图片/文档落盘位——
pub fn attachments_dir() -> PathBuf {
    ensure(data_root().join("attachments"))
}

/// 历史归档：archive/（旧备份/考试快照等低频留存的落位）
pub fn archive_dir() -> PathBuf {
    ensure(data_root().join("archive"))
}

/// 旧散落点 → 新结构的一次性迁移（幂等：目标已存在则跳过）。
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> bool {
    if std::fs::create_dir_all(dst).is_err() {
        return false;
    }
    let rd = match std::fs::read_dir(src) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let mut ok = true;
    for entry in rd.flatten() {
        let t = dst.join(entry.file_name());
        if entry.path().is_dir() {
            ok &= copy_dir_recursive(&entry.path(), &t);
        } else {
            ok &= std::fs::copy(entry.path(), &t).is_ok();
        }
    }
    ok
}

pub fn migrate_legacy() {
    let root = crate::tools::fs_read::project_root();
    let moves: Vec<(PathBuf, PathBuf)> = vec![
        // 旧 spill（项目根 .real/spill 的历史会话）
        (root.join(".real").join("spill"), data_root().join("spill")),
        // 旧请求留痕
        (
            root.join("server").join("data").join("req_dump.log"),
            req_dump_path(),
        ),
        // 旧数据库备份 → archive
        (
            root.join("server").join("data").join("real.db.bak-20260905-1155"),
            archive_dir().join("real.db.bak-20260905-1155"),
        ),
        (
            root.join("server").join("data").join("real.db.bak-20260905-1636"),
            archive_dir().join("real.db.bak-20260905-1636"),
        ),
    ];
    for (from, to) in moves {
        if from.exists() && !to.exists() {
            let action = if from.is_dir() {
                std::fs::rename(&from, &to).map(|_| "moved").unwrap_or("kept")
            } else {
                std::fs::copy(&from, &to).map(|_| "copied").unwrap_or("kept")
            };
            tracing::info!(from = %from.display(), to = %to.display(), action, "数据根迁移");
        }
    }
    // 技能库迁移：旧硬编码位 server/data/skills → 数据根 skills/（整目录复制）
    let old_skills = root.join("server").join("data").join("skills");
    let new_skills = skills_dir();
    let new_has_entries = new_skills.is_dir()
        && std::fs::read_dir(&new_skills).map(|mut d| d.next().is_some()).unwrap_or(false);
    if old_skills.is_dir() && !new_has_entries {
        if copy_dir_recursive(&old_skills, &new_skills) {
            tracing::info!(from = %old_skills.display(), to = %new_skills.display(), "技能库已迁移至数据根");
        }
    }
    // 配置文件迁移：models.json / pricing.json 曾写进程 cwd（server/）→ 数据根根目录
    for name in ["models.json", "pricing.json"] {
        let old_cfg = root.join("server").join(name);
        let new_cfg = data_root().join(name);
        if old_cfg.is_file() && !new_cfg.is_file() {
            if std::fs::copy(&old_cfg, &new_cfg).is_ok() {
                tracing::info!(from = %old_cfg.display(), to = %new_cfg.display(), "配置文件已迁移至数据根");
            }
        }
    }
    // 数据库：新位无库且旧位有 → 复制（连 WAL/SHM 一起，保证一致性）
    let old_db = root.join("server").join("data").join("real.db");
    let new_db = db_path();
    if old_db.exists() && !new_db.exists() {
        for suffix in ["", "-wal", "-shm"] {
            let src = PathBuf::from(format!("{}{}", old_db.display(), suffix));
            let dst = PathBuf::from(format!("{}{}", new_db.display(), suffix));
            if src.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }
        tracing::info!(from = %old_db.display(), to = %new_db.display(), "数据库已迁移至数据根");
    }
}

#[cfg(test)]
#[path = "data_root_tests.rs"]
mod data_root_tests;
