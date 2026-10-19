//! 注入物裁剪域：控制每轮注入上下文（记忆 / 事实 / 骨架）的体积，护前缀缓存。
//!
//! 从 `workflow.rs` 归位而来。原先四个补丁依次叠出：
//! `cap_inject`（从尾部一刀切）→ `cap_inject_weighted`（按段分额度，替代一刀切），
//! 配套 `inject_weight`（段落权重表）与 `env_cap`（环境变量读取）。
//! 它们与循环本体无关，只做「字符串进 → 字符串出」，故独立成域。

/// 注入物封顶（字符数；env 可调，0=不封顶）——控上下文体积、护前缀缓存。
pub fn cap_inject(s: String, cap: usize, label: &str) -> String {
    if cap == 0 || s.chars().count() <= cap {
        return s;
    }
    let mut out: String = s.chars().take(cap).collect();
    out.push_str(&format!("\n（{label}超过 {cap} 字已截断：需要完整内容时明确说明具体条目，后端可单独回灌）"));
    tracing::info!(label, cap, "注入物超限已截断");
    out
}

/// 读取整型环境变量，缺省时回退 `default`。
pub fn env_cap(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(default)
}

/// 段落权重：轻（认得你是谁）/ 重（别再犯的错、已定结论）/ 中（其余）。
pub fn inject_weight(title: &str) -> usize {
    /// 轻：认得你是谁、有什么凭证、有哪些技能 —— 有就行，不必多占。
    const LIGHT: [&str; 4] = ["用户偏好", "用户画像", "用户凭证", "可用技能"];
    /// 重：**别再犯的错**、**已经定过的结论**、**做过的活** —— 少一句就白跑一轮。
    const HEAVY: [&str; 6] = [
        "失败教训",
        "上轮结论",
        "历史任务记忆",
        "对话脉络",
        "长期记忆",
        "工作日志",
    ];
    if LIGHT.iter().any(|k| title.contains(k)) {
        1
    } else if HEAVY.iter().any(|k| title.contains(k)) {
        3
    } else {
        2
    }
}

/// **按段分额度的注入封顶** —— 替代「从尾部一刀切」。
pub fn cap_inject_weighted(s: String, total: usize, label: &str) -> String {
    if total == 0 || s.chars().count() <= total {
        return s;
    }
    // 分段：以「【」开头的行起新段
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut title = String::new();
    let mut body = String::new();
    for line in s.split_inclusive('\n') {
        if line.trim_start().starts_with('【') {
            if !title.is_empty() || !body.trim().is_empty() {
                sections.push((title.clone(), body.clone()));
            }
            title = line.trim().to_string();
            body.clear();
        } else {
            body.push_str(line);
        }
    }
    if !title.is_empty() || !body.trim().is_empty() {
        sections.push((title, body));
    }
    if sections.len() <= 1 {
        // 单段（或没分段）：退回原有的一刀切，行为不变
        return cap_inject(s, total, label);
    }
    let wsum: usize = sections.iter().map(|(t, _)| inject_weight(t)).sum::<usize>().max(1);
    let mut out = String::new();
    let mut cut = 0usize;
    for (t, b) in &sections {
        let full = format!("{t}{b}");
        let n = full.chars().count();
        let quota = total * inject_weight(t) / wsum;
        if n <= quota {
            out.push_str(&full);
        } else {
            out.extend(full.chars().take(quota));
            out.push_str(&format!(
                "\n…（本段 {n} 字超额度 {quota}，余下 {} 字已略——需要时 read 记忆目录下对应文件）\n",
                n - quota
            ));
            cut += 1;
        }
    }
    tracing::info!(label, sections = sections.len(), cut, total, "注入物按段分额度");
    out
}
