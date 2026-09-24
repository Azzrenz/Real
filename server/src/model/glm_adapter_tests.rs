//! model/glm_adapter.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req() -> ResponsesRequestBuilder {
        ResponsesRequest::builder("glm-5.3-flash")
    }

    #[test]
    fn stalled_stream_detection_1token_empty_spin() {
        // 现场指纹（复现两波）：无工具 + 产出 1 token + 思考 2 字 = 首 delta 后断流 → 判异常
        assert!(stream_stalled(true, 1, 2, false));
    }

    #[test]
    fn stalled_stream_detection_length_with_tiny_output() {
        // finish=length 截断 + 产出 ≤2 → 判异常（无论思考长短，length 却只产 1 token 必异常）
        assert!(stream_stalled(true, 1, 50, true));
        assert!(stream_stalled(true, 2, 3, true));
    }

    #[test]
    fn stalled_stream_not_false_positive() {
        // 正常单字回答（stop + 1 token + 思考正常长）→ 不误杀
        assert!(!stream_stalled(true, 1, 30, false));
        // 有工具调用 = 模型在推进 → 不算
        assert!(!stream_stalled(false, 1, 2, false));
        // 正常长回答 → 不算
        assert!(!stream_stalled(true, 800, 200, false));
        // 真截断（thinking 占满预算、产出大）→ 不归早停，留给 Incomplete 大产出路径
        assert!(!stream_stalled(true, 131_000, 50_000, true));
    }

    #[test]
    fn temperature_quantized_to_two_decimals() {
        // 0.2f32 直接 json! 会带 f64 伪影 0.20000000298023224 → GLM 1210 拒收
        let body = translate_request(&req().temperature(0.2).build());
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(
            serialized.contains("\"temperature\":0.2,") || serialized.contains("\"temperature\":0.2}"),
            "temperature 序列化必须 ≤2 位小数: {serialized}"
        );
        assert!(!serialized.contains("0.2000000"), "不允许 f64 伪影: {serialized}");
        assert_eq!(body["temperature"], json!(0.2));
    }

    #[test]
    fn request_tools_are_nested_function_format() {
        let r = req()
            .tools(vec![ToolDef::function(
                "read",
                "read a file",
                json!({"type": "object", "properties": {}}),
            )])
            .tool_choice(ToolChoice::Required)
            .build();
        let v = translate_request(&r);
        let t = &v["tools"][0];
        assert_eq!(t["type"], json!("function"));
        assert_eq!(t["function"]["name"], json!("read"));
        assert!(t.get("parameters").is_none(), "GLM 契约 parameters 必须嵌套在 function 内");
        assert_eq!(t["function"]["parameters"]["type"], json!("object"));
        assert_eq!(v["tool_choice"], json!("required"));
    }

    #[test]
    fn thinking_always_enabled_effort_mapped() {
        std::env::remove_var("REAL_GLM_CLEAR_THINKING");
        let r = req().reasoning_effort("high").build();
        let v = translate_request(&r);
        assert_eq!(v["thinking"]["type"], json!("enabled"));
        assert_eq!(v["thinking"]["clear_thinking"], json!(true));
        assert_eq!(v["reasoning_effort"], json!("high"));

        // env 可回退旧行为（对照用）
        std::env::set_var("REAL_GLM_CLEAR_THINKING", "false");
        let v3 = translate_request(&r);
        assert_eq!(v3["thinking"]["clear_thinking"], json!(false));
        std::env::remove_var("REAL_GLM_CLEAR_THINKING");

        // effort=none → 取最低档 low（GLM 思考不可关；官方 thinking.type 仅 enabled）
        let r2 = req().reasoning_effort("none").build();
        let v2 = translate_request(&r2);
        assert_eq!(v2["thinking"]["type"], json!("enabled"));
        assert_eq!(v2.get("reasoning_effort").and_then(|v| v.as_str()), Some("low"));
    }

    #[test]
    fn history_maps_tool_round_trip() {
        // assistant 带聚合 tool_calls + 顶层 FunctionCall 展开 + FunctionCallOutput → tool 角色
        let fc = FunctionCall {
            call_id: "c1".into(),
            name: "read".into(),
            arguments: r#"{"paths":["D:/a.rs"]}"#.into(),
        };
        let r = req()
            .input(vec![
                InputItem::user_message("看看"),
                InputItem::Message {
                    role: "assistant".into(),
                    content: json!([{"type": "output_text", "text": "好"}]),
                    tool_calls: Some(vec![json!({
                        "type": "function_call",
                        "call_id": "c1",
                        "name": "read",
                        "arguments": r#"{"paths":["D:/a.rs"]}"#
                    })]),
                },
                InputItem::function_call_output("c1", "文件内容"),
            ])
            .build();
        let v = translate_request(&r);
        let msgs = v["messages"].as_array().expect("messages");
        // system 无 instructions 时不注入；user + assistant + tool = 3 条
        assert_eq!(msgs.len(), 3, "聚合 tool_calls 展开后 assistant 只一条: {v}");
        assert_eq!(msgs[0]["role"], json!("user"));
        assert_eq!(msgs[0]["content"], json!("看看"));
        assert_eq!(msgs[1]["role"], json!("assistant"));
        assert_eq!(msgs[1]["content"], json!("好"));
        assert_eq!(msgs[1]["tool_calls"][0]["id"], json!("c1"));
        assert_eq!(msgs[1]["tool_calls"][0]["function"]["name"], json!("read"));
        assert_eq!(msgs[2]["role"], json!("tool"));
        assert_eq!(msgs[2]["tool_call_id"], json!("c1"));
        assert_eq!(msgs[2]["content"], json!("文件内容"));
        let _ = fc;
    }

    #[test]
    fn consecutive_function_calls_merge_into_one_assistant() {
        let r = req()
            .input(vec![
                InputItem::function_call("c1", "read", "{}"),
                InputItem::function_call("c2", "list", "{}"),
                InputItem::function_call_output("c1", "a"),
                InputItem::function_call_output("c2", "b"),
            ])
            .build();
        let v = translate_request(&r);
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3, "两个连续 fc 合并为一条 assistant + 两条 tool: {v}");
        assert_eq!(msgs[0]["tool_calls"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn reasoning_items_are_skipped() {
        let r = req()
            .input(vec![
                InputItem::user_message("hi"),
                InputItem::reasoning_item("思考中"),
            ])
            .build();
        let v = translate_request(&r);
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1, "Reasoning item 不进 GLM 消息序列");
    }

    #[test]
    fn image_blocks_map_to_glm_format() {
        let content = json!([
            {"type": "input_text", "text": "看图"},
            {"type": "input_image", "image_url": "data:image/png;base64,xxx", "detail": "low"}
        ]);
        let r = req()
            .input(vec![InputItem::Message {
                role: "user".into(),
                content,
                tool_calls: None,
            }])
            .build();
        let v = translate_request(&r);
        let parts = v["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], json!("text"));
        assert_eq!(parts[1]["type"], json!("image_url"));
        assert_eq!(parts[1]["image_url"]["url"], json!("data:image/png;base64,xxx"));
    }
}
