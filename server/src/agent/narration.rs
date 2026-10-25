//! 旁白缺失记账：模型连续几轮只发工具调用、一句话不写时，把这条事实回灌给它。
//!
//! 为什么需要运行时提醒（而不是只靠提示词）：`narration.md` 写了「每轮都要有」，
//! 但工具调用本身是**可执行的动作**，模型不发一言也能把任务做完 —— 提示词拦不住它。
//! 实测（2026-10-22，133 个工具轮）只有 4 轮带正文：第一轮写一句，之后全沉默。
//!
//! 与 `stall.rs` 的分工：那边判「有没有**进展**」，这里判「有没有**对用户说话**」——
//! 一个任务可能一直在推进却全程沉默，那族判不到。

/// 连续沉默多少轮触发一次提醒。1–2 轮是正常节奏（有时一句话跨两轮才算得清），
/// 3 轮还没开口，用户已经找不到自己在哪了。
pub const SILENT_ROUNDS_AT: u32 = 3;

/// 触发时的回灌文案。纯函数：入参只有沉默轮数，便于单测。
pub fn nudge(silent_rounds: u32) -> Option<String> {
    if silent_rounds < SILENT_ROUNDS_AT {
        return None;
    }
    Some(format!(
        "【旁白缺失】你已连续 {silent_rounds} 轮只发工具调用、一句话没写——用户眼前只有一串工具卡片，\
不知道你在查什么、为什么这么走。下一轮**发调用之前先写 1 句**（≤40 字）：看到了什么事实 + 因此决定怎么走。\
`reason` 是工具行的标题，顶替不了旁白。"
    ))
}

/// 追加到一次性尾部提示（不覆盖已有的停滞/久探提醒——两条一起发才完整）。
pub fn push_tail(tail: &mut Option<String>, hint: String) {
    match tail {
        Some(prev) => {
            prev.push_str("\n\n");
            prev.push_str(&hint);
        }
        None => *tail = Some(hint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_two_rounds_is_normal() {
        assert!(nudge(0).is_none());
        assert!(nudge(1).is_none());
        assert!(nudge(2).is_none(), "两句一开口是正常节奏，别催");
    }

    #[test]
    fn three_silent_rounds_asks_for_one_line() {
        let n = nudge(3).expect("第 3 轮必须提醒");
        assert!(n.contains("旁白缺失"), "块名要能被日志 grep 到：{n}");
        assert!(n.contains("先写 1 句"), "要给可执行的量，不是「注意一下」：{n}");
        assert!(
            n.contains("reason` 是工具行的标题"),
            "必须挡住「reason 已经说了」这个最常见的借口"
        );
        assert!(nudge(9).is_some(), "沉默越久越该催，不许只响一次");
    }

    /// 尾部提示是**一次性**通道，覆盖写会把停滞提醒悄悄吃掉。
    #[test]
    fn tail_appends_instead_of_overwriting() {
        let mut t = Some("【停滞唤醒】先停一下".to_string());
        push_tail(&mut t, "【旁白缺失】写一句".to_string());
        let s = t.unwrap();
        assert!(s.starts_with("【停滞唤醒】") && s.ends_with("写一句"), "{s}");
        assert!(s.contains("\n\n"), "两段之间要留空行，否则挤成一行读不出来");
    }
}
