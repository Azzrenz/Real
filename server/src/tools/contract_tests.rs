//! tools/contract.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// （信号契约 v1 定稿·§五）：错误码分类表——每个码必须有明确归属
    #[test]
    fn error_code_classification_is_complete() {
        // A 瞬态 → retry
        assert!(code_retryable("TIMEOUT"));
        assert!(code_retryable_with_change("RUN_FAILED"), "命令失败应『改参数后重试』");
        assert!(
            !code_retryable("RUN_FAILED"),
            "命令失败不是瞬态——原样重跑必然同样失败，不该进瞬态重试列"
        );
        assert!(code_retryable("READ_FAILED"));
        assert!(code_retryable("NETWORK_ERROR"));
        assert!(code_retryable("DB_CONNECT_FAIL"));
        assert!(code_retryable("EXEC_FAILED"));
        assert_eq!(code_next_action("RUN_FAILED"), "retry");
        // 写验证类 → verify（重试前先确认现状）
        assert_eq!(code_next_action("INTENT_MISMATCH"), "verify");
        assert_eq!(code_next_action("VERIFY_FAILED"), "verify");
        assert_eq!(code_next_action("SECURITY_PATTERN"), "verify");
        assert!(!code_retryable("VERIFY_FAILED"), "验证类不可盲目重试");
        // 确认/权限类 → ask_user
        assert_eq!(code_next_action("WRITE_REQUIRES_CONFIRM"), "ask_user");
        assert_eq!(code_next_action("PERMISSION_DENIED"), "ask_user");
        // 参数/路径类 → 模型改参数后重试（不惊动用户）
        assert!(code_retryable_with_change("NOT_FOUND"));
        assert!(code_retryable_with_change("OLD_TEXT_MISMATCH"));
        assert!(code_retryable_with_change("COMMAND_NOT_ALLOWED"));
        assert!(!code_retryable_with_change("NETWORK_ERROR"), "网络类不属改参数重试");
        // 网络不可达 → ask_user（明确不重试，防烧轮次）
        assert_eq!(code_next_action("WEB_NETWORK_UNREACHABLE"), "ask_user");
        assert!(!code_retryable("WEB_NETWORK_UNREACHABLE"));
    }

    /// 码表守卫：`registry.rs` 的**实际**判定是
    #[test]
    fn effective_next_action_is_consistent() {
        let effective = |c: &str| {
            if code_retryable_with_change(c) {
                "retry"
            } else {
                code_next_action(c)
            }
        };
        // 可重试类（瞬态 或 改参数）→ 组合判定必须 retry
        for c in [
            "TIMEOUT",
            "RUN_FAILED",
            "READ_FAILED",
            "NETWORK_ERROR",
            "NOT_FOUND",
            "OLD_TEXT_MISMATCH",
            "MODIFY_PARAM_UNFILLED",
            "FIND_AMBIGUOUS",
        ] {
            assert_eq!(effective(c), "retry", "{c} 标了可重试，组合判定却不是 retry");
        }
        // 必须用户介入的 → **绝不能**被判成 retry（否则等于把弹窗/权限问题烧成自动重试）
        for c in [
            "WRITE_REQUIRES_CONFIRM",
            "EDIT_REQUIRES_CONFIRM",
            "PERMISSION_DENIED",
            "SENSITIVE_FILE",
        ] {
            assert_eq!(effective(c), "ask_user", "{c} 需要用户确认，却被判成 {}", effective(c));
        }
        // RUN_FAILED 的**性质回归护栏**：改参数可重试，但**不是**瞬态
        assert!(code_retryable_with_change("RUN_FAILED"), "命令失败应『改参数后重试』");
        assert!(!code_retryable("RUN_FAILED"), "命令失败不是瞬态");
    }

    fn read_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 5},
                "mode": {"type": "string", "enum": ["auto", "full", "lines"], "default": "auto"},
                "start_line": {"type": "integer", "minimum": 1},
                "end_line": {"type": "integer", "minimum": 1}
            },
            "required": ["paths"],
            "additionalProperties": false
        })
    }

    #[test]
    fn missing_required_rejected() {
        let err = validate(&read_schema(), &json!({})).unwrap_err();
        assert_eq!(err.code, "MISSING_PARAM");
    }

    #[test]
    fn enum_rejected() {
        // （枚举宽容化）：enum 不再拒绝——保留原始值由工具层解释。
        let out = validate(&read_schema(), &json!({"paths": ["a"], "mode": "bogus"})).unwrap();
        assert_eq!(
            out["mode"], "bogus",
            "非枚举值应宽容放行（工具层解释）: {out}"
        );
    }

    #[test]
    fn extra_property_rejected() {
        let err = validate(&read_schema(), &json!({"paths": ["a"], "hack": 1})).unwrap_err();
        assert_eq!(err.code, "INVALID_PARAM");
    }

    #[test]
    fn default_injected_and_cleaned() {
        let out = validate(&read_schema(), &json!({"paths": ["a", "b"]})).unwrap();
        assert_eq!(out["mode"], "auto");
        assert!(out.get("hack").is_none());
    }

    #[test]
    fn min_items_rejected() {
        let err = validate(&read_schema(), &json!({"paths": []})).unwrap_err();
        assert_eq!(err.code, "INVALID_PARAM");
    }

    #[test]
    fn numeric_out_of_range_clamped() {
        // （宽容执行·**值维度**，与上面 enum 宽容化同源）：数值越界不再拒绝，夹到边界继续。
        let schema = json!({
            "type": "object",
            "properties": {"context": {"type": "integer", "minimum": 0, "maximum": 10, "default": 5}}
        });
        let out = validate(&schema, &json!({"context": 15})).unwrap();
        assert_eq!(out["context"], 10, "越界应夹到上限而不是报错: {out}");
        let out2 = validate(&schema, &json!({"context": -3})).unwrap();
        assert_eq!(out2["context"], 0, "越界应夹到下限: {out2}");
        let out3 = validate(&schema, &json!({"context": 7})).unwrap();
        assert_eq!(out3["context"], 7, "界内不动: {out3}");
        // 类型错仍硬错 —— 那是理解错，不是"想要更多"
        let err = validate(&schema, &json!({"context": "many"})).unwrap_err();
        assert_eq!(err.code, "INVALID_PARAM");
    }

    #[test]
    fn reason_meta_field_stripped_not_rejected() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"url": {"type": "string"}},
            "required": ["url"]
        });
        let cleaned = validate(&schema, &serde_json::json!({
            "url": "https://x", "reason": "打开页面看图"
        })).expect("reason 应被剥离而非拒绝");
        assert!(cleaned.get("reason").is_none(), "reason 不应进工具参数");
        // 其他未声明字段仍然拒绝
        let err = validate(&schema, &serde_json::json!({
            "url": "https://x", "foo": 1
        })).unwrap_err();
        assert_eq!(err.code, "INVALID_PARAM");
    }
}
