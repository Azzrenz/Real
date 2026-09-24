//! agent/planning/placeholder.rs 的测试外置（部门盘查：核心文件测试全部移出，此文件单管）
use super::*;

#[cfg(test)]
mod placeholder_util_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_step_number_accepts_en_format() {
        assert_eq!(parse_step_number("#E12"), Some(12));
        assert_eq!(parse_step_number("#E3"), Some(3));
        assert_eq!(parse_step_number("D:/x"), None);
        assert_eq!(parse_step_number(""), None);
    }

    #[test]
    fn collect_placeholders_finds_whole_refs() {
        let v = json!({"path": "#E6", "list": ["#E3", "plain"], "nested": {"a": "#E12"}});
        let phs = collect_placeholders(&v);
        assert!(phs.contains(&"#E6".to_string()));
        assert!(phs.contains(&"#E3".to_string()));
        assert!(phs.contains(&"#E12".to_string()));
        assert_eq!(phs.len(), 3);
    }

    #[test]
    fn find_unresolved_detects_en_and_new() {
        assert_eq!(
            find_unresolved_placeholder(&json!({"path": "#E5"})),
            Some("#E5".to_string())
        );
        assert_eq!(
            find_unresolved_placeholder(&json!({"note": "待插入 #NEW"})),
            Some("#NEW".to_string())
        );
        assert_eq!(
            find_unresolved_placeholder(&json!({"path": "D:/proj/main.rs"})),
            None
        );
    }

    #[test]
    fn truncate_limits_chars() {
        assert_eq!(truncate("hello", 3), "hel…");
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn parse_json_lenient_fixes_windows_slashes() {
        // `\E`（E 非合法转义）→ 补反斜杠保字面；`\x`（x 非法）同理修复
        let ok = parse_json_lenient(r#"{"path": "D:\proj\src"}"#);
        assert!(ok.is_some(), "非法转义应被修复后解析");
        let v = ok.unwrap();
        // 修复后字符串字面量是 "D:\\proj\\src"（每个反斜杠都补了一个）
        assert_eq!(v["path"], "D:\\proj\\src");
        // 合法转义不破坏
        let ok2 = parse_json_lenient(r#"{"cmd": "echo \"hi\""}"#);
        assert_eq!(ok2.unwrap()["cmd"], "echo \"hi\"");
    }
}
