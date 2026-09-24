//! sse 域的测试外置：事件坐标（run 身份）盖章契约。

#[cfg(test)]
mod with_run_tests {
    use crate::sse::with_run;
    use serde_json::json;

    #[test]
    fn stamps_run_id_into_object_payload() {
        let p = with_run(json!({"message": "任务已被取消"}), "ab12cd34");
        assert_eq!(p["run_id"], json!("ab12cd34"));
    }

    #[test]
    fn keeps_existing_keys_intact() {
        let p = with_run(json!({"text": "增量", "step_id": "call_1"}), "ffee0011");
        assert_eq!(p["text"], json!("增量"));
        assert_eq!(p["step_id"], json!("call_1"));
        assert_eq!(p["run_id"], json!("ffee0011"));
    }

    #[test]
    fn overwrites_stale_run_id() {
        // 同一载荷被两个 run 用过时，后者必须是权威（防串轮）
        let p = with_run(json!({"run_id": "old", "message": "x"}), "new1");
        assert_eq!(p["run_id"], json!("new1"));
    }

    #[test]
    fn empty_run_id_leaves_payload_untouched() {
        // 会话级事件（无 run 归属）不得凭空多出 run_id 字段
        let p = with_run(json!({"title": "会话改名"}), "");
        assert!(p.get("run_id").is_none());
        assert_eq!(p["title"], json!("会话改名"));
    }

    #[test]
    fn non_object_payload_is_returned_as_is() {
        let p = with_run(json!("plain"), "ab12cd34");
        assert_eq!(p, json!("plain"));
    }
}
