//! 溢出存储（spill）—— 三层存储机制的"全文层"

use std::path::PathBuf;

use crate::agent::history::th;

/// ⚠️ 出厂默认值锚点；**运行期取值走 `cur_read_spill_threshold_chars()`**（设置面板可热更）。
#[allow(dead_code)]
pub const READ_SPILL_THRESHOLD_CHARS: usize = th::READ_SPILL_THRESHOLD_CHARS;
/// lines 精读 spill 阈值 —— **出厂默认锚点**（运行期走 `cur_read_lines_spill_threshold_chars()`）
#[allow(dead_code)]
pub const READ_LINES_SPILL_THRESHOLD_CHARS: usize = th::READ_LINES_SPILL_THRESHOLD_CHARS;
/// 预览预算：head/tail 各多少字符（模型拿到概览，细节 read locator）。
/// ⚠️ 下面是**出厂默认**；运行期实际取值一律走 `cur_*`（设置面板「调优档位」可热更）。
#[allow(dead_code)]
const PREVIEW_HEAD_CHARS: usize = th::SPILL_PREVIEW_HEAD_CHARS;
#[allow(dead_code)]
const PREVIEW_TAIL_CHARS: usize = th::SPILL_PREVIEW_TAIL_CHARS;

// ── 运行期取值（设置面板「调优档位」热更；单一事实源在 `config::settings::Tuning`）──
// 这些值原先硬编码为常量，改一次要重编译；现在挂到设置面板的档位上，切档即生效。
/// read 全文落盘线（字符，信封口径）
pub fn cur_read_spill_threshold_chars() -> usize {
    crate::config::settings::tuning()
        .read_spill_threshold_chars
        .load(std::sync::atomic::Ordering::Relaxed)
}
/// read 精读（mode = lines）落盘线（字符）
pub fn cur_read_lines_spill_threshold_chars() -> usize {
    crate::config::settings::tuning()
        .read_lines_spill_threshold_chars
        .load(std::sync::atomic::Ordering::Relaxed)
}
/// 落盘预览·头部（字符）
fn cur_preview_head() -> usize {
    crate::config::settings::tuning()
        .spill_preview_head_chars
        .load(std::sync::atomic::Ordering::Relaxed)
}
/// 落盘预览·尾部（字符）
fn cur_preview_tail() -> usize {
    crate::config::settings::tuning()
        .spill_preview_tail_chars
        .load(std::sync::atomic::Ordering::Relaxed)
}
/// spill 单文件字节硬上限
pub fn cur_spill_max_file_bytes() -> usize {
    crate::config::settings::tuning()
        .spill_max_file_bytes
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// spill 保留天数（定）：超过 N 天的会话目录在 server 启动时自动清理。
pub const SPILL_RETENTION_DAYS: i64 = th::SPILL_RETENTION_DAYS;

pub const SPILL_MAX_FILE_BYTES: usize = th::SPILL_MAX_FILE_BYTES;

/// 非会话 spill 条目的短保留期（见 `th::SPILL_ORPHAN_RETENTION_DAYS`）。
pub const SPILL_ORPHAN_RETENTION_DAYS: i64 = th::SPILL_ORPHAN_RETENTION_DAYS;

// ── 会话压力缓存（进程内）───────────────────────────────────────────────
#[derive(Clone, Copy, Debug)]
struct PressureEntry {
    /// 该会话上一轮装配时的 input token。
    tokens: u64,
    /// 登记时刻的**单调序号**（越大越新）—— LRU 的"最近使用"凭据。
    seq: u64,
}

/// 压力缓存的**容量上限**（会话数）—— 超过就淘汰"最久未装配"的那一个。
const PRESSURE_CACHE_MAX: usize = 64;

/// 装配序号发生器（进程内单调递增）。
static PRESSURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static SESSION_PRESSURE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, PressureEntry>>,
> = std::sync::OnceLock::new();

fn pressure_map() -> &'static std::sync::Mutex<std::collections::HashMap<String, PressureEntry>> {
    SESSION_PRESSURE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 淘汰表中**最久未装配**的一条（`seq` 最小者），返回其 key。
fn evict_stalest(
    map: &mut std::collections::HashMap<String, PressureEntry>,
) -> Option<String> {
    let stalest = map
        .iter()
        .min_by_key(|(_, e)| e.seq)
        .map(|(k, _)| k.clone())?;
    map.remove(&stalest);
    Some(stalest)
}

/// 装配期登记本会话当前 input token（供动态阈值用）。同一会话每轮覆盖写。
pub fn set_session_pressure(session_id: &str, input_tokens: u64) {
    let seq = PRESSURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match pressure_map().lock() {
        Ok(mut m) => {
            m.insert(
                session_id.to_string(),
                PressureEntry { tokens: input_tokens, seq },
            );
            if m.len() > PRESSURE_CACHE_MAX {
                if let Some(evicted) = evict_stalest(&mut m) {
                    // 淘汰是**正常**路径（不是异常）：被淘汰的会话退回稳态阈值。
                    tracing::debug!(
                        evicted = %evicted,
                        cap = PRESSURE_CACHE_MAX,
                        "会话压力缓存达上限，淘汰最久未装配的会话（其阈值退回稳态）"
                    );
                }
            }
        }
        Err(e) => {
            // 毒锁不致命：动态阈值只是"更细"，退化成稳态常量照样正确。
            tracing::warn!(error = %e, "会话压力缓存加锁失败，本轮退回稳态阈值");
        }
    }
}

/// 读会话压力；**未登记 ⇒ 稳态水位**（使动态阈值等于原常量）。
fn session_pressure(session_id: &str) -> u64 {
    pressure_map()
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).map(|e| e.tokens))
        .unwrap_or_else(steady_watermark_tokens)
}

/// 稳态水位对应的 token 量（= `WINDOW_TOKENS × WINDOW_SAFE_PERMILLE / 1000`）。
fn steady_watermark_tokens() -> u64 {
    th::WINDOW_TOKENS * th::WINDOW_SAFE_PERMILLE / 1000
}

/// 按**字节**上限截断字符串，且落在 UTF-8 字符边界上（不会写出半个汉字）。
fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> (&str, bool) {
    if s.len() <= max_bytes {
        return (s, false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

/// 内部纯逻辑：清扫 base 下过期条目（测试注入临时目录，不碰真实数据）。
fn scan_and_cleanup(
    base: &std::path::Path,
    cutoff: chrono::DateTime<chrono::Utc>,
    orphan_cutoff: chrono::DateTime<chrono::Utc>,
    known_sessions: &std::collections::HashSet<String>,
) -> usize {
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(base) else {
        return 0;
    };
    for entry in entries.flatten() {
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let Ok(meta) = entry.metadata() else {
         continue;
        };
        let Ok(modified) = meta.modified() else {
         continue;
        };
        let mtime: chrono::DateTime<chrono::Utc> = modified.into();
        // 条目是不是"真会话"：目录名在库里有对应会话 ⇒ 是；否则（含根下平铺文件）⇒ 孤儿。
        let name = entry.file_name().to_string_lossy().to_string();
        let is_known_session = is_dir && known_sessions.contains(&name);
        let limit = if is_known_session { cutoff } else { orphan_cutoff };
        if mtime < limit {
         let res = if is_dir {
         std::fs::remove_dir_all(entry.path())
         } else {
         std::fs::remove_file(entry.path())
         };
         match res {
         Ok(_) => removed += 1,
         Err(e) => {
         tracing::warn!(error = %e, path = %entry.path().display(), "清理 spill 条目失败")
         }
         }
        }
    }
    removed
}

/// 清理过期 spill 条目（按 mtime 早于「now − 保留天数」判定），返回删除数。
pub fn cleanup_expired_with(
    retention_days: i64,
    orphan_days: i64,
    known_sessions: &std::collections::HashSet<String>,
) -> usize {
    let base = crate::path::data_root::spill_root();
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(retention_days);
    let orphan_cutoff = now - chrono::Duration::days(orphan_days);
    let removed = scan_and_cleanup(&base, cutoff, orphan_cutoff, known_sessions);
    if removed > 0 {
        tracing::info!(
         removed,
         base = %base.display(),
         retention_days,
         orphan_days,
         "已清理过期 spill 条目（真会话 {retention_days} 天 / 孤儿 {orphan_days} 天）"
        );
    }
    removed
}

/// spill 清理周期（1 小时）。
const SPILL_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// 挂后台周期清理器（**必须挂**，理由同 `spawn_retirement_sweeper`）。
pub fn spawn_spill_sweeper(pool: sqlx::SqlitePool) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(SPILL_SWEEP_INTERVAL);
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
         iv.tick().await;
         let known = match crate::db::repos::all_session_ids(&pool).await {
         Ok(s) => s,
         Err(e) => {
         tracing::warn!(
         error = %e,
         "取会话名单失败，跳过本轮 spill 清理（不降级为孤儿档：那会删真会话全文）"
         );
         continue;
         }
         };
         // 同步 IO（read_dir / remove_*）：spill 清理是每秒千级的目录遍历，
         let known_for_task = known.clone();
         let r = tokio::task::block_in_place(move || {
         cleanup_expired_with(
         SPILL_RETENTION_DAYS,
         SPILL_ORPHAN_RETENTION_DAYS,
         &known_for_task,
         )
         });
         if r > 0 {
         tracing::info!(removed = r, sessions = known.len(), "spill 周期清理完成");
         }
        }
    });
}

/// 落盘副本里图片 uri 的**替代文本**（模型回取时才不会看到几 MB 的 base64）。
const IMAGE_STRIPPED_NOTE: &str = "图片未随本文件落盘（base64 已剥离）——它另有通道：\
后端把它按**附件**回灌到本轮上下文（与从聊天框发图同一条路）。要看图直接看附件，\
不要 read 它，也不用 read 本文件来找图";

/// 从**载荷文本**里剥离 `data_url` 图片的 base64 —— 只服务于「计量」与「落盘」。
fn strip_image_payload(payload: &str) -> (std::borrow::Cow<'_, str>, usize) {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(payload) else {
        // 非 JSON（理论上不该有：`registry` 交来的就是序列化信封）⇒ 不动它，按原样计量。
        return (std::borrow::Cow::Borrowed(payload), 0);
    };
    let mut stripped = 0usize;
    if let Some(arr) = v.get_mut("content").and_then(|c| c.as_array_mut()) {
        for part in arr.iter_mut() {
            if part.get("type").and_then(|t| t.as_str()) != Some("image_ref") {
                continue;
            }
            let uri = part.get("uri").and_then(|u| u.as_str()).unwrap_or("");
            if !uri.starts_with("data:") {
                continue;
            }
            stripped += uri.len();
            part["uri"] = serde_json::json!(format!("({IMAGE_STRIPPED_NOTE}；原 base64 {} 字节)", uri.len()));
        }
    }
    if stripped == 0 {
        return (std::borrow::Cow::Borrowed(payload), 0);
    }
    (std::borrow::Cow::Owned(v.to_string()), stripped)
}

/// 判断工具结果是否需要 spill。
fn should_spill(
    name: &str,
    payload: &str,
    read_mode: Option<&str>,
    read_paths: &[String],
    session_id: &str,
) -> bool {
    // modify 结果小（changed:N），永不 spill。
    if matches!(name, "modify") {
        return false;
    }
    // read： 纳入（排除会让大文件 read 全量进上下文 = 体积最大头）——
    if name == "read" {
        // ① read spill 文件本身（模型按 spill_locator 回取全文）→ 豁免——
        if read_paths
         .iter()
         .any(|p| std::path::Path::new(p).starts_with(spill_dir(session_id)))
        {
         return false;
        }
        // ② lines 精读：模型明确要该段 → 高阈值（≈500 行 @ 80 字符/行）才溢出
        if read_mode == Some("lines") {
         return payload.chars().count() > cur_read_lines_spill_threshold_chars();
        }
        return payload.chars().count() > cur_read_spill_threshold_chars();
    }
    payload.chars().count() > th::spill_threshold_for(session_pressure(session_id))
}

/// 截断后给模型的**下一步动作指引** —— 按工具分派。
fn next_step_hint(tool: &str) -> &'static str {
    match tool {
        "search" | "find_files" | "list" => {
         "\n[下一步] 命中过多 ⇒ 加限定词（更独特的符号名 / 路径段）或缩小目录重搜；不要靠读全文来筛。"
        }
        "read" => {
         "\n[下一步] 取用 ⇒ read 带 start_line / end_line 只取需要的那一段；\
         单次 ≤500 行（≈40K 字符）不会落盘 —— 那是 read 精读线，是判据不是建议。\
         \n         另：落盘快照（spill 文件）永不二次落盘，读它不必逐段试探，一次读足。"
        }
        "run" => {
         "\n[下一步] 取用 ⇒ 把命令输出重定向到文件（`> out.txt`）再用 read 分段看；\
         或让命令自己只打印要点（head / tail / grep）。"
        }
        "audit" | "web_fetch" => {
         "\n[下一步] 取用 ⇒ 只回取需要的那一段（read + start_line / end_line），不要整读。"
        }
        _ => "",
    }
}

/// 计算 session 的 spill 目录
pub fn spill_dir(session_id: &str) -> PathBuf {
    crate::path::data_root::spill_root().join(sanitize_session(session_id))
}

/// session_id 消毒（防路径穿越）
fn sanitize_session(session_id: &str) -> String {
    let cleaned: String = session_id
        .chars()
        .map(|c| {
         if c.is_alphanumeric() || c == '-' || c == '_' {
         c
         } else {
         '_'
         }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

/// 全文 spill 落盘，返回 (文件路径, 是否成功)
async fn save_full(name: &str, session_id: &str, text: &str) -> Option<PathBuf> {
    let dir = spill_dir(session_id);
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return None;
    }
    // 文件名：`{统一时间戳}-{工具名}.json`（规范见 docs/20260915-数据落盘与命名规范.md）。
    let path = dir.join(format!(
        "{}-{name}.json",
        crate::path::data_root::stamp_ms()
    ));
    // 超限截断（UTF-8 边界安全）。截断说明放在**文件最前**——
    let (body, truncated) = truncate_utf8_bytes(text, cur_spill_max_file_bytes());
    let payload: std::borrow::Cow<'_, str> = if truncated {
        tracing::warn!(
         name,
         session_id,
         original_bytes = text.len(),
         kept_bytes = body.len(),
         "spill 全文超过单文件上限，已截断落盘（防止模型回取时撑爆上下文）"
        );
        std::borrow::Cow::Owned(format!(
         "【spill 截断】原文 {} 字节，超过单文件上限 {} 字节，此处**只保留前 {} 字节**（尾部已丢弃）。\n\
         若你要找的内容不在下面，说明它落在被丢弃的尾部——请改**更精确的命令**重跑\n\
         （加过滤/限定目录/只取需要的字段），不要指望从这份文件里拿到全文。\n\
         ─────────────── 以下为原文前 {} 字节 ───────────────\n{}",
         text.len(),
         SPILL_MAX_FILE_BYTES,
         body.len(),
         body.len(),
         body
        ))
    } else {
        std::borrow::Cow::Borrowed(text)
    };
    match tokio::fs::write(&path, payload.as_ref()).await {
        Ok(_) => Some(path),
        Err(_) => None,
    }
}

/// 构建预览（head/tail 截断对结构化 JSON 会恰好省略关键证据——
fn is_full_payload(v: &serde_json::Value) -> bool {
    v.get("tool_call_id").is_some()
        && v.get("content").map(|c| c.is_array()).unwrap_or(false)
}

/// 递归收正文候选：只认**承载内容**的键，免得把 `tool_call_id` / 路径 / 字段名当正文。
fn collect_body_texts(v: &serde_json::Value, depth: usize, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    match v {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(a) => {
         for x in a {
         collect_body_texts(x, depth + 1, out);
         }
        }
        serde_json::Value::Object(m) => {
         for k in ["content", "render_full", "text", "output", "data"] {
         if let Some(x) = m.get(k) {
         collect_body_texts(x, depth + 1, out);
         }
         }
        }
        _ => {}
    }
}

/// 完整载荷里**最长的那段正文** —— 预览的取样源。
fn longest_body_text(v: &serde_json::Value) -> Option<String> {
    let mut out = Vec::new();
    collect_body_texts(v, 0, &mut out);
    out.into_iter().max_by_key(|s| s.chars().count())
}

/// 预览：结构化结果出"形状 + 摘要"，文本流出"文件索引 + 头尾"。
fn build_preview(payload: &str, locator: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return head_tail_preview(payload, locator);
    };
    if !is_full_payload(&v) {
        return structured_preview(&v, locator);
    }
    let body = longest_body_text(&v).unwrap_or_default();
    let tool = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
    // 结构化工具：结果本来就是"清单 + 形状"，下钻后再按结构渲染（保住原有摘要层与形状）。
    if matches!(tool, "search" | "list" | "audit" | "find_files") {
        if let Ok(inner) = serde_json::from_str::<serde_json::Value>(&body) {
         return structured_preview(&inner, locator);
        }
        return structured_preview(&v, locator);
    }
    // 文本流工具（read / run / web_fetch …）：文件索引 + 头尾最有价值。
    let rf = v.get("render_full").and_then(|x| x.as_str()).unwrap_or("");
    let src: &str = if !rf.is_empty() && rf.chars().count() * 2 >= body.chars().count() {
        rf
    } else if body.is_empty() {
        payload
    } else {
        &body
    };
    if src.chars().count() > cur_preview_head() + cur_preview_tail() {
        return head_tail_preview(src, locator);
    }
    structured_preview(&v, locator)
}

/// 结构感知预览：按 JSON 结构渲染，而不是盲目截字符。
fn structured_preview(v: &serde_json::Value, locator: &str) -> String {
    let mut out = String::new();
    // ① 工具自带摘要层（audit 的 data.summary）——优先，语义最浓缩
    if let Some(s) = v.get("data").and_then(|d| d.get("summary")) {
        out.push_str("【工具摘要】\n");
        out.push_str(&serde_json::to_string_pretty(s).unwrap_or_else(|_| s.to_string()));
        out.push('\n');
    }
    // ② 结构概览：对象 key 清单（数组字段带长度，标量字段带预览）
    match v {
        serde_json::Value::Object(map) => {
         out.push_str("【结果结构】\n");
         for (k, val) in map {
         match val {
         serde_json::Value::Array(a) => {
         out.push_str(&format!("- {k}: [数组, {len} 项]\n", len = a.len()))
         }
         serde_json::Value::Object(o) => {
         out.push_str(&format!("- {k}: [对象, {} 字段]\n", o.len()))
         }
         serde_json::Value::String(s) => {
         let s: String = s.chars().take(120).collect();
         out.push_str(&format!(
         "- {k}: \"{s}{}\"\n",
         if s.chars().count() >= 120 { "…" } else { "" }
         ));
         }
         other => out.push_str(&format!("- {k}: {other}\n")),
         }
         }
         // 数组字段给前 3 条 + 总数（命中/文件列表的"形状"——路径不丢）
         let mut arrays: Vec<(&str, &Vec<serde_json::Value>)> = Vec::new();
         for (k, val) in map {
         if let serde_json::Value::Array(a) = val {
         if !a.is_empty() && k != "summary" {
         arrays.push((k, a));
         }
         }
         }
         if let Some(data_obj) = map.get("data").and_then(|d| d.as_object()) {
         for (k, val) in data_obj {
         if let serde_json::Value::Array(a) = val {
         if !a.is_empty() {
         arrays.push((k, a));
         }
         }
         }
         }
         for (k, a) in arrays {
         out.push_str(&format!("\n【{k} 前 3 条（共 {} 条）】\n", a.len()));
         for item in a.iter().take(3) {
         if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
         let outline = code_outline(content);
         if !outline.is_empty() {
         let p = item.get("path").and_then(|p| p.as_str()).unwrap_or("");
         out.push_str(&format!("- {p}\n{outline}\n"));
         continue;
         }
         }
         out.push_str(&format!(
         "- {}\n",
         serde_json::to_string(item)
         .unwrap_or_default()
         .chars()
         .take(200)
         .collect::<String>()
         ));
         }
         }
        }
        serde_json::Value::Array(a) => {
         out.push_str(&format!("【数组, 共 {} 项, 前 5 条】\n", a.len()));
         for item in a.iter().take(5) {
         out.push_str(&format!(
         "- {}\n",
         serde_json::to_string(item)
         .unwrap_or_default()
         .chars()
         .take(200)
         .collect::<String>()
         ));
         }
        }
        other => out.push_str(&serde_json::to_string_pretty(other).unwrap_or_default()),
    }
    out.push_str(&spill_tail(locator, "结构预览"));
    out
}

/// spill 预览的**回取尾**（两处预览共用一份，避免各写一套话术）。
fn spill_tail(locator: &str, preview_kind: &str) -> String {
    format!(
        "\n[全文层 · 未读] 这是{preview_kind}，正文未进上下文。\
回取：read {{\"paths\":[\"{locator}\"]}}（快照永不二次落盘，一次读足）。规则见 spill 说明。"
    )
}

fn code_outline(content: &str) -> String {
    let max_items = (content.chars().count() / 120)
        .clamp(th::CODE_OUTLINE_MIN_ITEMS, th::CODE_OUTLINE_MAX_ITEMS);
    let mut items: Vec<(usize, String)> = Vec::new();
    for line in content.lines() {
        let (line_no, code) = match line.split_once('\t') {
         Some((l, c)) => (l.trim().parse::<usize>().unwrap_or(0), c.trim()),
         None => (0usize, line.trim()),
        };
        if line_no == 0 {
         continue;
        }
        let c = code.trim_start();
        if c.is_empty()
         || c.starts_with("//")
         || c.starts_with('*')
         || c.starts_with("/*")
         || c.starts_with('#')
        {
         continue;
        }
        let is_struct = c.starts_with("fn ")
         || c.starts_with("pub fn ")
         || c.starts_with("async fn ")
         || c.starts_with("pub async fn ")
         || c.starts_with("impl")
         || c.starts_with("struct ")
         || c.starts_with("pub struct ")
         || c.starts_with("enum ")
         || c.starts_with("pub enum ")
         || c.starts_with("trait ")
         || c.starts_with("pub trait ")
         || c.starts_with("type ")
         || c.starts_with("pub type ")
         || c.starts_with("mod ")
         || c.starts_with("pub mod ")
         || c.starts_with("const ")
         || c.starts_with("pub const ")
         || c.starts_with("static ")
         || c.starts_with("pub static ")
         || c.starts_with("pub use ");
        if is_struct {
         let mut decl = c.to_string();
         // 只留声明行（fn 后常跟 body 首行 {，截断防刷屏）
         if decl.len() > 100 {
         decl.truncate(100);
         decl.push('…');
         }
         items.push((line_no, decl));
         if items.len() >= max_items {
         break;
         }
        }
    }
    if items.is_empty() {
        return String::new();
    }
    let total = content.lines().count();
    let mut out = format!("【结构大纲·约 {total} 行 {n} 个结构点】\n", n = items.len());
    for (ln, decl) in &items {
        out.push_str(&format!("L{ln:<5}{decl}\n"));
    }
    if items.len() >= max_items {
        out.push_str(&format!("… 结构点超 {max_items} 已截断——需更多细节用 read 读 spill 全文或带 start_line/end_line 精读\n"));
    }
    out
}

/// 文本流索引：等距抽样行首片段，给模型一张"全文件地图"
fn text_index(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total < 20 {
        return String::new();
    }
    let mut out = format!("【文件索引·共 {total} 行】\n");
    let samples = 12usize;
    let mut last = usize::MAX;
    for k in 0..samples {
        let ln = total * k / samples;
        if ln == last {
         continue;
        }
        last = ln;
        let head: String = lines[ln].chars().take(80).collect();
        if !head.trim().is_empty() {
         out.push_str(&format!("- L{}: {}\n", ln + 1, head));
        }
    }
    out
}

/// 文本流 head/tail 截断（阅读流头尾最有价值：read 全文/web 正文）
fn head_tail_preview(text: &str, locator: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    let head: String = chars.iter().take(cur_preview_head()).collect();
    let tail: String = if total > cur_preview_head() + cur_preview_tail() {
        chars.iter().skip(total - cur_preview_tail()).collect()
    } else {
        String::new()
    };
    let idx = text_index(text);
    let mut out = String::new();
    if !idx.is_empty() {
        out.push_str(&idx);
        out.push('\n');
    }
    out.push_str(&head);
    if !tail.is_empty() {
        out.push_str(&format!(
         "\n\n…[中间省略 {} 字符]…\n\n{}",
         total - cur_preview_head() - cur_preview_tail(),
         tail
        ));
    }
    out.push_str(&spill_tail(locator, "索引与头尾预览"));
    out
}

/// 把信封改成**已 spill 的形态**：正文换成预览、重复副本去掉、标记落盘位置。
pub fn mark_spilled(env: &mut crate::mcp::envelope::ToolEnvelope, locator: &str, preview: &str) {
    let meta = env.meta.get_or_insert_with(|| serde_json::json!({}));
    meta["spilled"] = serde_json::json!(true);
    meta["spill_locator"] = serde_json::json!(locator);
    // 去掉**重复副本**：全量已在落盘文件里，`render_full` 再随 item 进上下文纯属双份占用。
    env.render_full = None;
    // **图片部件原样留下**（换的是正文，不是图）——
    let kept: Vec<crate::mcp::envelope::ContentPart> = env
        .content
        .iter()
        .filter(|p| p.reachable_image_uri().is_some())
        .cloned()
        .collect();
    if !kept.is_empty() {
        meta["images_kept"] = serde_json::json!(kept.len());
    }
    let mut content = vec![crate::mcp::envelope::ContentPart::text(preview)];
    content.extend(kept);
    env.content = content;
}

/// 对工具结果执行 spill（在 registry.call 里 env 生成后调用）。
pub async fn maybe_spill(
    name: &str,
    session_id: &str,
    payload: &str,
    read_mode: Option<&str>,
    read_paths: &[String],
) -> Option<(String, String)> {
    // **先剥图片，再计量、再落盘**（见 `strip_image_payload`）：图片 base64 既不参与阈值，
    let (metered, stripped) = strip_image_payload(payload);
    if !should_spill(name, metered.as_ref(), read_mode, read_paths, session_id) {
        return None;
    }
    match save_full(name, session_id, metered.as_ref()).await {
        Some(path) => {
         let locator = path.display().to_string();
         tracing::debug!(
         session = session_id,
         tool = name,
         locator,
         "工具结果溢出存储（spill）"
         );
         // （结构感知预览）——head/tail 截断对结构化 JSON 恰好省略关键证据——
         let mut preview = build_preview(metered.as_ref(), &locator);
         if stripped > 0 {
         preview.push_str(&format!(
         "\n[图片] 本载荷含内联图片，其 base64（{stripped} 字节）已从**计量**与**落盘副本**中剥离；\
图片走附件通道直接回灌，看本轮上下文里的 📎，不要 read 它。"
         ));
         }
         preview.push_str(next_step_hint(name));
         Some((locator.clone(), preview))
        }
        None => {
         tracing::warn!(
         session = session_id,
         tool = name,
         "spill 落盘失败，保持原样返回"
         );
         None
        }
    }
}

#[cfg(test)]
#[path = "spill_tests.rs"]
mod spill_tests;
