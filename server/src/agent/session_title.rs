
use crate::model::types::{InputItem, ResponsesRequest};
use crate::state::AppState;
use serde_json::json;

/// 视为"未命名"的默认标题
const DEFAULT_TITLES: [&str; 2] = ["新任务", ""];
/// 已命名会话的漂移检查周期（user 消息数 % N == 0 才查，避免每轮烧钱）
const DRIFT_CHECK_EVERY: i64 = 3;
/// 标题长度硬上限由 `OutputLang::title_max_chars` 给出（中文 20 字 / 英文 40 字符）。

const SYSTEM_ZH: &str = r#"你是会话命名器。根据"当前标题"和"本轮对话"判断：
1. 当前标题还是默认名（如"新任务"）或与内容明显不符 → rename=true 并生成新标题
2. 话题已经转移（前面聊 A、现在已在深入聊 B）→ rename=true 并生成 B 主题的新标题
3. 主题没变 → rename=false
标题要求：≤12 个字，中文，概括**当前**话题核心，不加引号、不加标点、不带"关于"之类前缀。
输出 JSON（不要代码块）：{"rename": true, "title": "新标题"} 或 {"rename": false, "title": ""}"#;

const SYSTEM_EN: &str = r#"You name chat sessions. Given the "current title" and "this turn", decide:
1. The current title is still the default (like "New task") or clearly mismatched → rename=true with a new title
2. The topic has shifted (was about A, now deep into B) → rename=true with a B-focused title
3. The topic is unchanged → rename=false
Title rules: at most 6 words, in English, capturing the CURRENT topic; no quotes, no punctuation, no "about"-style prefix.
Output JSON (no code block): {"rename": true, "title": "a new title"} or {"rename": false, "title": ""}"#;

/// 任务完成后调用：够格就跑一次命名/漂移检查。任何失败静默。
pub async fn maybe_update_title(
    state: &AppState,
    session_id: &str,
    user_input: &str,
    answer: &str,
    model: &str,
) {
    // 总开关：REAL_SESSION_TITLE=0 可关（默认开——每 3 轮一次小调用，成本可忽略）
    if std::env::var("REAL_SESSION_TITLE").map(|v| v == "0").unwrap_or(false) {
        return;
    }
    let Ok(Some(session)) = crate::db::repos::get_session(&state.pool, session_id).await else {
        return;
    };
    let title = session.title.trim().to_string();
    let is_default = DEFAULT_TITLES.contains(&title.as_str());

    // 轮数 = user 消息数（新消息在 chat 路由先落库，complete 时计数即本轮序号）
    let user_msgs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'user'",
    )
    .bind(session_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);
    // 未命名 → 每次都尝试；已命名 → 每 3 轮查一次漂移
    if !is_default && user_msgs % DRIFT_CHECK_EVERY != 0 {
        return;
    }

    // 喂料：当前标题 + 本轮诉求 + 结论摘要（都截断，命名不需要全文）
    let ui: String = user_input.chars().take(200).collect();
    let head: String = answer.chars().take(200).collect();
    // 标题跟随全局输出语言（`settings` 表 `output_lang`）——与思考/旁白同一权威。
    let lang = crate::agent::output_lang::load(&state.pool).await;
    use crate::agent::output_lang::OutputLang;
    let (system, brief) = match lang {
        OutputLang::Zh => (
            SYSTEM_ZH,
            format!(
                "当前标题：{}\n本轮用户说：{}\n本轮结论（截断）：{}\n\n按系统提示输出 JSON。",
                if title.is_empty() { "（空）" } else { &title },
                ui,
                head
            ),
        ),
        OutputLang::En => (
            SYSTEM_EN,
            format!(
                "Current title: {}\nUser said this turn: {}\nThis turn's conclusion (truncated): {}\n\nFollow the system prompt and output JSON.",
                if title.is_empty() { "(empty)" } else { &title },
                ui,
                head
            ),
        ),
    };

    let req = ResponsesRequest::builder(model)
        .instructions(system.to_string())
        .input(vec![InputItem::user_message(&brief)])
        .temperature(0.1)
        .max_output_tokens(100)
        .user(session_id)
        .build();

    let Ok(res) = state.llm.stream_complete(&req).await else {
        return;
    };
    // 解析 JSON（模型可能裹代码块 → 取最外层 {..}）
    let t = res.output_text.trim();
    let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) else {
        return;
    };
    if b <= a {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&t[a..=b]) else {
        return;
    };
    let rename = v.get("rename").and_then(|x| x.as_bool()).unwrap_or(false);
    let new_title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    // 改名护栏：确认要改、非空、与现名不同、长度合理
    if !rename
        || new_title.is_empty()
        || new_title == title
        || new_title.chars().count() > lang.title_max_chars()
    {
        return;
    }
    // 落库 + 通知前端刷侧栏（失败静默）
    if crate::db::repos::update_session(&state.pool, session_id, Some(&new_title), None)
        .await
        .is_err()
    {
        return;
    }
    let _ = crate::sse::emit(
        state,
        session_id,
        crate::sse::EV_SESSION_TITLE,
        json!({ "title": new_title }),
    )
    .await;
}
