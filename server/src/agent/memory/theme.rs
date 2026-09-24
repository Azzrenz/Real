//! 主题加权记忆 + 偏好/凭证提取

use crate::db::repos;
use crate::error::AppResult;
use sqlx::SqlitePool;

// 关键词钩子——新对话"勾起"几百轮前的重要话题

/// 中文高频虚词单字（保守列表，只剔无信息量的；实义字如"看/删/建"保留）
const ZH_STOP: &[char] = &[
    '的', '了', '是', '在', '有', '我', '你', '他', '她', '它', '们',
    '就', '都', '也', '很', '一', '个', '不', '这', '那', '吗', '吧',
    '呢', '啊', '与', '及', '但', '若', '或', '和', '而',
];

/// 英文停用词（常见虚词 + 指令动词——"帮我/检查"没信息量，不参与检索）
const EN_STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "for", "with", "this", "that", "is", "are",
    "to", "of", "in", "on", "at", "by", "from", "it", "as", "be", "do", "does",
    "will", "would", "can", "could", "should", "have", "has", "was", "were",
    "not", "no", "yes", "you", "your", "we", "our", "they", "them", "i", "me",
    "my", "what", "how", "why", "please", "help", "check", "look", "see",
    "fix", "open", "run", "use", "get", "set", "make", "try", "want", "need",
];

/// 从用户消息提取检索关键词（**无中文分词器的近似方案**）
pub fn extract_keywords(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // 英文词
    for m in regex::Regex::new(r"[A-Za-z_][A-Za-z0-9_]{2,}")
        .unwrap_or_else(|_| regex::Regex::new(r"").unwrap())
        .find_iter(text)
    {
        let w = m.as_str().to_lowercase();
        if !EN_STOP.contains(&w.as_str()) && !out.contains(&w) {
            out.push(w);
        }
    }
    // 中文 3-gram（剔停用字后滑窗；只保留纯汉字 gram——排除英文/标点/空白）
    let cleaned: String = text.chars().filter(|c| !ZH_STOP.contains(c)).collect();
    let chars: Vec<char> = cleaned.chars().collect();
    if chars.len() >= 3 {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for i in 0..=chars.len() - 3 {
            let gram: String = chars[i..i + 3].iter().collect();
            if gram.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) && seen.insert(gram.clone()) {
                out.push(gram);
            }
        }
    }
    out
}

/// 主题是否命中关键词（任一词出现在主题文本中）
pub fn theme_matches(theme: &str, keywords: &[String]) -> bool {
    let t = theme.to_lowercase();
    keywords.iter().any(|k| !k.is_empty() && t.contains(k.as_str()))
}

// 主题打分 / 投入度定性

/// 从计划 objective / 用户消息提取"总提纲"（去指令前缀、去句尾标点、截断 ≤80 字）
fn normalize_theme(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for p in ["请帮我", "请给我", "请帮忙", "请帮我做", "帮我处理", "帮我做", "帮我", "请处理", "请",
              "我希望", "我想让", "我想要", "我要做", "能不能", "可以帮我"] {
        if s.starts_with(p) {
            s = s[p.len()..].trim().to_string();
            break;
        }
    }
    s = s.trim_end_matches(['。', '！', '？', '!', '?', '.', '，', ',', '；', ';']).to_string();
    crate::agent::plan::truncate(&s, 80).to_string()
}

/// 自动判断主题重要性（**重要性为主，重复率为辅**）
fn score_theme(text: &str) -> i64 {
    let t = text.to_lowercase();
    if t.chars().count() < 6 {
        return 50;
    }
    const PERMANENT: [&str; 12] = [
        "架构", "技术选型", "技术栈", "选型", "方向", "指令",
        "原则", "规范", "标准", "策略", "定位", "路线",
    ];
    for w in PERMANENT {
        if t.contains(w) {
            return 95;
        }
    }
    const STRONG: [&str; 21] = [
        "最重要", "核心", "关键", "重点是", "重点", "一定要", "务必", "特别",
        "决定", "我选", "我决定", "选择", "偏好", "喜欢", "必须", "坚持", "倾向",
        "不要", "不能再", "绝对不能", "记住",
    ];
    for w in STRONG {
        if t.contains(w) {
            return 90;
        }
    }
    70
}

/// 主题域归并判定——新主题与现有主题**双向关键词重合 ≥2** → 同一主题域。
fn same_theme_domain(new_theme: &str, existing_value: &str) -> bool {
    let nkws = extract_keywords(new_theme);
    let ekws = extract_keywords(existing_value);
    if nkws.len() + ekws.len() < 2 {
        return false;
    }
    let new_lower = new_theme.to_lowercase();
    let exist_lower = existing_value.to_lowercase();
    let mut hits = 0;
    for k in &nkws {
        if exist_lower.contains(k.as_str()) {
            hits += 1;
        }
    }
    for k in &ekws {
        if new_lower.contains(k.as_str()) {
            hits += 1;
        }
    }
    hits >= 2
}

/// 投入度定性——同主题/同主题域再次出现：+10；**达到 ≥90 直接定性 95 永恒**
fn engagement_score(old_priority: i64, new_base: i64) -> i64 {
    let engaged = old_priority + 10;
    let engaged = if engaged >= 90 { 95 } else { engaged };
    new_base.max(engaged).min(95)
}

/// 篇幅信号——**聊得多 = 重视**
fn apply_effort_boost(score: i64, theme: &str, user_input: &str) -> i64 {
    if score >= 95 {
        return score;
    }
    let kws = extract_keywords(user_input);
    let hits = kws.iter().filter(|k| theme_matches(theme, std::slice::from_ref(*k))).count();
    let long = user_input.chars().count() >= 300;
    if hits >= 2 || (long && hits >= 1) {
        (score + 10).min(95)
    } else {
        score
    }
}

/// 主题上限管理：**任务级重要（priority ≥ 90）永不淘汰**；只淘汰低权重最旧的。
async fn cleanup_themes(pool: &SqlitePool, workspace: &str) -> AppResult<()> {
    const MAX_THEMES: usize = 20;
    let themes = repos::recall_by_type_workspace(pool, workspace, "theme", 200).await?;
    if themes.len() <= MAX_THEMES {
        return Ok(());
    }
    let mut droppable: Vec<&repos::MemoryRow> =
        themes.iter().filter(|t| t.priority < 90).collect();
    if droppable.is_empty() {
        return Ok(());
    }
    droppable.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.created_at.cmp(&b.created_at)));
    let to_del = (themes.len() - MAX_THEMES).min(droppable.len());
    for r in droppable.iter().take(to_del) {
        repos::forget_memory_by_id(pool, r.id).await?;
    }
    Ok(())
}

/// 记忆工作区解析——与 build_memory_prompt 的注入侧**完全同链**
pub(crate) async fn resolve_ws(pool: &SqlitePool, session_id: &str, user_input: &str) -> String {
    let session_ws = repos::get_session_workspace(pool, session_id).await.ok().flatten();
    let ws = crate::path::normalize_workspace(
        &crate::path::extract_anchor_path(user_input).or(session_ws).unwrap_or_default(),
    );
    if !ws.is_empty() {
        return ws;
    }
    // 无锚定会话（纯对话/新会话首轮）：主题/偏好/凭证是用户级记忆，
    repos::get_recent_workspace(pool).await.unwrap_or_default()
}

/// 主题入库（workspace 级跨会话共享）：同主题/同主题域再次出现 → 投入度 +10
pub async fn store_theme(
    pool: &SqlitePool,
    session_id: &str,
    user_input: &str,
    objective: Option<&str>,
) -> AppResult<()> {
    let anchor = resolve_ws(pool, session_id, user_input).await;
    let raw = objective.map(|o| o.to_string()).unwrap_or_else(|| user_input.to_string());
    let theme = normalize_theme(&raw);
    if theme.chars().count() < 4 {
        return Ok(());
    }
    let key = format!("主题:{theme}");
    let mut score = score_theme(&raw);
    // 篇幅信号——聊得多 = 重视
    score = apply_effort_boost(score, &theme, user_input);
    // 投入度定性——同主题（key 相同）或同主题域（关键词重合 ≥2，跨措辞）再次出现
    let themes = repos::recall_by_type_workspace(pool, &anchor, "theme", 50).await?;
    let same_key = themes.iter().find(|t| t.key == key);
    let domain = if same_key.is_none() {
        themes.iter().find(|t| same_theme_domain(&theme, &t.value))
    } else {
        None
    };
    if let Some(old) = same_key.or(domain) {
        score = engagement_score(old.priority, score);
        // repos::remember 是 UPSERT（workspace+key 唯一）——沿用旧 key 直接覆盖
        repos::remember(pool, &anchor, session_id, &old.key, &theme, "theme", score).await?;
    } else {
        repos::remember(pool, &anchor, session_id, &key, &theme, "theme", score).await?;
    }
    cleanup_themes(pool, &anchor).await?;
    Ok(())
}

// 用户偏好记忆（回答粒度/风格/禁忌——跨轮跨会话生效，无需用户重申）

/// 提取用户消息中的偏好陈述（启发式）。覆盖：粒度/简洁/禁忌/风格。
pub fn extract_preferences(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let t = input.to_lowercase();
    // 粒度：用户要深度展开
    if ["具体到", "细节", "详细说", "详细讲", "一步一步", "分步骤", "展开讲", "深入讲"]
        .iter().any(|w| input.contains(w))
    {
        out.push("回答要具体到细节：用户需要深度展开，不要泛泛而谈".to_string());
    }
    // 简洁
    if ["简洁", "简单说", "少说废话", "别啰嗦", "简短", "概要", "一句话"]
        .iter().any(|w| input.contains(w))
    {
        out.push("回答要简洁直接：少铺垫、直接给结论".to_string());
    }
    // 禁忌
    // ⚠️ 语言禁忌（「不要用英文 / 用中文回答」）**不在这里沉淀** —— 输出语言由全局设置
    // `output_lang`（`agent/output_lang.rs`）唯一裁决。留两套判定会互相打架
    // （用户偏好说中文、但界面语言切到英文时无从取舍），故移除本分支。
    if t.contains("不要表情") || t.contains("别加表情") || t.contains("不要 emoji") || t.contains("不要emoji") {
        out.push("回复不要加表情符号".to_string());
    }
    if t.contains("不要术语") || t.contains("别用术语") || t.contains("说人话") {
        out.push("避免专业术语：用通俗语言解释".to_string());
    }
    if t.contains("正式") || t.contains("书面") {
        out.push("回复风格要正式书面".to_string());
    } else if t.contains("口语") || t.contains("随便聊") || t.contains("轻松点") {
        out.push("回复风格要口语化、轻松".to_string());
    }
    out
}

/// 用户偏好入库（workspace 级跨会话；同类别覆盖——最新表述胜出，UPSERT 直写）。
pub async fn store_preferences(pool: &SqlitePool, session_id: &str, user_input: &str) -> AppResult<()> {
    let anchor = resolve_ws(pool, session_id, user_input).await;
    for pref in extract_preferences(user_input) {
        let key = format!("偏好:{}", crate::agent::plan::truncate(&pref, 30));
        repos::remember(pool, &anchor, session_id, &key, &pref, "preference", 90).await?;
    }
    Ok(())
}

// 用户凭证（credential）——用户提供过的 token/密钥直接使用，绝不重新索取

/// 从消息里提取 GitHub token 明文（ghp_/gho_/ghs_/github_pat_ 前缀 + 长度校验）
fn find_github_token(input: &str) -> Option<String> {
    for prefix in ["github_pat_", "ghp_", "gho_", "ghs_"] {
        if let Some(idx) = input.find(prefix) {
            let rest = &input[idx + prefix.len()..];
            let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len());
            let token = format!("{prefix}{rest}", prefix = prefix, rest = &rest[..end]);
            if token.len() >= 24 {
                return Some(token);
            }
        }
    }
    None
}

/// 从用户消息提取凭证（GitHub token 明文，或"token 已给你"描述——无明文也记"已提供"）
pub fn extract_credentials(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = find_github_token(input) {
        out.push(t);
        return out;
    }
    let lower = input.to_lowercase();
    if lower.contains("token")
        && (lower.contains("给你") || lower.contains("已经给") || lower.contains("发给你")
            || lower.contains("已发") || lower.contains("检查一下") || lower.contains("还在用"))
    {
        out.push("已提供（描述，无明文）".to_string());
    }
    out
}

/// 凭证入库（priority 95 永久——这类信息绝不能丢、绝不能重新问）。
pub async fn store_credentials(pool: &SqlitePool, session_id: &str, user_input: &str) -> AppResult<()> {
    let anchor = resolve_ws(pool, session_id, user_input).await;
    for cred in extract_credentials(user_input) {
        let key = "凭证:github".to_string();
        let value = if cred.starts_with("gh") || cred.starts_with("github_pat_") {
            let masked = if cred.len() > 12 {
                format!("{}…{}", &cred[..8], &cred[cred.len() - 4..])
            } else {
                "（已提供）".to_string()
            };
            format!("用户已提供 GitHub 凭证（token {masked}，完整值已存）；push/fork/创建 PR 直接使用，不要重新索取")
        } else {
            format!("用户已提供 GitHub 凭证（{cred}）；push/fork/创建 PR 直接使用，不要重新索取")
        };
        repos::remember(pool, &anchor, session_id, &key, &value, "credential", 95).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod theme_tests;
