//! tools/base.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// read 工具的输出契约（测试内联——ToolContract 结构体已随清理审计删除）
    fn read_output_schema() -> Option<Value> {
        Some(json!({
            "type":"object",
            "properties":{
                "kind":{"type":"string"},
                "data":{"type":"object","properties":{
                    "files":{"type":"array","items":{"type":"object","properties":{
                        "path":{"type":"string"},"total_lines":{"type":"integer"},
                        "mode":{"type":"string"},"content":{"type":"string"}
                    }}}
                }}
            }
        }))
    }

    #[test]
    fn output_valid_json_passes() {
        // 合法输出：结构符合 output_schema → 通过并返回规范化值
        let out = r#"{"kind":"read_result","data":{"files":[{"path":"/a.rs","total_lines":10,"mode":"full","content":"fn main() {}"}]}}"#;
        let v = validate_output("read", &read_output_schema(), out).unwrap();
        assert_eq!(v["data"]["files"][0]["path"], "/a.rs");
    }

    #[test]
    fn output_invalid_json_rejected() {
        // 声明了 object 输出契约，但 run 返回非 JSON → OUTPUT_INVALID 拦截（不进模型）
        let out = "plain text that is not json";
        let err = validate_output("read", &read_output_schema(), out).unwrap_err();
        assert_eq!(err.code, "OUTPUT_INVALID");
    }

    #[test]
    fn output_wrong_field_type_rejected() {
        // 字段类型违规 → 输入闸门同款 INVALID_PARAM（双向对称）
        let out = r#"{"kind":"read_result","data":{"files":[{"path":123,"total_lines":10}]}}"#;
        let err = validate_output("read", &read_output_schema(), out).unwrap_err();
        assert_eq!(err.code, "INVALID_PARAM");
    }

    #[test]
    fn string_output_contract_passes_any_text() {
        // string 型输出契约：任意文本通过（对齐 str_replace_editor）
        let schema = Some(json!({"type":"string"}));
        let v = validate_output("audit", &schema, "任意报告文本\n多行").unwrap();
        assert_eq!(v, json!("任意报告文本\n多行"));
    }

    #[test]
    fn no_output_schema_skips_validation() {
        // 未迁移契约的工具：不校验，保持现状
        let v = validate_output("legacy_tool", &None, "anything").unwrap();
        assert_eq!(v, json!("anything"));
    }
}
