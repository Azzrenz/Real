//! 自动沉淀（B 档·**只读版**）：任务完成后跑一次复盘，产出「候选经验」推给前端看，**一个字都不落盘**。

use crate::model::types::{InputItem, ResponsesRequest};
use crate::state::AppState;
use serde_json::{json, Value};

/// 门控：本轮至少 2 次工具调用才值得复盘（低于此数多是闲聊/直答，复盘也是凑数）
const MIN_TOOLS: usize = 10;
/// 只有这三个工具会写文件。一轮里一次都没写过文件（只读、只查、只跑查看类命令）
const MUTATING_TOOLS: [&str; 3] = ["write", "edit", "modify"];
/// 喂给复盘的工具记录上限（多了费 token，少了看不出模式）
const TOOL_FEED: i64 = 8;
/// 结论喂料上限：长答案截断，复盘看的是"做了什么"不是复述全文
const ANSWER_FEED: usize = 600;

const SYSTEM: &str = r#"你是经验提炼器。从这一轮任务里挑出**可复用**的经验——下次遇同类任务真会用到的那种。

只挑四类：
1. 避坑：踩过的错、失败路径、工具限制
2. 流程：多步可复用操作
3. 外部源：核实过的入口/链接
4. 用户偏好：用户明确的纠正或认可

判定：
- 动作型（流程/脚本/带参数调用）→ type="skill"
- 信息型（事实/偏好/约定/数据）→ type="memory"

铁律：
- 每条必须带 evidence（来自本轮哪个工具、哪句用户原话）。**没有证据的不写**——宁缺毋滥。
- 已有技能清单里若有同类，填 merge_into（已有技能名），body 只写"要补的那一段"，不要重写全篇。
- 一轮最多 3 条。没什么可复用的就输出 {"items":[]}，不要凑数。

输出 JSON（不要代码块、不要解释）：
{"items":[{"type":"skill","name":"…","category":"架构|UI|工程|排障|应用","summary":"一句话","evidence":"…","body":"…","merge_into":"可选"}]}"#;

/// 任务完成后调用：够格就跑一次复盘，把候选经验推给前端。**任何失败都静默**——
pub async fn maybe_suggest(
    state: &AppState,
    session_id: &str,
    user_input: &str,
    answer: &str,
    tool_calls: usize,
    model: &str,
) {
    // 总开关：REAL_EVOLUTION=0 可整体关掉（默认开——只读不落盘，风险低）
    if std::env::var("REAL_EVOLUTION").map(|v| v == "0").unwrap_or(false) {
        return;
    }
    if tool_calls < MIN_TOOLS || answer.trim().is_empty() {
        return;
    }
    let tools = recent_tools(state, session_id).await;
    if !tools.iter().any(|t| MUTATING_TOOLS.contains(&t.as_str())) {
        return;
    }
    if tools.is_empty() {
        return;
    }
    let known = known_skill_names();
    let brief = build_brief(user_input, answer, &tools, &known);

    let req = ResponsesRequest::builder(model)
        .instructions(SYSTEM.to_string())
        .input(vec![InputItem::user_message(&brief)])
        .temperature(0.2)
        .max_output_tokens(1200)
        .user(session_id)
        .build();

    let Ok(res) = state.llm.stream_complete(&req).await else {
        return;
    };
    let items = parse_candidates(&res.output_text);
    if items.is_empty() {
        return;
    }
    let _ = crate::sse::emit(
        state,
        session_id,
        crate::sse::EV_EVOLUTION,
        json!({ "items": items, "tool_calls": tool_calls }),
    )
    .await;
}

/// 本轮工具记录：从 events 表倒着取最近 N 条（DB 是唯一事实源，不靠内存态）。
async fn recent_tools(state: &AppState, session_id: &str) -> Vec<String> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT payload_json FROM events WHERE session_id = ? AND kind = 'tool' \
         ORDER BY id DESC LIMIT ?",
    )
    .bind(session_id)
    .bind(TOOL_FEED)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut out = Vec::new();
    for raw in rows.into_iter().rev() {
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(arr) = v.get("tools").and_then(|t| t.as_array()) else {
            continue;
        };
        for t in arr {
            let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let action = t
                .get("action")
                .or_else(|| t.get("reason"))
                .and_then(|a| a.as_str())
                .unwrap_or("");
            let status = t.get("status").and_then(|s| s.as_str()).unwrap_or("");
            out.push(format!("- {name}｜{action}｜{status}"));
        }
    }
    out
}

/// 已有技能清单（名 + 一句话）：复盘拿它判重——同类就补，**不新建**（"补维度不新建"纪律）
fn known_skill_names() -> Vec<String> {
    let root = crate::routes::skills::skills_root();
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for e in rd.flatten() {
        let Ok(raw) = std::fs::read_to_string(e.path().join("SKILL.md")) else {
            continue;
        };
        let (name, desc, _cat, _body) = crate::routes::skills::parse_skill_md(&raw);
        if name.is_empty() {
            continue;
        }
        v.push(if desc.is_empty() {
            name
        } else {
            format!("{name}（{desc}）")
        });
    }
    v
}

fn build_brief(user_input: &str, answer: &str, tools: &[String], known: &[String]) -> String {
    let head: String = answer.chars().take(ANSWER_FEED).collect();
    format!(
        "## 用户最初要什么\n{user_input}\n\n\
         ## 这轮干了什么（按时间）\n{}\n\n\
         ## 最终结论（截断）\n{head}\n\n\
         ## 已有技能（有同类就填 merge_into 补它，别新建）\n{}\n\n\
         按系统提示输出 JSON。",
        tools.join("\n"),
        known.join("\n")
    )
}

/// 从模型输出里抠出候选：模型常用 ```json 包裹或前后带话 → 取最外层 {…} 再解析。
pub fn parse_candidates(raw: &str) -> Vec<Value> {
    let t = raw.trim();
    let (Some(start), Some(end)) = (t.find('{'), t.rfind('}')) else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    let Ok(obj) = serde_json::from_str::<Value>(&t[start..=end]) else {
        return Vec::new();
    };
    let Some(arr) = obj.get("items").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter(|it| {
            let ok_type = matches!(
                it.get("type").and_then(|v| v.as_str()),
                Some("skill") | Some("memory")
            );
            let text_of = |k: &str| {
                it.get(k)
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            };
            ok_type && text_of("name") && text_of("summary") && text_of("evidence")
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let raw = r#"{"items":[{"type":"skill","name":"x","summary":"s","evidence":"e"}]}"#;
        assert_eq!(parse_candidates(raw).len(), 1);
    }

    #[test]
    fn parses_fenced_json_with_chatter() {
        let raw = "好的，这是结果：\n```json\n{\"items\":[{\"type\":\"memory\",\"name\":\"m\",\"summary\":\"s\",\"evidence\":\"e\"}]}\n```\n以上";
        assert_eq!(parse_candidates(raw).len(), 1);
    }

    #[test]
    fn drops_items_without_evidence() {
        // 防注水核心：没证据的经验一律丢弃
        let raw = r#"{"items":[
            {"type":"skill","name":"有证据","summary":"s","evidence":"工具 run 第 3 步"},
            {"type":"skill","name":"没证据","summary":"s"},
            {"type":"瞎写","name":"类型不合法","summary":"s","evidence":"e"}
        ]}"#;
        let v = parse_candidates(raw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].get("name").and_then(|x| x.as_str()), Some("有证据"));
    }

    #[test]
    fn returns_empty_on_garbage() {
        assert!(parse_candidates("这段话里没有 JSON").is_empty());
        assert!(parse_candidates("").is_empty());
    }
}
