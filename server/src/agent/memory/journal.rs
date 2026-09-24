//! 工作日志与长期记忆（文件式）：让模型像人一样留痕——一轮一条的工作日志 + curated 长期记忆。

use std::path::{Path, PathBuf};

/// 本任务记忆上限（字符）。超了就该压缩——记忆贵在少而准，不在多。
pub const MEMORY_MAX_CHARS: usize = 3_000;

/// 每类长期知识的上限（字符）。
pub const LONGTERM_MAX_CHARS: usize = 2_000;

/// 长期知识**注入块**的总字符上限（三类均分）。取值要能容下常见的三类全文 ——
pub const LONGTERM_INJECT_CHARS: usize = 3_000;

/// 长期知识三类 —— **唯一事实源**：注入口径与话术都按这张表讲。
pub const LONGTERM_KINDS: [(&str, &str, &str); 3] = [
    (
        "project",
        "换个任务也成立的客观描述：架构 / 技术栈 / 目录布局 / 入口命令 / 环境路径",
        "# 项目事实\n\n（跨任务长期知识：这个项目是什么。只写客观、可复述的事实，不写过程。）\n",
    ),
    (
        "rules",
        "用户说过「以后都」「不要」「必须」的规矩：偏好 / 铁律 / 禁令 / 长期要求",
        "# 用户约定\n\n（跨任务长期知识：用户定过的规矩。写原话要点，不写解读。）\n",
    ),
    (
        "decisions",
        "将来会被问「为什么这么做」的：重要决定 + 理由 + 被否掉的方案",
        "# 关键决策\n\n（跨任务长期知识：做过的重要决定与理由，含被否掉的方案。）\n",
    ),
];

/// 项目标识：`<末段名>-<路径短哈希>` —— **稳定、可读、唯一**。
pub fn project_key(workspace: &str) -> String {
    let norm = crate::path::normalize_workspace(workspace);
    // 通用目录名：单看它认不出是哪个项目 ⇒ 补上父级一节
    const GENERIC: [&str; 9] = [
        "server", "src", "app", "client", "backend", "frontend", "web", "api", "code",
    ];
    let segs: Vec<String> = norm
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    let last = segs.last().cloned().unwrap_or_default();
    let tail = if GENERIC.contains(&last.as_str()) && segs.len() >= 2 {
        format!("{}-{}", segs[segs.len() - 2], last)
    } else {
        last
    };
    // 末段名一律小写：Windows 路径不区分大小写，`D:\proj` 与 `d:/proj` 必须落到**同一个**目录，
    let cleaned: String = tail
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut name = String::new();
    let mut prev_dash = false;
    for c in cleaned.chars() {
        if c == '-' {
            if !prev_dash && !name.is_empty() {
                name.push('-');
            }
            prev_dash = true;
        } else {
            name.push(c);
            prev_dash = false;
        }
    }
    let name: String = name.trim_matches('-').chars().take(40).collect();
    let name = if name.is_empty() {
        "project".to_string()
    } else {
        name
    };
    // FNV-1a 64（标准常量、跨版本稳定；不依赖 std 的 DefaultHasher —— 那个不保证不变）
    let canon = norm.replace('\\', "/").to_lowercase();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canon.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{name}-{:06x}", h & 0x00ff_ffff)
}

/// 项目目录：`<数据根>/projects/<项目标识>/` —— **这一个项目的家**。
pub fn project_dir(workspace: &str) -> PathBuf {
    crate::path::data_root::data_root()
        .join("projects")
        .join(project_key(workspace))
}

/// 任务工作空间里的**分类目录** —— **唯一事实源**：建目录、每轮注入口径、话术都按这张表讲。
pub const TASK_SUBDIRS: [(&str, &str); 5] = [
    ("artifacts", "**要留的成果** —— 这次真正做出来的东西（解包出的资源、生成的图片与数据、导出的成品）"),
    ("docs", "生成或落地的文档（报告 / 笔记 / 说明 / 清单）"),
    ("scripts", "你写的脚本"),
    ("cache", "**用完可丢**的中间件（解压临时目录 / 下载缓存 / 重定向的日志 / 编译中间物）"),
    ("attachments", "附件（用户上传的图片与文档，以及你截的图）"),
];

/// 附件分类的目录名 —— **上传侧（`routes/chat.rs`）引用它，不许自己写死字面量**。
pub const ATTACH_SUBDIR: &str = "attachments";

/// 任务工作空间里某个分类目录的绝对路径。
pub fn task_subdir(workspace: &str, task_key: &str, name: &str) -> PathBuf {
    task_dir(workspace, task_key).join(name)
}

/// 一次性迁移：把历史上的两处落点搬进**数据根的项目目录**。
fn migrate_into_project_dir(workspace: &str) {
    let dst = project_dir(workspace);
    let sources = [
        Path::new(workspace).join(".real").join("memory"),
        dst.join("memory"),
    ];
    for src in sources {
        if !src.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&src) else {
            continue;
        };
        for entry in rd.flatten() {
            let to = dst.join(modernize_name(&entry.file_name().to_string_lossy()));
            if to.exists() {
                continue;
            }
            let from = entry.path();
            let ok = if from.is_dir() {
                copy_tree(&from, &to)
            } else {
                std::fs::create_dir_all(&dst).is_ok() && std::fs::copy(&from, &to).is_ok()
            };
            if ok {
                tracing::info!(from = %from.display(), to = %to.display(), "已迁至数据根项目目录");
            }
        }
    }
}

/// 旧中文名 → ASCII 名：**只在迁移老数据时用**。
fn modernize_name(name: &str) -> String {
    match name {
        "任务" => "tasks".to_string(),
        "长期" => "longterm".to_string(),
        "产物" => "artifacts".to_string(),
        "文档" => "docs".to_string(),
        "脚本" => "scripts".to_string(),
        "缓存" => "cache".to_string(),
        "附件" => "attachments".to_string(),
        "项目事实.md" => "project.md".to_string(),
        "用户约定.md" => "rules.md".to_string(),
        "关键决策.md" => "decisions.md".to_string(),
        _ => match name.strip_prefix("日志-") {
            Some(rest) => format!("worklog-{rest}"),
            None => name.to_string(),
        },
    }
}

fn copy_tree(src: &Path, dst: &Path) -> bool {
    if std::fs::create_dir_all(dst).is_err() {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(src) else {
        return false;
    };
    let mut ok = true;
    for entry in rd.flatten() {
        let t = dst.join(modernize_name(&entry.file_name().to_string_lossy()));
        if entry.path().is_dir() {
            ok &= copy_tree(&entry.path(), &t);
        } else {
            ok &= std::fs::copy(entry.path(), &t).is_ok();
        }
    }
    ok
}

/// **本任务的工作空间**：`…/tasks/<task_key>/`
pub fn task_dir(workspace: &str, task_key: &str) -> PathBuf {
    project_dir(workspace).join("tasks").join(task_key)
}

/// 本任务的日志：`…/tasks/<task_key>/worklog-<date>.md`
pub fn task_journal_path(workspace: &str, task_key: &str, date: &str) -> PathBuf {
    task_dir(workspace, task_key).join(format!("worklog-{date}.md"))
}

/// 本任务的记忆：`…/任务/<task_key>/MEMORY.md`
pub fn task_memory_path(workspace: &str, task_key: &str) -> PathBuf {
    task_dir(workspace, task_key).join("MEMORY.md")
}

/// 长期知识目录：`…/longterm/`
pub fn longterm_dir(workspace: &str) -> PathBuf {
    project_dir(workspace).join("longterm")
}

/// 长期知识某类文件：`…/longterm/<kind>.md`
pub fn longterm_path(workspace: &str, kind: &str) -> PathBuf {
    longterm_dir(workspace).join(format!("{kind}.md"))
}

/// 长期知识**现状块**（注入用，每任务一次）。
pub fn longterm_block(workspace: &str, cap_chars: usize) -> String {
    if workspace.trim().is_empty() {
        return String::new();
    }
    let per = (cap_chars / LONGTERM_KINDS.len().max(1)).max(200);
    let mut parts: Vec<String> = Vec::new();
    for (kind, _, _) in LONGTERM_KINDS {
        let Ok(raw) = std::fs::read_to_string(longterm_path(workspace, kind)) else {
            continue;
        };
        let body = raw.trim();
        if body.is_empty() {
            continue;
        }
        let shown: String = if body.chars().count() > per {
            let head: String = body.chars().take(per).collect();
            format!("{head}\n…（本类超长已截断）")
        } else {
            body.to_string()
        };
        parts.push(format!("── {kind}.md ──\n{shown}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        "【项目长期知识】{}\n（跨任务累积。**内容已在此列出，不要再花调用去读**；要更新就直接改对应文件。）\n\n{}",
        longterm_dir(workspace).display(),
        parts.join("\n\n")
    )
}

/// 任务目录名：由会话创建时间派生（**北京时间 UTC+8**）。
pub fn task_key_from_created_at(created_at: &str) -> String {
    let shifted = chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.with_timezone(&chrono::Utc) + chrono::Duration::hours(8))
        .unwrap_or_else(|_| now_shifted());
    // `shifted` 的时区自称是 UTC（值已加 8 小时），所以不能靠 `%z` 取偏移（会输出 +0000），
    format!("{}+0800", shifted.format("%Y%m%dT%H%M%S"))
}

/// 今天日期（**北京时间 UTC+8**）：模型与人都在这一个时区，日志按本地日切分才符合直觉。
pub fn today() -> String {
    now_shifted().format("%Y-%m-%d").to_string()
}

fn now_shifted() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::hours(8)
}

/// 日志里"一条"的判定：以 `- ` 开头的行。
fn entries(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("- "))
        .collect()
}

/// KB 级小文件、每任务一次——同步读，不值得为它上 async IO。
fn read_text(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn seed_longterm_from_legacy(workspace: &str) {
    let legacy = project_dir(workspace).join("MEMORY.md");
    let Some(text) = read_text(&legacy) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    let target = longterm_path(workspace, LONGTERM_KINDS[0].0);
    if target.exists() {
        return;
    }
    let body = format!(
        "{}\n---\n（以下由旧版 `.real/memory/MEMORY.md` 自动接续；原文件保留不动。）\n\n{}",
        LONGTERM_KINDS[0].2, text
    );
    if let Some(dir) = target.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&target, body);
}

/// 生成「工作日志与长期记忆」注入段。
pub fn build_journal_prompt(workspace: &str, task_key: &str, include_longterm: bool) -> String {
    let ws = crate::path::normalize_workspace(workspace);
    if ws.is_empty() {
        // 连工作区都没有：只作相对表述，不编造绝对路径。
        return format!(
            "\n\n【工作日志与长期记忆】（数据根 `projects/<项目>/` 下）\n\
             - **本任务的工作空间** `tasks/<创建时刻>/`：日志 `worklog-<日期>.md`（一轮一条，追加）、\
             本任务记忆 `MEMORY.md`（只留结论，上限 {MEMORY_MAX_CHARS} 字）；产物按类放同目录：{}\n\
             - 跨任务长期知识放 `longterm/`，分三类：{}\n\
             - **任何生成物都放本任务目录，不要写进项目**；做完一件实事 → 追加一条；\
             换任务也成立的结论 → 写进对应的长期类；要回忆 → 直接 read，不要猜。\n",
            TASK_SUBDIRS
                .iter()
                .map(|(d, w)| format!("`{d}/` ← {w}"))
                .collect::<Vec<_>>()
                .join(" · "),
            LONGTERM_KINDS
                .iter()
                .map(|(k, _, _)| *k)
                .collect::<Vec<_>>()
                .join(" / "),
        );
    }
    let date = today();
    let tdir = task_dir(&ws, task_key);
    let log = task_journal_path(&ws, task_key, &date);
    let mem = task_memory_path(&ws, task_key);
    // 老位（`<workspace>/.real/memory/`）→ 数据根项目目录：一次性搬过来（幂等）
    migrate_into_project_dir(&ws);
    // 分类目录先建出来：模型只会 write 文件，不会先 mkdir（缺目录会白跑一轮）
    for (sub, _) in TASK_SUBDIRS {
        let _ = std::fs::create_dir_all(tdir.join(sub));
    }
    seed_longterm_from_legacy(&ws);

    let mut s = String::from("\n\n【工作日志与长期记忆】（像人一样留痕：干完记一笔，回头能查）\n");
    s.push_str(&format!(
        "- **本任务的工作空间**（只属于这次任务；产物与记忆都放这儿）：{}\n",
        tdir.display()
    ));
    let log_text = read_text(&log);
    // 自适应挂载：静态机制说明（产物分类判据等）在同一任务里只讲一次——
    let first_touch = log_text.is_none() && read_text(&mem).is_none();
    match &log_text {
        Some(t) => {
            let e = entries(t);
            let last = e.last().map(|l| l.trim()).unwrap_or("");
            s.push_str(&format!(
                "  · 日志 {}：今天已有 {} 条；最近一条：{}\n",
                log.display(),
                e.len(),
                crate::agent::plan::truncate(last, 120)
            ));
        }
        None => s.push_str(&format!("  · 日志 {}（今天还没记）\n", log.display())),
    }
    match read_text(&mem) {
        Some(t) => {
            let n = t.chars().count();
            let flag = if n > MEMORY_MAX_CHARS {
                "  ★ 已超上限：先压缩再写"
            } else {
                ""
            };
            s.push_str(&format!(
                "  · 本任务记忆 {}（{n} / {MEMORY_MAX_CHARS} 字符）{flag}\n",
                mem.display()
            ));
        }
        None => s.push_str(&format!(
            "  · 本任务记忆 {}（还没有；首次写入时新建，上限 {MEMORY_MAX_CHARS} 字符）\n",
            mem.display()
        )),
    }
    // 产物分类（唯一事实源 TASK_SUBDIRS）：**仅首次挂载时逐条列出判据**——
    if first_touch {
        for (sub, what) in TASK_SUBDIRS {
            s.push_str(&format!("  · `{sub}/` ← {what}\n"));
        }
        s.push_str("  · **任何生成物都放本任务目录，不要写进项目**（项目里只放要进版本库的资产）\n");
    } else {
        let dirs = TASK_SUBDIRS
            .iter()
            .map(|(d, _)| format!("`{d}/`"))
            .collect::<Vec<_>>()
            .join(" / ");
        s.push_str(&format!(
            "  · 产物分类：{dirs}（各目录判据见首轮注入）；任何生成物都放本任务目录，不写进项目\n"
        ));
    }

    if include_longterm {
        s.push_str("- 跨任务长期知识 `longterm/`（**只增不删**；任务目录被删也动不到它）：\n");
        for (kind, judge, _) in LONGTERM_KINDS {
            let p = longterm_path(&ws, kind);
            let body = match read_text(&p) {
                Some(t) => {
                    let n = t.chars().count();
                    let flag = if n > LONGTERM_MAX_CHARS {
                        "  ★ 已超上限：先压缩再写"
                    } else {
                        ""
                    };
                    format!("（{n} / {LONGTERM_MAX_CHARS} 字符）{flag}")
                }
                None => "（空）".to_string(),
            };
            s.push_str(&format!(
                "  · {kind} {} {body}\n      判据：{judge}\n",
                p.display()
            ));
        }
        s.push_str(
            "- 做完一件实事 → 往**本任务**今天的日志追加一条（时间 · 做了什么 → 产出落在哪 → 下一步）；\n  \
             换任务也成立的结论 / 用户定的规矩 / 重要决定 → 写进 `longterm/` 里对应的那一类（**只留结论**）；\n  \
             要回忆以前做过什么 → 直接 read 上面这些文件，**不要猜、不要问**。\n",
        );
    } else {
        s.push_str(
            "- 做完一件实事 → 往**本任务**今天的日志追加一条（时间 · 做了什么 → 产出落在哪 → 下一步）。\n",
        );
    }
    s
}

/// 删任务时清掉它的工作空间 —— **但留下工作日志与记忆**。
pub async fn purge_task_workspace(pool: &sqlx::SqlitePool, session_id: &str) {
    let Ok(Some(raw_ws)) = crate::db::repos::get_session_workspace(pool, session_id).await else {
        return;
    };
    let Ok(created) = crate::db::repos::get_session_created_at(pool, session_id).await else {
        return;
    };
    let ws = crate::path::normalize_workspace(&raw_ws);
    if ws.is_empty() {
        return;
    }
    let key = task_key_from_created_at(&created);
    let dir = task_dir(&ws, &key);
    let root = crate::path::data_root::data_root().join("projects");
    if !dir.starts_with(&root) || !dir.is_dir() {
        tracing::warn!(dir = %dir.display(), "工作空间路径不合法，放弃清理");
        return;
    }
    // ① 先救出工作日志与记忆（挪进 longterm/；撞名带任务时刻，绝不覆盖）
    let lt = longterm_dir(&ws);
    let _ = std::fs::create_dir_all(&lt);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name != "MEMORY.md" && !name.starts_with("worklog-") {
                continue;
            }
            let src = e.path();
            let mut dst = lt.join(&name);
            if dst.exists() {
                dst = lt.join(format!("{key}-{name}"));
            }
            if std::fs::rename(&src, &dst).is_err() {
                let _ = std::fs::copy(&src, &dst);
            }
        }
    }
    // ② 删掉工作空间本身
    if std::fs::remove_dir_all(&dir).is_ok() {
        tracing::info!(dir = %dir.display(), "任务工作空间已清理（日志与记忆已归档进 longterm/）");
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod journal_tests;
