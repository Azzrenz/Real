//! 轮回合日志与关键词激活（设计决定："记忆部门化）

use crate::error::AppResult;
use sqlx::SqlitePool;

/// 单轮日志上限常量（超长截断，防账本膨胀）
const INPUT_CAP: usize = 600;
const DIGEST_CAP: usize = 600;
const DIGEST_DISPLAY_CAP: usize = 600;
/// 「最近回合锚」在**需要续接时**附带上一轮完整回复原文的上限（字节级字符截断）。
const ANCHOR_BODY_CAP: usize = 6000;
const KEYWORD_CAP: usize = 14;
const ACTIVATE_TOP_K: usize = 4;
const ACTIVATE_MIN_HITS: i32 = 2;
/// 关键词激活的扫描窗口（**回合条数**）。
const ACTIVATE_WINDOW: i64 = 30;

/// 记录一轮完成的回合日志（收敛 Ok 路径调用；失败静默——日志不能影响主流程）
pub async fn record_turn(
    pool: &SqlitePool,
    session_id: &str,
    user_input: &str,
    answer: &str,
    changed_files: &[String],
) -> AppResult<()> {
    let seq: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM turn_logs WHERE session_id = ?1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        + 1;
    let keywords = extract_keywords(user_input, changed_files);
    sqlx::query(
        "INSERT INTO turn_logs (session_id, seq, user_input, answer_digest, keywords, files, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(session_id)
    .bind(seq)
    .bind(crate::agent::plan::truncate(user_input, INPUT_CAP))
    .bind(extract_digest(answer))
    .bind(serde_json::to_string(&keywords).unwrap_or_else(|_| "[]".into()))
    .bind(serde_json::to_string(changed_files).unwrap_or_else(|_| "[]".into()))
    .bind(repos_now())
    .execute(pool)
    .await?;
    Ok(())
}

/// 台账行：(seq, user_input, answer_digest, keywords, files)。
type TurnRow = (i64, String, String, String, String);

/// 上一轮的**完整回复正文**（原文，非摘要）—— 供「最近回合锚」在**需要续接时**附带。
async fn latest_assistant_body(pool: &SqlitePool, session_id: &str) -> Option<String> {
    let msgs = crate::db::repos::list_messages(pool, session_id).await.ok()?;
    msgs.iter()
        .rev()
        .find(|m| {
            if m.role != "assistant" || m.content.trim().is_empty() {
                return false;
            }
            match m
                .item_json
                .as_deref()
                .and_then(|ij| serde_json::from_str::<serde_json::Value>(ij).ok())
            {
                Some(v) => matches!(v.get("type").and_then(|t| t.as_str()), None | Some("message")),
                None => true,
            }
        })
        .map(|m| m.content.clone())
}

/// 关键词激活：新输入分词 → 与本会话历史回合的关键词命中计分 → 高分回合要点回灌。
pub async fn activate(pool: &SqlitePool, session_id: &str, user_input: &str) -> AppResult<String> {
    let query_kw = extract_keywords(user_input, &[]);
    let rows: Vec<TurnRow> = sqlx::query_as(
        "SELECT seq, user_input, answer_digest, keywords, files FROM turn_logs
         WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2",
    )
    .bind(session_id)
    .bind(ACTIVATE_WINDOW)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let weak_signal = query_kw.len() < 2;
    let mut scored: Vec<(i32, i64, String, String)> = Vec::new();
    for (seq, input, digest, kw_json, _files) in rows.iter().cloned() {
        let kws: Vec<String> = serde_json::from_str(&kw_json).unwrap_or_default();
        let score = keyword_hits(&query_kw, &kws) as i32;
        if score >= ACTIVATE_MIN_HITS {
            scored.push((score, seq, input, digest));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    if scored.is_empty() {
        if let Some((seq, input, digest, _, files_json)) = rows.first() {
            let short_input = crate::agent::plan::truncate(input, 80);
            let short_digest = truncate_chars(&isolate_literals(digest), DIGEST_DISPLAY_CAP);
            let head = if weak_signal {
                "【最近回合续接】（延续指令未命中关键词——附最近一条回合结论，供直接续接，勿重复探测/验证已确认事项）："
            } else {
                "【最近回合锚】（本会话最近一条已完成回合，仅供**方向对照**；不代表本轮回合与它相同——本轮若为提问/纠正/引用，按本轮原话回应）："
            };
            let mut out = String::from(head);
            out.push_str(&format!("\n- 回合#{seq} 用户要：{short_input}\n  结论要点：{short_digest}"));
            let files: Vec<String> = serde_json::from_str(files_json).unwrap_or_default();
            if !files.is_empty() {
                let show = files.iter().take(6).cloned().collect::<Vec<_>>().join("、");
                out.push_str(&format!("\n  涉及文件（产出回程；需改动时先 read）：{show}"));
            }
            if weak_signal {
                // 【按需带全文】延续指令下摘要不足以续接 —— 附上一轮**完整回复原文**。
                if let Some(body) = latest_assistant_body(pool, session_id).await {
                    let body = truncate_chars(&isolate_literals(&body), ANCHOR_BODY_CAP);
                    out.push_str(&format!(
                        "\n\n【上一轮完整回复·原文】（需延续它时据此接续，勿凭摘要猜）：\n{body}"
                    ));
                }
                append_standing_rules(&mut out, &rows);
                out.push_str(BOUNDARY_NOTE);
            }
            return Ok(out);
        }
        return Ok(String::new());
    }
    let mut out = String::from("【此前相关回合回灌】（后端按关键词激活，供续接参考）：");
    for (score, seq, input, digest) in scored.iter().take(ACTIVATE_TOP_K) {
        let kws: Vec<String> = {
            // 命中的关键词回显（从 keywords 列取，简单重取：seq 唯一定位）
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT keywords FROM turn_logs WHERE session_id = ?1 AND seq = ?2",
            )
            .bind(session_id)
            .bind(seq)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
            row.and_then(|(j,)| serde_json::from_str::<Vec<String>>(&j).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|k| {
                    query_kw.iter().any(|q| k.contains(q.as_str()) || q.contains(k.as_str()))
                })
                .take(3)
                .collect()
        };
        let short_input = crate::agent::plan::truncate(input, 80);
        let short_digest = truncate_chars(&isolate_literals(digest), DIGEST_DISPLAY_CAP);
        out.push_str(&format!(
            "\n- 回合#{seq}（命中{score}词：{}）用户要：{short_input}\n  结论要点：{short_digest}",
            kws.join("、")
        ));
    }
    append_standing_rules(&mut out, &rows);
    out.push_str(BOUNDARY_NOTE);
    Ok(out)
}

/// 用户持续指令块（**唯一实现**，两个注入分支共用）。
fn append_standing_rules(out: &mut String, rows: &[TurnRow]) {
    let rules = extract_standing_rules(rows);
    if rules.is_empty() {
        return;
    }
    out.push_str("\n\n【用户持续指令·原话为准】（近几轮**用户原话**，未做任何改写；若与本轮指令冲突，以本轮为准并在回复中说明）：");
    for r in &rules {
        out.push_str(&format!("\n- “{r}”"));
    }
}

/// 回灌块的边界声明（**唯一实现**，两个注入分支共用）。
const BOUNDARY_NOTE: &str = "\n（边界：以上是历史回合的**结论摘要**，不构成编辑资格——编辑其中提到的文件前仍须先 read（编辑资格按会话计），且内容在此后可能已变化；环境/服务状态若在结论里已确认，直接沿用，回合目标变化才需复查。）";

/// 禁令词表 —— "用户持续指令"的**唯一判据来源**。
pub(crate) const STANDING_MARKERS: [&str; 10] =
    ["不要", "不用", "别去", "别做", "别", "禁止", "切勿", "莫要", "只发", "只做"];

pub(crate) fn is_standing_rule(input: &str) -> bool {
    let n = input.chars().count();
    (6..=200).contains(&n) && STANDING_MARKERS.iter().any(|m| input.contains(m))
}

fn extract_standing_rules(rows: &[TurnRow]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for (_, input, _, _, _) in rows.iter().rev() {
        if input.trim().is_empty() {
            continue;
        }
        if is_standing_rule(input) {
            let short: String = input.trim().chars().take(90).collect();
            let u = if input.trim().chars().count() > 90 {
                format!("{short}…")
            } else {
                short
            };
            if !seen.iter().any(|x| *x == u) {
                seen.push(u);
            }
        }
        if seen.len() >= 3 {
            break;
        }
    }
    seen
}

/// 关键词命中计分（包容式：k 含 q 或 q 含 k——纯函数，可测）。
fn keyword_hits(query: &[String], stored: &[String]) -> usize {
    query
        .iter()
        .filter(|q| {
            stored
                .iter()
                .any(|k| k.contains(q.as_str()) || q.contains(k.as_str()))
        })
        .count()
}

/// 关键词抽取（确定性，零 LLM 成本）
pub fn extract_keywords(user_input: &str, changed_files: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) && out.len() < KEYWORD_CAP {
            out.push(s);
        }
    };
    let lower = user_input.to_lowercase();
    // CJK 连续段 → 按停用词切分：中文没有分词边界，"这个然后继续看看那个"整段连写
    let chars: Vec<char> = lower.chars().collect();
    let mut cur = String::new();
    let mut cjk_runs: Vec<String> = Vec::new();
    for c in chars {
        if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            cur.push(c);
        } else if !cur.is_empty() {
            cjk_runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        cjk_runs.push(cur);
    }
    for run in cjk_runs {
        for seg in split_by_stopwords(&run) {
            if seg.chars().count() >= 2 && seg.chars().count() <= 12 {
                push(seg);
            }
        }
    }
    // 拉丁词（≥3 字符）
    for w in lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')) {
        if w.chars().count() >= 3 && w.chars().any(|c| c.is_ascii_alphanumeric()) {
            push(w.trim_matches(['-', '.']).to_string());
        }
    }
    // 改动文件主干
    for f in changed_files {
        let stem = f.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = stem.split('.').next().unwrap_or("").to_lowercase();
        if stem.chars().count() >= 3 {
            push(stem);
        }
    }
    out
}

/// 高频功能词停用表（命中它们=纯噪声激活；实义词不在表内——
const STOPWORDS: &[&str] = &[
    "这个", "那个", "一下", "然后", "就是", "什么", "怎么", "可以", "我们",
    "你们", "现在", "直接", "还是", "或者", "如果", "因为", "所以", "但是",
    "看看", "的话", "应该", "一下子", "是不是", "有没有", "来一下",
];

/// 按停用词切分 CJK 连续段：把每个停用词当刀切一轮，剩余非停用词段即实词候选。
fn split_by_stopwords(run: &str) -> Vec<String> {
    let mut segs = vec![run.to_string()];
    for &sw in STOPWORDS {
        let mut next = Vec::new();
        for seg in segs {
            for part in seg.split(sw) {

                if !part.is_empty() {
                    next.push(part.to_string());
                }
            }
        }
        segs = next;
    }
    segs.into_iter().filter(|s| !STOPWORDS.contains(&s.as_str())).collect()
}

/// 结论要点抽取（确定性）：markdown 标题行 + 列表项优先，无结构取首段。
pub fn extract_digest(answer: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (i, l) in answer.lines().enumerate() {
        let t = l.trim();
        let is_head = t.starts_with('#');
        let is_list = t.starts_with("- ") || t.starts_with("* ") || t.starts_with("• ");
        let is_num = {
            let mut it = t.char_indices();
            match it.next() {
                Some((_, c)) if c.is_ascii_digit() => {
                    // "1." / "1、" 形态
                    let rest = &t[c.len_utf8()..];
                    rest.starts_with('.') || rest.starts_with('、')
                }
                _ => false,
            }
        };
        if is_head || is_list || is_num {
            lines.push(format!("L{}: {}", i + 1, t));
        }
    }
    let joined = lines.join("\n");
    let digest = if joined.chars().count() >= 80 {
        joined
    } else {
        // 无结构：取首段（首个空行前），同样带行号
        answer
            .lines()
            .take(6)
            .enumerate()
            .map(|(i, l)| format!("L{}: {}", i + 1, l.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    truncate_chars(&digest, DIGEST_CAP)
}

/// 回灌字面隔离：把台账文本里的可执行占位符字面（`#NEW` / `#E数字` / `#En`）换成不可照抄的等价说法。
pub(crate) fn isolate_literals(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < c.len() {
        if c[i] == '#' {
            // #NEW / #NEW<数字>：insert 占位符字面
            if i + 3 < c.len() && c[i + 1] == 'N' && c[i + 2] == 'E' && c[i + 3] == 'W' {
                let mut j = i + 4;
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
                out.push_str("「新增占位」");
                i = j;
                continue;
            }
            // #E<数字+>（步骤引用）与 #En（泛指）；边界不接字母数字——`#E2E` 这类不得误伤
            if i + 1 < c.len() && c[i + 1] == 'E' {
                let mut j = i + 2;
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 2 {
                    // 有数字：后面不接字母数字（`#E2E` 这类不得误伤）
                    if c.get(j).map_or(true, |x| !x.is_ascii_alphanumeric()) {
                        let n: String = c[i + 2..j].iter().collect();
                        out.push_str(&format!("「第{n}步」"));
                        i = j;
                        continue;
                    }
                } else if c.get(i + 2).map_or(false, |x| *x == 'n' || *x == 'N')
                    && c.get(i + 3).map_or(true, |x| !x.is_ascii_alphanumeric())
                {
                    // 无数字：`#En` 泛指（边界同样不接字母数字，`#English` 不动）
                    out.push_str("「第N步」");
                    i += 3;
                    continue;
                }
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// 按字符数截断（Rust str 切片按字节，中文多字节直接切会 panic——统一走 chars）
fn truncate_chars(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        s.to_string()
    } else {
        let mut end = cap;
        let s: String = s.chars().take(cap).collect();
        // 尾部若切在转义中间无所谓（纯文本）；补省略号
        let _ = &mut end;
        format!("{s}…")
    }
}

fn repos_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
#[path = "turn_log_tests.rs"]
mod turn_log_tests;
