//! 无进展止损域：判定「这段时间有没有产出」，只陈述事实，不判定模型做错了什么。
//!
//! 从 `workflow.rs` 归位而来。这里放**不依赖 `Ledger` 的纯判据**：
//! - `stall_reason_of` —— 判据本体（纯标量入参，便于单测）
//! - `empty_spin_signal` —— 结构性空响应特征（1-token 死循环）
//! - `edit_signature` —— 编辑签名（判「是否在同一处反复改」）
//!
//! 吃 `Ledger` 的适配层 `stall_reason` 留在 `workflow.rs`（它只负责把账本摊成标量）。

use crate::model::types::StreamResult;
use serde_json::Value;

/// 空转特征：0 工具 + 0 文本 + 输出≤2 token + 思考<40 字（1-token 死循环）。
pub fn empty_spin_signal(resp: &StreamResult) -> bool {
    resp.function_calls.is_empty()
        && resp.output_text.trim().is_empty()
        && resp.usage.as_ref().map(|u| u.output_tokens <= 2).unwrap_or(true)
        && resp.reasoning.trim().chars().count() < 40
}

/// 编辑签名：modify=hash(file+mode+find/replace 前缀)；write=hash(file+content 前缀+长度)。
pub fn edit_signature(tool: &str, args: &Value) -> Option<(String, u64)> {
    use std::hash::{Hash, Hasher};
    let file = args
        .get("file")
        .or_else(|| args.get("path"))
        .or_else(|| args.get("target"))
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    if file.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file.hash(&mut hasher);
    if tool == "modify" {
        let cs = args.get("change_spec").unwrap_or(args);
        let mode = cs.get("mode").and_then(|v| v.as_str()).unwrap_or("block");
        let find = cs
            .get("find")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(80).collect::<String>());
        let line = cs.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
        let replace = cs
            .get("replace")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(80).collect::<String>());
        mode.hash(&mut hasher);
        find.hash(&mut hasher);
        line.hash(&mut hasher);
        replace.hash(&mut hasher);
    } else {
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        content.chars().take(80).collect::<String>().hash(&mut hasher);
        content.chars().count().hash(&mut hasher);
    }
    Some((file, hasher.finish()))
}

/// 判据本体（纯标量入参 + 阈值显式传入，便于单测——`Ledger` 有 30+ 字段，整体构造成本高）。
pub fn stall_reason_of(
    stall_count: u32,
    rounds: u32,
    last_edit_round: u32,
    no_files: bool,
    stop_rounds: u32,
    dry_rounds: u32,
    min_rounds: u32,
) -> Option<&'static str> {
    // 总门限：短任务一律不判（保护正常短任务不被误伤）。
    // 注：曾有一档「零起点空转」排在本门限之前，实测**恒不触发**——`last_explore_round`
    // 在任务首轮即被「首读新文件 / 首次新命令」置位，20+ 轮级任务里该条件恒假
    // （全量日志 grep「停滞唤醒/无进展止损」零命中）。已撤，不保留死代码。
    if rounds < min_rounds {
        return None;
    }
    // ① 曾有实体进展、但已连续 stop_rounds 个零进展轮 ⇒ 大概率卡在同一处反复试（**从后面抓**）。
    if last_edit_round > 0 && stall_count >= stop_rounds {
        return Some(
            "无进展止损：已连续多个零进展轮无任何新产出（默认 12 轮，env REAL_STALL_STOP_ROUNDS 可调），\
疑似在绕圈——已收口并交付现有发现（省下的轮次即省下的费用）。\
这不是封死：**直接回复『继续』（可附新思路或纠偏信息），将带着断点与已有发现续跑**；也可把剩余工作拆小后重发。",
        );
    }
    // ② 从未改过文件的长任务 ⇒ 按"只诊断不改动"看待，到点也该有结论了。
    if last_edit_round == 0 && no_files && rounds >= dry_rounds {
        return Some(
            "无进展止损：本任务轮数已达兜底线且未产生任何文件改动（默认 30 轮，env REAL_STALL_DRY_ROUNDS 可调，\
按纯诊断任务处理），已收口并交付已查明的结论。\
若还需继续：**直接回复『继续』（可附新方向）即可带断点续跑**，或拆成更小的题重发。",
        );
    }
    None
}

/// 久探不产判据：**有过改动**（是干活任务）、**探索活跃**（`stall_count` 为 0，说明每轮都在读新东西）、
/// 但**距上次动手已满 `idle_rounds`** ⇒ 返回一次提醒文本（None = 无需提醒）。
///
/// 与 `stall_reason_of` 的分工（两者互补，判据不重叠）：
/// - `stall_reason_of` 管「原地打转」——复读旧文件/重跑同命令 ⇒ `stall_count` 涨，到点收窄/停机；
/// - 本判据管「一直在读**新**东西、却迟迟不动手」——这类任务 `stall_count` **恒为 0**，
///   上面那族永远判不到，会一路烧到轮次上限。**只提醒、不停机**，符合柔性化。
pub fn idle_explore_hint(
    rounds: u32,
    last_edit_round: u32,
    stall_count: u32,
    idle_rounds: u32,
) -> Option<String> {
    // 从未改过 = 纯诊断/问答任务，交给 dry 档兜底，不在这里催。
    if last_edit_round == 0 {
        return None;
    }
    // 已在原地打转的，交给 warn/narrow/stop 那族（它们的文案更贴切，且带收窄动作）。
    if stall_count > 0 {
        return None;
    }
    let idle = rounds.saturating_sub(last_edit_round);
    if idle < idle_rounds {
        return None;
    }
    Some(format!(
        "【久探未动手】你已连续 {idle} 轮只在读、没落过一处改动（上次动手在第 {last_edit_round} 轮）——\
读够就该动手了。请二选一：\
① 现在就改：把已查明的部分先落地，不确定的留 TODO 下一轮补；\
② 说清卡点：把你的判断和还缺的信息讲出来向用户提问，别继续无产出地读下去。"
    ))
}
