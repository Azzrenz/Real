//! workflow.rs 的测试外置（设计决定：workflow.rs 是主循环基础骨架文件，
use super::*;

#[cfg(test)]
mod upstream_unavailable_tests {
    use super::*;

    #[test]
    fn http_5xx_and_408_are_retryable() {
        // 现场：DeepSeek 说"服务繁忙"最常见就是以 503 回来，
        for s in [
            "LLM 流式 HTTP 500: internal error",
            "LLM 流式 HTTP 502: Bad Gateway",
            "LLM 流式 HTTP 503: 服务繁忙",
            "LLM 流式 HTTP 504: gateway timeout",
            "LLM 流式 HTTP 408: request timeout",
        ] {
            assert!(is_upstream_unavailable(&AppError::Llm(s.into())), "{s} 应判为上游不可用");
            assert!(is_transport_error(&AppError::Llm(s.into())), "{s} 应可重试");
        }
    }

    #[test]
    fn original_transport_errors_do_not_regress() {
        for s in [
            "SSE 读取失败: xxx",
            "流式网络请求失败: xxx",
            "SSE 解析失败: xxx",
            "流异常早停（第 3 次）",
        ] {
            assert!(is_transport_error(&AppError::Llm(s.into())), "{s} 应可重试（回归护栏）");
        }
    }

    #[test]
    fn model_and_request_errors_are_not_upstream() {
        // 400（请求本身坏）/ 402（余额不足）**不归上游**：重试一万次也不会好，
        assert!(!is_upstream_unavailable(&AppError::Llm(
            "LLM 流式 HTTP 400: invalid request".into()
        )));
        assert!(!is_upstream_unavailable(&AppError::Llm(
            "LLM 流式 HTTP 402: Insufficient Balance".into()
        )));
        // 模型自己的问题（空响应/校验失败）也不算上游。
        assert!(!is_upstream_unavailable(&AppError::Llm("模型连续空响应".into())));
        assert!(!is_transport_error(&AppError::Validation("字段缺失".into())));
    }
}

#[cfg(test)]
mod empty_spin_tests {
    use super::*;
    use crate::model::types::{StreamStatus, Usage};

    fn resp_with(fc: usize, text: &str, out_tok: u64, think: &str) -> StreamResult {
        let mut calls = Vec::new();
        for i in 0..fc {
            calls.push(FunctionCall {
                call_id: format!("c{i}"),
                name: "run".into(),
                arguments: "{}".into(),
            });
        }
        StreamResult {
            output_text: text.into(),
            reasoning: think.into(),
            function_calls: calls,
            usage: Some(Usage {
                input_tokens: 100,
                input_tokens_details: None,
                output_tokens: out_tok,
                output_tokens_details: None,
                reasoning_tokens: None,
                total_tokens: None,
            }),
            status: StreamStatus::Completed,
            error: None,
        }
    }

    #[test]
    fn one_token_spin_detected() {
        // 复现现场：out=1、无文本、无工具、思考 1 token
        let r = resp_with(0, "", 1, "The");
        assert!(empty_spin_signal(&r), "1-token 空转应命中");
    }

    #[test]
    fn tool_round_not_spin() {
        assert!(!empty_spin_signal(&resp_with(1, "", 1, "")), "有工具不算空转");
    }

    #[test]
    fn substantive_thinking_not_spin() {
        // 无工具但思考量大 = 设计决定：的正常推进（调研/构思），不熔断
        let long: String = "在分析".chars().cycle().take(200).collect();
        assert!(!empty_spin_signal(&resp_with(0, "", 600, &long)), "大输出思考不算空转");
    }

    #[test]
    fn plain_answer_not_spin() {
        assert!(!empty_spin_signal(&resp_with(0, "好了", 12, "")), "有答案不算空转");
    }
}

#[cfg(test)]
mod envelope_timeout_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_respects_tool_timeout_param() {
        // run 给了 timeout=300：信封应放到 315，而不是 60s 硬掐
        let t = tool_envelope_secs(60, "run", &json!({"command": "x", "timeout": 300}));
        assert_eq!(t, 315, "信封应跟随工具 timeout 参数 +15s 余量");
    }

    #[test]
    fn envelope_default_when_no_timeout_param() {
        assert_eq!(tool_envelope_secs(60, "read", &json!({"paths": ["a.rs"]})), 60);
        assert_eq!(tool_envelope_secs(120, "modify", &json!({})), 120);
    }

    #[test]
    fn envelope_ignores_nonpositive_timeout() {
        assert_eq!(tool_envelope_secs(60, "read", &json!({"timeout": 0})), 60);
        assert_eq!(tool_envelope_secs(60, "read", &json!({"timeout": -5})), 60);
    }

    #[test]
    fn envelope_smaller_than_default_keeps_default() {
        // 工具给了 10s（小于默认 60）——按默认走，不缩
        assert_eq!(tool_envelope_secs(60, "run", &json!({"timeout": 10})), 60);
    }
}

#[cfg(test)]
mod edit_signature_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signature_distinguishes_positions() {
        // 设计决定：同一文件不同位置的多次修改 = 正常进展——不同行号必须不同签名
        let a = edit_signature("modify", &json!({"file": "a.rs", "change_spec": {"line": 10, "replace": "x"}})).unwrap();
        let b = edit_signature("modify", &json!({"file": "a.rs", "change_spec": {"line": 20, "replace": "x"}})).unwrap();
        assert_ne!(a.1, b.1, "不同行号 = 不同签名（多点编辑绝不告警的前提）");
        let c = edit_signature("modify", &json!({"file": "a.rs", "change_spec": {"line": 10, "replace": "x"}})).unwrap();
        assert_eq!(a.1, c.1, "同位置同内容 = 同签名（原样重试可检测）");
    }

    #[test]
    fn signature_distinguishes_files() {
        let a = edit_signature("modify", &json!({"file": "a.rs", "change_spec": {"line": 10, "replace": "x"}})).unwrap();
        let b = edit_signature("modify", &json!({"file": "b.rs", "change_spec": {"line": 10, "replace": "x"}})).unwrap();
        assert_ne!(a.0, b.0, "文件不同");
        assert_ne!(a.1, b.1, "签名不同");
    }
}

#[cfg(test)]
mod stall_tests {
    use super::*;

    // 默认阈值（与 settings::DEF_STALL_* 对齐；env REAL_STALL_* 可调，测试用默认值）
    const STOP: u32 = 12;
    const DRY: u32 = 30;
    const MIN: u32 = 20;

    #[test]
    fn short_run_never_stalls() {
        // 总门限以下一律不判（短任务不该被误伤）
        assert!(stall_reason_of(99, 5, 0, false, STOP, DRY, MIN).is_none());
        assert!(stall_reason_of(19, 19, 0, false, STOP, DRY, MIN).is_none());
        assert!(stall_reason_of(19, 19, 0, true, STOP, DRY, MIN).is_none());
    }

    #[test]
    fn zero_progress_reaches_stop() {
        // 改过文件（第 8 轮）、此后 12 个零进展轮 ⇒ 触底
        assert!(stall_reason_of(12, 20, 8, false, STOP, DRY, MIN).is_some());
        assert!(stall_reason_of(11, 20, 8, false, STOP, DRY, MIN).is_none(), "11 个零进展轮不到触底");
        // 刚有实体进展（计数已归零）⇒ 不判
        assert!(stall_reason_of(0, 20, 20, false, STOP, DRY, MIN).is_none(), "刚改过不该止损");
    }

    #[test]
    fn explore_rounds_do_not_count() {
        assert!(stall_reason_of(2, 40, 10, false, STOP, DRY, MIN).is_none());
    }

    #[test]
    fn dry_diagnosis_run_stalls_at_threshold() {
        // 从未改文件 + 无文件产出：满 dry 才判（纯诊断任务给足轮次）
        assert!(stall_reason_of(0, 29, 0, true, STOP, DRY, MIN).is_none());
        assert!(stall_reason_of(0, 30, 0, true, STOP, DRY, MIN).is_some());
        // 从未改文件但 changed_files 非空（说明改动经别的路径产生）⇒ 不走②
        assert!(stall_reason_of(0, 40, 0, false, STOP, DRY, MIN).is_none());
    }

    #[test]
    fn reason_is_self_describing_with_resume() {
        let r = stall_reason_of(12, 20, 8, false, STOP, DRY, MIN).unwrap();
        assert!(r.starts_with("无进展止损"), "原因须自述家族，便于前端/日志归类");
        assert!(r.contains("『继续』"), "触底文案必须告诉用户回复『继续』即可续跑");
        let dry = stall_reason_of(0, 30, 0, true, STOP, DRY, MIN).unwrap();
        assert!(dry.contains("『继续』"), "纯诊断兜底文案同样必须带续跑指引");
    }
}

#[cfg(test)]
mod readmemo_tests {
    use super::*;

    #[test]
    fn skeleton_lines_match_four_langs() {
        assert!(is_skeleton_line("pub fn foo() -> u32 {"));
        assert!(is_skeleton_line("impl Display for Bar {"));
        assert!(is_skeleton_line("class Foo:"));
        assert!(is_skeleton_line("def main():"));
        assert!(is_skeleton_line("## 标题"));
        assert!(is_skeleton_line("export const x = 1;"));
        // 非符号行
        assert!(!is_skeleton_line("let x = 1;"));
        assert!(!is_skeleton_line(""));
        assert!(!is_skeleton_line(&"x".repeat(301)), "超长行不进骨架");
    }

    #[test]
    fn extract_skeleton_keeps_line_numbers_and_caps() {
        let src = "use a;\n\npub fn one() {\n    let x = 1;\n}\n\npub fn two() {}\n";
        let (lines, sk) = extract_skeleton(src);
        assert_eq!(lines, 7);
        assert!(sk.contains("3: pub fn one() {"), "骨架带行号：{sk}");
        assert!(sk.contains("7: pub fn two() {}"));
        assert!(!sk.contains("let x"), "缩进体行不进骨架");
    }

    #[test]
    fn extract_skeleton_caps_at_60_items() {
        let src: String = (0..100).map(|i| format!("pub fn f{i}() {{}}\n\n")).collect();
        let (_, sk) = extract_skeleton(&src);
        assert_eq!(sk.lines().count(), 60, "骨架封顶 60 条");
    }

    #[test]
    fn clip_utf8_never_splits_char() {
        let s = "中文内容测试";
        let c = clip_utf8(s, 7);
        assert!(c.len() <= 7);
        assert!(std::str::from_utf8(c.as_bytes()).is_ok(), "截断不得劈开字符");
        assert_eq!(clip_utf8("short", 10), "short");
    }
}
