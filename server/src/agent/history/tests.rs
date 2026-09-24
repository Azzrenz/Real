//! 历史装配域测试（测试随模块走；`super::*` 仍指模块入口，拆文件夹前是 history_tests.rs）。

pub(crate) use serde_json::json;
use super::*;

#[cfg(test)]
mod reconcile_dangling_tests {
    use super::*;

    fn asst_with_calls(ids: &[&str]) -> InputItem {
        let tcs: Vec<Value> = ids
            .iter()
            .map(|cid| {
                json!({"call_id": cid, "name": "run", "arguments": "{}", "type": "function_call"})
            })
            .collect();
        InputItem::Message { role: "assistant".into(), content: Value::Array(vec![]), tool_calls: Some(tcs) }
    }

    #[test]
    fn dangling_call_gets_closed() {
        let mut items = vec![asst_with_calls(&["c1"]), InputItem::user_message("继续")];
        let n = reconcile_dangling_calls(&mut items);
        assert_eq!(n, 1, "应补 1 个占位");
        match &items[1] {
            InputItem::Raw(v) => {
                assert_eq!(v["call_id"], json!("c1"));
                assert_eq!(v["type"], "function_call_output");
                assert!(v["output"].as_str().unwrap().contains("未执行"), "占位应说明中断");
            }
            other => panic!("应紧跟 assistant 插入 Raw 输出: {:?}", other),
        }
    }

    #[test]
    fn responded_call_untouched() {
        let out = InputItem::Raw(json!({"type": "function_call_output", "call_id": "c1", "output": "正常输出"}));
        let mut items = vec![asst_with_calls(&["c1"]), out, InputItem::user_message("继续")];
        let n = reconcile_dangling_calls(&mut items);
        assert_eq!(n, 0, "有输出的不应补");
        assert_eq!(items.len(), 3, "结构不变");
    }

    #[test]
    fn multiple_dangling_same_round_kept_in_order() {
        let mut items = vec![asst_with_calls(&["c1", "c2"])];
        let n = reconcile_dangling_calls(&mut items);
        assert_eq!(n, 2);
        let closed: Vec<String> = items
            .iter()
            .filter_map(|it| match it {
                InputItem::Raw(v) => v.get("call_id").and_then(|c| c.as_str()).map(|x| x.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(closed, vec!["c1".to_string(), "c2".to_string()], "按声明序闭合");
    }

    #[test]
    fn function_call_item_also_closed() {
        let mut items = vec![InputItem::FunctionCall {
            call_id: "fc1".into(),
            name: "run".into(),
            arguments: "{}".into(),
        }];
        let n = reconcile_dangling_calls(&mut items);
        assert_eq!(n, 1);
        assert!(matches!(&items[1], InputItem::Raw(v) if v["call_id"] == json!("fc1")));
    }
}

#[cfg(test)]
mod body_archive_tests {
    use super::*;

    fn um(t: &str) -> InputItem {
        InputItem::user_message(t)
    }

    /// 取 Message 文本（content 兼容 String/Array）——断言辅助
    fn text_of(it: &InputItem) -> String {
        match it {
            InputItem::Message { content: Value::String(t), .. } => t.clone(),
            InputItem::Message { content: Value::Array(parts), .. } => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        }
    }

    #[test]
    fn oldest_task_body_summarized_keep_recent_two() {
        // 3 个回合（user+assistant 交替），回合 1 有结论 → 压成摘要，保留最近 2 个回合原文
        let mut items = vec![
            um("回合1诉求"), InputItem::assistant_message("回合1回答正文很长……"),
            um("回合2诉求"), InputItem::assistant_message("回合2回答"),
            um("回合3诉求"), InputItem::assistant_message("回合3回答"),
        ];
        let user_idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
            .map(|(i, _)| i)
            .collect();
        let mut turns = std::collections::HashMap::new();
        turns.insert(
            1i64,
            (
                "回合1诉求完整版".to_string(),
                "回合1的结论：搜索链路已打通".to_string(),
                vec!["D:/x/loop_.rs".to_string()],
            ),
        );
        let (n, packs) = archive_old_body_impl_keep(&mut items, &user_idx, &turns, 2, "sess-test");
        assert_eq!(n, 1, "只归档最旧 1 个回合（保留最近 2）");
        assert_eq!(packs.len(), 1, "归档段各落一个原文包");
        assert!(
            packs[0].0.to_string_lossy().contains("-task-0001.json"),
            "原文包路径按回合序命名（stamp-task-NNNN.json）: {:?}",
            packs[0].0
        );
        assert!(
            packs[0].1.contains("回合1回答正文很长"),
            "原文包必须是**归档前**的正文（splice 之后取就没了）"
        );
        // items[0] 应为摘要（含回合#1 与结论），回合2/3 原文保留
        let t0 = text_of(&items[0]);
        assert!(t0.contains("回合 #1"), "摘要应标回合号: {t0}");
        assert!(t0.contains("搜索链路已打通"), "摘要应带结论: {t0}");
        assert!(t0.contains("回合1诉求完整版"), "摘要应带原诉求(压缩产物全量): {t0}");
        assert!(t0.contains("涉及文件（需改动先 read）：D:/x/loop_.rs"), "摘要应带涉及文件: {t0}");
        assert!(t0.contains("原文包（该回合全部工具调用与回执）："), "摘要应带全文指针: {t0}");
        let texts: Vec<String> = items.iter().map(text_of).collect();
        assert!(texts[1].contains("回合2诉求") && texts[2].contains("回合2回答"), "回合2应原文保留");
        assert!(texts[texts.len()-1].contains("回合3回答"), "回合3应原文保留");
        assert_eq!(items.len(), 5, "3 段(6条) → 摘要化 1 段(2条变1条) = 5");
    }

    #[test]
    fn archived_summary_never_truncates_anchor() {
        let mut items = vec![
            um("上轮诉求：找可验证 Agent 框架/工具的测试题"),
            InputItem::assistant_message("上轮回答正文……"),
            um("下载你认为该下载的，准备好测试"),
            InputItem::assistant_message("本轮回答"),
            um("第三轮诉求：继续整理下载结果"),
            InputItem::assistant_message("第三轮回答"),
        ];
        let user_idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
            .map(|(i, _)| i)
            .collect();
        let mut turns = std::collections::HashMap::new();
        // 结论摘要 400 字（超过旧 take(280) 窗口），下游项目锚点在尾部
        let mut digest = String::new();
        for i in 0..28 {
            digest.push_str(&format!("- 要点{i}：记忆类测试分类说明，长度填充到超过旧截断窗口\n"));
        }
        digest.push_str("- 🎯 归宿：这些测试题是下载下来**给下游项目做测试**用的，不是跑 MemoryData 本身\n");
        let ask = "D:\\agent-bench 里有哪些能验证 Agent 框架/工具执行/上下文/记忆的测试——下载下来给下游项目用";
        turns.insert(1i64, (ask.to_string(), digest.clone(), Vec::new()));
        let (n, _) = archive_old_body_impl_keep(&mut items, &user_idx, &turns, 2, "sess-test");
        assert_eq!(n, 1, "最旧 1 个回合归档，保留最近 2 个原文（keep_recent=2 显式注入）");
        let t0 = text_of(&items[0]);
        assert!(t0.contains("给下游项目做测试"), "归档摘要不得截断归宿锚点(下游项目): {t0}");
        assert!(t0.contains("D:\\agent-bench"), "归档摘要应带完整诉求(含路径): {t0}");
        assert!(t0.contains("要点0"), "归档摘要应带结论要点开头: {t0}");
        assert!(t0.contains("要点27"), "归档摘要应带结论要点结尾(不得取前280字): {t0}");
    }

    #[test]
    fn no_digest_marks_legacy() {
        let mut items = vec![um("旧诉求"), InputItem::assistant_message("旧回答")];
        let user_idx = vec![0usize];
        // 只有 1 回合 + keep=2 → 不动
        let (n, _) = archive_old_body_impl_keep(
            &mut items,
            &user_idx,
            &std::collections::HashMap::new(),
            40,
            "sess-test",
        );
        assert_eq!(n, 0);
        assert_eq!(items.len(), 2);
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::*;
    use serde_json::json;

    fn read_call(id: &str, mode: &str) -> InputItem {
        InputItem::FunctionCall {
            call_id: id.into(),
            name: "read".into(),
            arguments: json!({"paths": ["D:/x/loop_.rs"], "mode": mode}).to_string(),
        }
    }
    fn read_call_path(id: &str, path: &str) -> InputItem {
        InputItem::FunctionCall {
            call_id: id.into(),
            name: "read".into(),
            arguments: json!({"paths": [path], "mode": "full"}).to_string(),
        }
    }
    fn tool_out(id: &str, body: &str) -> InputItem {
        InputItem::Raw(json!({"type": "function_call_output", "call_id": id, "output": body}))
    }

    #[test]
    fn dedup_keeps_latest_full_read_output() {
        // 夹具要**真实体量**：stub 本身约 120 字节，原文比它短时"变短才改"门槛会挡住
        let old_body = format!("旧版全文AAA{}", "x".repeat(400));
        let new_body = format!("新版全文BBB{}", "y".repeat(400));
        let mut items = vec![
            read_call("c1", "full"),
            tool_out("c1", &old_body),
            InputItem::assistant_message("中间旁白"),
            read_call("c2", "full"),
            tool_out("c2", &new_body),
        ];
        dedup_reads_in_task(&mut items, 0);
        match &items[1] {
            InputItem::Raw(v) => {
                let o = v["output"].as_str().unwrap();
                assert!(o.contains("重复读取已略"), "旧副本应 stub 化: {o}");
                assert!(o.contains("loop_.rs"), "stub 应带路径");
            }
            _ => panic!("应仍是 Raw"),
        }
        match &items[4] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().starts_with("新版全文BBB"),
                "最新副本保留"
            ),
            _ => panic!(),
        }
        assert!(matches!(&items[2], InputItem::Message { .. }), "旁白不动");
    }

    #[test]
    fn dedup_ignores_partial_reads_and_other_tools() {
        let mut items = vec![
            read_call("p1", "lines"),
            tool_out("p1", "片段内容"),
            InputItem::FunctionCall {
                call_id: "w1".into(),
                name: "write".into(),
                arguments: r#"{"path":"D:/x/a.py"}"#.into(),
            },
            tool_out("w1", "写入结果"),
        ];
        dedup_reads_in_task(&mut items, 0);
        match &items[1] {
            InputItem::Raw(v) => assert_eq!(v["output"], "片段内容", "lines 读不去重"),
            _ => panic!(),
        }
        match &items[3] {
            InputItem::Raw(v) => assert_eq!(v["output"], "写入结果", "非 read 工具不去重"),
            _ => panic!(),
        }
    }

    #[test]
    fn dedup_stubs_preview_before_full() {
        // 扩展回归：auto 预览 + 显式 full 的组合——
        let mk = |id: &str, mode: &str| InputItem::FunctionCall {
            call_id: id.into(),
            name: "read".into(),
            arguments: serde_json::json!({"paths": ["D:/x/registry.rs"], "mode": mode}).to_string(),
        };
        let mut items = vec![
            mk("p1", "auto"),
            tool_out("p1", &format!("预览片段{}", "x".repeat(400))),
            mk("f1", "full"),
            tool_out("f1", &format!("全文完整版{}", "y".repeat(400))),
        ];
        dedup_reads_in_task(&mut items, 0);
        match &items[1] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().contains("重复读取已略"),
                "full 之前的预览应 stub: {:?}",
                v["output"]
            ),
            _ => panic!(),
        }
        match &items[3] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().starts_with("全文完整版"),
                "全文保留"
            ),
            _ => panic!(),
        }
    }

    #[test]
    fn dedup_scopes_per_path() {
        let mut items = vec![
            read_call_path("a1", "D:/x/a.py"),
            tool_out("a1", &format!("A 旧{}", "x".repeat(400))),
            read_call_path("b1", "D:/x/b.py"),
            tool_out("b1", &format!("B 唯一{}", "y".repeat(400))),
            read_call_path("a2", "D:/x/a.py"),
            tool_out("a2", &format!("A 新{}", "z".repeat(400))),
        ];
        dedup_reads_in_task(&mut items, 0);
        match &items[1] {
            InputItem::Raw(v) => assert!(v["output"].as_str().unwrap().contains("重复读取已略"), "a.py 旧副本略"),
            _ => panic!(),
        }
        match &items[3] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().starts_with("B 唯一"),
                "b.py 唯一读保留"
            ),
            _ => panic!(),
        }
        match &items[5] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().starts_with("A 新"),
                "a.py 最新保留"
            ),
            _ => panic!(),
        }
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use serde_json::json;

    fn user_goal(id: &str) -> InputItem {
        InputItem::Message {
            role: "user".into(),
            content: json!([{"type": "input_text", "text": format!("回合{id}：修某问题")}]),
            tool_calls: None,
        }
    }
    fn tool_out(id: &str, body: &str) -> InputItem {
        InputItem::Raw(json!({"type": "function_call_output", "call_id": id, "output": body}))
    }
    fn reasoning(id: &str) -> InputItem {
        InputItem::Reasoning {
            content: Some(json!([{"type": "reasoning_text", "text": id}])),
            summary: None,
        }
    }
    fn answer(text: &str) -> InputItem {
        InputItem::assistant_message(text)
    }

    #[test]
    fn archive_keeps_last_task_and_conclusions() {
        // 输入取**真实体量**：stub 文案本身约 150 字节，比它还短的输出归档只会变大
        let big = "旧回合的大块工具输出；".repeat(60);
        let mut items = vec![
            user_goal("1"),
            tool_out("c1", &big),
            reasoning("r1"),
            answer("回合1结论：缺口在 X"),
            user_goal("2"),
            tool_out("c2", "新回合的工具输出"),
            answer("回合2结论：已修复"),
        ];
        // 最后一个回合起点 = index 4
        archive_tasks_before(&mut items, 4, 0);
        match &items[1] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().contains("已归档"),
                "旧回合工具输出应归档"
            ),
            _ => panic!(),
        }
        assert!(
            matches!(&items[3], InputItem::Message { .. }),
            "旧回合最终回答保留"
        );
        assert!(matches!(&items[2], InputItem::Reasoning { .. }), "reasoning 不动（契约）");
        match &items[5] {
            InputItem::Raw(v) => assert_eq!(v["output"], "新回合的工具输出", "最后回合完整"),
            _ => panic!(),
        }
    }

    #[test]
    fn archive_needs_two_tasks() {
        let mut items = vec![user_goal("1"), tool_out("c1", "唯一回合输出"), answer("结论")];
        archive_tasks_before(&mut items, 0, 0);
        match &items[1] {
            InputItem::Raw(v) => assert_eq!(v["output"], "唯一回合输出", "单回合不归档"),
            _ => panic!(),
        }
    }

    #[test]
    fn hard_cap_stubs_oldest_tasks_when_topic_continuous() {
        // 话题连续（引用判据永不触发）→ 全靠硬顶兜底：3 回合各 250KB 工具输出（共 750KB
        let big = "大".repeat(250_000);
        let mut items = vec![
            user_goal("1"),
            tool_out("c1", &big),
            answer("回合1结论"),
            user_goal("2"),
            tool_out("c2", &big),
            answer("回合2结论"),
            user_goal("3"),
            tool_out("c3", &big),
            answer("回合3结论"),
        ];
        let user_idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
            .map(|(i, _)| i)
            .collect();
        // 硬顶现在带目标参数（一次改到位到硬顶的 60%）+ 返回改写段数
        let n = archive_hard_cap(&mut items, &user_idx, RAW_OUTPUT_HARD_CAP_BYTES * 6 / 10, 8);
        assert!(n >= 1, "应至少改写一个回合段");
        // 最后回合（idx 6 起的回合3）必须完整保留
        match &items[7] {
            InputItem::Raw(v) => assert_eq!(v["output"], big, "最后回合工具输出保留"),
            _ => panic!(),
        }
        // 至少最旧回合（idx1 的回合1）被 stub
        match &items[1] {
            InputItem::Raw(v) => assert!(
                v["output"].as_str().unwrap().contains("已归档"),
                "最旧回合应被硬顶强制归档: {:?}",
                v["output"].as_str().unwrap().chars().take(20).collect::<String>()
            ),
            _ => panic!(),
        }
    }

    #[test]
    fn stub_span_keeps_call_locator() {
        let big = "旧回合的大块工具输出；".repeat(200);
        let mut items = vec![
            user_goal("1"),
            InputItem::FunctionCall {
                call_id: "c1".into(),
                name: "read".into(),
                arguments: json!({"paths": ["D:/x/loop_.rs"], "mode": "full"}).to_string(),
            },
            tool_out("c1", &big),
            answer("回合1结论"),
            user_goal("2"),
            tool_out("c2", "新回合输出"),
        ];
        let saved = stub_span(&mut items, 0, 4);
        assert!(saved > 1_000, "返回的应是**实测省下的字节**（硬顶累加器据此更新）: {saved}");
        match &items[2] {
            InputItem::Raw(v) => {
                let o = v["output"].as_str().unwrap();
                assert!(o.contains("read"), "stub 应带工具名: {o}");
                assert!(o.contains("paths=D:/x/loop_.rs"), "stub 应带定位: {o}");
                assert!(o.contains("已归档"), "仍是归档 stub: {o}");
            }
            _ => panic!(),
        }
        match &items[5] {
            InputItem::Raw(v) => assert_eq!(v["output"], "新回合输出", "区间外不动"),
            _ => panic!(),
        }
    }

    #[test]
    fn hard_cap_counts_only_real_changes() {
        // 段内回执极短（stub 文案比原文还长）⇒ 压不动。
        let mut items = vec![
            user_goal("1"),
            tool_out("c1", "短"),
            answer("结论1"),
            user_goal("2"),
            tool_out("c2", "短"),
            answer("结论2"),
        ];
        let user_idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
            .map(|(i, _)| i)
            .collect();
        // 目标 1 字节 ⇒ 必然走遍所有段
        let n = archive_hard_cap(&mut items, &user_idx, 1, 8);
        assert_eq!(n, 0, "压不动的段不得计入改写数（旧口径在此虚报）");
        match &items[1] {
            InputItem::Raw(v) => assert_eq!(v["output"], "短", "没省下字节就别改"),
            _ => panic!(),
        }
    }

    #[test]
    fn hard_cap_skips_tiny_gain() {
        let big = "x".repeat(250_000);
        let mut items = vec![
            user_goal("1"),
            tool_out("c1", &big),
            answer("结论1"),
            user_goal("2"),
            tool_out("c2", &big),
            answer("结论2"),
        ];
        let user_idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
            .map(|(i, _)| i)
            .collect();
        // 当前 raw ≈ 500KB；目标 490KB（只超 2%）→ 不动
        let n = archive_hard_cap(&mut items, &user_idx, 490_000, 8);
        assert_eq!(n, 0, "收益不足门槛不得改写（否则白破一次缓存）");
        match &items[1] {
            InputItem::Raw(v) => assert_eq!(v["output"], big, "原文应原样保留"),
            _ => panic!(),
        }
        // 目标 200KB（超出远超 8%）→ 动手
        let n2 = archive_hard_cap(&mut items, &user_idx, 200_000, 8);
        assert!(n2 >= 1, "收益足够时应改写");
    }

    #[test]
    fn stub_span_keeps_spill_pointer() {
        // read 大文件时后端会落 spill 副本并给指针；归档**必须把指针留下** ——
        let filler = "该文件正文很长，需要先看索引。".repeat(40);
        let body = format!(
            "L1: {filler}\n\n[⚠️ 正文尚未进入你的上下文：以上只是索引与头尾预览。全文已存至 D:\\spill\\sess\\read_1.json——声称已阅或回答细节前，必须先用 read 读取该文件获取正文（可按索引行段精读）。]"
        );
        let mut items = vec![
            user_goal("1"),
            InputItem::FunctionCall {
                call_id: "c1".into(),
                name: "read".into(),
                arguments: json!({"paths": ["D:/x/big.rs"], "mode": "full"}).to_string(),
            },
            tool_out("c1", &body),
            answer("回合1结论"),
            user_goal("2"),
            tool_out("c2", "新回合输出"),
        ];
        stub_span(&mut items, 0, 4);
        match &items[2] {
            InputItem::Raw(v) => {
                let o = v["output"].as_str().unwrap();
                assert!(o.contains("read_1.json"), "必须留下 spill 副本指针: {o}");
                assert!(o.contains("别重读源文件"), "应劝阻重读源文件: {o}");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rewrite_gain_threshold_scales_with_input() {
        // 代价 ∝ input ⇒ 门槛也该 ∝ input。
        assert_eq!(govern::rewrite_min_gain_pct(0), 8, "小 input 用基准");
        assert_eq!(govern::rewrite_min_gain_pct(60_000), 8, "60K 仍是基准");
        assert_eq!(govern::rewrite_min_gain_pct(160_000), 15, "160K → 15%");
        assert_eq!(govern::rewrite_min_gain_pct(260_000), 22, "260K → 22%");
        assert_eq!(govern::rewrite_min_gain_pct(10_000_000), 25, "封顶 25%（别无限抬高把上下文撑爆）");
    }

    /// 收益门槛是**比例**（代价 ∝ input），五条改写路径共用。
    #[test]
    fn rewrite_min_saved_bytes_is_a_ratio() {
        assert_eq!(govern::rewrite_min_saved_bytes(100_000, 0), 8_000, "8% x 100KB");
        assert_eq!(govern::rewrite_min_saved_bytes(1_000, 0), 80, "小 input 按比例缩");
        assert_eq!(
            govern::rewrite_min_saved_bytes(3_000_000, 10_000_000),
            750_000,
            "大 input 吃满 25% 封顶"
        );
    }

    #[test]
    fn call_locator_uses_shared_key_table() {
        assert_eq!(
            call_locator("read", r#"{"paths":["D:/x/a.rs"],"mode":"full"}"#),
            "paths=D:/x/a.rs",
            "read 取 paths（定位键表唯一来源）"
        );
        assert_eq!(call_locator("modify", r#"{"file":"D:/x/b.rs"}"#), "file=D:/x/b.rs");
        assert_eq!(call_locator("run", r#"{"command":"cargo build"}"#), "command=cargo build");
        // 未列表具 → 退化取第一个字符串值（保证有定位，不猜语义）
        assert_eq!(call_locator("mystery", r#"{"whatever":"v"}"#), "whatever=v");
        // 参数不是 JSON（截断/异常）→ 空定位，由调用方退化为通用文案
        assert_eq!(call_locator("read", "{坏 JSON"), "");
    }

    #[test]
    fn stale_attachments_drops_old_images_keeps_recent() {
        // 设计决定：附件传完 2-3 轮即压（用户重发带新副本，结论已在后续轮）。
        fn user_with_img(txt: &str) -> InputItem {
            InputItem::Message {
                role: "user".into(),
                content: json!([
                    {"type": "input_text", "text": txt},
                    {"type": "input_image", "image_url": format!("data:image/png;base64,{}", "A".repeat(4_000)), "detail": "low"}
                ]),
                tool_calls: None,
            }
        }
        let mut items = vec![
            user_with_img("回合1带图"),
            answer("回合1结论"),
            user_with_img("回合2带图"),
            answer("回合2结论"),
            user_with_img("回合3带图"),
            answer("回合3结论"),
            user_goal("4"),
            answer("回合4结论"),
        ];
        stale_attachments(&mut items, 3, 0);
        // 回合1（idx0）：图降级 → 无 base64、有占位
        match &items[0] {
            InputItem::Message { content, .. } => {
                let s = serde_json::to_string(content).unwrap();
                assert!(!s.contains("base64"), "回合1 图应归档: {s}");
                assert!(s.contains("已归档"), "回合1 应有占位说明: {s}");
            }
            _ => panic!(),
        }
        // 回合2/3（idx2/idx4）：最近窗口内，图保留
        for i in [2usize, 4] {
            match &items[i] {
                InputItem::Message { content, .. } => {
                    let s = serde_json::to_string(content).unwrap();
                    assert!(s.contains("base64"), "idx{i} 图应保留（最近窗口内）");
                }
                _ => panic!(),
            }
        }
    }

    /// 门槛：**省不到线就一处不动**（不是"少压一点"，是"完全不压"）。
    #[test]
    fn stale_attachments_skips_whole_batch_below_threshold() {
        // 同上：图必须**显著大于**占位文本，否则压缩无收益
        fn img(txt: &str) -> InputItem {
            InputItem::Message {
                role: "user".into(),
                content: json!([
                    {"type": "input_text", "text": txt},
                    {"type": "input_image", "image_url": format!("data:image/png;base64,{}", "A".repeat(4_000)), "detail": "low"}
                ]),
                tool_calls: None,
            }
        }
        let mut items = vec![
            img("回合1带图"),
            answer("回合1结论"),
            user_goal("2"),
            answer("回合2结论"),
            user_goal("3"),
            answer("回合3结论"),
            user_goal("4"),
            answer("回合4结论"),
        ];
        let before = serde_json::to_string(&items).unwrap();
        assert_eq!(
            stale_attachments(&mut items, 3, 1_000_000),
            0,
            "够不到门槛应返回 0"
        );
        assert_eq!(
            serde_json::to_string(&items).unwrap(),
            before,
            "够不到门槛时一个字都不该动"
        );
        // 门槛归零（冷缓存）⇒ 正常压
        assert!(stale_attachments(&mut items, 3, 0) > 0, "门槛归零应正常压");
    }
}

#[cfg(test)]
mod reference_lapse_tests {
    use super::*;

    fn setup(task_count: usize, kws: &[(usize, &str)], inputs: &[(usize, &str)]) -> (
        std::collections::HashMap<usize, Vec<String>>,
        std::collections::HashMap<usize, String>,
    ) {
        let mut kw: std::collections::HashMap<usize, Vec<String>> = Default::default();
        for (seq, k) in kws {
            kw.insert(*seq, vec![k.to_string()]);
        }
        let mut input: std::collections::HashMap<usize, String> = Default::default();
        for (seq, t) in inputs {
            input.insert(*seq, t.to_string());
        }
        let _ = task_count;
        (kw, input)
    }

    #[test]
    fn referenced_in_window_keeps() {
        // T1 kw=定价，T2 输入提及定价（窗口内被引用）→ T1 保留
        let (kw, input) = setup(3, &[(1, "定价"), (2, "配置")], &[(2, "继续改定价阈值"), (3, "换一个话题")]);
        assert_eq!(retire_old_tasks(3, &kw, &input, 2, usize::MAX), vec![2], "T1 被引用应保留，T2 断续应归档");
    }

    #[test]
    fn lapsed_two_tasks_archives() {
        // T1 kw=定价，T2/T3 都不提 → T1 连续 2 回合未引用 → 归档
        let (kw, input) = setup(3, &[(1, "定价"), (2, "配置")], &[(2, "改配置项"), (3, "另一话题")]);
        assert_eq!(retire_old_tasks(3, &kw, &input, 2, usize::MAX), vec![1, 2]);
    }

    #[test]
    fn no_keywords_conservative_keep() {
        let (kw, input) = setup(3, &[], &[(2, "随便"), (3, "其他")]);
        assert_eq!(retire_old_tasks(3, &kw, &input, 2, usize::MAX), Vec::<usize>::new(), "无关键词保守保留");
    }

    #[test]
    fn inclusive_match_counts_as_reference() {
        // T1 kw=缓存，T2 输入含"缓存命中率"（包含式命中）→ T1 保留；
        let (kw, input) = setup(3, &[(1, "缓存"), (2, "配置")], &[(2, "缓存命中太低了"), (3, "别的")]);
        assert_eq!(retire_old_tasks(3, &kw, &input, 2, usize::MAX), vec![2], "T1 被包含式引用保留，T2 断续归档");
    }

    /// **年龄硬判据**：超龄的回合即使被后续任务引用，也必须退场。
    #[test]
    fn aged_round_retires_even_if_referenced() {
        // task_count=5、fresh=3 ⇒ T1 的 age=4 超龄；
        let (kw, input) = setup(
            5,
            &[(1, "定价"), (2, "配置")],
            &[(2, "继续改定价阈值"), (3, "别的"), (4, "无关"), (5, "无关")],
        );
        assert_eq!(
            retire_old_tasks(5, &kw, &input, 2, 3),
            vec![1, 2],
            "T1 age=4 超龄必退（无视引用）；T2 age=3 未超但断续 ⇒ 退"
        );
    }

    /// 新鲜回合不被年龄判据误伤 —— 用户口径："撑死有个 3 回就可以了"。
    #[test]
    fn fresh_rounds_are_kept() {
        // window=4 ⇒ window_start=2，让"T2 引用 T1"落在引用窗口内
        let (kw, input) = setup(5, &[(1, "定价")], &[(2, "继续改定价阈值")]);

        // fresh=3：T1 的 age=4 > 3 ⇒ 年龄硬判据退它（无视引用）
        assert_eq!(
            retire_old_tasks(5, &kw, &input, 4, 3),
            vec![1],
            "age=4 超龄，即使被引用也必须退"
        );

        // fresh=100：年龄不触发 ⇒ 引用豁免生效 ⇒ 原文保留
        assert_eq!(
            retire_old_tasks(5, &kw, &input, 4, 100),
            Vec::<usize>::new(),
            "年龄未超时，被引用的 T1 必须保留"
        );

        // 新鲜档（age ≤ fresh）在任何情况下都不因年龄退场
        let (kw2, input2) = setup(4, &[], &[]);
        assert!(
            !retire_old_tasks(4, &kw2, &input2, 2, 3).contains(&3),
            "age=1 的新鲜回合必须保留"
        );
    }

    /// 年龄判据对**无关键词**的回合同样生效 —— 存量会话（`turn_logs` 无数据）
    #[test]
    fn age_applies_even_without_keywords() {
        let (kw, input) = setup(6, &[], &[]);
        assert_eq!(
            retire_old_tasks(6, &kw, &input, 2, 3),
            vec![1, 2],
            "T1(age=5)/T2(age=4) 超龄 ⇒ 退；T3 起 age≤3 保留"
        );
    }
}

// 回合内轮间治理（成本优化·三）
#[cfg(test)]
mod govern_in_task_tests {
    use super::*;

    /// 回合内 read 调用（assistant 聚合形态）
    fn asst_read(cid: &str, path: &str, mode: &str) -> InputItem {
        let tcs = vec![json!({
            "type": "function_call", "call_id": cid, "name": "read",
            "arguments": json!({"paths": [path], "mode": mode}).to_string(),
        })];
        InputItem::Message { role: "assistant".into(), content: Value::Array(vec![]), tool_calls: Some(tcs) }
    }

    fn big_out(cid: &str, n: usize) -> InputItem {
        InputItem::FunctionCallOutput { call_id: cid.into(), output: "x".repeat(n) }
    }

    #[test]
    fn gate_fires_only_above_safe_line() {
        let safe = crate::agent::history::govern::window_safe_bytes();
        // 未越线：60% 水位 + 剩余 90 轮（旧口径会因外推放行）
        let below = safe * 6 / 10;
        assert_eq!(
            crate::agent::history::intask::elastic_trigger_bytes(below, 10, 100, None),
            usize::MAX,
            "未越线 ⇒ 一个字节都不动（外推预测正是白破缓存的来源）"
        );
        // 越线：必须放行，且目标线低于当前值、不高于安全线
        let over = safe + 1;
        let t = crate::agent::history::intask::elastic_trigger_bytes(over, 10, 100, None);
        assert!(t < over, "越线 ⇒ 目标线必须低于当前值，闸门才放行：{t} vs {over}");
        assert!(t <= safe, "目标线不该高于安全线：{t} vs {safe}");
        // 压完还有余量（一次压到 60% 水位，不是压到底）
        assert!(
            t >= safe * 5 / 10,
            "一次压到 60% 水位即止，不该压到底：{t} vs {safe}"
        );
    }

    #[test]
    fn dedup_in_task_stubs_older_reads_and_keeps_anchor() {
        let mut items = vec![
            InputItem::user_message("目标"),
            asst_read("c1", "a.rs", "full"),
            big_out("c1", 10_000),
            asst_read("c2", "a.rs", "full"),
            big_out("c2", 10_000),
        ];
        let saved = govern_in_task_ungated(&mut items);
        assert!(saved > 5_000, "应 stub c1 的 10KB 副本");
        match &items[2] {
            InputItem::FunctionCallOutput { output, .. } => assert!(output.contains("重复读取已略")),
            o => panic!("c1 输出应被 stub: {:?}", o),
        }
        match &items[4] {
            InputItem::FunctionCallOutput { output, .. } => assert!(output.starts_with("xxx"), "锚点原文保留"),
            o => panic!("c2 是权威全文，不应动: {:?}", o),
        }
    }

    #[test]
    fn dedup_skips_when_savings_below_floor() {
        let mut items = vec![
            InputItem::user_message("目标"),
            asst_read("c1", "a.rs", "full"),
            big_out("c1", 500),
            asst_read("c2", "a.rs", "full"),
            big_out("c2", 500),
        ];
        let saved = govern_in_task_ungated(&mut items);
        assert_eq!(saved, 0, "收益不足不破缓存");
        match &items[2] {
            InputItem::FunctionCallOutput { output, .. } => assert_eq!(output.len(), 500),
            o => panic!("{:?}", o),
        }
    }

    #[test]
    fn cap_stubs_oldest_and_keeps_recent_window() {
        let mut items: Vec<InputItem> = vec![InputItem::user_message("目标")];
        for k in 0..12 {
            items.push(big_out(&format!("c{k}"), 12_000));
        }
        let saved = govern_in_task_ungated(&mut items);
        assert!(
            saved > 40_000,
            "应把总量压回预算内（cap={}）：saved={saved}",
            midgovern_raw_cap_bytes()
        );
        // 保留窗口 = **最近 `MIDGOVERN_KEEP_OUTPUTS` 条**（工作面）。
        let keep = MIDGOVERN_KEEP_OUTPUTS;
        let keep_from = items.len().saturating_sub(keep);
        for idx in keep_from..items.len() {
            match &items[idx] {
                InputItem::FunctionCallOutput { output, .. } => {
                    assert_eq!(
                        output.len(),
                        12_000,
                        "items[{idx}] 属最近 {keep} 条工作面，不许动"
                    );
                }
                o => panic!("items[{idx}]: {:?}", o),
            }
        }
        // 更旧的（窗口外）至少 stub 掉一条
        match &items[1] {
            InputItem::FunctionCallOutput { output, .. } => assert!(output.contains("已压缩")),
            o => panic!("最旧输出应被 stub: {:?}", o),
        }
    }

    #[test]
    fn keep_window_is_three_outputs() {
        let mut items: Vec<InputItem> = vec![InputItem::user_message("目标")];
        for k in 0..40 {
            items.push(big_out(&format!("c{k}"), 30_000));
        }
        let _ = govern_in_task_ungated(&mut items);
        let stubbed = items
            .iter()
            .filter(|it| {
                matches!(it, InputItem::FunctionCallOutput { output, .. } if output.len() != 30_000)
            })
            .count();
        assert_eq!(
            stubbed, 37,
            "窗口必须收到 3（可压 37 条）；若为 32 说明常量还是 8（窗口没收窄）"
        );
        // 最近 3 条（c37..c39 = items[38..41]）是工作面，原文必须保留
        for idx in 38..41 {
            match &items[idx] {
                InputItem::FunctionCallOutput { output, .. } => {
                    assert_eq!(output.len(), 30_000, "items[{idx}] 属最近 3 条工作面，不许动");
                }
                o => panic!("items[{idx}]: {o:?}"),
            }
        }
        // 窗口外第 4 条（c36 = items[37]）必须已被压 —— 这一条是「窗口=3」与「窗口=8」的分界
        match &items[37] {
            InputItem::FunctionCallOutput { output, .. } => assert_ne!(
                output.len(),
                30_000,
                "c36 已滑出 3 条窗口，必须被压（若仍完整说明窗口没收窄）"
            ),
            o => panic!("items[37]: {o:?}"),
        }
    }

    /// 闸门回归：**外推终点不会撞窗口**时，`govern_in_task` 必须一个字都不动。

    /// ① 越线时旧叙述必须折成一行；最近 N 条原样；且**幂等**。
    #[test]
    fn old_narrations_folded_but_recent_kept() {
        let mut items = vec![InputItem::user_message("目标")];
        for i in 0..8 {
            items.push(InputItem::assistant_message(&format!(
                "第{i}轮叙述：{}",
                "这是一段很长的自然语言描述。".repeat(20)
            )));
            items.push(InputItem::user_message(&format!("继续 {i}")));
        }
        // budget=0 ⇒ 恒越线（专测折叠本体；闸门那一侧由 `in_task_governance_skips_below_trigger_line` 守）
        let saved = super::intask::fold_old_narrations(&mut items, 4, 0);
        assert!(saved > 0, "越线时旧叙述必须被折叠");
        let texts: Vec<String> = items
            .iter()
            .filter_map(|it| match it {
                InputItem::Message { role, content, .. } if role == "assistant" => {
                    super::intask::narration_text(content)
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 8);
        for (i, t) in texts.iter().enumerate() {
            if i < 4 {
                assert!(
                    t.contains(super::intask::NARRATION_FOLDED_MARK),
                    "第 {i} 条（旧，越出保留窗口）应已折叠"
                );
            } else {
                assert!(
                    !t.contains(super::intask::NARRATION_FOLDED_MARK),
                    "第 {i} 条（最近 4 条内）必须原样 —— 那是模型正在引用的工作面"
                );
            }
        }
        assert_eq!(
            super::intask::fold_old_narrations(&mut items, 4, 0),
            0,
            "已折叠的条目不得再动（防『折叠的折叠』把信息越折越假）"
        );
    }

    /// ② 截断必须落在**字符边界**上 —— 中文按字节硬切会 panic。
    #[test]
    fn narration_fold_never_splits_a_char() {
        let text = "中".repeat(100);
        let (head, cut) = super::intask::truncate_on_char_boundary(&text, 120);
        assert!(cut, "300 > 120，必须截");
        assert!(head.len() <= 120);
        assert_eq!(head.len() % 3, 0, "必须回退到字符边界，否则截出半个汉字");
        assert!(text.starts_with(head));
        let (same, cut2) = super::intask::truncate_on_char_boundary("短", 120);
        assert!(!cut2, "未超上限不得截");
        assert_eq!(same, "短");
    }

    /// ③ 旧轮次推理被摘成决策句 —— 把**写了没接线**的 `item::placeholder_reasoning_text`
    #[test]
    fn old_reasoning_is_folded_in_task() {
        let mut items: Vec<InputItem> = Vec::new();
        for i in 0..6 {
            items.push(InputItem::Reasoning {
                content: Some(json!([{
                    "type": "reasoning_text",
                    "text": format!(
                        "先看 A，因为它是入口。所以结论：应该改 {i}。因此下一步验证 B。{}",
                        "啰嗦的过程描述。".repeat(30)
                    )
                }])),
                summary: None,
            });
            items.push(InputItem::user_message(&format!("步骤 {i}")));
        }
        let saved = super::intask::fold_old_reasoning(&mut items, 3);
        assert!(saved > 0, "越出保留窗口的推理必须被摘成决策句");
        assert_eq!(
            super::intask::fold_old_reasoning(&mut items, 3),
            0,
            "幂等：`placeholder_reasoning_text` 自带 CONDENSED_MARK，二次调用不得再动"
        );
    }

    #[test]
    fn in_task_governance_skips_below_trigger_line() {
        let safe = super::govern::window_safe_bytes();
        let chunk = safe / 100;
        let mut items = vec![
            InputItem::user_message("目标"),
            asst_read("c1", "a.rs", "full"),
            big_out("c1", chunk),
            asst_read("c2", "a.rs", "full"),
            big_out("c2", chunk),
        ];
        assert!(
            items_bytes(&items) < safe / 2,
            "样本必须远低于安全线，否则本用例无意义（safe={safe}）"
        );
        let before: String = items.iter().map(|it| format!("{it:?}")).collect();
        assert_eq!(
            govern_in_task(&mut items, 5, 50, None),
            0,
            "外推终点不撞窗口 ⇒ 不得改写（改写 = 白破前缀缓存）"
        );
        let after: String = items.iter().map(|it| format!("{it:?}")).collect();
        assert_eq!(after, before, "闸门拦下时条目必须逐字节不变");
    }

    #[test]
    fn elastic_uses_real_delta_not_avg() {
        let safe = super::govern::window_safe_bytes() as u64;
        let now = (safe / 10) as usize;
        // 探针差分：10 轮（round 10 → 20）只涨线的 1% ⇒ 真实增量极平缓。
        let delta = (safe / 100) as usize;
        assert_eq!(
            super::intask::elastic_trigger_bytes(
                now,
                20,
                100,
                Some((10, now.saturating_sub(delta)))
            ),
            usize::MAX,
            "真实增量平缓 ⇒ 不得动手"
        );
    }

    #[test]
    fn steep_delta_below_safe_line_still_does_not_fire() {
        let safe = super::govern::window_safe_bytes() as u64;
        let now = (safe / 2) as usize;
        // 探针差分：10 轮涨了 safe/4 ⇒ per_round = safe/40；remain = 80
        let got = super::intask::elastic_trigger_bytes(
            now,
            20,
            100,
            Some((10, now - (safe / 4) as usize)),
        );
        assert_eq!(
            got,
            usize::MAX,
            "水位未越线 ⇒ 无论增量多陡都不许动手（不预测未来 —— 这是三改的核心契约）"
        );
    }

    #[test]
    fn raw_form_outputs_are_governed_too() {
        let mut items = vec![
            InputItem::user_message("目标"),
            asst_read("c1", "a.rs", "full"),
            InputItem::Raw(json!({"type": "function_call_output", "call_id": "c1", "output": "x".repeat(10_000)})),
            asst_read("c2", "a.rs", "full"),
            InputItem::Raw(json!({"type": "function_call_output", "call_id": "c2", "output": "y".repeat(10_000)})),
        ];
        let saved = govern_in_task_ungated(&mut items);
        assert!(saved > 5_000, "Raw 形态副本同样治理");
        match &items[2] {
            InputItem::Raw(v) => assert!(v["output"].as_str().unwrap().contains("重复读取已略")),
            o => panic!("{:?}", o),
        }
    }

    #[test]
    fn typed_tool_output_must_be_retired_too() {
        let long = "x".repeat(5000);
        let mut items = vec![
            InputItem::user_message("旧回合"),
            InputItem::function_call("c1", "read", "{}"),
            InputItem::function_call_output("c1", &long),
            InputItem::user_message("新回合"),
        ];
        super::archive_tasks_before(&mut items, 3, 0);
        let txt = super::tool_out_text(&items[2]).expect("应能取到工具输出");
        assert!(txt.contains("已归档"), "具名输出必须被退场 stub: {txt}");
        // 摘要形态比旧的"头 300 字"长一点（它多带了要点），但仍远短于原文 5000
        assert!(txt.len() < 800, "退场后必须显著变短: {}", txt.len());
    }

    #[test]
    fn slim_strips_lineno_prefix() {
        // 结构提取靠"行首关键字"，行号前缀会让判据全部失配 —— 剥不掉就等于白抽
        assert_eq!(super::strip_lineno("   12\tpub fn f() {"), "pub fn f() {");
        assert_eq!(super::strip_lineno("1\tcode"), "code");
        // 非行号前缀（正常含 tab 的代码）不得误剥
        assert_eq!(super::strip_lineno("let a\t= 1;"), "let a\t= 1;");
        assert_eq!(super::strip_lineno("pub fn f() {"), "pub fn f() {");
    }

    #[test]
    fn reasoning_placeholder_hits_both_carriers() {
        // reasoning 有两种载体：独立 Reasoning 条目 + assistant 消息里的 reasoning_text 块
        let mut r = InputItem::reasoning_item("很长的思考内容……");
        assert!(super::placeholder_reasoning_text(&mut r), "独立 reasoning 应被占位");
        match &r {
            InputItem::Reasoning { content: Some(Value::Array(b)), .. } => {
                assert!(
                    b[0]["text"].as_str().unwrap_or("").starts_with(super::CONDENSED_MARK),
                    "应替换为**决策句摘要**（不再压成空格：理由不可再生，必须留）"
                );
            }
            _ => panic!(),
        }
        // assistant 消息：reasoning_text 占位，但 tool_calls 与正文必须原样
        let mut m = InputItem::Message {
            role: "assistant".into(),
            content: json!([
                {"type": "reasoning_text", "text": "思考……"},
                {"type": "output_text", "text": "正文不能动"}
            ]),
            tool_calls: Some(vec![json!({"call_id": "c1", "name": "read"})]),
        };
        assert!(super::placeholder_reasoning_text(&mut m), "assistant 载体应被占位");
        if let InputItem::Message { content, tool_calls, .. } = &m {
            assert!(content[0]["text"].as_str().unwrap_or("").starts_with(super::CONDENSED_MARK));
            assert_eq!(content[1]["text"], Value::String("正文不能动".into()), "正文不得改动");
            assert!(tool_calls.is_some(), "tool_calls 不得丢失");
        }
        // 纯文本 assistant / user：不动
        let mut plain = InputItem::assistant_message("普通回答");
        assert!(!super::placeholder_reasoning_text(&mut plain), "无 reasoning 不替换");
    }

    #[test]
    fn experiment_switch_defaults() {
        // 两个实验开关的**实际**默认值（断言的是现状，不是愿望）
        std::env::remove_var("REAL_REASONING_PLACEHOLDER");
        std::env::remove_var("REAL_DROP_PREV_TOOL_OUTPUTS");
        assert!(
            super::reasoning_placeholder_enabled(),
            "reasoning 占位：当前默认开（极性反向），见上方注释"
        );
        assert!(
            !super::drop_prev_tool_outputs_enabled(),
            "跨回合退场默认关"
        );
    }

    #[test]
    fn anchored_window_moves_only_when_input_over_budget() {
        let (s, w) = super::window_start_decision(Some(100), 900, 100, 150, 50_000, 80_000);
        assert_eq!(s, 100, "input 未超预算时起点不动 —— 哪怕条数早已越界");
        assert!(!w, "不动就不写锚");
        // ② 首次无锚 → 建锚在 len-low，需写
        let (s, w) = super::window_start_decision(None, 300, 100, 150, 0, 80_000);
        assert_eq!(s, 200);
        assert!(w);
        // ③ 超预算 ≥2× → 收到 low
        let (s, w) = super::window_start_decision(Some(100), 900, 100, 150, 160_000, 80_000);
        assert!(w, "超预算 2× 应前移");
        assert_eq!(s, 800, "≥2× 收到 low（保留 100 条）");
        // ④ 超 1.0~2.0× → 保留条数从 high 线性收
        let (s, w) = super::window_start_decision(Some(700), 900, 100, 150, 100_000, 80_000);
        assert!(w);
        assert!(s > 700 && s < 800, "介于两档之间应部分前移：{s}");
        // ⑤ 锚已比目标更靠后 → 不退（锚只能前进）
        let (s, w) = super::window_start_decision(Some(880), 900, 100, 150, 320_000, 80_000);
        assert_eq!(s, 880, "锚只能前进不能后退");
        assert!(!w);
        // ⑥ 锚点越界（会话被清理）→ 收敛到当前长度
        let (s, _) = super::window_start_decision(Some(999), 120, 100, 150, 0, 80_000);
        assert_eq!(s, 120);
    }

    #[test]
    fn in_task_old_outputs_are_stubbed_but_recent_kept() {
        // 夹具跟随起切线，不写死字节数：历史上这里写死过 8000，起切线抬高后
        let big_sz = crate::config::settings::DEF_TASK_STUB_MIN_BYTES + 1024;
        let big = "x".repeat(big_sz);
        let small = "y".repeat(100);
        let mut items = vec![
            InputItem::user_message("回合开始"),
            InputItem::function_call_output("c1", &big),
            InputItem::function_call_output("c2", &small),
            InputItem::function_call_output("c3", &big),
            InputItem::function_call_output("c4", &big),
            InputItem::function_call_output("c5", &big),
            InputItem::function_call_output("c6", &big),
        ];
        let user_idx = vec![0usize];
        let saved = super::govern_in_task_outputs(
            &mut items,
            &user_idx,
            2,
            crate::config::settings::DEF_TASK_STUB_MIN_BYTES,
            1_000,
        );
        assert!(saved > 6_000, "应压掉 3 条大回执，实省字节: {saved}");
        let stubbed = (0..items.len())
            .filter(|&i| super::tool_out_text(&items[i]).map(|s| s.contains("已压缩")).unwrap_or(false))
            .count();
        assert_eq!(stubbed, 3, "应压掉 3 条（c1/c3/c4），小回执不算");
        let t1 = super::tool_out_text(&items[1]).unwrap();
        assert!(t1.contains("已压缩") && t1.len() < 900, "c1 应压成结构化摘要: {}", t1.len());
        assert_eq!(super::tool_out_text(&items[2]).unwrap().len(), 100, "小回执不动");
        assert!(super::tool_out_text(&items[3]).unwrap().contains("已压缩"), "c3 应压");
        assert!(super::tool_out_text(&items[4]).unwrap().contains("已压缩"), "c4 应压");
        for i in 5..7 {
            assert_eq!(super::tool_out_text(&items[i]).unwrap().len(), big_sz, "最近 2 条必须保留");
        }
    }

    #[test]
    fn in_task_stub_is_type_and_spill_aware() {
        let big = "x".repeat(6000);
        let mut items = vec![
            InputItem::user_message("回合开始"),
            InputItem::function_call("c_read", "read", "{}"),
            InputItem::function_call_output("c_read", &big),
            InputItem::function_call("c_read2", "read", "{}"),
            InputItem::function_call_output(
                "c_read2",
                &format!(
                    "{big}\n全文已落盘：D:\\spill\\20260915T100322123+0800-call00abc.txt——需要细节请 read 该文件"
                ),
            ),
            InputItem::function_call("c_run", "run", "{}"),
            InputItem::function_call_output("c_run", &big),
            InputItem::function_call("c_keep", "run", "{}"),
            InputItem::function_call_output("c_keep", &big),
        ];
        let user_idx = vec![0usize];
        let saved = super::govern_in_task_outputs(&mut items, &user_idx, 1, 1000, 1_000);
        assert!(saved > 5_000, "应压 2 条，实省字节: {saved}");
        let stubbed = (0..items.len())
            .filter(|&i| super::tool_out_text(&items[i]).map(|s| s.contains("已压缩")).unwrap_or(false))
            .count();
        assert_eq!(stubbed, 2, "应压 2 条：read(带 spill) + run(执行类)");
        let kept = super::tool_out_text(&items[2]).unwrap();
        assert_eq!(kept.len(), big.len(), "探索类无 spill 指针的回执必须保留原文");
        let spilled = super::tool_out_text(&items[4]).unwrap();
        assert!(
            spilled.contains("全文可回取") && spilled.contains("read"),
            "有 spill 的摘要须给可回取指引: {spilled}"
        );
        assert!(
            spilled.contains("call00abc.txt"),
            "路标须带真实 spill 路径: {spilled}"
        );
        let executed = super::tool_out_text(&items[6]).unwrap();
        assert!(executed.contains("已压缩"), "执行类回执应压: {executed}");
    }

    #[test]
    fn in_task_old_outputs_kept_when_below_threshold() {
        // 起切线以下的回执即使很旧也不动（避免无谓破坏前缀）
        let mid = "z".repeat(500);
        let mut items = vec![
            InputItem::user_message("t"),
            InputItem::function_call_output("c1", &mid),
            InputItem::function_call_output("c2", &mid),
            InputItem::function_call_output("c3", &mid),
            InputItem::function_call_output("c4", &mid),
        ];
        assert_eq!(
            super::govern_in_task_outputs(
                &mut items,
                &[0usize],
                2,
                crate::config::settings::DEF_TASK_STUB_MIN_BYTES,
                1_000
            ),
            0,
            "小回执不动"
        );
    }

    #[test]
    fn in_task_frag_segment_is_folded_into_one_summary() {
        let frag = "y".repeat(400);
        let mut items = vec![InputItem::user_message("t")];
        for k in 0..40 {
            items.push(InputItem::function_call(&format!("c{k}"), "run", "{}"));
            items.push(InputItem::function_call_output(&format!("c{k}"), &frag));
        }
        let out_idx: Vec<usize> = (0..items.len())
            .filter(|&i| super::tool_out_text(&items[i]).is_some())
            .collect();
        assert_eq!(out_idx.len(), 40, "夹具：40 条输出");
        let sum = |it: &[InputItem]| -> usize {
            out_idx
                .iter()
                .map(|&i| super::tool_out_text(&it[i]).unwrap().len())
                .sum()
        };
        let before = sum(&items);
        let saved = super::govern_in_task_outputs(
            &mut items,
            &[0usize],
            2,
            crate::config::settings::DEF_TASK_STUB_MIN_BYTES,
            1_000,
        );
        assert!(saved > 8_000, "40 条 400 字符碎片应整段收掉，实省: {saved}");
        let head = super::tool_out_text(&items[out_idx[0]]).unwrap();
        assert!(
            head.contains("本段") && head.contains("次工具回执"),
            "段首应为段摘要: {head}"
        );
        assert!(
            head.contains("要细节"),
            "摘要须给出重新定位的指引（否则模型只能凭记忆）: {head}"
        );
        let marks = out_idx
            .iter()
            .filter(|&&i| super::tool_out_text(&items[i]).unwrap() == super::FRAG_MERGED_MARK)
            .count();
        assert!(marks >= 30, "除段首与最近 2 条外应压成占位，实际 {marks} 条");
        let after = sum(&items);
        assert!(
            after * 4 < before,
            "段摘要后总体积应降到 1/4 以下：{before} → {after}"
        );
    }

    #[test]
    fn in_task_old_reasoning_is_condensed_by_bytes() {
        // 9 条 × 20 K 字节 = 180 K，预算 32 K ⇒ 只保得住最新 1 条（2 条就 40 K 超了）
        let fat = "x".repeat(20_000);
        let mut items = vec![InputItem::user_message("回合开始")];
        for i in 0..9 {
            items.push(InputItem::reasoning_item(&format!("{fat}#{i}")));
        }
        let n = super::govern_reasoning(&mut items, super::TASK_KEEP_REASONING_BYTES, 0);
        assert_eq!(n, 8, "预算 32 K / 单条 20 K ⇒ 只保最新 1 条，其余 8 条摘掉");
        // 最新一条**无条件**原文保留 —— 那是当前工作面
        assert!(
            super::has_reasoning_text(&items[9]),
            "最新一条必须保留原文，否则模型立刻失去上下文"
        );
        let txt = match &items[1] {
            InputItem::Reasoning { content: Some(Value::Array(b)), .. } => {
                b[0]["text"].as_str().unwrap_or("").to_string()
            }
            o => panic!("第 1 条形态异常: {o:?}"),
        };
        assert!(
            txt.starts_with(super::CONDENSED_MARK),
            "应摘成摘要占位（不是裸空格）: {txt}"
        );
        // 幂等：再跑一次不应再算作改写
        assert_eq!(
            super::govern_reasoning(&mut items, super::TASK_KEEP_REASONING_BYTES, 0),
            0,
            "已占位的不重复计"
        );
    }

    /// 回归：**思考压在「入库那一刻」，回合内治理不得再动它**。
    #[test]
    fn reasoning_slimmed_at_intake_not_in_task() {
        // ① 超预算的得裁：头 5K 字 + 中 30K 字 + 尾 3K 字（远超 12 KB 预算）
        let fat = format!(
            "{}{}{}",
            "头".repeat(5_000),
            "中".repeat(30_000),
            "尾".repeat(3_000)
        );
        assert!(
            fat.len() > super::INTAKE_REASONING_BUDGET_BYTES,
            "前提：样本必须超过入库预算"
        );
        let slim = super::slim_reasoning_intake(&fat);
        assert!(slim.len() < fat.len(), "超预算的思考必须被裁");
        assert!(
            slim.len() <= super::INTAKE_REASONING_BUDGET_BYTES,
            "裁后应回到预算内，实际 {}",
            slim.len()
        );
        assert!(
            slim.contains(super::REASONING_INTAKE_MARK.trim()),
            "必须留下省略标记，让模型知道这里被压过"
        );

        // ② 预算内的不得动
        let small = "正常思考：先读文件，再改 error 处理。";
        assert_eq!(
            super::slim_reasoning_intake(small),
            small,
            "预算内的思考必须原样放行"
        );

        // ③ 回合内入口不得再改写思考，否则整段前缀作废
        let mut items = vec![InputItem::user_message("回合开始")];
        items.push(InputItem::reasoning_item(&fat));
        let before: usize = items.iter().map(super::reasoning_text_bytes).sum();
        let _ = super::govern_in_task_ungated(&mut items);
        let after: usize = items.iter().map(super::reasoning_text_bytes).sum();
        assert_eq!(
            after, before,
            "回合内治理已撤除 —— 改写已发出过的前缀会让该点之后全部按未命中重算"
        );
    }

    #[test]
    fn cross_round_reasoning_is_folded_to_zero_content() {
        let mut items = vec![InputItem::user_message("第一回合")];
        for i in 0..3 {
            items.push(InputItem::reasoning_item(&format!("历史推理{i}，这是一段很长的思考过程")));
        }
        items.push(InputItem::user_message("第二回合"));
        for i in 0..2 {
            items.push(InputItem::reasoning_item(&format!("当前推理{i}，仍在工作面内")));
        }
        // 预算给足 ⇒ 当前回合的两条必然全留，隔离出"跨回合折叠"这一个变量
        let n = super::govern_reasoning(&mut items, 32_000, 4);
        assert_eq!(n, 3, "跨回合的 3 条必须全部折叠，实际 {n}");

        let text_of = |it: &InputItem| -> String {
            match it {
                InputItem::Reasoning { content: Some(Value::Array(b)), .. } => {
                    b[0]["text"].as_str().unwrap_or("").to_string()
                }
                o => panic!("形态异常: {o:?}"),
            }
        };
        for i in 1..=3 {
            assert_eq!(
                text_of(&items[i]),
                super::FOLDED_MARK,
                "第 {i} 条是跨回合推理，必须折叠成零内容"
            );
        }
        for i in 5..=6 {
            assert!(
                !text_of(&items[i]).starts_with(super::FOLDED_MARK),
                "第 {i} 条在当前回合内，不得被折叠"
            );
        }
        // 幂等：折叠过的再跑一次不再计
        assert_eq!(
            super::govern_reasoning(&mut items, 32_000, 4),
            0,
            "已折叠的不重复计"
        );
    }

    #[test]
    fn in_task_stub_skips_when_few_outputs() {
        // 输出必须**超过起切线**，否则 0 是"没到门槛"造成的，测不出"条数不够不压"这条规则
        let fat = "z".repeat(crate::config::settings::DEF_TASK_STUB_MIN_BYTES + 1024);
        let mut items = vec![
            InputItem::user_message("t"),
            InputItem::function_call_output("c1", &fat),
            InputItem::function_call_output("c2", &fat),
        ];
        assert_eq!(
            super::govern_in_task_outputs(
                &mut items,
                &[0usize],
                2,
                crate::config::settings::DEF_TASK_STUB_MIN_BYTES,
                1_000
            ),
            0,
            "≤3 条不动"
        );
    }

    #[test]
    fn in_task_stub_honours_caller_threshold() {
        // 同一批数据只改门槛：门槛高得够不着 ⇒ 一条不压；门槛压得很低 ⇒ 该压的全压。
        let big = "x".repeat(9_000);
        let small = "y".repeat(50);
        let build = || {
            vec![
                InputItem::user_message("t"),
                InputItem::function_call_output("c1", &big),
                InputItem::function_call_output("c2", &small),
                InputItem::function_call_output("c3", &big),
                InputItem::function_call_output("c4", &big),
                InputItem::function_call_output("c5", &big),
                InputItem::function_call_output("c6", &big),
            ]
        };
        let mut high = build();
        assert_eq!(
            super::govern_in_task_outputs(&mut high, &[0usize], 2, 1_000_000, 1_000),
            0,
            "门槛高到没人够得着 ⇒ 不压"
        );
        let mut low = build();
        let saved = super::govern_in_task_outputs(&mut low, &[0usize], 2, 100, 1_000);
        assert!(saved > 20_000, "门槛压到 100 ⇒ 大回执都压，实省字节: {saved}");
        let stubbed = (0..low.len())
            .filter(|&i| super::tool_out_text(&low[i]).map(|s| s.contains("已压缩")).unwrap_or(false))
            .count();
        assert_eq!(stubbed, 3, "三条大回执压下（小回执与最近两条不动）");
    }
}

#[cfg(test)]
mod pairing_closure_tests {
    use super::*;

    fn asst(ids: &[&str]) -> InputItem {
        let tcs: Vec<Value> = ids
            .iter()
            .map(|cid| json!({"call_id": cid, "name": "run", "arguments": "{}", "type": "function_call"}))
            .collect();
        InputItem::Message { role: "assistant".into(), content: Value::Array(vec![]), tool_calls: Some(tcs) }
    }

    fn out_raw(cid: &str) -> InputItem {
        InputItem::Raw(json!({"type": "function_call_output", "call_id": cid, "output": "x"}))
    }

    #[test]
    fn orphan_output_is_dropped() {
        let mut items = vec![asst(&["c_keep"]), out_raw("c_keep"), out_raw("c_gone")];
        let n = drop_orphan_outputs(&mut items);
        assert_eq!(n, 1, "应剔掉 1 条孤儿回执");
        assert_eq!(items.len(), 2, "配对完好的那条必须保留");
        assert!(items.iter().any(|it| out_at_call_id(it) == Some("c_keep")));
        assert!(!items.iter().any(|it| out_at_call_id(it) == Some("c_gone")));
    }

    #[test]
    fn clean_pairing_is_untouched() {
        let mut items = vec![asst(&["c1"]), out_raw("c1")];
        assert_eq!(drop_orphan_outputs(&mut items), 0, "配对完整时零改动");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn dangling_call_is_kept_and_closed_by_placeholder() {
        // 调用悬空（回执还没落）是"工具仍在执行"的合法中间态
        let mut items = vec![asst(&["c_pending"])];
        assert_eq!(drop_orphan_outputs(&mut items), 0, "没有可剔的孤儿回执");
        assert_eq!(items.len(), 2, "声明 + 其配对闭合占位");
        assert_eq!(out_at_call_id(&items[1]), Some("c_pending"), "占位必须紧跟声明");
    }

    #[test]
    fn request_boundary_closes_both_directions() {
        // 请求出口必须**双向闭合**：缺输出的一侧补占位，缺调用的一侧剔回执。
        let items = vec![asst(&["c_missing"]), out_raw("c_gone")];
        let out = reconcile_for_request(&items);
        assert!(
            out.iter().any(|it| out_at_call_id(it) == Some("c_missing")),
            "缺输出的一侧应补配对闭合占位"
        );
        assert!(
            !out.iter().any(|it| out_at_call_id(it) == Some("c_gone")),
            "缺调用的一侧应被剔除"
        );
    }

    // ── 归档边界：终点不跨越配对 ──

    #[test]
    fn archive_end_never_splits_pair() {
        // 真实成因：用户在工具执行中途插话 —— 调用在回合1、回执落到回合2。
        let items = vec![
            InputItem::user_message("回合1"),
            asst(&["c1"]),
            InputItem::user_message("回合2（插话）"),
            out_raw("c1"),
        ];
        let e = pair_safe_end(&items, 0, 2);
        assert!(e <= 1, "终点必须收窄到调用之前，否则会切出孤儿回执: {e}");
    }

    #[test]
    fn archive_end_unchanged_when_pair_stays_inside() {
        let items = vec![
            InputItem::user_message("回合1"),
            asst(&["c1"]),
            out_raw("c1"),
            InputItem::user_message("回合2"),
        ];
        assert_eq!(pair_safe_end(&items, 0, 3), 3, "配对完整时终点不动（不许无谓少压）");
    }

    #[test]
    fn archive_end_skips_span_whose_output_lost_its_call() {
        // 段内回执的调用在段外（同一次切分的另一侧）→ 整段不删，
        let items = vec![asst(&["c1"]), InputItem::user_message("回合2"), out_raw("c1")];
        assert_eq!(pair_safe_end(&items, 1, 3), 1, "段起点与其后的回执不可分 → 跳过该段");
    }
}

#[cfg(test)]
mod orphan_order_tests {
    use super::*;

    fn asst(ids: &[&str]) -> InputItem {
        let tcs: Vec<Value> = ids
            .iter()
            .map(|cid| json!({"call_id": cid, "name": "run", "arguments": "{}", "type": "function_call"}))
            .collect();
        InputItem::Message { role: "assistant".into(), content: Value::Array(vec![]), tool_calls: Some(tcs) }
    }

    fn out_raw(cid: &str) -> InputItem {
        InputItem::Raw(json!({"type": "function_call_output", "call_id": cid, "output": "x"}))
    }

    #[test]
    fn output_preceding_its_reused_call_id_is_orphan() {
        let mut items = vec![
            out_raw("c1"),
            InputItem::user_message("中间"),
            asst(&["c1"]),
            out_raw("c1"),
        ];
        let n = drop_orphan_outputs(&mut items);
        assert_eq!(n, 1, "位于其调用之前的回执必须被剔除");
        let pos_call = items
            .iter()
            .position(|it| !call_ids_of(it).is_empty())
            .expect("调用应还在");
        let pos_out = items
            .iter()
            .position(|it| out_at_call_id(it) == Some("c1"))
            .expect("后一条回执应还在");
        assert!(pos_out > pos_call, "留下的回执必须位于其调用之后");
    }

    #[test]
    fn outputs_after_their_call_are_kept() {
        let mut items = vec![asst(&["c1", "c2"]), out_raw("c1"), out_raw("c2")];
        assert_eq!(drop_orphan_outputs(&mut items), 0, "顺序正确的配对不得被误删");
        assert_eq!(items.len(), 3);
    }
}

#[cfg(test)]
mod call_arg_stub_tests {
    //! 跨回合压「工具调用参数」（上下文里最大的一块，占 37.7%）。
    use super::*;
    use crate::model::types::FunctionCall;

    /// 造一个回合：user 消息 + 带 tool_calls 的 assistant 消息
    fn task(items: &mut Vec<InputItem>, cid: &str, args_len: usize) {
        items.push(InputItem::user_message("干活"));
        let args = serde_json::json!({
            "path": format!("D:/proj/f{cid}.rs"),
            "content": "x".repeat(args_len)
        })
        .to_string();
        items.push(InputItem::assistant_with_tools(
            "",
            "写文件",
            &[FunctionCall {
                call_id: cid.into(),
                name: "write".into(),
                arguments: args,
            }],
        ));
    }

    fn args_of(it: &InputItem) -> String {
        match it {
            InputItem::Message {
                tool_calls: Some(tcs),
                ..
            } => tcs
                .first()
                .and_then(|t| t.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        }
    }

    /// 只压「距今 ≥ N 个回合」的调用；**最近 N 个回合原样保留**（模型还在引用）。
    #[test]
    fn only_old_tasks_are_stubbed() {
        let mut items = Vec::new();
        for cid in ["c1", "c2", "c3", "c4", "c5"] {
            task(&mut items, cid, 2_000);
        }
        let user_idx: Vec<usize> = (0..5).map(|i| i * 2).collect();
        assert_eq!(
            super::stub_old_call_args(&mut items, &user_idx, 3, 800),
            2,
            "5 个回合保留最近 3 ⇒ 只压前 2 个"
        );
        assert!(args_of(&items[1]).contains(super::CALL_ARG_STUB_MARK), "旧回合应被压");
        assert!(args_of(&items[3]).contains(super::CALL_ARG_STUB_MARK), "旧回合应被压");
        for i in [5, 7, 9] {
            assert!(
                !args_of(&items[i]).contains(super::CALL_ARG_STUB_MARK),
                "最近 3 个回合绝不能动（模型正在引用）"
            );
        }
    }

    /// 幂等：压过的再跑不算改写 —— **这是"不重复破前缀缓存"的关键**。
    #[test]
    fn stubbing_is_idempotent() {
        let mut items = Vec::new();
        for cid in ["c1", "c2", "c3", "c4", "c5"] {
            task(&mut items, cid, 2_000);
        }
        let user_idx: Vec<usize> = (0..5).map(|i| i * 2).collect();
        assert_eq!(super::stub_old_call_args(&mut items, &user_idx, 3, 800), 2);
        assert_eq!(
            super::stub_old_call_args(&mut items, &user_idx, 3, 800),
            0,
            "第二次必须 0 —— 否则每轮都破一次缓存，压体积的收益会被失效吃光"
        );
    }

    /// 压掉正文，但**定位信息必须留下**（写过哪个文件）—— 这就是"做过什么"的记忆。
    #[test]
    fn locator_survives_and_stays_valid_json() {
        let mut items = Vec::new();
        for cid in ["c1", "c2", "c3", "c4"] {
            task(&mut items, cid, 2_000);
        }
        let user_idx: Vec<usize> = (0..4).map(|i| i * 2).collect();
        assert_eq!(super::stub_old_call_args(&mut items, &user_idx, 2, 800), 2);
        let s = args_of(&items[1]);
        assert!(s.contains("D:/proj/fc1.rs"), "路径必须留下（要知道写过哪个文件）: {s}");
        assert!(!s.contains(&"x".repeat(200)), "正文必须被丢掉: {s}");
        assert!(s.contains("_was"), "必须留下变更迹象: {s}");
        assert!(
            serde_json::from_str::<serde_json::Value>(&s).is_ok(),
            "压完仍必须是合法 JSON: {s}"
        );
    }

    /// 小于门槛的不动 —— 省不了多少，却要多破一次缓存。
    #[test]
    fn tiny_args_untouched() {
        let mut items = Vec::new();
        for cid in ["c1", "c2", "c3", "c4"] {
            task(&mut items, cid, 10);
        }
        let user_idx: Vec<usize> = (0..4).map(|i| i * 2).collect();
        assert_eq!(
            super::stub_old_call_args(&mut items, &user_idx, 2, 800),
            0,
            "参数低于门槛 ⇒ 一条不压"
        );
    }

    /// 只压"重工具"：`read`/`search` 这类小参数工具即便很大也不动（白名单）。
    #[test]
    fn only_heavy_tools_are_eligible() {
        let mut items = Vec::new();
        for cid in ["c1", "c2", "c3", "c4"] {
            items.push(InputItem::user_message("干活"));
            let args = serde_json::json!({"paths": ["D:/a.rs"], "pattern": "x".repeat(3_000)}).to_string();
            items.push(InputItem::assistant_with_tools(
                "",
                "搜",
                &[FunctionCall {
                    call_id: cid.into(),
                    name: "search".into(),
                    arguments: args,
                }],
            ));
        }
        let user_idx: Vec<usize> = (0..4).map(|i| i * 2).collect();
        assert_eq!(
            super::stub_old_call_args(&mut items, &user_idx, 2, 800),
            0,
            "search 不在重工具白名单 ⇒ 不压"
        );
    }
}

#[cfg(test)]
mod call_id_uniqueness_tests {
    use super::*;

    fn asst_args(cid: &str, name: &str, args: &str) -> InputItem {
        InputItem::Message {
            role: "assistant".into(),
            content: Value::Array(vec![]),
            tool_calls: Some(vec![json!({
                "call_id": cid, "name": name, "arguments": args, "type": "function_call"
            })]),
        }
    }

    fn out_raw(cid: &str) -> InputItem {
        InputItem::Raw(json!({"type": "function_call_output", "call_id": cid, "output": "x"}))
    }

    fn ids_of(items: &[InputItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|it| call_ids_of(it).first().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn duplicate_identical_pair_is_dropped() {
        let mut items = vec![
            asst_args("c1", "run", "{\"a\":1}"), out_raw("c1"),
            asst_args("c1", "run", "{\"a\":1}"), out_raw("c1"),
            asst_args("c1", "run", "{\"a\":1}"), out_raw("c1"),
        ];
        let n = drop_orphan_outputs(&mut items);
        assert_eq!(n, 4, "后两对（声明+回执）应被剔除");
        assert_eq!(items.len(), 2, "只留一对");
    }

    #[test]
    fn extra_output_without_extra_call_is_dropped() {
        // 一个调用 + 两条回执 → 上游报 Duplicate tool output
        let mut items = vec![asst_args("c1", "run", "{}"), out_raw("c1"), out_raw("c1")];
        assert_eq!(drop_orphan_outputs(&mut items), 1, "多余回执应被剔除");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn same_id_different_content_is_renamed_not_dropped() {
        // 同 id 但内容不同：不得静默丢信息 → 改名保留
        let mut items = vec![
            asst_args("c1", "run", "{\"a\":1}"), out_raw("c1"),
            asst_args("c1", "run", "{\"b\":2}"), out_raw("c1"),
        ];
        let n = drop_orphan_outputs(&mut items);
        assert_eq!(n, 0, "内容不同不得丢");
        assert_eq!(items.len(), 4, "两条调用都要在");
        let ids = ids_of(&items);
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "重名必须被改开");
    }

    #[test]
    fn distinct_ids_survive_untouched() {
        let mut items = vec![
            asst_args("c1", "run", "{}"), out_raw("c1"),
            asst_args("c2", "run", "{}"), out_raw("c2"),
        ];
        assert_eq!(drop_orphan_outputs(&mut items), 0);
        assert_eq!(items.len(), 4);
    }

    #[test]
    fn orphan_output_before_any_call_is_still_dropped() {
        let mut items = vec![out_raw("c1"), InputItem::user_message("中"), asst_args("c1", "run", "{}"), out_raw("c1")];
        assert_eq!(drop_orphan_outputs(&mut items), 1);
        let ids = ids_of(&items);
        assert_eq!(ids, vec!["c1".to_string()]);
    }
}

#[cfg(test)]
mod pairing_invariant_tests {
    use super::*;

    fn asst(ids: &[&str]) -> InputItem {
        let tcs: Vec<Value> = ids
            .iter()
            .map(|cid| json!({"call_id": cid, "name": "run", "arguments": "{}", "type": "function_call"}))
            .collect();
        InputItem::Message { role: "assistant".into(), content: Value::Array(vec![]), tool_calls: Some(tcs) }
    }

    fn asst_one(cid: &str, name: &str, args: &str) -> InputItem {
        InputItem::Message {
            role: "assistant".into(),
            content: Value::Array(vec![]),
            tool_calls: Some(vec![json!({"call_id": cid, "name": name, "arguments": args, "type": "function_call"})]),
        }
    }

    fn out_raw(cid: &str) -> InputItem {
        InputItem::Raw(json!({"type": "function_call_output", "call_id": cid, "output": "x"}))
    }

    fn out_typed(cid: &str) -> InputItem {
        InputItem::FunctionCallOutput { call_id: cid.into(), output: "x".into() }
    }

    fn fc(cid: &str, name: &str, args: &str) -> InputItem {
        InputItem::FunctionCall { call_id: cid.into(), name: name.into(), arguments: args.into() }
    }

    fn assert_closed(items: &[InputItem]) {
        let v = pairing_violations(items);
        assert!(v.is_empty(), "配对不变式被破坏：{v:?}");
    }

    fn outs_for(items: &[InputItem], cid: &str) -> usize {
        items.iter().filter(|it| out_at_call_id(it) == Some(cid)).count()
    }

    #[test]
    fn repeated_declaration_with_single_output_does_not_leave_call_bare() {
        // 成因 ①：声明重复追加、回执只有一份（旧逻辑把这份唯一的回执带走 → 400）
        let mut items = vec![
            asst_one("c1", "read", "{}"),
            asst_one("c1", "read", "{}"),
            out_raw("c1"),
        ];
        close_tool_pairing(&mut items, "占位");
        assert_closed(&items);
        assert_eq!(outs_for(&items, "c1"), 1, "必须恰好留一份回执");
        assert_eq!(items.iter().filter(|it| !call_ids_of(it).is_empty()).count(), 1, "重复声明只留一份");
    }

    #[test]
    fn function_call_carrier_declaration_keeps_its_output() {
        // 成因 ②：声明是 FunctionCall 载体（配对表原先只登记 Message.tool_calls）
        let mut items = vec![fc("c2", "run", "{}"), out_typed("c2")];
        close_tool_pairing(&mut items, "占位");
        assert_closed(&items);
        assert_eq!(items.len(), 2, "声明与回执都必须在");
        assert_eq!(outs_for(&items, "c2"), 1);
    }

    #[test]
    fn output_before_declaration_is_moved_not_lost() {
        let mut items = vec![out_raw("c3"), InputItem::user_message("中间"), asst(&["c3"])];
        close_tool_pairing(&mut items, "占位");
        assert_closed(&items);
        assert_eq!(outs_for(&items, "c3"), 1, "回执不得丢");
    }

    #[test]
    fn more_declarations_than_outputs_get_placeholders_not_400() {
        // 同 id 不同内容：改名保留；回执不够就地补占位——绝不让声明裸奔
        let mut items = vec![
            asst_one("c4", "run", "{\"a\":1}"),
            asst_one("c4", "run", "{\"b\":2}"),
        ];
        close_tool_pairing(&mut items, "占位");
        assert_closed(&items);
        assert_eq!(items.len(), 4, "两条声明 + 两条回执（1 真 + 1 占位）");
    }

    #[test]
    fn multi_call_message_stays_closed() {
        let mut items = vec![asst(&["x1", "x2"]), out_raw("x1"), out_raw("x2")];
        close_tool_pairing(&mut items, "占位");
        assert_closed(&items);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn reconcile_for_request_always_closes_all_three_shapes() {
        for items in [
            vec![asst(&["c1"]), out_raw("c1"), out_raw("c1")],
            vec![out_raw("c9"), asst(&["c9"])],
            vec![asst(&["a"]), out_raw("b")],
            vec![fc("c7", "run", "{}")],
        ] {
            let out = reconcile_for_request(&items);
            assert_closed(&out);
        }
    }

    #[test]
    fn existing_pairing_is_byte_identical_after_closure() {
        // 配对本来就完整时，闭合成"零改动"（不许无谓改写历史）
        let items = vec![
            InputItem::user_message("回合"),
            asst(&["k1"]),
            out_raw("k1"),
            asst(&["k2"]),
            out_raw("k2"),
        ];
        let out = reconcile_for_request(&items);
        assert_eq!(out.len(), items.len(), "配对完整时条目数不得变化");
        assert_closed(&out);
    }
}

#[cfg(test)]
mod replay_real_session_tests {
    use super::*;
    use std::collections::HashSet;

    /// 声明无对应回执的 call_id 数（裸奔）——闭合前用它量真实病灶。
    fn count_bare(items: &[InputItem]) -> usize {
        let mut decl: HashSet<String> = HashSet::new();
        let mut outp: HashSet<String> = HashSet::new();
        for it in items {
            if let Some(cid) = out_at_call_id(it) {
                outp.insert(cid.to_string());
            } else {
                for c in call_ids_of(it) {
                    decl.insert(c.to_string());
                }
            }
        }
        decl.difference(&outp).count()
    }

    /// 回放真实会话，量一次**回合内治理**的实际收成（需真实库，默认 `#[ignore]`）。
    #[tokio::test]
    #[ignore]
    async fn replay_real_session_intask_compaction() {
        let db = std::env::var("REPLAY_DB").unwrap_or_else(|_| {
            let appdata = std::env::var("APPDATA").unwrap_or_default();
            format!("{appdata}/real-agent/db/real.db")
        });
        let sid = std::env::var("REPLAY_SESSION")
            .unwrap_or_else(|_| "b7a68d28-4f44-4261-8574-40432851b41d".to_string());
        let url = format!("sqlite:{}?mode=ro", db.replace('\\', "/"));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("打开真实库失败");
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT item_json FROM messages WHERE session_id = ?1 AND item_json IS NOT NULL ORDER BY rowid",
        )
        .bind(&sid)
        .fetch_all(&pool)
        .await
        .expect("读 messages 失败");
        let items: Vec<InputItem> = rows
            .iter()
            .filter_map(|j| serde_json::from_str::<InputItem>(j).ok())
            .collect();
        let user_idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, InputItem::Message { role, .. } if role == "user"))
            .map(|(i, _)| i)
            .collect();
        // `govern_in_task_outputs` 的设计口径是**只管当前回合**（从 `user_idx.last()` 起）——
        let mut tb = 0usize;
        let mut ta = 0usize;
        for (k, &s) in user_idx.iter().enumerate() {
            let e = user_idx.get(k + 1).copied().unwrap_or(items.len());
            let out_bytes: usize = (s..e)
                .filter_map(|i| super::tool_out_text(&items[i]).map(|t| t.len()))
                .sum();
            let mut seg: Vec<InputItem> = items[s..e].to_vec();
            let b = super::items_bytes(&seg);
            let _sv = super::govern_in_task_outputs(
                &mut seg,
                &[0usize],
                crate::config::settings::DEF_TASK_KEEP_OUTPUTS,
                crate::config::settings::DEF_TASK_STUB_MIN_BYTES,
                super::govern::intask_total_bytes(),
            );
            let a = super::items_bytes(&seg);
            // 对照（旧口径）：把门槛抬到够不着 = 复现「单条 < 门槛即跳过」的老判据
            let mut seg_old: Vec<InputItem> = items[s..e].to_vec();
            let _ = super::govern_in_task_outputs(
                &mut seg_old,
                &[0usize],
                crate::config::settings::DEF_TASK_KEEP_OUTPUTS,
                1_000_000,
                super::govern::intask_total_bytes(),
            );
            let ao = super::items_bytes(&seg_old);
            println!(
                "  · 回合 {k:2}：{:4} 条 · 回执 {out_bytes:6}（触发线 {}） · 省 {:6}（新） / {:6}（旧口径）",
                seg.len(),
                b.saturating_sub(a),
                b.saturating_sub(ao),
                super::govern::intask_total_bytes()
            );
            tb += b;
            ta += a;
        }
        println!(
            "[回放·回合内治理] 会话 {sid}：{} 个回合合计 {tb} → {ta} 字节（省 {}，{:.1}%）",
            user_idx.len(),
            tb.saturating_sub(ta),
            (tb.saturating_sub(ta)) as f64 * 100.0 / tb.max(1) as f64
        );
    }

    #[tokio::test]
    #[ignore]
    async fn replay_real_session_pairing() {
        let db = std::env::var("REPLAY_DB").unwrap_or_else(|_| {
            let appdata = std::env::var("APPDATA").unwrap_or_default();
            format!("{appdata}/real-agent/db/real.db")
        });
        let sid = std::env::var("REPLAY_SESSION")
            .unwrap_or_else(|_| "fdb574f2-9ef4-4240-990b-9c2483f8163f".to_string());
        let url = format!("sqlite:{}?mode=ro", db.replace('\\', "/"));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("打开真实库失败");
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT item_json FROM messages WHERE session_id = ?1 AND item_json IS NOT NULL ORDER BY rowid",
        )
        .bind(&sid)
        .fetch_all(&pool)
        .await
        .expect("读 messages 失败");

        let mut items: Vec<InputItem> = rows
            .iter()
            .filter_map(|j| serde_json::from_str::<InputItem>(j).ok())
            .collect();
        println!("[回放] 库行 {} → 反序列化 {} 条（session {sid}）", rows.len(), items.len());
        println!("[回放] 闭合前：声明裸奔 = {} 个 call_id", count_bare(&items));

        let (dropped, renamed, filled) =
            close_tool_pairing(&mut items, "（回放）该工具仍在执行中，此为配对闭合占位");
        println!("[回放] 闭合：丢重复 {dropped} / 改名保留 {renamed} / 补占位 {filled}");
        println!("[回放] 闭合后：声明裸奔 = {} 个 call_id", count_bare(&items));

        let bad = pairing_violations(&items);
        println!("[回放] 闭合后：不变式违规 = {} 条", bad.len());
        for b in bad.iter().take(15) {
            println!("[回放]   {b}");
        }
        assert!(bad.is_empty(), "闭合后仍违反配对不变式（取前 3 条）：{:?}", &bad[..bad.len().min(3)]);
    }
}

/// 「当轮帧可复现」回归 —— 依据 `docs/20260915-三段式复核-中段不可复现.md`。
#[cfg(test)]
mod frame_replay_tests {
    use super::*;

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// 模拟 `routes/chat.rs`：用户原文进库（含旁路键 `attachments`）。
    async fn insert_user_raw(pool: &sqlx::SqlitePool, sid: &str, text: &str) -> String {
        let mut ij = json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}],
        });
        ij["attachments"] = json!([{"name": "a.png"}]);
        crate::db::repos::insert_message(pool, sid, "user", text, Some(&ij.to_string()))
            .await
            .unwrap()
            .id
    }

    /// 末条的 `content` 值（统一走 `Value::to_string()`，两侧同一把尺子）
    fn last_content(items: &[InputItem]) -> String {
        match items.last() {
            Some(InputItem::Message { content, .. }) => content.to_string(),
            other => panic!("末条不是 message：{other:?}"),
        }
    }

    async fn row_item_json(pool: &sqlx::SqlitePool, id: &str) -> String {
        let (js,): (String,) = sqlx::query_as("SELECT item_json FROM messages WHERE id = ?1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        js
    }

    #[tokio::test]
    async fn 帧写回后重建得到同一字节() {
        let pool = test_pool().await;
        let s = crate::db::repos::create_session(&pool, "t", "t").await.unwrap();
        let row_id = insert_user_raw(&pool, &s.id, "把中段断点查清楚").await;

        // ① 首次重建：末条是**用户原文**，且必须把它的库内行 id 交出来（workflow 要靠它写回）
        let h1 = build_history(&pool, &s.id).await.unwrap();
        assert_eq!(
            h1.last_user_id.as_deref(),
            Some(row_id.as_str()),
            "末条 user 的行 id 必须交出，否则调用方无法把帧写回同一行"
        );
        let raw = last_content(&h1.items);
        assert!(raw.contains("把中段断点查清楚"), "首次重建应拿到用户原文：{raw}");

        // ② 模拟 workflow：渲染当轮帧，**替换**末条，并写回同一行
        let frame = json!([{
            "type": "input_text",
            "text": "【本轮用户输入】把中段断点查清楚\n\n{contract}"
        }])
        .to_string();
        crate::db::repos::update_message_item_content(&pool, &row_id, &frame)
            .await
            .unwrap();

        // ③ 再重建：**必须**逐字节等于刚写回的帧 ⇒ 下一个 run 不会在此分叉
        let h2 = build_history(&pool, &s.id).await.unwrap();
        assert_eq!(
            last_content(&h2.items),
            frame,
            "重建结果必须与写回的帧逐字节一致，否则该位置之后每轮按 miss 重算"
        );
        assert_eq!(h2.last_user_id.as_deref(), Some(row_id.as_str()), "行 id 应保持稳定");

        // ④ 反例：帧与用户原文本就不同 —— 所以"不写回"必然分叉（这条是给未来的自己看的）
        assert_ne!(raw, frame, "若两者相同，本机制就没有存在意义，说明帧没被真正渲染");

        // ⑤ 只换 content：行的其余结构仍在（可反序列化回同一条 InputItem）
        let stored = row_item_json(&pool, &row_id).await;
        assert!(
            serde_json::from_str::<InputItem>(&stored).is_ok(),
            "写回后必须仍能反序列化为 InputItem：{stored}"
        );
    }
}

/// 旧帧瘦身 —— 依据 `docs/20260915-三段式复核-中段不可复现.md`。
#[cfg(test)]
mod stale_frame_tests {
    use super::*;

    const FULL: &str = "【本轮用户输入】{goal}【项目上下文】（会话记忆与项目画像：本项目已做过什么）\n记忆内容\n\n【记忆使用指引】…\n\n（本轮原话，未必是任务指令——提问/纠正/粘贴的引用按字面回应；仅指令作为方向基准。）\n\n## 工作规范（与本次任务相关）\n\n### SP-SHELL\n规则正文\n";

    fn frame(goal: &str, full: bool) -> InputItem {
        let t = if full { FULL.replace("{goal}", goal) } else { format!("【本轮用户输入】{goal}") };
        InputItem::Message {
            role: "user".into(),
            content: json!([{ "type": "input_text", "text": t }]),
            tool_calls: None,
        }
    }

    fn text_of(it: &InputItem) -> String {
        match it {
            InputItem::Message { content: Value::Array(bs), .. } => bs
                .first()
                .and_then(|b| b.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        }
    }

    #[test]
    fn 旧帧只留本轮用户输入_近两帧原样() {
        let mut items = vec![
            frame("旧任务A", true),
            frame("旧任务B", true),
            frame("近任务C", true),
            frame("当轮D", true),
        ];
        let saved = trim_stale_frames(&mut items, 2, 0);
        assert!(saved > 0, "前两个回合的帧应当被瘦身");
        assert_eq!(text_of(&items[0]), "【本轮用户输入】旧任务A", "只留 goal");
        assert_eq!(text_of(&items[1]), "【本轮用户输入】旧任务B");
        assert!(text_of(&items[2]).contains("【项目上下文】"), "近 2 回合须保全貌");
        assert!(text_of(&items[3]).contains("## 工作规范"), "当轮帧更不许动");
        // 幂等：已经瘦过的不再产生收益（也就不会再记一条 ctx.rewrite）
        assert_eq!(trim_stale_frames(&mut items, 2, 0), 0, "瘦身必须幂等");
    }

    /// 门槛：**省不到线就一处不动**。
    #[test]
    fn 旧帧瘦身_省不到门槛就一处不动() {
        let mut items = vec![
            frame("旧任务A", true),
            frame("旧任务B", true),
            frame("当轮C", true),
        ];
        let before: Vec<String> = items.iter().map(text_of).collect();
        assert_eq!(
            trim_stale_frames(&mut items, 1, 1_000_000),
            0,
            "够不到门槛应返回 0"
        );
        assert_eq!(
            items.iter().map(text_of).collect::<Vec<_>>(),
            before,
            "够不到门槛时一个字都不该动"
        );
        // 门槛归零（冷缓存）⇒ 正常瘦身
        assert!(trim_stale_frames(&mut items, 1, 0) > 0, "门槛归零应正常瘦身");
    }

    #[test]
    fn 找不到段界就一个字不动() {
        // 裸帧（没有记忆/说明/规范）：不能猜着砍 —— 宁可不动，不可改错
        let mut items = vec![frame("只有目标", false), frame("x", false), frame("y", false)];
        let before: Vec<String> = items.iter().map(text_of).collect();
        assert_eq!(trim_stale_frames(&mut items, 1, 0), 0);
        assert_eq!(items.iter().map(text_of).collect::<Vec<_>>(), before);
    }

    /// 段标记的**资产契约**：`trim_stale_frames` 靠这几个串划界，
    #[test]
    fn frame_marks_match_assets() {
        let tpl = crate::agent::context::section(
            include_str!("../../../prompts/workflow/round_frame.md"),
            "frame",
        );
        assert!(tpl.contains(FRAME_HEAD), "段首标记与 round_frame.md 不一致：{tpl}");
        assert!(tpl.contains(FRAME_NOTE_MARK), "「本轮原话」那句的措辞变了");
        let mem = crate::agent::context::section(
            include_str!("../../../prompts/workflow/round_frame.md"),
            "memory",
        );
        assert!(mem.starts_with(FRAME_MEMORY_MARK), "记忆段首行变了：{mem}");
        let contract = crate::agent::specs::match_specs("这个文件太大了 读不完 全部读");
        assert!(
            contract.contains(FRAME_CONTRACT_MARK),
            "规范头变了（specs::match_specs 输出头）：{contract}"
        );
    }

    /// 全套治理只认**一个比例**：水位线 = 发送线 = 深压线。
    #[test]
    fn one_ratio_for_whole_pipeline() {
        use super::govern::{th, window_safe_bytes, window_safe_tokens};
        // `OBSERVED_ROUND_BYTES` 是进程级 static，跨用例共享 ⇒ 先复位。
        super::govern::reset_observed_round_bytes();
        assert_eq!(
            th::WINDOW_SAFE_PERMILLE,
            5,
            "水位线：窗口 0.5%（2026-09-22 由 70 降到 5 —— 存量压到 ~5.2K token）"
        );
        assert_eq!(
            th::SEND_SAFE_PERMILLE,
            70,
            "发送线 = 窗口 7%（≈73.4K token）：单回合尾段(≈1 万 token)的 7 倍余量"
        );
        let safe = window_safe_tokens();
        // 水位线**只放宽不收窄**（`watermark_bytes` 的下界就是稳态值）⇒ 断区间而非定值
        assert!(
            (5_242..=28_000).contains(&safe),
            "水位线应落在 [稳态 5,242, 硬顶 ≈27.4K]，实测 {}",
            safe
        );
        assert_eq!(
            safe,
            (window_safe_bytes() as u64 * 100 / th::WINDOW_B_PER_TOKEN_X100) as i64,
            "token 与字节两条口径必须落在**同一个点**（自动推导后尤其）"
        );
    }
}

/// `enforce_token_ceiling`（第 ⑦ 步发送前兜底）的回归。
#[cfg(test)]
mod token_ceiling_tests {
    use super::govern::th;
    use super::*;

    fn est_tokens_of(bytes: usize) -> usize {
        bytes * 100 / th::WINDOW_B_PER_TOKEN_X100 as usize
    }

    fn cap_tokens() -> usize {
        (th::WINDOW_TOKENS * th::SEND_SAFE_PERMILLE / 1000) as usize
    }

    fn asst(text: &str) -> InputItem {
        InputItem::Message {
            role: "assistant".into(),
            content: Value::String(text.into()),
            tool_calls: None,
        }
    }

    /// 越线后必须**尽量贴着 cap** 保留历史，而不是退到最老的可切点。
    #[test]
    fn keeps_prefix_as_long_as_possible() {
        let cap = cap_tokens();
        let body = "x".repeat(2_000);
        // 200 轮 × 2 条 ≈ 40 万字节 ⇒ est ≈ 11 万 token > cap（≈7.3 万）
        let mut items: Vec<InputItem> = (0..200)
            .flat_map(|i| {
                [
                    InputItem::user_message(&format!("第{i}轮 {body}")),
                    asst("收到"),
                ]
            })
            .collect();
        let total = items.len();

        let dropped = enforce_token_ceiling(&mut items);

        assert!(dropped > 0, "样本必须越线，否则用例没意义");
        assert!(dropped < total, "切点落在 user 上，至少要留一条");
        let after = est_tokens_of(items_bytes(&items));
        assert!(after <= cap, "残留仍越线：{after} > {cap}");
        assert!(
            after * 2 > cap,
            "砍穿了：只留 {after} token（cap={cap}），应当保留接近 cap 的历史 —— \
             这是 `.rev()` 回归的症状（现场曾 dropped=718 / remaining=1 / est=21）"
        );
    }

    /// 越线且「最后一条 user 起」本身就超 cap（单条巨大）⇒ 只保这一段，
    #[test]
    fn giant_last_user_keeps_only_that_segment() {
        let cap = cap_tokens();
        // 单条 ≈ 3×cap 的巨型 user
        let huge = "y".repeat(cap * 3 * th::WINDOW_B_PER_TOKEN_X100 as usize / 100);
        let mut items = vec![
            InputItem::user_message("老一"),
            InputItem::user_message("老二"),
            InputItem::user_message(&huge),
        ];

        let dropped = enforce_token_ceiling(&mut items);

        assert_eq!(dropped, 2, "只切到「最后一条 user」");
        assert_eq!(items.len(), 1);
        assert!(
            matches!(&items[0], InputItem::Message { role, .. } if role == "user"),
            "保留下来的必须还是那条 user"
        );
    }

    /// 没越线就一个字不动 —— 别把「正常」当成「该压」。
    #[test]
    fn under_cap_is_noop() {
        let mut items = vec![InputItem::user_message("短"), asst("也不长")];
        let before = items.len();
        assert_eq!(enforce_token_ceiling(&mut items), 0);
        assert_eq!(items.len(), before, "未越线不得改动");
    }
}
