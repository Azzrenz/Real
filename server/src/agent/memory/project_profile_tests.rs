    use super::{target_anchor_full, AnchorInput};

    /// 造一个锚行输入（测试简写）
    fn ctx<'a>(
        ws: Option<&'a str>,
        named: Option<(&'a str, &'a str)>,
        prev: Option<&'a str>,
    ) -> AnchorInput<'a> {
        AnchorInput {
            workspace: ws,
            named,
            prev,
        }
    }

    #[test]
    fn named_path_wins_over_workspace() {
        // 点名路径必须真实存在（实现里用 is_dir / is_file 判定）—— 用编译期注入的
        // crate 目录，任何机器上都成立；工作区给一个不存在的路径，以证明点名优先。
        let real = env!("CARGO_MANIFEST_DIR");
        let t = target_anchor_full(
            &format!("把 {real} 里的东西优化一下"),
            ctx(Some("D:////NoSuchWorkspace"), None, None),
        )
        .expect("点名必须产出锚");
        assert!(t.contains(real), "点名优先: {t}");
        assert!(t.contains("用户本轮点名"), "须标出来源: {t}");
    }

    #[test]
    fn workspace_used_when_no_named_path() {
        let t = target_anchor_full("把这个 prompt 优化一下", ctx(Some("D:////project-b"), None, None))
            .expect("有工作区应产出锚");
        assert!(t.contains("project-b"), "回落到会话工作区: {t}");
        assert!(t.contains("会话工作区"), "须标出来源: {t}");
    }

    #[test]
    fn absent_when_nothing_known() {
        assert!(
            target_anchor_full("优化一下", ctx(None, None, None)).is_none(),
            "无工作区且未点名 → 不注入"
        );
        assert!(
            target_anchor_full("优化一下", ctx(Some("   "), None, None)).is_none(),
            "空白同样不注入"
        );
    }

    // ---- 二期：锚行拆两条 + 口语项目名命中名录 + 换项目标注 ----

    #[test]
    fn anchor_split_into_task_and_root() {
        let t = target_anchor_full("帮我看下这个函数为什么慢", ctx(Some(r"D:\proj"), None, None))
            .expect("有工作区应产出锚");
        assert!(
            t.contains("【本轮任务】帮我看下这个函数为什么慢"),
            "第一行须是本轮任务原话（实报：多轮后搞不清本轮问什么）: {t}"
        );
        assert!(t.contains(r"【项目根】D:\proj"), "第二行须是项目根: {t}");
    }

    #[test]
    fn registry_alias_beats_workspace_and_marks_switch() {
        // 上一轮在另一个项目，本轮口语点名 "real" → 必须切到 D:\proj，并如实标注切换
        let t = target_anchor_full(
            "你给我查real这个问题是在哪儿",
            ctx(
                Some(r"D:\proj"),
                Some((r"D:\proj", "real")),
                Some(r"D:\project-c"),
            ),
        )
        .expect("名录命中应产出锚");
        assert!(t.contains(r"【项目根】D:\proj"), "名录命中须生效: {t}");
        assert!(t.contains("命中项目名录"), "来源须写名录命中: {t}");
        assert!(
            t.contains("本轮切换") && t.contains(r"D:\project-c"),
            "换项目须如实标注: {t}"
        );
    }

    #[test]
    fn no_switch_note_when_same_root() {
        let t = target_anchor_full(
            "继续",
            ctx(Some(r"D:\proj"), Some((r"D:\proj", "real")), Some(r"D:/proj/")),
        )
        .expect("应产出锚");
        assert!(!t.contains("本轮切换"), "同根（仅分隔符差异）不得谎称切换: {t}");
    }

    #[test]
    fn dirty_prev_value_never_reported() {
        // 上一轮根是存量脏值 → 不许出现在锚行里（每轮被当权威播报正是旧病）
        let t = target_anchor_full(
            "继续",
            ctx(Some(r"D:\proj"), None, Some("s://www.bilibili.com/video")),
        )
        .expect("应产出锚");
        assert!(!t.contains("bilibili"), "脏值不得回灌锚行: {t}");
    }
