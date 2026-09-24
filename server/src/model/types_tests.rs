//! model/types.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 锁定 DeepSeek **Responses API** 的 tools 契约：扁平格式，name/description/parameters
    #[test]
    fn tools_serializes_flat_with_top_level_name() {
        let req = ResponsesRequest::builder("deepseek-flash")
            .tools(vec![ToolDef::function(
                "submit_plan",
                "submit a plan",
                json!({"type":"object","properties":{}}),
            )])
            .tool_choice(ToolChoice::Auto)
            .build();
        let v = serde_json::to_value(&req).unwrap();
        let tools = v["tools"].as_array().expect("tools must be an array");
        assert_eq!(tools.len(), 1);
        let t = &tools[0];
        assert_eq!(t["type"], json!("function"));
        // 顶层必须有 name（Responses API 要求），不能是嵌套 function 包裹
        assert_eq!(
            t["name"],
            json!("submit_plan"),
            "name must be top-level (Responses API)"
        );
        assert_eq!(t["description"], json!("submit a plan"));
        assert!(
            t.get("parameters").is_some(),
            "parameters must be top-level"
        );
        assert!(
            t.get("function").is_none(),
            "Responses API tools are flat, no 'function' wrapper"
        );
    }

    /// 锁定 Responses API 的 reasoning 回传格式：独立 `reasoning` item，content 为
    #[test]
    fn reasoning_item_serializes_as_official_item() {
        let item = InputItem::reasoning_item("思考内容");
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["type"], json!("reasoning"));
        assert_eq!(v["content"][0]["type"], json!("reasoning_text"));
        assert_eq!(v["content"][0]["text"], json!("思考内容"));
        // round-trip：反序列化还原（历史重建走 Deserialize 路径）
        let back: InputItem = serde_json::from_value(v).unwrap();
        assert_eq!(
            back.assistant_tool_call_ids(),
            Vec::<String>::new(),
            "reasoning item 无 tool_calls"
        );
    }

    #[test]
    fn raw_function_call_output_normalized_and_wire_typed() {
        let raw = InputItem::Raw(json!({
            "type": "function_call_output",
            "call_id": "call_abc",
            "output": "{\"status\":\"success\"}"
        }));
        let wire = to_wire_items(&[raw]);
        let ser = serde_json::to_string(&wire[0]).unwrap();
        assert!(ser.contains("\"type\":\"function_call_output\""), "wire 类型必须是 function_call_output: {ser}");
        assert!(!ser.contains("\"type\":\"raw\""), "不得以 raw 上送: {ser}");
        assert!(ser.contains("call_abc"), "call_id 保留: {ser}");
    }

    #[test]
    fn raw_unknown_type_stays_raw() {
        let raw = InputItem::Raw(json!({"type": "something_new", "x": 1}));
        let wire = to_wire_items(&[raw.clone()]);
        assert_eq!(wire.len(), 1);
    }

    #[test]
    fn user_message_between_call_and_output_is_deferred() {
        let items = vec![
            InputItem::function_call("c1", "run", "{}"),
            InputItem::user_message("插话：你跑的是 dev"),
            InputItem::function_call_output("c1", "{\"ok\":true}"),
        ];
        let wire = to_wire_items(&items);
        let kinds: Vec<&str> = wire.iter().map(|i| match i {
            InputItem::FunctionCall { .. } => "call",
            InputItem::FunctionCallOutput { .. } => "output",
            InputItem::Message { role, .. } => if role == "user" { "user" } else { "msg" },
            _ => "other",
        }).collect();
        assert_eq!(kinds, vec!["call", "output", "user"], "user 消息必须后移到输出之后: {kinds:?}");
        let ser = serde_json::to_string(&wire).unwrap();
        assert!(ser.contains("你跑的是 dev"), "插话内容不得丢失");
    }

    #[test]
    fn wire_selfcheck_flags_orphan_call_and_passes_after_reorder() {
        // 自检：裸 Raw / 无输出 / 输出未就绪插 user 三类病因必须被抓到
        let bad_raw = vec![InputItem::Raw(json!({"call_id": "c1"}))];
        assert!(validate_wire_items(&bad_raw).is_some(), "裸 Raw 应被判定");

        let orphan = vec![InputItem::function_call("c1", "run", "{}")];
        assert!(validate_wire_items(&orphan).is_some(), "无输出应被判定");

        // 归一（to_wire_items 会做紧邻重排）后必须通过
        let items = vec![
            InputItem::function_call("c1", "run", "{}"),
            InputItem::user_message("插话"),
            InputItem::function_call_output("c1", "{}"),
        ];
        let wire = to_wire_items(&items);
        assert!(validate_wire_items(&wire).is_none(), "紧邻重排后自检应通过: {:?}", validate_wire_items(&wire));
    }

    #[test]
    fn matrix_interjection_during_call() {
        let items = vec![
            InputItem::function_call("c1", "run", "{}"),
            InputItem::user_message("插话：你跑的是 dev"),
            InputItem::function_call_output("c1", "{}"),
        ];
        let wire = to_wire_items(&items);
        assert!(validate_wire_items(&wire).is_none(), "插话场景自检应通过");
        let kinds: Vec<&str> = wire.iter().map(kind_of).collect();
        assert_eq!(kinds, vec!["call", "output", "user"], "插话必须后移: {kinds:?}");
    }

    // 场景二：用户取消（调用声明后无输出，靠请求前闭合兜住）
    #[test]
    fn matrix_cancel_then_new_task() {
        let items = vec![
            InputItem::function_call("c1", "run", "{}"),
            InputItem::user_message("停下，改做别的"),
        ];
        let reconciled = crate::agent::history::reconcile_for_request(&items);
        let wire = to_wire_items(&reconciled);
        assert!(validate_wire_items(&wire).is_none(), "取消场景应被闭合兜住");
        let kinds: Vec<&str> = wire.iter().map(kind_of).collect();
        assert_eq!(kinds, vec!["call", "output", "user"], "占位输出须在 user 之前: {kinds:?}");
    }

    // 场景三：工具超时（错误信封作为输出）
    #[test]
    fn matrix_tool_timeout() {
        let items = vec![
            InputItem::function_call("c1", "run", "{}"),
            InputItem::function_call_output("c1", "{\"status\":\"error\",\"timed_out\":true}"),
            InputItem::user_message("超时了怎么办"),
        ];
        let wire = to_wire_items(&items);
        assert!(validate_wire_items(&wire).is_none(), "超时场景自检应通过");
    }

    // 场景四：缓存命中（多个调用 + 插话混排，须全部紧邻）
    #[test]
    fn matrix_cache_hit_multi_calls() {
        let items = vec![
            InputItem::function_call("c1", "read", "{}"),
            InputItem::function_call("c2", "run", "{}"),
            InputItem::user_message("插话"),
            InputItem::function_call_output("c1", "{\"cached\":true}"),
            InputItem::function_call_output("c2", "{\"cached\":true}"),
        ];
        let wire = to_wire_items(&items);
        assert!(validate_wire_items(&wire).is_none(), "多调用+插话场景自检应通过");
        let kinds: Vec<&str> = wire.iter().map(kind_of).collect();
        let user_pos = kinds.iter().position(|k| *k == "user").unwrap();
        let last_out = kinds.iter().rposition(|k| *k == "output").unwrap();
        assert!(user_pos > last_out, "插话须在所有输出之后: {kinds:?}");
    }

    fn kind_of(it: &InputItem) -> &'static str {
        match it {
            InputItem::FunctionCall { .. } => "call",
            InputItem::FunctionCallOutput { .. } => "output",
            InputItem::Message { role, .. } => if role == "user" { "user" } else { "msg" },
            _ => "other",
        }
    }
}

#[cfg(test)]
mod ephemeral_attachment_tests {
    use super::*;

    fn tool_image_block() -> InputItem {
        InputItem::user_message_blocks(json!([
            {"type": "input_text", "text": format!(
                "{}read` 返回了 1 张图片 —— 见附件", InputItem::IMAGE_ATTACHMENT_MARK)},
            {"type": "input_image", "image_url": "data:image/png;base64,AAAA", "detail": "high"}
        ]))
    }

    /// 用户从聊天框发图走的也是 `input_image`：结构一样，只有文案不同。
    fn user_image_block() -> InputItem {
        InputItem::user_message_blocks(json!([
            {"type": "input_text", "text": "帮我看看这张图"},
            {"type": "input_image", "image_url": "data:image/png;base64,BBBB", "detail": "high"}
        ]))
    }

    #[test]
    fn tool_attachment_is_claimed_but_user_image_is_not() {
        assert!(tool_image_block().is_ephemeral_image_attachment());
        assert!(
            !user_image_block().is_ephemeral_image_attachment(),
            "用户自己发的图不能被当成临时附件 —— 那等于把它从对话里删掉"
        );
    }

    #[test]
    fn plain_items_are_never_claimed() {
        assert!(!InputItem::user_message("普通用户消息").is_ephemeral_image_attachment());
        assert!(!InputItem::assistant_message("普通回复").is_ephemeral_image_attachment());
        assert!(!InputItem::function_call_output("c1", "{}").is_ephemeral_image_attachment());
    }
}
