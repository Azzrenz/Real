//! InputItem 原语 —— 度量、识别、整段压缩。**两条通道共用同一份**（唯一实现）。
use crate::model::types::InputItem;
use serde_json::Value;
/// **全量 items 体积**（字节）—— 改写效果的**可测口径**。
pub(crate) fn items_bytes(items: &[InputItem]) -> usize {
    items
        .iter()
        .map(|it| serde_json::to_string(it).map(|j| j.len()).unwrap_or(0))
        .sum()
}

/// 条目类型短名 —— **探针与改写记账共用同一把尺子**。
pub(crate) fn item_kind(it: &InputItem) -> String {
    match it {
        InputItem::Message { role, .. } => format!("msg:{role}"),
        InputItem::FunctionCallOutput { .. } => "tool_out".to_string(),
        InputItem::Reasoning { .. } => "reasoning".to_string(),
        InputItem::FunctionCall { .. } => "call".to_string(),
        InputItem::Raw(_) => "raw".to_string(),
    }
}

/// **前缀指纹** —— 一眼看出"前缀在哪一条断"。
pub(crate) fn cache_probe(session: &str, items: &[InputItem], phase: &str) {
    // 与前一轮的**逐条对齐**（不看盲区）—— 断点定位靠它，见 `diff_with_last_round`。
    if phase == "round" {
        diff_with_last_round(session, items);
    }
    let mode = std::env::var("REAL_CACHE_DEBUG").unwrap_or_else(|_| "1".into());
    if mode == "0" || mode.eq_ignore_ascii_case("off") {
        return;
    }
    use std::hash::{Hash, Hasher};
    let mut total = 0usize;
    let mut parts: Vec<String> = Vec::new();
    let full = mode.eq_ignore_ascii_case("full");
    for (i, it) in items.iter().enumerate() {
        let js = serde_json::to_string(it).unwrap_or_default();
        total += js.len();
        if !(full || i < 12 || i + 6 >= items.len()) {
            continue;
        }
        let kind = item_kind(it);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        js.hash(&mut h);
        parts.push(format!("{i}|{kind}|{}|{:x}", js.len(), h.finish()));
    }
    tracing::info!(
        phase,
        items = items.len(),
        bytes = total,
        probe = %parts.join(" "),
        "cache-probe"
    );
}

/// 与**上一轮发出去的载荷**逐条对比 —— 前缀缓存的断点就是「第一个不同的下标」。
fn diff_with_last_round(session: &str, items: &[InputItem]) {
    use std::hash::{Hash, Hasher};
    fn fp(it: &InputItem) -> (u64, usize) {
        let js = serde_json::to_string(it).unwrap_or_default();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        js.hash(&mut h);
        (h.finish(), js.len())
    }
    static LAST: std::sync::Mutex<
        Option<std::collections::HashMap<String, Vec<(u64, usize)>>>,
    > = std::sync::Mutex::new(None);
    let now: Vec<(u64, usize)> = items.iter().map(fp).collect();
    let Ok(mut guard) = LAST.lock() else { return };
    let map = guard.get_or_insert_with(Default::default);
    let prev = match map.get(session) {
        None => {
            map.insert(session.to_string(), now);
            return;
        }
        Some(p) => p.clone(),
    };
    let prev_len = prev.len();
    let mut first: Option<usize> = None;
    let mut diffs = 0usize;
    let mut at: Vec<String> = Vec::new();
    for (i, cur) in now.iter().enumerate() {
        if prev.get(i) == Some(cur) {
            continue;
        }
        first.get_or_insert(i);
        diffs += 1;
        if at.len() < 8 {
            at.push(format!("{i}|{}|{}", item_kind(&items[i]), cur.1));
        }
    }
    match first {
        // 已发出过的条目被改写：**这是真断点**，不是正常追加。
        Some(f) if f < prev_len => {
            // **白花的钱 = 上一轮从 f 到末尾的那批字节**（它们已经付过一次，现在作废）。
            let old_tail: usize = prev[f..].iter().map(|(_, n)| *n).sum();
            let new_tail: usize = now[f..].iter().map(|(_, n)| *n).sum();
            // 判别"真改写"与"上一轮尾部本来就是一次性的"
            let replaced = prev_len - f;
            if replaced > 1 {
                tracing::warn!(
                    first = f,
                    prev_len,
                    replaced,
                    diffs,
                    old_tail_bytes = old_tail,
                    new_tail_bytes = new_tail,
                    at = %at.join(" "),
                    "cache-probe 对齐：**中段被改写**（不止一条）—— 白付的是上一轮那段字节"
                );
            } else {
                tracing::info!(
                    first = f,
                    prev_len,
                    replaced,
                    old_tail_bytes = old_tail,
                    "cache-probe 对齐：仅上一轮尾部被本轮新增顶掉（一次性便条，非浪费）"
                );
            }
        }
        Some(f) => {
            tracing::info!(
                first = f,
                prev_len,
                new_items = now.len().saturating_sub(prev_len),
                "cache-probe 对齐：纯追加（前缀全命中）"
            );
        }
        None => tracing::info!(prev_len, "cache-probe 对齐：逐条一致"),
    }
    map.insert(session.to_string(), now);
}

/// 是否为"携带 thinking 文本"的条目（独立 Reasoning 条目 / assistant 消息里的 reasoning_text 块）。
pub(crate) fn prefix_probe(
    session: &str,
    instructions_bytes: usize,
    tools: &[crate::model::types::ToolDef],
) {
    let tools_bytes = serde_json::to_string(tools).map(|j| j.len()).unwrap_or(0);
    static LAST: std::sync::Mutex<Option<std::collections::HashMap<String, (usize, usize)>>> =
        std::sync::Mutex::new(None);
    let Ok(mut guard) = LAST.lock() else { return };
    let map = guard.get_or_insert_with(Default::default);
    let now = (instructions_bytes, tools_bytes);
    match map.get(session) {
        None => tracing::info!(
            session,
            instructions_bytes,
            tools_bytes,
            "prefix-probe 基线：本会话首帧的 32KB 前缀（跨重启比对这一行）"
        ),
        Some(&prev) if prev != now => tracing::warn!(
            session,
            prev_instructions = prev.0,
            instructions_bytes,
            prev_tools = prev.1,
            tools_bytes,
            "prefix-probe **漂移**：32KB 前缀变了 —— 此后本会话整段按 miss 重算"
        ),
        Some(_) => {}
    }
    map.insert(session.to_string(), now);
}

pub(crate) fn has_reasoning_text(it: &InputItem) -> bool {
    match it {
        InputItem::Reasoning { .. } => true,
        InputItem::Message { role, content, .. } if role == "assistant" => {
            matches!(content, Value::Array(blocks) if blocks.iter().any(|b| {
                b.get("type").and_then(|t| t.as_str()) == Some("reasoning_text")
            }))
        }
        _ => false,
    }
}

pub(crate) fn reasoning_text_bytes(it: &InputItem) -> usize {
    fn blocks_bytes(blocks: &[Value]) -> usize {
        blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("reasoning_text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .map(str::len)
            .sum()
    }
    match it {
        InputItem::Reasoning { content, summary } => {
            let mut n = 0usize;
            if let Some(Value::Array(b)) = content {
                n += blocks_bytes(b);
            }
            if let Some(b) = summary {
                n += blocks_bytes(b);
            }
            n
        }
        InputItem::Message { role, content, .. } if role == "assistant" => match content {
            Value::Array(b) => blocks_bytes(b),
            _ => 0,
        },
        _ => 0,
    }
}

pub(crate) fn tool_out_call_id(it: &InputItem) -> Option<&str> {
    match it {
        InputItem::FunctionCallOutput { call_id, .. } => Some(call_id.as_str()),
        InputItem::Raw(v) => v.get("call_id").and_then(|c| c.as_str()),
        _ => None,
    }
}

/// 取工具输出正文（两种载体统一）
pub(crate) fn tool_out_text(it: &InputItem) -> Option<&str> {
    match it {
        InputItem::FunctionCallOutput { output, .. } => Some(output.as_str()),
        InputItem::Raw(v) => {
            if v.get("call_id").is_some() {
                v.get("output").and_then(|o| o.as_str())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 覆盖工具输出正文（两种载体统一）；返回是否写入
pub(crate) fn set_tool_out_text(it: &mut InputItem, text: &str) -> bool {
    match it {
        InputItem::FunctionCallOutput { output, .. } => {
            *output = text.to_string();
            true
        }
        InputItem::Raw(v) => {
            if v.get("call_id").is_some() && v.get("output").is_some() {
                v["output"] = Value::String(text.to_string());
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// reasoning 压缩：**保住决策句，弃掉过程句**。
pub(crate) fn placeholder_reasoning_text(it: &mut InputItem) -> bool {
    // 只在**真正发生变化**时返回 true（幂等）：否则每次重建都把已占位的条目再算一次"改写"。
    fn patch(blocks: &mut Vec<Value>) -> bool {
        let mut hit = false;
        for b in blocks.iter_mut() {
            if b.get("type").and_then(|t| t.as_str()) != Some("reasoning_text") {
                continue;
            }
            let cur = b.get("text").and_then(|t| t.as_str()).unwrap_or("");
            if cur == " " || cur.starts_with(CONDENSED_MARK) {
                continue;
            }
            b["text"] = Value::String(condense_reasoning(cur));
            hit = true;
        }
        hit
    }
    match it {
        InputItem::Reasoning { content, summary } => {
            let mut hit = false;
            if let Some(Value::Array(blocks)) = content {
                hit |= patch(blocks);
            }
            if let Some(blocks) = summary {
                hit |= patch(blocks);
            }
            hit
        }
        InputItem::Message { role, content, .. } if role == "assistant" => {
            if let Value::Array(blocks) = content {
                patch(blocks)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// 跨回合推理折叠：**留结构、清内容** —— 零内容进 input。
pub(crate) fn blank_reasoning_text(it: &mut InputItem) -> bool {
    fn patch(blocks: &mut Vec<Value>) -> bool {
        let mut hit = false;
        for b in blocks.iter_mut() {
            if b.get("type").and_then(|t| t.as_str()) != Some("reasoning_text") {
                continue;
            }
            let cur = b.get("text").and_then(|t| t.as_str()).unwrap_or("");
            if cur == FOLDED_MARK {
                continue;
            }
            b["text"] = Value::String(FOLDED_MARK.to_string());
            hit = true;
        }
        hit
    }
    match it {
        InputItem::Reasoning { content, summary } => {
            let mut hit = false;
            if let Some(Value::Array(blocks)) = content {
                hit |= patch(blocks);
            }
            if let Some(blocks) = summary {
                hit |= patch(blocks);
            }
            hit
        }
        InputItem::Message { role, content, .. } if role == "assistant" => {
            if let Value::Array(blocks) = content {
                patch(blocks)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// 统计条目里 `reasoning_text` 块的总字节数（0 = 不含推理）。
pub(crate) fn reasoning_len(it: &InputItem) -> usize {
    fn sum(blocks: &[Value]) -> usize {
        blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("reasoning_text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .map(|t| t.len())
            .sum()
    }
    match it {
        InputItem::Reasoning { content, summary } => {
            let mut n = 0;
            if let Some(Value::Array(b)) = content {
                n += sum(b);
            }
            if let Some(b) = summary {
                n += sum(b);
            }
            n
        }
        InputItem::Message { content, .. } => {
            if let Value::Array(b) = content {
                sum(b)
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// 工具回执**剥空字段** —— 机械判据，不是字段清单。
pub(crate) fn strip_empty_fields(it: &mut InputItem) -> bool {
    fn strip(v: &mut Value) -> bool {
        let mut hit = false;
        match v {
            Value::Object(m) => {
                let keys: Vec<String> = m.keys().cloned().collect();
                for k in keys {
                    let drop = match m.get(&k) {
                        Some(Value::Null) => true,
                        Some(Value::String(s)) => s.is_empty(),
                        Some(Value::Bool(b)) => !*b,
                        _ => false,
                    };
                    if drop {
                        m.remove(&k);
                        hit = true;
                    } else if let Some(child) = m.get_mut(&k) {
                        hit |= strip(child);
                    }
                }
            }
            Value::Array(a) => {
                for child in a.iter_mut() {
                    hit |= strip(child);
                }
            }
            _ => {}
        }
        hit
    }
    let Some(text) = tool_out_text(it).map(str::to_string) else {
        return false;
    };
    let Ok(mut v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    if !strip(&mut v) {
        return false;
    }
    let new = v.to_string();
    if new.len() >= text.len() {
        return false;
    }
    set_tool_out_text(it, &new)
}

/// 从推理内容里摘出**决策句**（零 LLM，纯机械：按判据词挑行）。
fn condense_reasoning(text: &str) -> String {
    const CUES: [&str; 12] = [
        "因为", "所以", "因此", "结论", "原因", "失败", "改为", "应该", "必须", "注意", "问题",
        "根因",
    ];
    const PICK_CHARS: usize = 120;
    const MAX_PICKS: usize = 4;
    let mut picked: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if CUES.iter().any(|c| t.contains(c)) {
            picked.push(t.chars().take(PICK_CHARS).collect());
        }
        if picked.len() >= MAX_PICKS {
            break;
        }
    }
    if picked.is_empty() {
        if let Some(first) = text.lines().find(|l| !l.trim().is_empty()) {
            picked.push(first.trim().chars().take(PICK_CHARS).collect());
        }
    }
    if picked.is_empty() {
        return CONDENSED_MARK.to_string();
    }
    format!("{CONDENSED_MARK}{}", picked.join("；"))
}

/// 工具定位键表（**唯一事实源**）：归档 stub 与跨回合参数压缩共用一份。
pub(crate) fn keep_keys_for(name: &str) -> &'static [&'static str] {
    match name {
        "read" => &["paths", "path"],
        "write" | "modify" => &["path", "file"],
        "run" => &["command", "cwd"],
        "search" => &["pattern", "path"],
        "list" | "find_files" => &["path", "pattern"],
        "web_fetch" => &["url"],
        "audit" => &["path"],
        _ => &[],
    }
}

/// 定位值上限（字符）—— 「钥匙键」不跟着一起截短。
fn locator_cap(tool: &str, key: &str) -> usize {
    if super::govern::is_key_field(tool, key) {
        super::govern::th::LOCATOR_KEY_VALUE_CHARS
    } else {
        super::govern::th::LOCATOR_VALUE_CHARS
    }
}

/// 一条调用的一行定位（`path=D:/x/loop_.rs · mode=full`）。
pub(crate) fn call_locator(name: &str, args: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(args) else {
        return String::new();
    };
    let as_text = |val: &Value| -> String {
        match val {
            Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str())
                .take(2)
                .collect::<Vec<_>>()
                .join(" "),
            Value::String(s) => s.clone(),
            o => o.to_string(),
        }
    };
    let mut parts: Vec<String> = Vec::new();
    for k in keep_keys_for(name) {
        if let Some(val) = v.get(*k) {
            let s = as_text(val);
            if !s.is_empty() {
                parts.push(format!("{k}={}", s.chars().take(locator_cap(name, *k)).collect::<String>()));
            }
        }
    }
    if parts.is_empty() {
        if let Some(o) = v.as_object() {
            if let Some((k, val)) = o.iter().find(|(_, val)| val.is_string()) {
                parts.push(format!(
                    "{k}={}",
                    as_text(val).chars().take(locator_cap(name, k)).collect::<String>()
                ));
            }
        }
    }
    parts.join(" · ")
}

/// 全量 items 的调用定位表：call_id → (工具名, 定位行)。
pub(crate) fn call_locators(
    items: &[InputItem],
) -> std::collections::HashMap<String, (String, String)> {
    let mut m = std::collections::HashMap::new();
    for it in items {
        match it {
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                m.insert(call_id.clone(), (name.clone(), call_locator(name, arguments)));
            }
            InputItem::Message {
                tool_calls: Some(tcs),
                ..
            } => {
                for tc in tcs {
                    let (Some(cid), Some(name)) = (
                        tc.get("call_id").and_then(|c| c.as_str()),
                        tc.get("name").and_then(|n| n.as_str()),
                    ) else {
                        continue;
                    };
                    let args = tc.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                    m.insert(cid.to_string(), (name.to_string(), call_locator(name, args)));
                }
            }
            InputItem::Raw(v) => {
                if v.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    let (Some(cid), Some(name)) = (
                        v.get("call_id").and_then(|c| c.as_str()),
                        v.get("name").and_then(|n| n.as_str()),
                    ) else {
                        continue;
                    };
                    let args = v.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                    m.insert(cid.to_string(), (name.to_string(), call_locator(name, args)));
                }
            }
            _ => {}
        }
    }
    m
}

/// 工具回执正文里的 **spill 全文指针**（`全文已存至 <path>` / `全文已落盘：<path>`）。
pub(crate) fn spill_pointer_of(text: &str) -> Option<String> {
    const MARKS: [&str; 2] = ["全文已存至 ", "全文已落盘："];
    for m in MARKS {
        if let Some(i) = text.find(m) {
            let rest = &text[i + m.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '—' || c == '）' || c == '\n')
                .unwrap_or(rest.len());
            let p = rest[..end].trim();
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
    }
    None
}

pub(crate) fn digest(name: &str, loc: &str, original: &str) -> String {
    let kind = super::govern::kind_of_tool(name);
    let who = if name.is_empty() { "工具" } else { name };
    let head = if loc.is_empty() {
        format!("【{who} · 原文 {} 字符】", original.chars().count())
    } else {
        format!("【{who} {loc} · 原文 {} 字符】", original.chars().count())
    };
    let body = match kind {
        // 探索型：要点 = **结构大纲**。头是版权注释/use 语句，留了等于没留。
        super::govern::ToolKind::Explore => outline_of(original),
        super::govern::ToolKind::Execute => tail_head_of(original),
        // 写入型 / 通讯型：定位已说明对象，要点给首个有意义行。
        _ => first_meaningful_line(original),
    };
    let mut out = head;
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
    }
    match spill_pointer_of(original) {
        Some(p) => out.push_str(&format!(
            "\n（全文可回取：{p}——要细节就 read 它，别重读源文件，那会再进一次上下文）"
        )),
        None => out.push_str("\n（全文已退场；需要细节按原参数重新执行）"),
    }
    out.chars().take(super::govern::th::DIGEST_TOTAL_CHARS).collect()
}

/// 结构大纲（带行号）—— 探索型的要点形态。
fn outline_of(text: &str) -> String {
    let stripped: String = text
        .lines()
        .map(|l| super::strip_lineno(l).trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    let pts = crate::tools::fs_common::extract_structure(&stripped);
    if pts.is_empty() {
        return first_meaningful_line(text);
    }
    let max = super::govern::th::DIGEST_MAX_OUTLINE;
    let mut out = String::from("要点（结构点·带行号，可按区间 read 精读）：");
    for (ln, kind, name) in pts.iter().take(max) {
        out.push_str(&format!("\n  L{ln} {kind} {name}"));
    }
    if pts.len() > max {
        out.push_str(&format!("\n  …另有 {} 个结构点", pts.len() - max));
    }
    out.chars().take(super::govern::th::DIGEST_CHARS).collect()
}

fn tail_head_of(text: &str) -> String {
    let n = text.chars().count();
    let hcap = super::govern::th::DIGEST_HEAD_CHARS;
    let tcap = super::govern::th::DIGEST_TAIL_CHARS;
    if n <= hcap + tcap {
        return format!("要点：{}", text.trim());
    }
    let head: String = text.chars().take(hcap).collect();
    let tail: String = text.chars().skip(n - tcap).collect();
    format!(
        "要点（结论/报错在尾部）：\n  头：{}…\n  …（中间 {} 字符已退场）…\n  尾：{}",
        head.trim(),
        n - hcap - tcap,
        tail.trim()
    )
}

/// 首个有意义的一行（剥行号前缀后）—— 写入型/通讯型的要点形态。抽不到给空串（不编造）。
fn first_meaningful_line(text: &str) -> String {
    for l in text.lines() {
        let t = super::strip_lineno(l).trim();
        if !t.is_empty() {
            return format!("要点：{}", t.chars().take(200).collect::<String>());
        }
    }
    String::new()
}

/// 归档 stub 正文（**路标式**）：带上"这曾是哪一次调用"的定位。
pub(crate) fn archive_stub_text(name: &str, loc: &str, original: &str) -> String {
    format!("（已归档）{}", digest(name, loc, original))
}

/// 归档执行体：[start, end) 区间的工具输出 → **路标式**归档 stub（配对不破，reasoning 不动）。
pub(crate) fn stub_span(items: &mut Vec<InputItem>, start: usize, end: usize) -> usize {
    let locs = call_locators(items);
    let mut saved = 0usize;
    let end = end.min(items.len());
    for i in start..end {
        let Some(cid) = tool_out_call_id(&items[i]).map(str::to_string) else {
            continue;
        };
        let (name, loc) = locs.get(&cid).cloned().unwrap_or_default();
        let original = tool_out_text(&items[i]).unwrap_or("").to_string();
        // 唯一判据（`govern::verdict`）：L2 跨回合 —— 正文退场，定位留下。
        let has_spill = super::spill_path_of(&original).is_some();
        if super::govern::verdict(
            super::govern::kind_of_tool(&name),
            super::govern::Stage::CrossTask,
            has_spill,
        ) == super::govern::Action::Keep
        {
            continue;
        }
        let stub = archive_stub_text(&name, &loc, &original);
        saved += super::stub_output_at(items, i, &stub);
    }
    saved
}

/// 回合起点：`role == "user"` 的位置。
pub(crate) fn user_positions(items: &[InputItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
        .map(|(i, _)| i)
        .collect()
}

/// **reasoning 治理 —— 唯一实现**。
pub(crate) fn govern_reasoning(
    items: &mut [InputItem],
    keep_bytes: usize,
    cur_round_start: usize,
) -> usize {
    let split = cur_round_start.min(items.len());
    let mut n = 0usize;
    // ① 跨回合：一律折叠（零内容）
    for i in 0..split {
        if has_reasoning_text(&items[i]) && blank_reasoning_text(&mut items[i]) {
            n += 1;
        }
    }
    // ② 当前回合内：按字节预算保最近若干条
    let idxs: Vec<usize> = (split..items.len())
        .filter(|&i| has_reasoning_text(&items[i]))
        .collect();
    if idxs.is_empty() {
        return n;
    }
    // 从最新往前扩，直到超预算；超出的那条及更早 → 摘成决策句
    let mut acc = 0usize;
    let mut keep_from = 0usize;
    for k in (0..idxs.len()).rev() {
        let sz = reasoning_text_bytes(&items[idxs[k]]);
        if k + 1 == idxs.len() || acc + sz <= keep_bytes {
            acc += sz;
            keep_from = k;
        } else {
            keep_from = k + 1;
            break;
        }
    }
    for &i in &idxs[..keep_from] {
        if placeholder_reasoning_text(&mut items[i]) {
            n += 1;
        }
    }
    n
}

/// 滚动压缩（分层退场·中间档）：3+ 轮未被引用的输出 → **结构化摘要**。
pub(crate) fn roll_span(items: &mut Vec<InputItem>, start: usize, end: usize) {
    let locs = call_locators(items);
    let end = end.min(items.len());
    for i in start..end {
        let Some(cid) = tool_out_call_id(&items[i]).map(str::to_string) else {
            continue;
        };
        let Some(out) = tool_out_text(&items[i]).map(str::to_string) else {
            continue;
        };
        if out.contains(ARCHIVED_MARK) || out.contains(ROLLED_MARK) {
            continue;
        }
        let (name, loc) = locs.get(&cid).cloned().unwrap_or_default();
        let new = format!("{ROLLED_MARK}{}", digest(&name, &loc, &out));
        set_tool_out_text(&mut items[i], &new);
    }
}

/// 摘要形态标记 —— **幂等判据**（见到它就不再压：不重复破前缀缓存）。
pub(crate) const ARCHIVED_MARK: &str = "（已归档）";
pub(crate) const ROLLED_MARK: &str = "（滚动压缩）";
pub(crate) const CONDENSED_MARK: &str = "（已摘要）";
pub(crate) const FOLDED_MARK: &str = "（推理已折叠）";
