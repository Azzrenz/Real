//! 多轮任务记忆：路径锚定 + 结论继承 + 环境事实提取 + 对话滚存/滚动压缩（workspace/session/user 三级）

use crate::db::repos;
use crate::error::AppResult;
use crate::path::extract_anchor_path;
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicI64, Ordering};

use super::theme;

/// 进程内压缩冷却（30 秒防抖；跨重启丢失无妨——冷却只是防抖）
static LAST_COMPACT_AT: AtomicI64 = AtomicI64::new(0);

/// 会话工作区挂接（机制接线）：确保会话绑有工作区，为记忆注入供锚。
pub async fn ensure_session_workspace(
    pool: &SqlitePool,
    session_id: &str,
    user_input: &str,
) -> Option<String> {
    // 名录冷启动补登（每进程一次，幂等）：把历史会话工作区里的项目根补进名录，
    super::project_registry::ensure_backfilled(pool).await;
    // ① 本轮消息路径锚定（path 域统一管线）——**永远优先**
    let mut resolved = extract_anchor_path(user_input)
        .map(|p| crate::path::normalize_workspace(&p))
        .filter(|p| !p.is_empty());
    // ② 项目名录（`project_registry`）：口语项目名当轮切换 ——
    if resolved.is_none() {
        if let Some((p, _alias)) = super::project_registry::lookup(pool, user_input).await {
            let norm = crate::path::normalize_workspace(&p);
            if !norm.is_empty() {
                resolved = Some(norm);
            }
        }
    }
    // ③ 会话已存 —— 连续多轮没改口时**保持不动**（降级为兜底
    if resolved.is_none() {
        if let Ok(Some(ws)) = repos::get_session_workspace(pool, session_id).await {
            // 存量卫生（自愈历史脏值）：非目录 / URL / 任务附件目录一律视为无效——
            if super::project_registry::is_usable_workspace(&ws) {
                resolved = Some(ws);
            }
        }
    }
    // ④ 默认工作区（settings default_workspace；单项目阶段兜底，显式锚定优先于它）
    if resolved.is_none() {
        if let Ok(Some(raw)) = repos::get_setting(pool, "default_workspace").await {
            let d = crate::path::normalize_workspace(raw.trim());
            if !d.is_empty() {
                resolved = Some(d);
            }
        }
    }
    let ws = resolved?;
    let _ = repos::set_session_workspace(pool, session_id, &ws).await;
    // 自动积累名录：本次锚定的项目根登记进去（非法根：附件目录/无工程标记，自动跳过）
    let _ = super::project_registry::register(pool, &ws).await;
    // 同步**进程内**工作区：`workspace::current` 决定 run 的 cwd 兜底（prepare_args.rs:783）、
    if std::path::Path::new(&ws).is_dir() {
        crate::agent::workspace::note_cwd(session_id, &ws);
    }
    Some(ws)
}

/// 续接语义判断（记忆分层·设计决定："新建任务独立记忆"）
pub fn is_resume_request(user_input: &str) -> bool {
    let lower = user_input.to_lowercase();
    lower.contains("继续")
        || lower.contains("续接")
        || lower.contains("接着")
        || lower.contains("resume")
}

// 记忆作用域开关（`settings` 键 `memory_scope`）—— **唯一实现**在

/// 路径锚定提取在 path 域（crate::path::extract_anchor_path）——路径归统一管线。
pub async fn build_memory_prompt(
    pool: &SqlitePool,
    session_id: &str,
    user_input: &str,
) -> AppResult<Option<String>> {
    ensure_session_workspace(pool, session_id, user_input).await;
    // **作用域开关**（唯一实现 `repos::memory_scope_shared`，写侧共用）
    let shared = repos::memory_scope_shared(pool).await;
    // 关掉共享时，把"按工作区查"的结果收敛回本会话（同一个过滤只写一次，三处读取共用）
    let keep_scope = |rows: Vec<repos::MemoryRow>| -> Vec<repos::MemoryRow> {
        if shared {
            rows
        } else {
            rows.into_iter()
                .filter(|r| r.session_id == session_id)
                .collect()
        }
    };
    let anchor = extract_anchor_path(user_input);
    // 只按 session 查（recall_session）
    let mut rows = repos::recall_session(pool, session_id, 40).await?;
    let session_ws = crate::db::repos::get_session_workspace(pool, session_id)
        .await
        .ok()
        .flatten();
    let ws = crate::path::normalize_workspace(&anchor.clone().or(session_ws).unwrap_or_default());
    // workspace 级合并（历史结论/
    let is_exam_hall = ws.starts_with("d:\\bench_ws") || ws.starts_with("d:/bench_ws");
    if shared && !ws.is_empty() && !is_exam_hall && is_resume_request(user_input) {
        if let Ok(ws_rows) = repos::recall_workspace_scope(pool, &ws, "workspace", 60).await {
            // 合并：workspace 级的 conclusion/conversation_summary/modification/error 补进来
            for r in ws_rows {
                if matches!(
                    r.record_type.as_str(),
                    "conclusion"
                        | "modification"
                        | "conversation_summary"
                        | "conversation"
                        | "error"
                ) && !rows.iter().any(|x| x.id == r.id)
                {
                    rows.push(r);
                }
            }
            // 按 priority/created_at 重排（recall_session 语义保持一致）
            rows.sort_by(|a, b| {
                b.priority
                    .cmp(&a.priority)
                    .then(b.created_at.cmp(&a.created_at))
            });
        }
    }
    // 分区：结论/修改继承；失败教训（防螺旋）；conversation 脉络
    let mut heritage: Vec<String> = Vec::new();
    let mut lessons: Vec<String> = Vec::new();
    let mut threads: Vec<String> = Vec::new();
    for r in rows.iter() {
        let line = match r.record_type.as_str() {
            "conclusion" => format!(
                "- {}：{}",
                r.key,
                crate::agent::plan::truncate(&r.value, 300)
            ),
            "modification" => format!("- 已修改：{}", crate::agent::plan::truncate(&r.value, 200)),
            // 失败教训单独注入——模型看到"试过X失败"就避开
            "error" => {
                lessons.push(format!("- {}", crate::agent::plan::truncate(&r.value, 200)));
                continue;
            }
            "conversation_summary" => {
                format!("- 📜 {}", crate::agent::plan::truncate(&r.value, 300))
            }
            "conversation" => format!("- 💬 {}", crate::agent::plan::truncate(&r.value, 300)),
            _ => continue,
        };
        // 结论/修改 → 继承区；conversation → 脉络区（设计决定：6 条算 6 轮，翻倍）
        if matches!(r.record_type.as_str(), "conclusion" | "modification") {
            if heritage.len() < 6 {
                heritage.push(line);
            }
        } else {
            if threads.len() < 6 {
                threads.push(line);
            }
        }
    }

    // 会话记忆为空时不再整体短路——主题/偏好/凭证（workspace 级）仍需注入；
    let mut out = String::new();
    if !rows.is_empty() {
        out.push_str("【历史任务记忆】（本工作区此前任务的记录：已完成事项、结论与涉及路径）\n");
    }
    if !heritage.is_empty() {
        out.push_str("【上轮结论与修改】（此前轮次的结论与已做修改）\n");
        for h in heritage {
            out.push_str(&format!("{h}\n"));
        }
    }
    // 失败教训注入——尝试失败的路径/方案
    if !lessons.is_empty() {
        out.push_str("【上轮失败教训】（以下路径/方案此前尝试失败）\n");
        for l in lessons.iter().take(8) {
            out.push_str(&format!("{l}\n"));
        }
    }
    if !threads.is_empty() {
        out.push_str("【最近对话脉络】（背景参考）\n");
        for t in threads {
            out.push_str(&format!("{t}\n"));
        }
    }
    // 本轮提及的路径已由 orchestration 的 path_facts / target_anchor 统一注入，

    // —— 主题加权 / 偏好 / 凭证 / 核心画像（写入侧 theme.rs）——
    let mem_ws = theme::resolve_ws(pool, session_id, user_input).await;
    // 【对话主题】双轨注入：保底（仅续接语义——内容类防绑架纪律，与上方 workspace 合并
    if let Ok(v) = repos::recall_by_type_workspace(pool, &mem_ws, "theme", 200).await {
        let mut themes = keep_scope(v);
        if !themes.is_empty() {
            themes.sort_by(|a, b| {
                b.priority
                    .cmp(&a.priority)
                    .then(b.created_at.cmp(&a.created_at))
            });
            let mut top: Vec<String> = Vec::new();
            let mut seen: Vec<i64> = Vec::new();
            if is_resume_request(user_input) {
                for t in themes.iter().filter(|t| t.priority >= 60).take(3) {
                    top.push(format!(
                        "- 🎯 {}（重要度 {}）",
                        crate::agent::plan::truncate(&t.value, 100),
                        t.priority
                    ));
                    seen.push(t.id);
                }
            }
            let kws = theme::extract_keywords(user_input);
            if !kws.is_empty() {
                let hooked: Vec<String> = themes
                    .iter()
                    .filter(|t| {
                        t.priority >= 50 && !seen.contains(&t.id) && theme::theme_matches(&t.value, &kws)
                    })
                    .take(3)
                    .map(|t| {
                        format!(
                            "- 🪝 {}（重要度 {}）",
                            crate::agent::plan::truncate(&t.value, 100),
                            t.priority
                        )
                    })
                    .collect();
                if !hooked.is_empty() {
                    out.push_str("【钩子命中的历史主题】（以下历史主题与当前问题相关）\n");
                    for l in hooked {
                        out.push_str(&format!("{l}\n"));
                    }
                }
            }
            if !top.is_empty() {
                out.push_str("【对话主题】（本工作区历来的核心话题，按重要度排序）\n");
                for l in top {
                    out.push_str(&format!("{l}\n"));
                }
            }
        }
    }
    // 【用户偏好】——回答粒度/风格/禁忌，注入（`shared` 时为用户级；关共享时收敛到本会话）
    if let Ok(v) = repos::recall_by_type_workspace(pool, &mem_ws, "preference", 10).await {
        let prefs = keep_scope(v);
        if !prefs.is_empty() {
            out.push_str("【用户偏好】（用户明确要求的回答方式）\n");
            for p in prefs {
                out.push_str(&format!(
                    "- {}（重要度 {}）\n",
                    crate::agent::plan::truncate(&p.value, 120),
                    p.priority
                ));
            }
        }
    }
    // 【用户凭证】——已提供的直接使用，绝不重新索取（"API 能访问但没 token，
    if let Ok(v) = repos::recall_by_type_workspace(pool, &mem_ws, "credential", 5).await {
        let creds = keep_scope(v);
        if !creds.is_empty() {
            out.push_str("【用户凭证】（用户已提供的凭证）\n");
            for c in creds {
                out.push_str(&format!(
                    "- 🔑 {}\n",
                    crate::agent::plan::truncate(&c.value, 240)
                ));
            }
        }
    }
    // 【用户核心画像】（L3 永恒记忆：settings 表 memory_persona，全局唯一；
    if let Ok(Some(p)) = repos::get_setting(pool, "memory_persona").await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&p) {
            let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            let (identity, needs, principles) = (g("identity"), g("needs"), g("principles"));
            if !identity.is_empty() || !needs.is_empty() || !principles.is_empty() {
                out.push_str("【用户核心画像】（用户身份/初心/最深需求）\n");
                if !identity.is_empty() {
                    out.push_str(&format!(
                        "- 🧭 身份：{}\n",
                        crate::agent::plan::truncate(&identity, 200)
                    ));
                }
                if !needs.is_empty() {
                    out.push_str(&format!(
                        "- 🎯 核心需求：{}\n",
                        crate::agent::plan::truncate(&needs, 300)
                    ));
                }
                if !principles.is_empty() {
                    out.push_str(&format!(
                        "- 📏 核心原则：{}\n",
                        crate::agent::plan::truncate(&principles, 300)
                    ));
                }
            }
        }
    }
    let task_key = crate::agent::memory::journal::task_key_from_created_at(
        &repos::get_session_created_at(pool, session_id).await.unwrap_or_default(),
    );
    out.push_str(&crate::agent::memory::journal::build_journal_prompt(
        &mem_ws, &task_key, shared,
    ));
    // 出口滤网：记忆是**回灌管道**，库里流的是自由文本（含模型自己写下的回答全文、分类摘要、
    Ok(Some(crate::agent::memory::turn_log::isolate_literals(&out)))
}

/// 仅写对话滚存（错误/取消路径复用，审计 A2：中断不能丢轮次脉络）
pub async fn store_conversation_roll(
    pool: &SqlitePool,
    session_id: &str,
    user_input: &str,
    answer: &str,
) -> AppResult<()> {
    let session_ws = crate::db::repos::get_session_workspace(pool, session_id)
        .await
        .ok()
        .flatten();
    let raw_anchor = extract_anchor_path(user_input)
        .or(session_ws)
        .unwrap_or_default();
    let anchor = crate::path::normalize_workspace(&raw_anchor);
    let conv_value = format!(
        "用户: {}\n→ AI: {}",
        user_input,
        answer,
    );
    // E1 唯一约束修复：conversation 是多行追加语义 → key 带时间戳（与 UNIQUE(workspace,key) 兼容）
    repos::remember(
        pool,
        &anchor,
        session_id,
        &format!("对话:{}", repos::now()),
        &conv_value,
        "conversation",
        30,
    )
    .await?;
    maybe_compact_conversations(pool, &anchor, session_id).await?;
    Ok(())
}

/// 滚动压缩（Real 4 号备份 compress_history：压早前轮、保留最近 N 条原文）
pub fn summarize_conversations(lines: &[String]) -> String {
    lines.join("\n")
}

pub async fn maybe_compact_conversations(
    pool: &SqlitePool,
    workspace: &str,
    session_id: &str,
) -> AppResult<()> {
    // 阈值在设置面板可调（运行时设置 compact_* 热生效，E8 压缩阈值调优）；
    let tun = crate::config::settings::tuning();
    let trigger = tun.compact_trigger.load(std::sync::atomic::Ordering::Relaxed);
    let keep_raw = tun.compact_keep_raw.load(std::sync::atomic::Ordering::Relaxed);
    let cooldown_secs: i64 = 30;
    // 压力阈值（字符数近似 token；压力触发——上下文接近上限才压，不按条数机械压）
    let pressure_chars = tun.compact_pressure_chars.load(std::sync::atomic::Ordering::Relaxed);
    if workspace.is_empty() {
        return Ok(());
    }
    let convs = repos::recall_by_type_workspace(pool, workspace, "conversation", 200).await?;
    // 双重触发条件：① 条数超 trigger（跨轮记忆 ≥20 条底线已满足，开始考虑压缩最旧）
    if convs.len() <= trigger {
        return Ok(());
    }
    let total_chars: usize = convs.iter().map(|r| r.value.chars().count()).sum();
    if total_chars < pressure_chars {
        tracing::debug!(
            "[compact] 跳过：{} 条但累计 {} 字符 < 压力阈值 {}（压力触发，不机械按条数压）",
            convs.len(),
            total_chars,
            pressure_chars
        );
        return Ok(());
    }

    // 防退化：自上次压缩后必须新增了对话（convs DESC 第一条最新 > 最后 summary 时间）
    if let Ok(summaries) =
        repos::recall_by_type_workspace(pool, workspace, "conversation_summary", 1).await
    {
        if let Some(last) = summaries.first() {
            if let Some(newest) = convs.first() {
                if newest.created_at <= last.created_at {
                    tracing::debug!("[compact] 跳过：压缩后无新增对话（防退化循环）");
                    return Ok(());
                }
            }
        }
    }

    // 冷却：窗口内刚压过（进程级防抖）
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let last = LAST_COMPACT_AT.load(Ordering::Relaxed);
    if last != 0 && now_s - last < cooldown_secs {
        tracing::debug!("[compact] 跳过：{cooldown_secs}s 冷却中");
        return Ok(());
    }

    // 压缩：DESC 排序，尾部为最旧；保留最近 keep_raw 条，其余合成摘要后删除
    let to_roll: Vec<&repos::MemoryRow> = convs.iter().skip(keep_raw).collect();
    if to_roll.is_empty() {
        return Ok(());
    }
    let summary_text = summarize_conversations(
        &to_roll.iter().rev().map(|r| r.value.clone()).collect::<Vec<_>>(),
    );
    for r in to_roll.iter() {
        repos::forget_memory_by_id(pool, r.id).await?;
    }
    repos::remember(
        pool,
        workspace,
        session_id,
        // E1 唯一约束修复：summary 每次压缩一条新记录 → key 带时间戳
        &format!("对话摘要:{}", repos::now()),
        &format!("（早前对话滚动摘要）{}", summary_text),
        "conversation_summary",
        25,
    )
    .await?;
    LAST_COMPACT_AT.store(now_s, Ordering::Relaxed);
    tracing::info!(
        "[compact] 滚动压缩：{} 条旧对话 → 摘要（保留最近 {} 条原文）",
        to_roll.len(),
        keep_raw
    );
    Ok(())
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod memory_tests;

#[cfg(test)]
mod summarize_conversations_tests {
    use super::summarize_conversations;

    #[test]
    fn tail_anchor_survives() {
        let lines: Vec<String> = (0..50)
            .map(|i| {
                if i == 49 {
                    "记忆测试结论：run_real_eval 4/4 通过，锚点必须存活".to_string()
                } else {
                    format!("普通对话条目 {}：{}", i, "填充内容。".repeat(30))
                }
            })
            .collect();
        let out = summarize_conversations(&lines);
        assert!(out.contains("锚点必须存活"), "尾部锚点被砍：{}", out);
        assert!(!out.contains("已省略"), "摘要直通不应有省略标注");
    }
    #[test]
    fn per_line_head_anchor_survives() {
        let long = format!("{}{}", "关键锚点开头·", "x".repeat(500));
        let out = summarize_conversations(&[long]);
        assert!(out.contains("关键锚点开头"), "条内锚点应保留");
        assert!(out.contains("xxxx"), "长条原文应全量保留（不截取）");
    }}
