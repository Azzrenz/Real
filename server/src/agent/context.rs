//! Context budget: static-first, dynamic-last, prefix-cache friendly.

/// 从合并 prompt 文件切出 marker 段。marker 形如 `<!-- #name -->`
pub fn section(file: &str, name: &str) -> String {
    let start_tag = format!("<!-- #{name} -->");
    let Some(s) = file.find(&start_tag) else {
        tracing::warn!(
            name,
            "话术段未找到：该段注入为空。检查对应 prompts 文件里的 <!-- #name --> 标记是否拼错"
        );
        return String::new();
    };
    let body = &file[s + start_tag.len()..];
    let end = body.find("\n<!-- #").map(|i| i).unwrap_or(body.len());
    strip_html_comments(body[..end].trim())
}

/// 剥掉 HTML 注释块（含跨行）。未闭合的注释视为"注记一直到段末"。
fn strip_html_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// 收敛循环系统提示（固定前缀——与 append-only 协议配套，禁止随轮变化）。
pub static CONVERGENT_SYSTEM_PROMPT: std::sync::LazyLock<&'static str> =
    std::sync::LazyLock::new(|| {
        let persona = section(include_str!("../../prompts/workflow/system.md"), "persona");
        let narration = section(include_str!("../../prompts/workflow/narration.md"), "narration");
        // 留痕纪律（工作日志与长期记忆）：静态部分=怎么记；动态路径由记忆块里的
        let journal = section(include_str!("../../prompts/workflow/journal.md"), "journal");
        let discipline = section(
            include_str!("../../prompts/workflow/output_discipline.md"),
            "output_discipline",
        );
        Box::leak(
            format!("{persona}\n\n{discipline}\n\n{narration}\n\n{journal}").into_boxed_str(),
        )
    });

/// 模板渲染：按 {name} 记号做字面替换（不做转义解析——模板是纯文本，
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod context_tests {
    use super::section;
    use super::CONVERGENT_SYSTEM_PROMPT;

    #[test]
    fn section_slices_by_marker() {
        let doc = "<!-- #a -->A text\n<!-- #b -->B text";
        assert_eq!(section(doc, "a"), "A text");
        assert_eq!(section(doc, "b"), "B text");
    }

    #[test]
    fn section_last_marker_runs_to_eof() {
        let doc = "<!-- #a -->A\n<!-- #b -->B tail";
        assert_eq!(section(doc, "b"), "B tail");
    }

    #[test]
    fn section_missing_marker_returns_empty() {
        assert_eq!(section("plain doc", "x"), "");
    }

    /// 段体里的 HTML 注释**必须被剥掉** —— 注记是给人看的，绝不能进提示词。
    #[test]
    fn section_strips_html_comments() {
        let doc = "<!-- #a -->\n<!-- 给人看的注记\n第二行 -->\n正文 A\n<!-- #b -->B";
        assert_eq!(section(doc, "a"), "正文 A", "段首注释必须剥掉");
        // 段中注释同样剥（将来的注记可能写在正文中间）
        assert_eq!(section("<!-- #a -->前<!-- 注 -->后", "a"), "前后", "段中注释必须剥掉");
        // 未闭合的注释视为"注记一直到段末"：宁可不注入，也不把半截注记发给模型
        assert_eq!(section("<!-- #a -->正文\n<!-- 忘了收尾", "a"), "正文", "未闭合注释按到段末处理");
        // 反面：正文本身不被误伤
        assert_eq!(section("<!-- #a -->a < b 且 c > d", "a"), "a < b 且 c > d");
    }

    /// 段名拼错 = 静默注入空内容（比注入全文更隐蔽）——用测试钉住每个 marker 真的存在。
    #[test]
    fn all_prompt_sections_resolve() {
        let cases: &[(&str, &[&str])] = &[
            (include_str!("../../prompts/workflow/system.md"), &["persona"]),
            (
                include_str!("../../prompts/workflow/narration.md"),
                &["narration"],
            ),
            (
                include_str!("../../prompts/workflow/round_frame.md"),
                &["frame", "memory"],
            ),
            (
                include_str!("../../prompts/workflow/journal.md"),
                &["journal"],
            ),
            (
                include_str!("../../prompts/workflow/output_discipline.md"),
                &["output_discipline"],
            ),
            (
                include_str!("../../prompts/workflow/mirror.md"),
                &["action", "progress", "attribution", "terminate"],
            ),
        ];
        for (file, names) in cases {
            for n in *names {
                assert!(!section(file, n).is_empty(), "话术段 {n} 未取到内容");
            }
        }
    }

    /// 执行纪律必须随"固定前缀"（系统提示）发出，且**只此一份**。
    #[test]
    fn convergent_prompt_carries_execution_discipline() {
        let p = *CONVERGENT_SYSTEM_PROMPT;
        assert!(p.contains("动手前四条铁律"), "铁律段缺失");
        assert!(
            p.contains("不许用「你要哪条：1/2/3」当挡箭牌"),
            "选项菜单禁令必须保留（旧版措辞：不用选项菜单）"
        );
        assert!(
            p.contains("【用户持续指令·原话为准】"),
            "必须按名引用回灌块：turn_log.rs:175 每轮都在注入它，prompt 不点名 = 模型拿到用户历轮禁令却不知道要遵守"
        );
        assert!(
            p.contains("只在含糊到指不出对象时才问"),
            "含糊续接的判据必须保留（旧版措辞：先与上一任务的目标实体简短确认）"
        );
        assert!(
            p.contains("改文件前必须先 read 它"),
            "改前必读是硬闸，不许退回"
        );
    }

    /// 取不取事实由「答案是否依赖事实」决定，**不由模式决定** —— 这条是拿底线换来的。
    #[test]
    fn persona_puts_fact_need_above_mode() {
        let p = *CONVERGENT_SYSTEM_PROMPT;
        assert!(
            p.contains("只在含糊到指不出对象时才问"),
            "提问门槛缺失（旧「判据链第一问」的现代表述）"
        );
        assert!(
            p.contains("能从本轮输入 + 根推断出对象就直接干"),
            "正解缺失：能推断就直接干，不许退化成「凡不确定先问一句」"
        );
        let persona = section(include_str!("../../prompts/workflow/system.md"), "persona");
        assert!(
            !persona.contains("不调用工具"),
            "禁止把「不调用工具」加回 persona：它与「不说假话」冲突（问机制时必须能查证）"
        );
    }

    /// 「对话态不按执行日志写」这条**例外**必须住在旁白部门（narration.md），不在 persona。
    #[test]
    fn narration_carries_mode_switch_rule() {
        let n = section(include_str!("../../prompts/workflow/narration.md"), "narration");
        assert!(n.contains("执行日志"), "旁白形态的定调名必须在这里");
        assert!(n.contains("只在任务态生效"), "形态切换的例外必须住在旁白部门");
        assert!(n.contains("这轮在推进任务吗"), "切形态要给判据，不给清单");
    }

    #[test]
    fn narration_keeps_layout_and_division_discipline() {
        let n = section(include_str!("../../prompts/workflow/narration.md"), "narration");
        for k in ["表格分行写", "行内不用分号挤多步"] {
            assert!(n.contains(k), "旁白排版纪律缺失：{k}（09-11 用户实报，09-16 曾被瘦身抹掉）");
        }
        assert!(
            n.contains("归工具调用的 `reason`")
                || n.contains("「这一步要做什么」归工具调用的"),
            "开场条必须与 §四 分工对齐：意图归 reason，旁白只说方向与依据"
        );
        assert!(
            n.contains("与「同批发齐」共存"),
            "必须给「一轮发多个调用」时的旁白写法——否则同批发齐会把旁白的位置挤没"
        );
    }

    #[test]
    fn persona_carries_execution_stance() {
        let p = *CONVERGENT_SYSTEM_PROMPT;
        for k in [
            "诊断封顶 2 轮",
            "禁止换口径重试",
            "现象工具一次性",
            "无改动轮必须自曝",
        ] {
            assert!(p.contains(k), "「执行方式」缺条款：{k}");
        }
        assert!(
            p.contains("2 轮内定不了论"),
            "「诊断封顶」必须写出出口（2 轮后按最可能假设直接改），否则又是一句口号"
        );
        assert!(
            p.contains("改前 / 改后对照"),
            "对照纪律必须保留：它是「证明这轮真的改了」的唯一凭据"
        );
    }

    #[test]
    fn persona_carries_research_mode() {
        let p = *CONVERGENT_SYSTEM_PROMPT;
        assert!(p.contains("不许把诊断包装成「研究态」来延长"), "研究态的唯一出口必须留在固定前缀里");
        assert!(
            p.contains("研究态只有一个出口：侦察完落到一个确定动作"),
            "出口的判据必须写死，不许只留一个标签"
        );
    }

    /// 留痕机制必须**成对**存在：话术（静态=怎么记）+ 注入（动态=写到哪）。
    #[test]
    fn journal_mechanism_is_paired() {
        let p = *CONVERGENT_SYSTEM_PROMPT;
        assert!(p.contains("工作日志与长期记忆"), "留痕话术段缺失");
        assert!(
            p.contains("今天的日志记了吗"),
            "收尾自检句缺失——这一套的价值全在「写下来」"
        );
        let injected =
            crate::agent::memory::journal::build_journal_prompt("", "2026-09-12-14-55-19", true);
        assert!(
            injected.contains("【工作日志与长期记忆】"),
            "注入段的块名必须与话术里按名引用的一致：{injected}"
        );
    }

    /// 思考骨架**不许写成小标题清单**，且必须**自带长度上限**。
    #[test]
    fn thinking_skeleton_is_prose_not_checklist() {
        let p = *CONVERGENT_SYSTEM_PROMPT;
        assert!(p.contains("不写小标题"), "骨架必须显式禁掉小标题，否则模型照抄加粗标签");
        assert!(
            p.contains("先说这一步要达到什么") && p.contains("最后说落点"),
            "三件事要用「先…再…最后…」串成话（可抄的形已被取消）"
        );
        assert!(
            p.contains("一到三句"),
            "必须给总句数上限：思考按最贵的档计费、且会作为下一轮输入再送一次，长度是成本"
        );
        for old in ["**要做什么**", "**凭什么**", "**怎么落**", "各 1~2 句"] {
            assert!(
                !p.contains(old),
                "旧骨架写法 `{old}` 又出现了：短标签会被原样抄进思考给用户看，或把长度放开"
            );
        }
    }
}
