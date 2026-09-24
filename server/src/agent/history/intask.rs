//! 回合内治理与入库瘦身 —— 去重、上限、占位、附件保留。
use crate::model::types::InputItem;
use super::*;
/// 附件保留窗口：近 N 个回合内保原文，更早的压成占位（图片/文档体积大且已不再被引用）。
pub(crate) const STALE_ATTACH_KEEP_TASKS: usize = 3;

/// 附件退场：近 N 个回合内保原文，更早的**含图**条目压成占位。
pub(crate) fn stale_attachments(
    items: &mut Vec<InputItem>,
    keep_tasks: usize,
    min_saved: usize,
) -> usize {
    let user_idx: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
        .map(|(i, _)| i)
        .collect();
    if user_idx.len() <= keep_tasks {
        return 0;
    }
    let keep_from = user_idx[user_idx.len() - keep_tasks];
    // ① 先算：把"压完长什么样"在内存里造出来，只累计字节差，**不写回**。
    let mut planned: Vec<(usize, serde_json::Value)> = Vec::new();
    let mut potential = 0usize;
    for (i, it) in items.iter().enumerate().take(keep_from) {
        let InputItem::Message { content, .. } = it else {
            continue;
        };
        let serde_json::Value::Array(parts) = content else {
            continue;
        };
        let has_img = parts.iter().any(|p| {
            p.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t.contains("image"))
                .unwrap_or(false)
        });
        if !has_img {
            continue;
        }
        let mut kept: Vec<serde_json::Value> = Vec::with_capacity(parts.len() + 1);
        for p in parts.iter() {
            let t = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if t.contains("image") {
                continue;
            }
            kept.push(p.clone());
        }
        kept.push(serde_json::json!({
            "type": "input_text",
            "text": "（早期截图/附件已归档：其分析结论在后续对话轮中；如需重新查看请重新发送——从本地重拖即可）"
        }));
        let after = serde_json::Value::Array(kept);
        let before_len = content.to_string().len();
        let after_len = after.to_string().len();
        if after_len < before_len {
            potential += before_len - after_len;
            planned.push((i, after));
        }
    }
    if potential < min_saved {
        return 0;
    }
    // ② 够门槛了才写回
    for (i, newc) in planned {
        if let InputItem::Message { content, .. } = &mut items[i] {
            *content = newc;
        }
    }
    potential
}

/// 帧保留窗口：近 N 个回合内保**帧全貌**（含记忆与命中的规范），更早的只留「本轮用户输入」。
pub(crate) const STALE_FRAME_KEEP_TASKS: usize = 0;

/// 帧体三段的分界标记 —— **与产出它们的资产一一对应**
pub(crate) const FRAME_HEAD: &str = "【本轮用户输入】";
pub(crate) const FRAME_MEMORY_MARK: &str = "【项目上下文】";
pub(crate) const FRAME_NOTE_MARK: &str = "（本轮原话，未必是任务指令";
pub(crate) const FRAME_CONTRACT_MARK: &str = "## 工作规范（与本次任务相关）";

pub(crate) fn trim_stale_frames(
    items: &mut Vec<InputItem>,
    keep_tasks: usize,
    min_saved: usize,
) -> usize {
    let user_idx = user_positions(items);
    if user_idx.len() <= keep_tasks {
        return 0;
    }
    // keep_tasks = 0 ⇒ 只保【当前这一帧】全貌，更早的帧全部瘦身。
    let keep_from = if keep_tasks == 0 { user_idx[user_idx.len() - 1] } else { user_idx[user_idx.len() - keep_tasks] };
    // ① 先算：只累计可省字节，**不写回** —— 够门槛才动（省零头破整条缓存，差 50 倍）
    let mut planned: Vec<(usize, usize, String)> = Vec::new();
    let mut potential = 0usize;
    for (i, it) in items.iter().enumerate().take(keep_from) {
        let InputItem::Message { role, content, .. } = it else {
            continue;
        };
        if role != "user" {
            continue;
        }
        let Value::Array(blocks) = content else {
            continue;
        };
        for (bi, b) in blocks.iter().enumerate() {
            let Some(obj) = b.as_object() else {
                continue;
            };
            let Some(t) = obj.get("text").and_then(|v| v.as_str()) else {
                continue;
            };
            if !t.starts_with(FRAME_HEAD) {
                continue;
            }
            let cut = [FRAME_MEMORY_MARK, FRAME_NOTE_MARK, FRAME_CONTRACT_MARK]
                .iter()
                .filter_map(|m| t.find(m))
                .min();
            let Some(cut) = cut else {
                continue;
            };
            let trimmed = t[..cut].trim_end().to_string();
            if trimmed.len() >= t.len() {
                continue;
            }
            potential += t.len() - trimmed.len();
            planned.push((i, bi, trimmed));
        }
    }
    if potential < min_saved {
        return 0;
    }
    // ② 够门槛才写回
    for (i, bi, text) in planned {
        if let InputItem::Message { content: Value::Array(blocks), .. } = &mut items[i] {
            if let Some(obj) = blocks[bi].as_object_mut() {
                obj.insert("text".into(), Value::String(text));
            }
        }
    }
    potential
}

/// 入库瘦身·思考：**单条思考超预算**时，保「头（方向）+ 尾（结论）」、弃中段。
pub(crate) const INTAKE_REASONING_BUDGET_BYTES: usize = 6_000;
const REASONING_HEAD_BYTES: usize = 3_200;
const REASONING_TAIL_BYTES: usize = 1_600;
/// 中段省略标记 —— **也让模型知道这里被压过**，避免它以为自己的思考本就如此。
pub(crate) const REASONING_INTAKE_MARK: &str =
    "\n\n…（此处省略中段过程：含代码预演等冗余内容，已按入库预算裁去）…\n\n";

pub fn slim_reasoning_intake(text: &str) -> String {
    if text.len() <= INTAKE_REASONING_BUDGET_BYTES {
        return text.to_string();
    }
    // 按**字符边界**切，绝不切坏 UTF-8（中文 3 字节/字符，按字节硬切会产生半个字符）
    let head: String = text
        .char_indices()
        .take_while(|(i, _)| *i < REASONING_HEAD_BYTES)
        .map(|(_, c)| c)
        .collect();
    let cut = text.len().saturating_sub(REASONING_TAIL_BYTES);
    let tail: String = text
        .char_indices()
        .skip_while(|(i, _)| *i < cut)
        .map(|(_, c)| c)
        .collect();
    format!("{head}{REASONING_INTAKE_MARK}{tail}")
}

/// 剥掉 read 的行号前缀（`   12\tcode` → `code`）；无前缀原样返回。
pub(crate) fn strip_lineno(line: &str) -> &str {
    if let Some(idx) = line.find('\t') {
        let p = &line[..idx];
        if !p.is_empty() && p.chars().all(|c| c == ' ' || c.is_ascii_digit()) {
            return &line[idx + 1..];
        }
    }
    line
}

/// 实验开关（默认关）：`REAL_DROP_PREV_TOOL_OUTPUTS=1`
pub fn drop_prev_tool_outputs_enabled() -> bool {
    matches!(
        std::env::var("REAL_DROP_PREV_TOOL_OUTPUTS")
            .map(|v| v.trim().to_lowercase())
            .as_deref(),
        Ok("1") | Ok("on") | Ok("true")
    )
}

/// 重复读去重（装配路径）—— **委托给唯一实现** `dedup_reads_in_task`。
pub(crate) fn dedup_read_outputs(items: &mut Vec<InputItem>, min_saved: usize) -> usize {
    dedup_reads_in_task(items, min_saved)
}

// 回合内轮间治理（成本优化·三）

/// 回合内 Raw/工具输出总字节硬顶：超过从最旧输出起 stub。
pub(crate) fn midgovern_raw_cap_bytes() -> usize {
    super::govern::intask_total_bytes()
}
/// 永远保留最近 N 条工具输出原文（当前工作面——模型正在引用的输出绝不动）
pub(crate) const MIDGOVERN_KEEP_OUTPUTS: usize = super::govern::th::INTASK_KEEP_RECENT;
/// 读去重最小收益（字节）：省不出这个数不值得破前缀缓存（随水位线动态化，同上）。
pub(crate) fn midgovern_min_saved_bytes() -> usize {
    super::govern::intask_min_saved_bytes()
}

/// 历史总量（字节）—— 弹性水位闸门的判据。
pub(crate) fn intake_bytes_of(items: &[InputItem]) -> usize {
    items
        .iter()
        .map(|it| serde_json::to_string(it).map(|s| s.len()).unwrap_or(0))
        .sum()
}

/// **治理启动线下限（字节）** —— 弹性水位的地板，防止外推值过小导致压得过狠。
pub(crate) const MIDGOVERN_TRIGGER_FLOOR_BYTES: usize = 100_000;

/// **弹性水位线（字节）** —— 不是固定值，按"照当前速度涨下去会不会撞窗口"实时算。
pub(super) fn elastic_trigger_bytes(
    now: usize,
    _round: u32,
    _max_rounds: u32,
    _prev: Option<(u32, usize)>,
) -> usize {
    // 安全线取自 `govern::window_safe_bytes`（唯一事实源）。装配期的正文摘要接力用同一份 ——
    let safe_bytes = super::govern::window_safe_bytes();
    if now <= safe_bytes {
        return usize::MAX;
    }
    // 压到水位线本身（不再 ×0.6 留余量）：就一个比例。
    let target = safe_bytes.max(MIDGOVERN_TRIGGER_FLOOR_BYTES);
    target.min(now - 1)
}

/// L1 回合内治理入口（workflow 每 N 轮调一次）—— **本层只此一个入口**。
pub fn govern_in_task(
    items: &mut Vec<InputItem>,
    round: u32,
    max_rounds: u32,
    prev: Option<(u32, usize)>,
) -> usize {
    let now = intake_bytes_of(items);
    // 登记「本轮平均新增」——水位线自动推导的输入（`govern::observe_round_bytes`）。
    if let Some((prev_round, prev_bytes)) = prev {
        let rounds = round.saturating_sub(prev_round).max(1) as usize;
        super::govern::observe_round_bytes(now.saturating_sub(prev_bytes) / rounds);
    }
    if now <= elastic_trigger_bytes(now, round, max_rounds, prev) {
        return 0;
    }
    govern_in_task_ungated(items)
}

/// L1 治理**本体**（无闸门）—— 测试直接用它验证治理逻辑；
pub(crate) fn govern_in_task_ungated(items: &mut Vec<InputItem>) -> usize {
    let user_idx = user_positions(items);
    let mut saved = dedup_reads_in_task(items, midgovern_min_saved_bytes());
    saved += govern_in_task_outputs(
        items,
        &user_idx,
        MIDGOVERN_KEEP_OUTPUTS,
        super::govern::th::INTASK_MIN_ITEM_BYTES,
        midgovern_raw_cap_bytes(),
    );
    saved += fold_old_narrations(
        items,
        MIDGOVERN_KEEP_NARRATIONS,
        super::govern::intask_total_bytes(),
    );
    saved += fold_old_reasoning(items, MIDGOVERN_KEEP_REASONING);
    saved
}

/// 回合内保留的最近 assistant 叙述条数（是边界，不是触发条件）。
pub(crate) const MIDGOVERN_KEEP_NARRATIONS: usize = 4;

/// 折叠后 assistant 叙述保留的**字节**上限（一行）。
pub(crate) const NARRATION_FOLD_BYTES: usize = 120;

/// 收益下限：原文小于它就别折（折完反而更长）。
pub(crate) const NARRATION_FOLD_MIN_BYTES: usize = NARRATION_FOLD_BYTES * 3;

/// 折叠标记 —— 同时是**幂等闸**（已折叠的不再动，防"折叠的折叠"）。
pub(crate) const NARRATION_FOLDED_MARK: &str = "（早期叙述已折叠）";

/// 按**字符边界**安全截断（不切断多字节字符），返回 (切片, 是否截了)。
pub(crate) fn truncate_on_char_boundary(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

/// 取 assistant 消息里的正文字段（只认 `output_text`），返回拼接结果。
pub(crate) fn narration_text(content: &Value) -> Option<String> {
    let arr = content.as_array()?;
    let mut s = String::new();
    for part in arr {
        if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                s.push_str(t);
            }
        }
    }
    (!s.is_empty()).then_some(s)
}

pub(crate) fn fold_old_narrations(
    items: &mut Vec<InputItem>,
    keep_recent: usize,
    budget_bytes: usize,
) -> usize {
    // 越线才动手 —— 与 `govern_in_task_outputs` 同一个"启动线"语义
    if intake_bytes_of(items) <= budget_bytes {
        return 0;
    }
    let mut idxs: Vec<usize> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let InputItem::Message { role, content, .. } = it else {
            continue;
        };
        if role != "assistant" {
            continue;
        }
        if let Some(t) = narration_text(content) {
            if t.len() >= NARRATION_FOLD_MIN_BYTES && !t.contains(NARRATION_FOLDED_MARK) {
                idxs.push(i);
            }
        }
    }
    if idxs.len() <= keep_recent {
        return 0;
    }
    let cut = idxs.len() - keep_recent;
    let mut saved = 0usize;
    for &i in &idxs[..cut] {
        saved += fold_one_narration(&mut items[i]);
    }
    saved
}

/// 折叠单条 assistant 叙述，返回省下的字节。非 `output_text` 的 part 原样保留。
fn fold_one_narration(it: &mut InputItem) -> usize {
    let InputItem::Message { content, .. } = it else {
        return 0;
    };
    let Some(arr) = content.as_array() else {
        return 0;
    };
    let mut saved = 0usize;
    let mut out: Vec<Value> = Vec::with_capacity(arr.len());
    for part in arr {
        if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                if t.len() >= NARRATION_FOLD_MIN_BYTES {
                    let (head, _) = truncate_on_char_boundary(t, NARRATION_FOLD_BYTES);
                    let new = format!("{head}…{NARRATION_FOLDED_MARK}");
                    saved += t.len().saturating_sub(new.len());
                    out.push(serde_json::json!({"type": "output_text", "text": new}));
                    continue;
                }
            }
        }
        out.push(part.clone());
    }
    if saved > 0 {
        *content = Value::Array(out);
    }
    saved
}

/// 回合内保留的最近**带推理**条目数（是边界，不是触发条件）。
pub(crate) const MIDGOVERN_KEEP_REASONING: usize = 3;

pub(crate) fn fold_old_reasoning(items: &mut Vec<InputItem>, keep_recent: usize) -> usize {
    let idxs: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| super::item::reasoning_len(it) > 0)
        .map(|(i, _)| i)
        .collect();
    if idxs.len() <= keep_recent {
        return 0;
    }
    let cut = idxs.len() - keep_recent;
    let mut saved = 0usize;
    for &i in &idxs[..cut] {
        let before = super::item::reasoning_len(&items[i]);
        if super::item::placeholder_reasoning_text(&mut items[i]) {
            let after = super::item::reasoning_len(&items[i]);
            // 只在**真变小**时记账（占位可能比原文长 —— 极短的推理别硬折）
            if after < before {
                saved += before - after;
            }
        }
    }
    saved
}

/// 解析 read 调用参数 → (路径, mode)。兼容 paths[] 单元素与旧 path 单键。
pub(crate) fn parse_read_args(arguments: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(arguments).ok()?;
    let mode = v.get("mode").and_then(|m| m.as_str()).unwrap_or("full").to_string();
    let paths: Vec<&str> = v
        .get("paths")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();
    if paths.len() == 1 {
        Some((paths[0].to_string(), mode))
    } else if paths.is_empty() {
        v.get("path").and_then(|p| p.as_str()).map(|p| (p.to_string(), mode))
    } else {
        None
    }
}

/// 统一输出形态视图：输出条目 → (call_id, 序列化字节数)
pub(crate) fn output_view(it: &InputItem) -> Option<(String, usize)> {
    match it {
        InputItem::FunctionCallOutput { call_id, output } => {
            Some((call_id.clone(), output.len()))
        }
        InputItem::Raw(v) => {
            let cid = v.get("call_id").and_then(|c| c.as_str())?;
            v.get("output")?;
            Some((cid.to_string(), serde_json::to_string(v).unwrap_or_default().len()))
        }
        _ => None,
    }
}

/// stub 一条输出，返回省下的字节数（stub 自身按 ~120 字节计）。
pub(crate) fn stub_output_at(items: &mut Vec<InputItem>, idx: usize, stub: &str) -> usize {
    let old = match &items[idx] {
        InputItem::FunctionCallOutput { output, .. } => output.len(),
        InputItem::Raw(v) => serde_json::to_string(v).map(|j| j.len()).unwrap_or(0),
        _ => return 0,
    };
    if old <= stub.len() + 120 {
        return 0;
    }
    let saved = old - (stub.len() + 120);
    match &mut items[idx] {
        InputItem::FunctionCallOutput { output, .. } => *output = stub.to_string(),
        InputItem::Raw(v) => {
            if v.get("call_id").is_some() && v.get("output").is_some() {
                v["output"] = Value::String(stub.to_string());
            }
        }
        _ => return 0,
    }
    saved
}

/// 回合内读去重（带最小收益守卫）：同路径重复 read，权威 = 最近一次 full 读，
pub(crate) fn dedup_reads_in_task(items: &mut Vec<InputItem>, min_saved: usize) -> usize {
    // ① 登记所有单文件 read 调用：call_id → (路径, mode)
    let mut read_calls: std::collections::HashMap<String, (String, String)> = Default::default();
    for it in items.iter() {
        match it {
            InputItem::FunctionCall { call_id, name, arguments } if name == "read" => {
                if let Some(pm) = parse_read_args(arguments) {
                    read_calls.insert(call_id.clone(), pm);
                }
            }
            InputItem::Message { tool_calls: Some(tcs), .. } => {
                for tc in tcs {
                    let name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name != "read" {
                        continue;
                    }
                    let Some(cid) = tc.get("call_id").and_then(|c| c.as_str()) else {
                        continue;
                    };
                    let args = tc.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                    if let Some(pm) = parse_read_args(args) {
                        read_calls.insert(cid.to_string(), pm);
                    }
                }
            }
            _ => {}
        }
    }
    if read_calls.is_empty() {
        return 0;
    }
    // ② 每路径最后一次 full 读的输出下标（权威锚点）
    let mut last_full: std::collections::HashMap<String, usize> = Default::default();
    for (i, it) in items.iter().enumerate() {
        if let Some((cid, _)) = output_view(it) {
            if let Some((p, mode)) = read_calls.get(&cid) {
                if mode == "full" {
                    last_full.insert(p.clone(), i);
                }
            }
        }
    }
    if last_full.is_empty() {
        return 0;
    }
    let mut candidates: Vec<(usize, String)> = Vec::new();
    let mut candidate_bytes = 0usize;
    for (i, it) in items.iter().enumerate() {
        if let Some((cid, bytes)) = output_view(it) {
            if let Some((p, _)) = read_calls.get(&cid) {
                if let Some(anchor) = last_full.get(p) {
                    if i < *anchor {
                        candidates.push((i, p.clone()));
                        candidate_bytes += bytes;
                    }
                }
            }
        }
    }
    if candidate_bytes.saturating_sub(candidates.len() * 120) < min_saved {
        return 0;
    }
    let mut saved = 0usize;
    for (i, path) in candidates {
        // **定位不能跟着正文一起退场**（范本验收条件一）：stub 必须带路径 ——
        let stub = format!(
            "（回合内重复读取已略：{path} 全文以最近一次 full 读取为准，已在本回合历史中）"
        );
        saved += stub_output_at(items, i, &stub);
    }
    saved
}

/// 现读现索引：为归档占位生成原文件的轻量地图（总行数 + 等距 5 个抽样行片段）
pub(crate) fn file_index_of(path: &str) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    if total == 0 {
        return String::new();
    }
    let mut out = format!("该文件共 {total} 行。结构抽样：");
    let picks = [0usize, total / 4, total / 2, total * 3 / 4, total - 1];
    let mut last = usize::MAX;
    for ln in picks {
        if ln >= total || ln == last {
            continue;
        }
        last = ln;
        let head: String = lines[ln].chars().take(70).collect();
        if !head.trim().is_empty() {
            out.push_str(&format!(" L{}「{}」", ln + 1, head));
        }
    }
    out.push('。');
    out
}

/// **L1 回合内工具输出治理 —— 本层唯一部门**。
pub(crate) fn govern_in_task_outputs(
    items: &mut Vec<InputItem>,
    user_idx: &[usize],
    keep_recent: usize,
    min_item_bytes: usize,
    budget_bytes: usize,
) -> usize {
    let Some(&start) = user_idx.last() else {
        return 0;
    };
    // read 调用登记：call_id → (路径, mode)
    let mut read_calls: std::collections::HashMap<String, (String, String)> = Default::default();
    for it in items.iter() {
        match it {
            InputItem::FunctionCall { call_id, name, arguments } if name == "read" => {
                if let Some(pm) = parse_read_args(arguments) {
                    read_calls.insert(call_id.clone(), pm);
                }
            }
            InputItem::Message { tool_calls: Some(tcs), .. } => {
                for tc in tcs {
                    if tc.get("name").and_then(|n| n.as_str()) != Some("read") {
                        continue;
                    }
                    let Some(cid) = tc.get("call_id").and_then(|c| c.as_str()) else {
                        continue;
                    };
                    let args = tc.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                    if let Some(pm) = parse_read_args(args) {
                        read_calls.insert(cid.to_string(), pm);
                    }
                }
            }
            _ => {}
        }
    }
    let slots: Vec<(usize, usize)> = items
        .iter()
        .enumerate()
        .filter(|(i, _)| *i >= start)
        .filter_map(|(i, it)| output_view(it).map(|(_, b)| (i, b)))
        .collect();
    if slots.is_empty() {
        return 0;
    }
    // 触发唯一：**回合内输出总体积超预算**（没有"条数超了就压"那种无条件改写）
    let mut total: usize = slots.iter().map(|(_, b)| b).sum();
    if total <= budget_bytes {
        return 0;
    }
    let stubbed_len = super::govern::th::STUB_EST_BYTES;
    let keep_from = slots.len().saturating_sub(keep_recent);
    let locs = call_locators(items);
    let mut saved = 0usize;
    let mut seg: Vec<(usize, usize, String, String)> = Vec::new();
    for &(i, bytes) in slots.iter().take(keep_from) {
        if total <= budget_bytes {
            break;
        }
        // ── 类型分派（唯一判据实现 `govern::verdict`；各机制不再自己写"要不要压"）──
        let text_now = tool_out_text(&items[i]).unwrap_or("").to_string();
        // 幂等闸：本函数**每次装配都会被调用**（`archive.rs` 每轮一次），已压过的条目
        if text_now.contains("回合内已压缩") {
            saved += flush_frag_segment(items, &mut seg, min_item_bytes, &mut total);
            continue;
        }
        let cid = tool_out_call_id(&items[i]).map(str::to_string).unwrap_or_default();
        let (name, loc) = locs.get(&cid).cloned().unwrap_or_default();
        let spl = super::spill_path_of(&text_now);
        if super::govern::verdict(
            super::govern::kind_of_tool(&name),
            super::govern::Stage::InWindow,
            spl.is_some(),
        ) == super::govern::Action::Keep
        {
            // 不可压的条目同时是**段边界**（别把它卷进摘要）
            saved += flush_frag_segment(items, &mut seg, min_item_bytes, &mut total);
            continue;
        }
        if text_now.len() < min_item_bytes {
            // 碎片：**不逐条压**（单条省不出体积，却按"最早改动点"整段破缓存）——
            seg.push((i, bytes, name, loc));
            continue;
        }
        // 大条目：先结算前面的碎片段，再按原逐条逻辑压
        saved += flush_frag_segment(items, &mut seg, min_item_bytes, &mut total);
        // 产物 = **结构化摘要**（唯一形态）。spill 指针由 `digest` 的第三段自带 ——
        let stub = match read_calls.get(&cid) {
            Some((p, mode)) => {
                let idx = file_index_of(p);
                format!(
                    "（回合内已压缩）早期 read：{p}（mode={mode}）读过一次。{idx}\
                     要细节：用 search 在该文件内检索，或按行号区间 read 精读——\
                     不要凭\"已读过\"的记忆作答。"
                )
            }
            None => format!("（回合内已压缩）{}", super::digest(&name, &loc, &text_now)),
        };
        let s = stub_output_at(items, i, &stub);
        if s > 0 {
            total = total.saturating_sub(bytes.saturating_sub(stubbed_len));
            saved += s;
        }
    }
    // 收尾：最后一段碎片
    saved += flush_frag_segment(items, &mut seg, min_item_bytes, &mut total);
    saved
}

/// 段内**非首条**的占位。信息已并入段首摘要，这里只留一个"这里被压过"的痕迹。
pub(crate) const FRAG_MERGED_MARK: &str = "（回合内已压缩·并入本段摘要）";

/// 一段碎片合成**一条**摘要（写回段首，其余压成占位），返回实际省下的字节。
fn flush_frag_segment(
    items: &mut Vec<InputItem>,
    seg: &mut Vec<(usize, usize, String, String)>,
    min_item_bytes: usize,
    total: &mut usize,
) -> usize {
    if seg.is_empty() {
        return 0;
    }
    let sum: usize = seg.iter().map(|x| x.1).sum();
    // 单条成段没意义（省不出体积）；合计够不着门槛 = 面板口径不许压
    if seg.len() < 2 || sum < min_item_bytes {
        seg.clear();
        return 0;
    }
    let stub = frag_segment_stub(seg, sum);
    // 收益守卫：省得还没写进去的多就不动（与 `stub_output_at` 的"省不出不写"同源）
    let floor = stub.len() + FRAG_MERGED_MARK.len() * (seg.len() - 1) + 200;
    if sum <= floor {
        seg.clear();
        return 0;
    }
    // 段首 = 摘要的落点。**直接写回**，不走 `stub_output_at` 的单条守卫 ——
    let old0 = replace_output_text(items, seg[0].0, &stub);
    if old0 == 0 {
        seg.clear();
        return 0;
    }
    let mut saved = old0.saturating_sub(stub.len());
    *total = total.saturating_sub(old0).saturating_add(stub.len());
    for (n, (idx, bytes, _, _)) in seg.iter().enumerate() {
        if n == 0 {
            continue;
        }
        let s = stub_output_at(items, *idx, FRAG_MERGED_MARK);
        if s > 0 {
            *total = total.saturating_sub(*bytes).saturating_add(FRAG_MERGED_MARK.len());
            saved += s;
        }
    }
    seg.clear();
    saved
}

/// 直接改写一条输出的正文（**不带单条长度守卫**），返回改写前的长度（0 = 形态不适用，未改写）。
fn replace_output_text(items: &mut Vec<InputItem>, idx: usize, text: &str) -> usize {
    let old = match &items[idx] {
        InputItem::FunctionCallOutput { output, .. } => output.len(),
        InputItem::Raw(v) => serde_json::to_string(v).map(|j| j.len()).unwrap_or(0),
        _ => return 0,
    };
    match &mut items[idx] {
        InputItem::FunctionCallOutput { output, .. } => *output = text.to_string(),
        InputItem::Raw(v) => {
            if v.get("call_id").is_some() && v.get("output").is_some() {
                v["output"] = Value::String(text.to_string());
            } else {
                return 0;
            }
        }
        _ => return 0,
    }
    old
}

/// 段摘要正文 —— 逐条列「工具 + 定位 + 字节」，超出只报总数。
fn frag_segment_stub(seg: &[(usize, usize, String, String)], sum: usize) -> String {
    let max = super::govern::th::DIGEST_MAX_OUTLINE;
    let mut out = format!(
        "（回合内已压缩）本段 {} 次工具回执 · 合计 {sum} 字符，逐条原文已退场：",
        seg.len()
    );
    for (_, b, name, loc) in seg.iter().take(max) {
        let who = if name.is_empty() { "工具" } else { name.as_str() };
        if loc.is_empty() {
            out.push_str(&format!("\n  · {who}（{b} 字符）"));
        } else {
            out.push_str(&format!("\n  · {who} {loc}（{b} 字符）"));
        }
    }
    if seg.len() > max {
        out.push_str(&format!("\n  …另有 {} 条同类回执", seg.len() - max));
    }
    out.push_str(
        "\n要细节：按上面的定位重新执行该工具，或 read 对应文件——\
         不要凭\"已经看过\"的记忆作答。",
    );
    out.chars().take(super::govern::th::DIGEST_TOTAL_CHARS).collect()
}
