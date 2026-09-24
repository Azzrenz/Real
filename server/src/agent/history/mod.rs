//! 历史装配（**唯一入口** `build_history`）。

mod archive;
mod govern;
mod intask;
mod item;
mod pairing;
mod retire;
mod window;

pub(crate) use archive::*;
pub(crate) use govern::th;
pub(crate) use intask::*;
pub(crate) use item::*;
pub(crate) use pairing::*;
pub(crate) use retire::*;
pub(crate) use window::*;

// 历史装配域：历史装配变换/去重/归档归本域，workflow.rs 只留编排主循环与账本。

use crate::error::AppResult;
use crate::model::types::InputItem;
use serde_json::Value;
use std::collections::HashSet;

// 历史重建（多轮会话前缀）

/// 历史重建的产物。
pub struct History {
    /// 从库重建出来的历史条目。
    pub items: Vec<InputItem>,
    /// **当轮用户轮的库内行 id**（仅当重建后末条是 user 消息时给出）。
    pub last_user_id: Option<String>,
}

/// 生效的历史窗口低线：`REAL_HISTORY_WINDOW` 环境变量优先，否则用常量默认值。
pub(crate) fn history_window_low() -> usize {
    std::env::var("REAL_HISTORY_WINDOW")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(HISTORY_WINDOW_LOW)
}

pub async fn build_history(pool: &sqlx::SqlitePool, session_id: &str) -> AppResult<History> {
    let mut msgs = crate::db::repos::list_messages(pool, session_id).await?;
    let low = history_window_low();
    let high = low * 3;
    let talk: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user" || m.role == "assistant")
        .map(|(i, _)| i)
        .collect();
    let key = format!("hist_start:{session_id}");
    let stored: Option<usize> = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key = ?1",
    )
    .bind(&key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .and_then(|v| v.parse().ok());
    // 窗口起点决策要"**真实 input**"与"**当前预算**"两把尺子（动态判据：按体积不按条数）——
    let input_tokens = session_input_tokens(pool, session_id).await;
    // 登记本会话压力，供**动态 spill 阈值**（`th::spill_threshold_for`）使用。
    crate::mcp::spill::set_session_pressure(session_id, input_tokens.max(0) as u64);
    // 启动线 = **窗口安全线**（与 ①`archive_old_tasks` / ⑤`archive_old_body` 同一把尺子）。
    let budget = govern::window_safe_tokens() as u64;
    let (start, write_anchor) =
        window_start_decision(stored, talk.len(), low, high, input_tokens, budget);
    if write_anchor {
        tracing::info!(
            session = %session_id,
            talk = talk.len(),
            start,
            moved = stored.is_some(),
            "锚定窗口：建锚/前移（本次为新锚点，前缀将从这里起算）"
        );
        put_setting(pool, &key, &start.to_string()).await;
    }
    if talk.len() > start {
        msgs.drain(..talk[start]);
    }
    let mut items = Vec::new();
    let mut item_ids: Vec<Option<String>> = Vec::new();
    let mut item_orig: Vec<String> = Vec::new();
    let mut assistant_call_ids: HashSet<String> = HashSet::new();
    // 第一遍：收 assistant 的 call_id（fc→output 配对依据）
    for m in &msgs {
        if m.role == "assistant" {
            if let Some(ij) = m.item_json.as_deref() {
                if let Ok(it) = serde_json::from_str::<InputItem>(ij) {
                    for cid in call_ids_of(&it) {
                        assistant_call_ids.insert(cid.to_string());
                    }
                }
            }
        }
    }
    // 第二遍：按序还原
    let rows_in = msgs.len();
    let mut tool_orphans: Vec<String> = Vec::new();
    for m in msgs {
        match m.role.as_str() {
            "assistant" | "user" => {
                // 已被取消的插话（cancel 时作废）：跳过——它从未被消费，
                if let Some(ij) = m.item_json.as_deref() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(ij) {
                        if v.get("interjection").and_then(|x| x.as_bool()).unwrap_or(false)
                            && v.get("cancelled").and_then(|x| x.as_bool()).unwrap_or(false)
                        {
                            continue;
                        }
                    }
                }
                if let Some(ij) = m.item_json.as_deref() {
                    if let Ok(it) = serde_json::from_str::<InputItem>(ij) {
                        // 基准必须与 `persist_rewrites` 的比较式**同一个来源**（都是
                        item_orig.push(serde_json::to_string(&it).unwrap_or_default());
                        item_ids.push(Some(m.id.clone()));
                        items.push(it);
                        continue;
                    }
                }
                if !m.content.trim().is_empty() {
                    let it = if m.role == "user" {
                        InputItem::user_message(&m.content)
                    } else {
                        InputItem::assistant_message(&m.content)
                    };
                    item_orig.push(serde_json::to_string(&it).unwrap_or_default());
                    item_ids.push(Some(m.id.clone()));
                    items.push(it);
                }
            }
            "reasoning" => {
                if let Some(ij) = m.item_json.as_deref() {
                    if let Ok(it) = serde_json::from_str::<InputItem>(ij) {
                        // 同 assistant/user 分支：基准统一取 `to_string(&item)`，见上。
                        item_orig.push(serde_json::to_string(&it).unwrap_or_default());
                        item_ids.push(Some(m.id.clone()));
                        items.push(it);
                    }
                }
            }
            "tool" => {
                if let Some(ij) = m.item_json.as_deref() {
                    if let Ok(v) = serde_json::from_str::<Value>(ij) {
                        if v.get("name").is_some() && v.get("arguments").is_some() {
                            continue;
                        }
                        if let Some(cid) = v.get("call_id").and_then(|c| c.as_str()) {
                            if assistant_call_ids.contains(cid) {
                                let it = match crate::model::types::normalize_raw_item(&v) {
                                    Some(typed) => typed,
                                    None => InputItem::Raw(v),
                                };
                                item_orig.push(serde_json::to_string(&it).unwrap_or_default());
                                item_ids.push(Some(m.id.clone()));
                                items.push(it);
                            } else {
                                // ⚠️ **配对断裂**：声明它的那条 assistant 已不在历史里（被改写 / 被裁掉），
                                tool_orphans.push(cid.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // 孤儿工具回执记账（见 "tool" 分支里那段说明）
    if !tool_orphans.is_empty() {
        tracing::warn!(
            session = %session_id,
            rows = rows_in,
            emitted = items.len(),
            dropped = tool_orphans.len(),
            call_ids = %tool_orphans.join(","),
            "重建丢弃了孤儿工具回执：声明它的 assistant 已不在历史里 ⇒ items 少若干条，前缀将从这些位置整体左移"
        );
    }
    // 当轮用户轮的库内行 id —— **供调用方把"当轮帧"写回同一行**。
    let last_user_id: Option<String> = match (items.last(), item_ids.last()) {
        (Some(InputItem::Message { role, .. }), Some(Some(id))) if role == "user" => {
            Some(id.clone())
        }
        _ => None,
    };
    // ① 不改变条数的改写：回合边界归档（路标 stub / 硬顶 / 参数压缩）+ 附件退场 + 读去重
    let idle_seconds = session_idle_seconds(pool, session_id).await;
    let cold = govern::cache_is_cold(idle_seconds);
    let may_rewrite = govern::assembly_rewrite_allowed(idle_seconds, input_tokens);
    tracing::info!(
        session = %session_id,
        idle_seconds,
        input_tokens,
        cold,
        may_rewrite,
        deep_cut = govern::deep_cut_tokens(cold),
        "装配期：改写闸门判定（冷或越线才动）"
    );
    if may_rewrite {
        // ① 深压（回合边界归档）—— 与 ②③ 同闸门：热缓存下不动历史一根毫毛
        archive_old_tasks(&mut items, pool, session_id, cold).await;
        // 收益门槛（**②③③b 三条路径同源**，冷缓存时归零）
        let gate = |before: usize| -> usize {
            if cold {
                0
            } else {
                govern::rewrite_min_saved_bytes(before, input_tokens)
            }
        };
        // 附件归档（②）：图片/文档传完 2-3 轮即压成占位——用户从本地
        let before_attachments = items_bytes(&items);
        stale_attachments(&mut items, STALE_ATTACH_KEEP_TASKS, gate(before_attachments));
        record_rewrite(pool, session_id, "attachments", before_attachments, &items).await;
        // 读去重（③）
        let before_read_dedup = items_bytes(&items);
        dedup_read_outputs(&mut items, gate(before_read_dedup));
        record_rewrite(pool, session_id, "read_dedup", before_read_dedup, &items).await;
        // 旧帧瘦身（③b）：帧里的记忆与规范**每轮都会被新帧重新渲染**，历史里那些是陈旧副本。
        let before_frames = items_bytes(&items);
        trim_stale_frames(&mut items, STALE_FRAME_KEEP_TASKS, gate(before_frames));
        record_rewrite(pool, session_id, "frames", before_frames, &items).await;
    } else {
        tracing::info!(
            session = %session_id,
            idle_seconds,
            input_tokens,
            window_safe_tokens = govern::window_safe_tokens(),
            "装配期改写闸门：热缓存且未越窗口线 —— 本轮跳过去冗余改写，保住前缀缓存"
        );
    }
    // ④ 退场落库：把上面的改写写回库（原文先落 spill）——此后重建读到同一形态。
    persist_rewrites(pool, session_id, &items, &item_ids, &item_orig).await;
    // ⑤ 正文摘要接力（**改长度**：整段 splice 成一条摘要 + 原文落回合包）——
    if may_rewrite {
        archive_old_body(&mut items, pool, session_id, cold).await;
    }
    // ⑥ 工具配对闭合（**唯一出口**，必须放在所有改写之后；**不受闸门约束** ——
    let _ = close_tool_pairing(
        &mut items,
        "回合被中断：该工具调用声明后未执行（服务重启/取消），此输出为配对闭合占位——需要则重新发起",
    );
    // ⑦ 发送前按比例裁：历史只留窗口的 `SEND_SAFE_PERMILLE`‰，多的从最老处整条删。
    let before_ceiling = items_bytes(&items);
    let dropped = enforce_token_ceiling(&mut items);
    if dropped > 0 {
        record_rewrite(pool, session_id, "send_cap", before_ceiling, &items).await;
        tracing::warn!(
            session = %session_id,
            dropped_items = dropped,
            remaining = items.len(),
            est_tokens = items_bytes(&items) * 100 / th::WINDOW_B_PER_TOKEN_X100 as usize,
            cap_tokens = (th::WINDOW_TOKENS * th::SEND_SAFE_PERMILLE / 1000) as usize,
            "发送前 token 兜底：历史越线，已从最老处丢弃（否则上游 400 ⇒ 前端「执行失败」）"
        );
        // 丢完再闭合一次配对：切点在 user 上正常不会断，但此处是安全网，宁可多跑一次。
        let _ = close_tool_pairing(
            &mut items,
            "回合被中断：该工具调用声明后未执行（服务重启/取消），此输出为配对闭合占位——需要则重新发起",
        );
    }
    // 重建指纹（`phase=rebuild`）：与 `orchestration::cache_probe`（`phase=round`）**同一套格式**。
    cache_probe(session_id, &items, "rebuild");
    Ok(History { items, last_user_id })
}

/// 发送前**按比例裁剪**：历史只留窗口的 `SEND_SAFE_PERMILLE`‰（现值 70‰，真值见 `govern::th`）。
pub(crate) fn enforce_token_ceiling(items: &mut Vec<InputItem>) -> usize {
    let cap = govern::send_safe_tokens() as usize;
    // 字节 → 估算 token（与 `window_safe_bytes` 同一把尺子：WINDOW_B_PER_TOKEN_X100）
    let est = |b: usize| -> usize { b * 100 / th::WINDOW_B_PER_TOKEN_X100 as usize };
    if est(items_bytes(&items[..])) <= cap {
        return 0;
    }
    // 合法切点：user 消息下标。从「最后一条 user」往回找，取**最早的**那个
    let user_idx: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
        .map(|(i, _)| i)
        .collect();
    // 升序 `find`：命中「最小的合法切点」⇒ drain 掉的前缀最短 ⇒ 保留最多历史。
    let cut = user_idx
        .iter()
        .copied()
        .find(|&ui| est(items_bytes(&items[ui..])) <= cap)
        // 兜底：连「最后一条 user 起」都超线（单条巨大），那就只保这一段 ——
        .or_else(|| user_idx.last().copied())
        .unwrap_or(0);
    if cut > 0 {
        items.drain(..cut);
    }
    cut
}

#[cfg(test)]
mod tests;
