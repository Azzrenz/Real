//! model/llm.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with(reasoning: Option<&str>, choice: Option<ToolChoice>) -> ResponsesRequest {
        let mut b = ResponsesRequest::builder("test-model");
        if let Some(effort) = reasoning {
            b = b.reasoning_effort(effort);
        }
        if let Some(c) = choice {
            b = b.tool_choice(c);
        }
        b.build()
    }

    #[test]
    fn sanitize_thinking_plus_required_downgrades_to_none() {
        // 坑位 F1 复现：thinking(low) + required → 发送层必须关思考保 required
        let req = req_with(Some("low"), Some(ToolChoice::Required));
        let clean = RealLlm::sanitize(&req).expect("low+required 必须被降级");
        assert!(
            matches!(clean.tool_choice, Some(ToolChoice::Required)),
            "required 契约必须保留"
        );
        let effort = clean
            .reasoning
            .as_ref()
            .map(|r| r.effort.as_str())
            .unwrap_or("");
        assert_eq!(effort, "none", "思考必须被关闭");
    }

    #[test]
    fn sanitize_none_plus_required_untouched() {
        let req = req_with(Some("none"), Some(ToolChoice::Required));
        assert!(
            RealLlm::sanitize(&req).is_none(),
            "none+required 是合法组合，不降级"
        );
    }

    #[test]
    fn sanitize_thinking_plus_auto_kept() {
        // 方案 A：thinking(high) + auto 不再降级，新模型 thinking 与 auto 可并行
        let req = req_with(Some("high"), Some(ToolChoice::Auto));
        assert!(
            RealLlm::sanitize(&req).is_none(),
            "high+auto 不应降级（thinking 与 auto 可并行）"
        );
    }

    #[test]
    fn sanitize_no_reasoning_untouched() {
        let req = req_with(None, Some(ToolChoice::Required));
        assert!(RealLlm::sanitize(&req).is_none(), "未开启思考时无需修正");
    }

    #[test]
    fn function_calls_returns_all_parallel_calls() {
        // DeepSeek V4 Flash：parallel_tool_calls 始终为 true，单响应可能含多个 function_call。
        let resp = ResponseObject {
            id: None,
            status: Some("completed".into()),
            output: vec![
                OutputItem::FunctionCall {
                    id: None,
                    call_id: "c1".into(),
                    name: "read".into(),
                    arguments: "{}".into(),
                    status: None,
                },
                OutputItem::FunctionCall {
                    id: None,
                    call_id: "c2".into(),
                    name: "write".into(),
                    arguments: "{}".into(),
                    status: None,
                },
            ],
            output_text: None,
            usage: None,
            error: None,
        };
        let calls = resp.function_calls();
        assert_eq!(calls.len(), 2, "必须返回全部并行调用，不能只取第一个");
        assert_eq!(calls[0].name, "read");
        assert_eq!(calls[1].name, "write");
    }

    #[test]
    fn completed_response_overrides_streamed_fc_arguments() {
        // 流式收集到 2 个 fc，但 arguments 都是空（模拟 API 丢参数）
        let streamed = vec![
            FunctionCall {
                call_id: "c1".into(),
                name: "submit_next_action".into(),
                arguments: "{}".into(),
            },
            FunctionCall {
                call_id: "c2".into(),
                name: "submit_next_action".into(),
                arguments: "{}".into(),
            },
        ];
        // completed 的完整 response：同 call_id 但 arguments 完整
        let completed = ResponseObject {
            id: None,
            status: Some("completed".into()),
            output: vec![
                OutputItem::FunctionCall {
                    id: None,
                    call_id: "c1".into(),
                    name: "submit_next_action".into(),
                    arguments: r#"{"tool_name":"read","tool_args":{"path":"D:/x/a.rs"},"reason":"读文件"}"#.into(),
                    status: None,
                },
                OutputItem::FunctionCall {
                    id: None,
                    call_id: "c2".into(),
                    name: "submit_next_action".into(),
                    arguments: r#"{"tool_name":"search","tool_args":{"path":"D:/x"},"reason":"列目录"}"#.into(),
                    status: None,
                },
            ],
            output_text: None,
            usage: None,
            error: None,
        };
        let merged = resolve_function_calls(streamed, &StreamStatus::Completed, Some(&completed));
        assert_eq!(merged.len(), 2, "completed 覆盖后应有 2 个 fc");
        assert!(
            merged[0].arguments.contains("D:/x/a.rs"),
            "completed 覆盖后 arguments 必须完整（流式空参被修复）: {}",
            merged[0].arguments
        );
        assert!(
            merged[1].arguments.contains("D:/x"),
            "第二个 fc 的 arguments 也应完整: {}",
            merged[1].arguments
        );
    }

    #[test]
    fn incomplete_keeps_streamed_fc() {
        // 截断（incomplete）时无完整 response → 保留流式已收集的（不丢已收的）
        let streamed = vec![FunctionCall {
            call_id: "c1".into(),
            name: "submit_next_action".into(),
            arguments: "{}".into(),
        }];
        let merged = resolve_function_calls(streamed.clone(), &StreamStatus::Incomplete, None);
        assert_eq!(merged.len(), 1, "incomplete 保留流式 fc");
        assert_eq!(merged[0].call_id, "c1");
    }

    #[test]
    fn completed_also_empty_keeps_streamed_for_teaching() {
        let streamed = vec![FunctionCall {
            call_id: "c1".into(),
            name: "submit_next_action".into(),
            arguments: "{}".into(),
        }];
        // completed 里同 call_id 的 arguments 也是空（API 源头丢参）
        let completed = ResponseObject {
            id: None,
            status: Some("completed".into()),
            output: vec![OutputItem::FunctionCall {
                id: None,
                call_id: "c1".into(),
                name: "submit_next_action".into(),
                arguments: "".into(),
                status: None,
            }],
            output_text: None,
            usage: None,
            error: None,
        };
        let merged = resolve_function_calls(streamed, &StreamStatus::Completed, Some(&completed));
        assert_eq!(merged.len(), 1, "completed 也空时保留 streamed fc");
        assert_eq!(
            merged[0].arguments, "{}",
            "两者都空 → 空参保留（交给教学，不让模型以为没传）"
        );
    }

    #[test]
    fn merge_fills_only_empty_streamed_from_completed() {
        // 混合：streamed 里 c1 空参、c2 带参；completed 两者都带参 → 只补 c1，c2 保持原样
        let streamed = vec![
            FunctionCall {
                call_id: "c1".into(),
                name: "submit_next_action".into(),
                arguments: "{}".into(),
            },
            FunctionCall {
                call_id: "c2".into(),
                name: "submit_next_action".into(),
                arguments:
                    r#"{"tool_name":"search","tool_args":{"path":"D:/x"},"reason":"列目录"}"#
                        .into(),
            },
        ];
        let completed = ResponseObject {
            id: None,
            status: Some("completed".into()),
            output: vec![
                OutputItem::FunctionCall {
                    id: None, call_id: "c1".into(), name: "submit_next_action".into(),
                    arguments: r#"{"tool_name":"read","tool_args":{"path":"D:/x/a.rs"},"reason":"读文件"}"#.into(), status: None,
                },
                OutputItem::FunctionCall {
                    id: None, call_id: "c2".into(), name: "submit_next_action".into(),
                    arguments: r#"{"tool_name":"search","tool_args":{"path":"D:/x"},"reason":"列目录"}"#.into(), status: None,
                },
            ],
            output_text: None, usage: None, error: None,
        };
        let merged = resolve_function_calls(streamed, &StreamStatus::Completed, Some(&completed));
        assert_eq!(merged.len(), 2);
        assert!(
            merged[0].arguments.contains("D:/x/a.rs"),
            "空参 c1 必须被补全: {}",
            merged[0].arguments
        );
        assert!(
            merged[1].arguments.contains("search"),
            "带参 c2 保持: {}",
            merged[1].arguments
        );
    }

    #[test]
    fn reasoning_output_roundtrips_for_rewind() {
        // 思考模式：上一次响应的 reasoning 必须在下一请求原样回传，否则 API 报 400。
        let item = OutputItem::Reasoning {
            content: Some(serde_json::json!([{"type":"reasoning_text","text":"让我想想"}])),
            summary: None,
        };
        let input = InputItem::from_output_item(&item);
        match input {
            InputItem::Reasoning { content, summary } => {
                assert_eq!(
                    content,
                    Some(serde_json::json!([{"type":"reasoning_text","text":"让我想想"}])),
                    "思考内容必须原样回传（对象/数组结构不丢）"
                );
                assert!(summary.is_none(), "仅 content 时应置空 summary");
            }
            other => assert!(
                false,
                "Reasoning 必须转成 Reasoning 枚举回传，实际: {other:?}"
            ),
        }
    }
}
