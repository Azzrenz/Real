//! 工具配对与回执闭合（**唯一出口**）。
use crate::model::types::InputItem;
use serde_json::{json, Value};

// ── 配对闭合的实现要点 ────────────────────────────────────────────────────

/// 该条目是不是工具回执；是则给出其 call_id（Raw 形态一并覆盖）。
pub(crate) fn out_at_call_id(it: &InputItem) -> Option<&str> {
    match it {
        InputItem::FunctionCallOutput { call_id, .. } => Some(call_id.as_str()),
        InputItem::Raw(v) => {
            let t = v.get("type").and_then(|x| x.as_str());
            if t == Some("function_call_output") || (t.is_none() && v.get("output").is_some()) {
                v.get("call_id").and_then(|c| c.as_str())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 把回执挂到指定 call_id（改名时用）。
pub(crate) fn out_set_call_id(it: &mut InputItem, to: &str) {
    match it {
        InputItem::FunctionCallOutput { call_id, .. } => *call_id = to.to_string(),
        InputItem::Raw(v) => {
            if let Some(o) = v.as_object_mut() {
                o.insert("call_id".into(), Value::String(to.to_string()));
            }
        }
        _ => {}
    }
}

/// 声明签名：判定同一 call_id 的多次出现是否"同一次调用被重复追加"。
pub(crate) fn decl_sig(it: &InputItem) -> String {
    match it {
        InputItem::Message { tool_calls: Some(tcs), .. } => tcs
            .iter()
            .map(|tc| {
                format!(
                    "{}\u{1}{}",
                    tc.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                    tc.get("arguments").and_then(|x| x.as_str()).unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\u{2}"),
        InputItem::FunctionCall { name, arguments, .. } => format!("{name}\u{1}{arguments}"),
        InputItem::Raw(v) => format!(
            "{}\u{1}{}",
            v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("arguments").and_then(|x| x.as_str()).unwrap_or("")
        ),
        _ => String::new(),
    }
}

/// 声明里每个 call 的去留与最终名。
pub(crate) struct DeclSlot {
    keep: bool,
    final_cid: String,
    out_idx: Option<usize>,
}

/// 补一条配对闭合占位回执。
pub(crate) fn placeholder_output(cid: &str, note: &str) -> InputItem {
    let payload = serde_json::json!({
        "status": "error",
        "is_error": true,
        "content": [{"type": "text", "text": note}]
    })
    .to_string();
    InputItem::Raw(json!({
        "type": "function_call_output",
        "call_id": cid,
        "output": payload
    }))
}

/// 按最终名改写声明的 call_id（Message 形态重建 tool_calls，丢掉的条目不再出现）。
pub(crate) fn apply_decl_slots(it: &mut InputItem, slots: &[DeclSlot]) {
    match it {
        InputItem::FunctionCall { call_id, .. } => {
            if let Some(s) = slots.first() {
                *call_id = s.final_cid.clone();
            }
        }
        InputItem::Raw(v) => {
            if let (Some(s), Some(o)) = (slots.first(), v.as_object_mut()) {
                o.insert("call_id".into(), Value::String(s.final_cid.clone()));
            }
        }
        InputItem::Message { tool_calls: Some(tcs), .. } => {
            let mut rebuilt: Vec<Value> = Vec::with_capacity(slots.len());
            for (i, tc) in tcs.iter().enumerate() {
                if let Some(s) = slots.get(i) {
                    if !s.keep {
                        continue;
                    }
                    let mut tc = tc.clone();
                    if let Some(o) = tc.as_object_mut() {
                        o.insert("call_id".into(), Value::String(s.final_cid.clone()));
                    }
                    rebuilt.push(tc);
                }
            }
            *tcs = rebuilt;
        }
        _ => {}
    }
}

/// 工具配对闭合（唯一出口）。返回 `(剔除条数, 改名条数, 补占位条数)`。
pub fn close_tool_pairing(items: &mut Vec<InputItem>, note: &str) -> (usize, usize, usize) {
    use std::collections::{HashMap, HashSet, VecDeque};

    let mut slots: Vec<Option<InputItem>> =
        std::mem::take(items).into_iter().map(Some).collect();

    // ---- 0. 回执池：call_id → 回执下标（按出现序，**不看位置先后**） ----------
    let mut pool: HashMap<String, VecDeque<usize>> = HashMap::new();
    let mut is_out: HashSet<usize> = HashSet::new();
    for (i, s) in slots.iter().enumerate() {
        if let Some(it) = s {
            if let Some(cid) = out_at_call_id(it) {
                pool.entry(cid.to_string()).or_default().push_back(i);
                is_out.insert(i);
            }
        }
    }

    // ---- 1. 逐条声明定分配：保留 / 改名 / 丢重复 / 配回执 -------------------
    let mut used_ids: HashSet<String> = HashSet::new();
    for s in slots.iter() {
        if let Some(it) = s {
            for cid in call_ids_of(it) {
                used_ids.insert(cid.to_string());
            }
        }
    }
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut taken: HashSet<usize> = HashSet::new();
    let mut plan: HashMap<usize, Vec<DeclSlot>> = HashMap::new();
    let mut renamed = 0usize;
    let mut filled = 0usize;

    for (i, s) in slots.iter().enumerate() {
        let ids = match s {
            Some(it) => call_ids_of(it),
            None => continue,
        };
        if ids.is_empty() {
            continue;
        }
        let sig = match s {
            Some(it) => decl_sig(it),
            None => continue,
        };
        let mut out_slots: Vec<DeclSlot> = Vec::with_capacity(ids.len());
        for cid in ids {
            let key = (cid.to_string(), sig.clone());
            if seen.contains(&key) {
                // 同 id 同内容：这是"同一次调用被重复追加"的第二份 → 丢弃
                out_slots.push(DeclSlot { keep: false, final_cid: cid.to_string(), out_idx: None });
                continue;
            }
            let mut final_cid = cid.to_string();
            if seen.iter().any(|(c, _)| c == cid) {
                // 同 id 不同内容：改名保留，不静默丢信息
                let mut n = 2;
                while used_ids.contains(&format!("{cid}-d{n}")) {
                    n += 1;
                }
                final_cid = format!("{cid}-d{n}");
                used_ids.insert(final_cid.clone());
                renamed += 1;
            }
            seen.insert((final_cid.clone(), sig.clone()));
            let out_idx = pool
                .get_mut(cid)
                .and_then(|q| q.pop_front())
                .filter(|oi| taken.insert(*oi));
            if out_idx.is_none() {
                // 声明没有回执可配 → 就地补占位（**绝不让声明裸奔**）
                filled += 1;
            }
            out_slots.push(DeclSlot { keep: true, final_cid, out_idx });
        }
        plan.insert(i, out_slots);
    }

    // ---- 2. 装配：非回执项按原序发出；存活声明就地发出，正后方紧跟其回执 -----
    let mut out: Vec<InputItem> = Vec::with_capacity(slots.len());
    let mut dropped = 0usize;
    for i in 0..slots.len() {
        if is_out.contains(&i) {
            if !taken.contains(&i) {
                dropped += 1;
            }
            continue;
        }
        let it = match slots[i].take() {
            Some(it) => it,
            None => continue,
        };
        match plan.get(&i) {
            None => out.push(it),
            Some(decl_slots) => {
                if !decl_slots.iter().any(|s| s.keep) {
                    dropped += 1;
                    continue;
                }
                let mut it = it;
                apply_decl_slots(&mut it, decl_slots);
                out.push(it);
                for s in decl_slots.iter().filter(|s| s.keep) {
                    match s.out_idx {
                        Some(oi) => match slots[oi].take() {
                            Some(mut o) => {
                                out_set_call_id(&mut o, &s.final_cid);
                                out.push(o);
                            }
                            None => out.push(placeholder_output(&s.final_cid, note)),
                        },
                        None => out.push(placeholder_output(&s.final_cid, note)),
                    }
                }
            }
        }
    }

    *items = out;
    if dropped > 0 || renamed > 0 || filled > 0 {
        tracing::warn!(
            removed = dropped,
            renamed,
            filled,
            "配对闭合：按声明重建回执（剔孤儿/并重复/补占位）"
        );
    }
    (dropped, renamed, filled)
}

// ---- 装配期入口：请求装配的最后一道，必须在所有改写之后调 -------------------

pub fn reconcile_for_request(items: &[InputItem]) -> Vec<InputItem> {
    let mut out = items.to_vec();
    close_tool_pairing(
        &mut out,
        "该工具**仍在执行中**，输出将在其后到达；此输出仅为配对闭合占位——不要基于它做判断",
    );
    out
}

/// 不变式自检：返回违规说明（空 = 配对闭合完好）。
#[cfg(test)]
pub(crate) fn pairing_violations(items: &[InputItem]) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for it in items {
        if let Some(cid) = out_at_call_id(it) {
            if open.is_empty() {
                bad.push(format!("孤儿回执：{cid}"));
                continue;
            }
            match open.iter().position(|c| c == cid) {
                Some(p) => {
                    open.remove(p);
                }
                None => bad.push(format!("回执 {cid} 与待配声明 {:?} 不匹配", open)),
            }
        } else {
            let ids = call_ids_of(it);
            if ids.is_empty() {
                if !open.is_empty() {
                    bad.push(format!("待配声明 {:?} 与回执之间夹了非回执项", open));
                }
            } else {
                if !open.is_empty() {
                    bad.push(format!("声明 {:?} 未闭合就来了新声明 {:?}", open, ids));
                }
                open = ids.iter().map(|s| s.to_string()).collect();
            }
        }
    }
    if !open.is_empty() {
        bad.push(format!("声明裸奔（无回执）：{open:?}"));
    }
    bad
}

// ---- 测试适配：旧的两个方向已并入 `close_tool_pairing` 的唯一出口 ----------

#[cfg(test)]
pub(crate) fn drop_orphan_outputs(items: &mut Vec<InputItem>) -> usize {
    close_tool_pairing(items, "（测试）配对闭合占位").0
}

#[cfg(test)]
pub(crate) fn reconcile_dangling_calls(items: &mut Vec<InputItem>) -> usize {
    close_tool_pairing(
        items,
        "回合被中断：该工具调用声明后未执行（服务重启/取消），此输出为配对闭合占位——需要则重新发起",
    )
    .2
}

/// 一条条目**引入**的调用 id（三种载体：FunctionCall / assistant.tool_calls / Raw function_call）。
pub(crate) fn call_ids_of(it: &InputItem) -> Vec<&str> {
    match it {
        InputItem::FunctionCall { call_id, .. } => vec![call_id.as_str()],
        InputItem::Message {
            tool_calls: Some(tcs),
            ..
        } => tcs
            .iter()
            .filter_map(|tc| tc.get("call_id").and_then(|c| c.as_str()))
            .collect(),
        InputItem::Raw(v) => {
            if v.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                v.get("call_id").and_then(|c| c.as_str()).into_iter().collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// **归档终点必须不跨越任何一对"调用/回执"**。
pub(crate) fn pair_safe_end(items: &[InputItem], start: usize, mut end: usize) -> usize {
    loop {
        let mut cut: Option<usize> = None;
        for i in start..end {
            for cid in call_ids_of(&items[i]) {
                // 该调用的回执是否存在于段外？（段外才算"会被切开的另一半"）
                let out_elsewhere = items
                    .iter()
                    .enumerate()
                    .any(|(j, _)| (j < start || j >= end) && out_at_call_id(&items[j]) == Some(cid));
                if out_elsewhere {
                    cut = Some(cut.map_or(i, |c: usize| c.min(i)));
                }
            }
        }
        match cut {
            Some(c) if c > start => end = c,
            _ => break,
        }
    }
    if end <= start {
        return start;
    }
    // 反向检查：段内回执的调用若在段外，删掉回执会留下悬空调用 → 这一段整体不删。
    let dangling_out = (start..end).any(|j| {
        out_at_call_id(&items[j]).is_some_and(|cid| {
            !(start..end).any(|i| call_ids_of(&items[i]).contains(&cid))
        })
    });
    if dangling_out {
        return start;
    }
    end
}
