//! 契约装配域：把任务目标映射成「域工作流话术 + 工作规范」的组合。
//!
//! 从 `workflow.rs` 归位而来。内核自己零域知识——它只负责把注册表里命中的话术拼起来，
//! 与主循环的状态推进无关，故独立成域。

/// 域系统提示：classify 命中的域工作流话术；零域命中返回空列表。
pub fn domain_prompts(goal: &str) -> Vec<&'static str> {
    crate::domains::registered()
        .iter()
        .filter(|d| d.classify(goal))
        .filter_map(|d| d.system_prompt())
        .collect()
}

/// 组合契约：命中域的工作流话术 + 命中的工作规范（内核零域知识）。
pub fn contract_for(goal: &str) -> String {
    let mut out = domain_prompts(goal).join("\n\n");
    let specs = crate::agent::specs::match_specs(goal);
    if specs.is_empty() {
        return out;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&specs);
    out
}
