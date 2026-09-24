//! origin_guard.rs 的测试外置（测试不进核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_default_whitelist_variants() {
        for origin in [
            "http://tauri.localhost",
            "https://tauri.localhost",
            "tauri://localhost",
            // host 为 tauri.localhost 时任意 scheme / 端口都放行
            "http://tauri.localhost:1234",
            "https://tauri.localhost:55000",
            "http://localhost:8618",
            "http://127.0.0.1:8618",
            "http://localhost:8943",
            "http://127.0.0.1:8943",
            // 尾部 `/` 与大小写不敏感
            " http://localhost:8943/ ",
            "HTTP://LocalHost:8943",
        ] {
            assert!(is_allowed_origin(Some(origin)), "应放行: {origin}");
        }
    }

    #[test]
    fn allows_absent_or_blank_origin() {
        // 非浏览器客户端（curl / 后端自调用 / 部分协议）不带 Origin
        assert!(is_allowed_origin(None));
        assert!(is_allowed_origin(Some("")));
        assert!(is_allowed_origin(Some("   ")));
    }

    #[test]
    fn rejects_foreign_and_null_origins() {
        for origin in [
            "http://evil.com",
            "https://real.example.com",
            "null",
            "NULL",
            "Null",
            // 后缀混淆：host 不是 tauri.localhost，必须拒
            "http://tauri.localhost.evil.com",
            "https://tauri.localhost.evil.com",
            "tauri://localhost.evil.com",
            "file://",
            "http://localhost:1234",
        ] {
            assert!(!is_allowed_origin(Some(origin)), "应拒绝: {origin}");
        }
    }

    #[test]
    fn parse_origins_trims_and_drops_empty() {
        assert_eq!(
            parse_origins("  http://a.com , ,http://b.com  "),
            vec!["http://a.com".to_string(), "http://b.com".to_string()]
        );
        assert!(parse_origins("   ").is_empty());
        assert!(parse_origins("").is_empty());
        assert_eq!(parse_origins("only"), vec!["only".to_string()]);
    }

    #[test]
    fn split_scheme_host_extracts_host_without_port() {
        assert_eq!(
            split_scheme_host("http://tauri.localhost:1234/x"),
            ("http".to_string(), "tauri.localhost".to_string())
        );
        assert_eq!(
            split_scheme_host("tauri://localhost"),
            ("tauri".to_string(), "localhost".to_string())
        );
        assert_eq!(
            split_scheme_host("https://127.0.0.1:8943"),
            ("https".to_string(), "127.0.0.1".to_string())
        );
        // 无 scheme 的裸 host
        assert_eq!(split_scheme_host("evil.com"), (String::new(), "evil.com".to_string()));
    }
}
