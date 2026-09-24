//! 跨回合归档与压缩 —— 全部**门槛**都在这一个文件里（唯一事实源）。
use crate::model::types::InputItem;
use super::*;
/// 取最近一次 `llm.usage` 的真实 input token（归档门槛的取数口）。
pub(crate) async fn session_input_tokens(pool: &sqlx::SqlitePool, session_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT json_extract(payload_json, '$.input') FROM events
         WHERE session_id = ?1 AND kind = 'llm.usage' ORDER BY id DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .and_then(|v| v)
    .unwrap_or(0)
}

/// 距上一次 LLM 调用的空档（秒）—— 装配期改写闸门的取数口（同 `llm.usage` 表，取法一致）。
pub(crate) async fn session_idle_seconds(pool: &sqlx::SqlitePool, session_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT CAST((julianday('now') - julianday(created_at)) * 86400.0 AS INTEGER) FROM events
         WHERE session_id = ?1 AND kind = 'llm.usage' ORDER BY id DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|v| v.max(0))
    .unwrap_or(i64::MAX)
}

/// 归档启动线（token）—— **按窗口线推，且按缓存冷热分档**，不再是一个固定小值。
pub(crate) fn archive_floor_tokens(cache_cold: bool) -> i64 {
    super::govern::deep_cut_tokens(cache_cold)
}
pub(crate) const LEGACY_BLANKET_TOKENS: i64 = super::govern::th::LEGACY_BLANKET_TOKENS;
pub(crate) const REFERENCE_WINDOW: usize = 2;
/// 绝对体积硬顶（**会压到本值的 60%**）：防"话题连续"会话里引用判据永不触发 → 上下文无限。
pub(crate) const RAW_OUTPUT_HARD_CAP_BYTES: usize = super::govern::th::HARD_CAP_BYTES;
/// token → 字节系数：把 `token_budget`（token）接到 `archive_hard_cap`（字节）**同一个实现**上。
pub(crate) const BYTES_PER_TOKEN_EST: usize = super::govern::th::BYTES_PER_TOKEN_EST;
/// 工具回执**新鲜回合数**（唯一事实源在 `govern::th`）。
pub(crate) const AGE_FRESH_ROUNDS: usize = super::govern::th::RETIRE_FRESH_ROUNDS;
pub(crate) const REWRITE_COOLDOWN_TASKS: usize = super::govern::th::REWRITE_COOLDOWN_TASKS;
/// 超过它直接压**路标**；中间档（fresh < age ≤ 本值）走**滚动压缩**（结构化摘要）。
pub(crate) const AGE_ROLLING_ROUNDS: usize = super::govern::th::RETIRE_ROLL_ROUNDS;

/// 记一条「历史被改过」的事件。
pub(crate) async fn record_rewrite(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    source: &str,
    before_bytes: usize,
    items: &[InputItem],
) {
    let after_bytes = items_bytes(items);
    if after_bytes >= before_bytes {
        return;
    }
    let payload = serde_json::json!({
        "source": source,
        "before_bytes": before_bytes,
        "after_bytes": after_bytes,
        "saved_bytes": before_bytes - after_bytes,
    })
    .to_string();
    let _ = sqlx::query(
        "INSERT INTO events (session_id, kind, payload_json, created_at) VALUES (?1, 'ctx.rewrite', ?2, datetime('now'))",
    )
    .bind(session_id)
    .bind(&payload)
    .execute(pool)
    .await;
    tracing::info!(
        session = %session_id,
        source,
        saved_bytes = before_bytes - after_bytes,
        "装配期改写（此前不记账的三个阶段之一）"
    );
}

pub(crate) async fn archive_old_tasks(
    items: &mut Vec<InputItem>,
    pool: &sqlx::SqlitePool,
    session_id: &str,
    cache_cold: bool,
) {
    let floor = archive_floor_tokens(cache_cold);
    let cur_toks =
        (items_bytes(items) as u64 * 100 / super::govern::th::WINDOW_B_PER_TOKEN_X100) as i64;
    if cur_toks <= floor {
        return;
    }
    // 收益门槛：按**当前** input 规模动态算一次（代价 ∝ input，见 `govern::rewrite_min_gain_pct`）。
    let gain_pct = super::govern::rewrite_min_gain_pct(cur_toks);
    // 回合起点 = user 消息（goal_frame 以 user role 进入历史）。
    let user_idx: Vec<usize> = user_positions(items);
    let task_count = user_idx.len();
    if task_count >= 2 {
        let last_start = user_idx[task_count - 1];
        if drop_prev_tool_outputs_enabled() {
            archive_tasks_before(items, last_start, gain_pct);
            tracing::info!(session = %session_id, "实验开关：跨回合工具回执已全退场");
        }
        // reasoning 占位**不在这里做** —— 合并前这里有第二个循环（对 [0, last_start) 全线占位），
    }
    // 少于 2 个回合：没有"旧回合"可归档
    if task_count < 2 {
        return;
    }
    // 轮日志（seq 从 1 起，与回合序对齐；无记录 = 存量会话）
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT seq, keywords, user_input FROM turn_logs WHERE session_id = ?1 ORDER BY seq",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if rows.is_empty() {
        // 存量兜底：一刀切阈值（引用断续判不了——没有关键词数据）
        if cur_toks > LEGACY_BLANKET_TOKENS {
            archive_tasks_before(items, user_idx[task_count - 1], gain_pct);
        }
        return;
    }
    let mut kw: std::collections::HashMap<usize, Vec<String>> = Default::default();
    let mut input: std::collections::HashMap<usize, String> = Default::default();
    for (seq, kjson, uin) in rows {
        kw.insert(seq as usize, serde_json::from_str(&kjson).unwrap_or_default());
        input.insert(seq as usize, uin.to_lowercase());
    }
    // 引用断续判定（纯函数部分单测覆盖）：返回未被引用的回合序（1-based）
    let mark_key = format!("session_rewrite_mark:{session_id}");
    let last_mark: usize = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
        .bind(&mark_key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let cooling = task_count.saturating_sub(last_mark) < REWRITE_COOLDOWN_TASKS;
    let before_tokens = session_input_tokens(pool, session_id).await;
    // 改写前的真实 items 体积 —— 与改写后的同一把尺子量两次，差值才是真效果
    let before_bytes = items_bytes(items);
    let mut rewritten = 0usize;
    if !cooling {
        // 退场候选：年龄超限（硬）+ 引用断续（软）—— 判据唯一，见 `retire_old_tasks`。
        let retiring = retire_old_tasks(task_count, &kw, &input, REFERENCE_WINDOW, AGE_FRESH_ROUNDS);
        // **收益门槛前置**：先估可回收字节，不够就不动。
        let reclaimable: usize = retiring
            .iter()
            .map(|&seq| {
                let start = user_idx[seq - 1];
                let end = user_idx.get(seq).copied().unwrap_or(items.len());
                reclaimable_bytes(items, start, end)
            })
            .sum();
        if reclaimable * 100 >= before_bytes * gain_pct {
            for seq in retiring {
                let start = user_idx[seq - 1];
                let end = user_idx.get(seq).copied().unwrap_or(items.len());
                let age = task_count - seq;
                if age > AGE_ROLLING_ROUNDS {
                    stub_span(items, start, end);
                    rewritten += 1;
                } else {
                    // 进到这里必有 age > AGE_FRESH_ROUNDS（`retire_old_tasks` 已保证）
                    roll_span(items, start, end);
                    rewritten += 1;
                }
            }
        } else {
            tracing::debug!(
                session = %session_id,
                reclaimable,
                needed = before_bytes * gain_pct / 100,
                "退场收益不足门槛，本轮不动历史（保前缀缓存）"
            );
        }
    }
    // ── 安全网类改写 ──────────────────────────────────────────────────────
    let toks_now = session_input_tokens(pool, session_id).await;
    let budget = crate::config::settings::tuning()
        .token_budget
        .load(std::sync::atomic::Ordering::Relaxed);
    let mut safety_rewrites = 0usize;
    // 门槛在函数入口已按当时 input 算好（`gain_pct`）—— 全函数共用同一把尺子
    {
        // ① 体积硬顶（**函数内不受冷却约束**——一旦决定要压，就压到位；但整段代码
        safety_rewrites +=
            archive_hard_cap(items, &user_idx, RAW_OUTPUT_HARD_CAP_BYTES * 6 / 10, gain_pct);
        // ② token 预算（受同一改写冷却约束——冷却的意义就是"别每轮都破前缀缓存"）
        if !cooling && budget > 0 && toks_now as u64 > budget {
            let target_bytes = (budget * 6 / 10) as usize * BYTES_PER_TOKEN_EST;
            safety_rewrites += archive_hard_cap(items, &user_idx, target_bytes, gain_pct);
        }
    }
    let task_keep = crate::config::settings::tuning()
        .task_keep_outputs
        .load(std::sync::atomic::Ordering::Relaxed);
    let stub_min = crate::config::settings::tuning()
        .task_stub_min_bytes
        .load(std::sync::atomic::Ordering::Relaxed);
    // L1 回合内输出治理 —— **唯一实现**在 `intask::govern_in_task_outputs`（本文件不再有
    let in_task_rewrites = govern_in_task_outputs(
        items,
        &user_idx,
        task_keep,
        stub_min,
        super::govern::intask_total_bytes(),
    );
    // reasoning 治理 —— **唯一实现**（`item::govern_reasoning`），**两段式**
    let cur_round_start = user_idx.last().copied().unwrap_or(0);
    let in_task_reasoning = if reasoning_placeholder_enabled() {
        govern_reasoning(items, TASK_KEEP_REASONING_BYTES, cur_round_start)
    } else {
        0
    };
    // 工具回执**剥空字段**（`null` / `""` / `false`）—— 机械判据，不枚举字段名。
    let mut slim_rewrites = 0usize;
    for it in items.iter_mut() {
        if strip_empty_fields(it) {
            slim_rewrites += 1;
        }
    }
    // 跨回合压「工具调用参数」正文（上下文里最大的一块，见本文件顶部该节说明）。
    let call_arg_rewrites =
        stub_old_call_args(items, &user_idx, CALL_ARG_MIN_AGE_TASKS, CALL_ARG_MIN_BYTES);
    let rewritten = rewritten
        + safety_rewrites
        + in_task_rewrites
        + in_task_reasoning
        + slim_rewrites
        + call_arg_rewrites;
    if rewritten > 0 {
        let payload = serde_json::json!({
            // 阶段标识：装配期有四个会改历史的阶段，消费方靠它把断裂归到具体一处
            "source": "tasks",
            "task_count": task_count,
            "spans": rewritten,
            // 起点：上一次 LLM 调用的真实 input（token）。仅供参考，
            "before_tokens": before_tokens,
            "before_bytes": before_bytes,
            "after_bytes": items_bytes(items),
            "saved_bytes": before_bytes.saturating_sub(items_bytes(items)),
        })
        .to_string();
        let _ = sqlx::query(
            "INSERT INTO events (session_id, kind, payload_json, created_at) VALUES (?1, 'ctx.rewrite', ?2, datetime('now'))",
        )
        .bind(session_id)
        .bind(&payload)
        .execute(pool)
        .await;
        put_setting(pool, &mark_key, &task_count.to_string()).await;
    }
    // ⚠️ 正文摘要接力（`archive_old_body`）**不在这里调**：它把整段 splice 成一条摘要 =
}

pub(crate) async fn persist_rewrites(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    items: &[InputItem],
    ids: &[Option<String>],
    orig: &[String],
) {
    if items.len() != ids.len() || items.len() != orig.len() {
        tracing::warn!(
            session = %session_id,
            items = items.len(),
            ids = ids.len(),
            orig = orig.len(),
            "退场落库：平行数组不等长（有改长度的操作排在落库之前）——按较短长度截断执行"
        );
    }
    let n = items.len().min(ids.len()).min(orig.len());
    let mut changed = 0usize;
    // 改在**哪一条**，比改了几条重要得多
    let mut at: Vec<String> = Vec::new();
    let mut first_changed: Option<usize> = None;
    for i in 0..n {
        let Some(id) = ids[i].as_deref() else { continue };
        let Ok(js) = serde_json::to_string(&items[i]) else { continue };
        if js == orig[i] {
            continue;
        }
        first_changed.get_or_insert(i);
        if at.len() < 16 {
            at.push(format!(
                "{i}|{}|{}→{}",
                super::item::item_kind(&items[i]),
                orig[i].len(),
                js.len()
            ));
        }
        // 原文落 spill（可回取）：只对"有实质内容"的原件落盘，跳过已是 stub 的
        if orig[i].len() > 500 && !orig[i].contains("已归档") && !orig[i].contains("已略") {
            // 落**该会话自己的目录**（不在 spill 根平铺）——规范见
            let dir = crate::mcp::spill::spill_dir(session_id);
            // `id` 是完整 uuid：**不截断** —— 该路径会写进归档摘要给模型，模型按摘要里的
            let path = dir.join(format!(
                "{}-hist-{id}.json",
                crate::path::data_root::stamp_ms()
            ));
            if std::fs::create_dir_all(&dir).is_ok() {
                let _ = std::fs::write(&path, orig[i].as_bytes());
            }
        }
        let _ = crate::db::repos::update_message_item_json(pool, id, &js).await;
        changed += 1;
    }
    if changed > 0 {
        // 代价口径：从**首个改动点**起的全部字节，都会在下一轮按 miss 重算
        let tail_bytes = first_changed.map(|i| items_bytes(&items[i..])).unwrap_or(0);
        let mid = first_changed.map(|i| i + 2 < items.len()).unwrap_or(false);
        if mid {
            tracing::warn!(
                session = %session_id,
                changed,
                at = %at.join(" "),
                tail_bytes,
                "退场落库：**中段改写** —— 首个改动点之后的前缀全部失效，这批字节下一轮按 miss 重算"
            );
        } else {
            tracing::info!(
                session = %session_id,
                changed,
                at = %at.join(" "),
                tail_bytes,
                "退场落库：改写结果已写回（只在尾部，不动前缀）"
            );
        }
    }
}

/// 归档摘要源：该回合的（诉求原话, 结论要点, 涉及文件）——`turn_logs` 三列**全用上**。
pub(crate) type TurnDigest = (String, String, Vec<String>);

/// 归档原文包路径（**唯一来源**：摘要里的指针与落盘共用，不许各拼一份）。
fn task_pack_path(session_id: &str, seq: usize) -> std::path::PathBuf {
    crate::mcp::spill::spill_dir(session_id).join(format!(
        "{}-task-{seq:04}.json",
        crate::path::data_root::stamp_secs()
    ))
}

/// 正文摘要接力·主体：超「正文保留回合数」的旧正文 → 摘要化 + 原文包落盘。
pub(crate) async fn archive_old_body(
    items: &mut Vec<InputItem>,
    pool: &sqlx::SqlitePool,
    session_id: &str,
    cache_cold: bool,
) {
    // 启动线 = max(面板下限, 窗口线) —— **不再是一个固定的小值**，且窗口线按缓存冷热分档。
    let toks = session_input_tokens(pool, session_id).await;
    let live_bytes = (toks.max(0) as u64 * super::govern::th::WINDOW_B_PER_TOKEN_X100 / 100) as usize;
    let floor_bytes = crate::config::settings::tuning()
        .body_archive_floor_bytes
        .load(std::sync::atomic::Ordering::Relaxed);
    if live_bytes <= floor_bytes.max(super::govern::deep_cut_bytes(cache_cold)) {
        return;
    }
    // 记账仍用 items 的真实体积（改写前后差值才是真效果），与判据分开。
    let before_bytes = items_bytes(items);
    // 回合起点在本函数内重算：调用点已移到所有改写之后，调用方的 user_idx 不再适用
    let user_idx: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
        .map(|(i, _)| i)
        .collect();
    let keep_recent = crate::config::settings::tuning()
        .archive_keep_recent
        .load(std::sync::atomic::Ordering::Relaxed);
    let keep = keep_recent.min(user_idx.len().saturating_sub(1)).max(2);
    if user_idx.len() <= keep {
        return;
    }
    // 摘要源：诉求 + 结论 + 涉及文件（seq 与回合序对齐，无记录=存量会话走兜底）
    let turns: std::collections::HashMap<i64, TurnDigest> = sqlx::query_as(
        "SELECT seq, user_input, answer_digest, files FROM turn_logs WHERE session_id = ?1 ORDER BY seq",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map(|rows: Vec<(i64, String, String, String)>| {
        rows.into_iter()
            .map(|(seq, ask, digest, files)| {
                (
                    seq,
                    (ask, digest, serde_json::from_str(&files).unwrap_or_default()),
                )
            })
            .collect()
    })
    .unwrap_or_default();
    let (n, packs) = archive_old_body_impl_keep(items, &user_idx, &turns, keep, session_id);
    let mut written = 0usize;
    for (path, body) in &packs {
        if std::fs::write(path, body.as_bytes()).is_ok() {
            written += 1;
        }
    }
    if n > 0 {
        tracing::info!(
            session = %session_id,
            archived_tasks = n,
            packs = written,
            "回合正文摘要接力归档（原文已落回合包，摘要带指针）"
        );
        record_rewrite(pool, session_id, "body_archive", before_bytes, items).await;
    }
}

/// 正文摘要化·参数化版本（`keep_recent` 可注入，测试能显式传值）。
pub(crate) fn archive_old_body_impl_keep(
    items: &mut Vec<InputItem>,
    user_idx: &[usize],
    turns: &std::collections::HashMap<i64, TurnDigest>,
    keep_recent: usize,
    session_id: &str,
) -> (usize, Vec<(std::path::PathBuf, String)>) {
    let keep_from = user_idx.len().saturating_sub(keep_recent);
    // 从最旧回合起逐段替换（splice 从后往前避免 index 位移）
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for (k, &start) in user_idx.iter().enumerate() {
        if k >= keep_from {
            break;
        }
        let raw_end = user_idx.get(k + 1).copied().unwrap_or(items.len());
        // **配对安全**：终点必须落在"不跨越任何一对调用/回执"的位置，否则删完会剩孤儿回执。
        let end = pair_safe_end(items, start, raw_end);
        if end <= start {
            continue;
        }
        spans.push((k + 1, start, end));
    }
    let mut done = 0usize;
    let mut packs: Vec<(std::path::PathBuf, String)> = Vec::new();
    for (seq, start, end) in spans.into_iter().rev() {
        if start >= items.len() {
            continue;
        }
        // 该回合 user 原问（span 起点是 user 消息）——content 兼容 String 与 Array(input_text)
        let ask = match &items[start] {
            InputItem::Message { content: Value::String(t), .. } => t.clone(),
            InputItem::Message { content: Value::Array(parts), .. } => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        let pack_path = task_pack_path(session_id, seq);
        let original = serde_json::to_value(&items[start..end]).unwrap_or(Value::Null);
        let body = serde_json::json!({
            "任务": seq,
            "说明": "该回合归档前的完整历史（诉求 / 回答 / 工具调用与回执原文）。摘要是索引，本文件是全文——要看细节 read 本文件，勿凭记忆猜。",
            "items": original,
        })
        .to_string();
        let (stored_ask, stored_digest, stored_files) = turns
            .get(&(seq as i64))
            .cloned()
            .unwrap_or_default();
        let ask_full = if !stored_ask.is_empty() { stored_ask } else { ask };
        let files_line = if stored_files.is_empty() {
            String::new()
        } else {
            format!(
                "\n涉及文件（需改动先 read）：{}",
                stored_files
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、")
            )
        };
        // 用户持续指令原话：随摘要**长期存活**（激活窗口只有 30 轮，滑出就再也捞不回来）
        let rule_line = if crate::agent::memory::turn_log::is_standing_rule(&ask_full) {
            format!(
                "\n【用户指令·原话，长期有效】“{}”",
                crate::agent::plan::truncate(&ask_full, 90)
            )
        } else {
            String::new()
        };
        let summary = format!(
            "【回合 #{seq} 已归档：诉求与结论】诉求：{ask_full}
结论：{digest}{files_line}{rule_line}
原文包（该回合全部工具调用与回执）：{pack}（摘要不够用时 read 它取全文，勿凭记忆猜）",
            // 结论同样过回灌字面隔离：归档摘要与「最近回合锚」是**同一条管道**的两个出口，
            digest = if !stored_digest.is_empty() {
                crate::agent::memory::turn_log::isolate_literals(&stored_digest)
            } else {
                "（无结论记录——存量早期回合）".to_string()
            },
            pack = pack_path.display(),
        );
        items.splice(start..end, [InputItem::user_message(&summary)]);
        packs.push((pack_path, body));
        done += 1;
    }
    (done, packs)
}

/// 从回执里抽 spill 路径（抽到就写进路标：模型可直接 read，不必重跑工具）。
pub(crate) fn spill_path_of(out: &str) -> Option<String> {
    for key in ["spill_path\":\"", "全文已落盘："] {
        if let Some(i) = out.find(key) {
            let rest = &out[i + key.len()..];
            let end = rest
                .find(['"', '—', '（', '\n', ']'])
                .unwrap_or(rest.len().min(260));
            let p = rest[..end].trim().replace("\\\\", "\\");
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

/// 回合内保留的 reasoning 条数（更早的占位为单空格）
pub(crate) const TASK_KEEP_REASONING_BYTES: usize = super::govern::th::INTASK_KEEP_REASONING_BYTES;

// 跨回合压「工具调用参数」—— 上下文里**最大的一块**

/// 距今 ≥ 几个回合才压（当前回合内模型还在引用，不能动）。
pub(crate) const CALL_ARG_MIN_AGE_TASKS: usize = super::govern::th::ARG_MIN_AGE_TASKS;
pub(crate) const CALL_ARG_MIN_BYTES: usize = super::govern::th::ARG_MIN_BYTES;
/// 压过的路标键：**幂等判据**（见到它就不再压，避免每轮重复破前缀缓存）。
pub(crate) const CALL_ARG_STUB_MARK: &str = "_stub";

const CALL_ARG_HEAVY_TOOLS: &[&str] = &["run", "write", "modify"];

/// 跨回合压掉"重工具"的调用参数正文。返回压掉几处。
pub(crate) fn stub_old_call_args(
    items: &mut Vec<InputItem>,
    user_idx: &[usize],
    min_age_tasks: usize,
    min_bytes: usize,
) -> usize {
    let task_count = user_idx.len();
    if task_count <= min_age_tasks {
        return 0;
    }
    let mut n = 0usize;
    for ti in 0..task_count - min_age_tasks {
        let start = user_idx[ti];
        let end = user_idx.get(ti + 1).copied().unwrap_or(items.len());
        for i in start..end {
            let InputItem::Message {
                tool_calls: Some(tcs),
                ..
            } = &mut items[i]
            else {
                continue;
            };
            for tc in tcs.iter_mut() {
                if stub_one_call_args(tc, min_bytes) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// 压单个调用：把 `arguments` 整体换成**合法 JSON 路标**（定位信息 + 省略说明）。
fn stub_one_call_args(tc: &mut Value, min_bytes: usize) -> bool {
    let Some(name) = tc.get("name").and_then(|v| v.as_str()).map(str::to_string) else {
        return false;
    };
    if !CALL_ARG_HEAVY_TOOLS.contains(&name.as_str()) {
        return false;
    }
    // 唯一判据复核（`govern::verdict`）：参数是 **T1 完成即失效** —— 动作结束、产物已落盘，
    if super::govern::verdict(
        super::govern::kind_of_tool(&name),
        super::govern::Stage::JustDone,
        false,
    ) == super::govern::Action::Keep
    {
        return false;
    }
    let Some(arg) = tc.get("arguments") else {
        return false;
    };
    let text = match arg {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    // 太小不动；已压过不动（幂等 —— 这是"不重复破缓存"的关键）
    if text.len() < min_bytes || text.contains(CALL_ARG_STUB_MARK) {
        return false;
    }
    // 定位行 = 共享原语（`call_locator`）—— 与归档 stub 同一份取键规则
    let loc = call_locator(&name, &text);
    let mut obj = serde_json::Map::new();
    obj.insert(CALL_ARG_STUB_MARK.to_string(), Value::String(loc));
    obj.insert(
        "note".to_string(),
        Value::String(format!(
            "原参数 {} 字符已省略（{name} 的正文对后续轮次无用）；要现内容请直接 read 该文件 / 重跑该命令",
            text.len()
        )),
    );
    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
        let inner = parsed.get("change_spec").unwrap_or(&parsed);
        let mut hint: Vec<String> = Vec::new();
        for k in ["find", "old", "replace", "new", "content", "script"] {
            if let Some(sv) = inner.get(k).and_then(|x| x.as_str()) {
                if let Some(first) = sv.lines().find(|l| !l.trim().is_empty()) {
                    hint.push(format!(
                        "{k}={}",
                        first.trim().chars().take(80).collect::<String>()
                    ));
                }
            }
        }
        if !hint.is_empty() {
            obj.insert("_was".to_string(), Value::String(hint.join(" · ")));
        }
    }
    tc["arguments"] = Value::String(Value::Object(obj).to_string());
    true
}

/// 绝对体积硬顶（纯函数）：话题连续会话引用判据永不触发 → 上下文无限膨胀。
pub(crate) fn archive_hard_cap(
    items: &mut Vec<InputItem>,
    user_idx: &[usize],
    target_bytes: usize,
    min_gain_pct: usize,
) -> usize {
    let raw_bytes = |items: &[InputItem]| -> usize {
        items
            .iter()
            .filter_map(|it| match it {
                InputItem::Raw(v) => serde_json::to_string(v).ok().map(|j| j.len()),
                InputItem::FunctionCallOutput { .. } => serde_json::to_string(it).ok().map(|j| j.len()),
                _ => None,
            })
            .sum()
    };
    let mut acc = raw_bytes(items);
    // 收益门槛（滞回）：超出的份额不够多 → 不值得为它作废整条缓存
    if acc <= target_bytes || (acc - target_bytes) * 100 < acc * min_gain_pct {
        return 0;
    }
    let mut n = 0usize;
    for ti in 0..user_idx.len().saturating_sub(1) {
        if acc <= target_bytes {
            break;
        }
        let start = user_idx[ti];
        let end = user_idx.get(ti + 1).copied().unwrap_or(items.len());
        let before = raw_bytes(&items[start..end]);
        if before == 0 {
            continue;
        }
        stub_span(items, start, end);
        let after = raw_bytes(&items[start..end]);
        if after >= before {
            continue;
        }
        acc = acc.saturating_sub(before - after);
        n += 1;
    }
    n
}

/// **退场判据（唯一）**：旧回合（`1..task_count`）里哪些该退场。
pub(crate) fn retire_old_tasks(
    task_count: usize,
    kw: &std::collections::HashMap<usize, Vec<String>>,
    input: &std::collections::HashMap<usize, String>,
    window: usize,
    fresh_rounds: usize,
) -> Vec<usize> {
    let window_start = task_count.saturating_sub(window) + 1;
    let mut out = Vec::new();
    for i in 1..task_count {
        // ① 年龄硬判据：超龄一律退场（本轮新增的那条腿）
        if task_count - i > fresh_rounds {
            out.push(i);
            continue;
        }
        // ② 年龄未超 ⇒ 再看引用断续
        let kws_i = match kw.get(&i) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        let mut referenced = false;
        for j in window_start..=task_count {
            if j <= i {
                continue;
            }
            let later_input = input.get(&j).cloned().unwrap_or_default();
            let later_kws = kw.get(&j).cloned().unwrap_or_default();
            if kws_i.iter().any(|k| {
                later_input.contains(k.as_str())
                    || later_kws
                        .iter()
                        .any(|lk| lk.contains(k.as_str()) || k.contains(lk.as_str()))
            }) {
                referenced = true;
                break;
            }
        }
        if !referenced {
            out.push(i);
        }
    }
    out
}

/// 区间内**可回收字节**的预估（不改长度、不做快照回滚）
pub(crate) fn reclaimable_bytes(items: &[InputItem], start: usize, end: usize) -> usize {
    let end = end.min(items.len());
    (start..end)
        .filter_map(|i| tool_out_text(&items[i]).map(str::len))
        .map(|n| n.saturating_sub(super::govern::th::STUB_EST_BYTES))
        .sum()
}

pub fn reasoning_placeholder_enabled() -> bool {
    !matches!(
        std::env::var("REAL_REASONING_PLACEHOLDER")
            .map(|v| v.trim().to_lowercase())
            .as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

/// 旧回合区间工具输出退场（= `stub_span` 的一个调用形态，**不再各写一份循环**）。
pub(crate) fn archive_tasks_before(
    items: &mut Vec<InputItem>,
    last_start: usize,
    gain_pct: usize,
) -> usize {
    let end = last_start.min(items.len());
    let before = items_bytes(items);
    if before == 0 || end == 0 {
        return 0;
    }
    let reclaimable = reclaimable_bytes(items, 0, end);
    if reclaimable * 100 < before * gain_pct {
        return 0;
    }
    stub_span(items, 0, end)
}
