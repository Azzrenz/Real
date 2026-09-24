
    #[test]
    fn findstr_forward_slash_path_converted_to_backslash() {
        let cmd = "findstr /n /i /c:\"pricing.json\" D:/project-b/server/src/pricing.rs";
        let out = super::normalize_drive_slash(cmd);
        assert!(out.contains("D:\\project-b\\server\\src\\pricing.rs"), "盘符路径应转反斜杠: {out}");
        assert!(out.contains("/c:\"pricing.json\""), "引号内开关模式不应被改: {out}");
    }

    #[test]
    fn findstr_url_in_quotes_untouched() {
        let cmd = "findstr /i \"https://example.com/a\" D:/log/app.log";
        let out = super::normalize_drive_slash(cmd);
        assert!(out.contains("https://example.com/a"), "引号内 URL 不应被改: {out}");
        assert!(out.contains("D:\\log\\app.log"), "盘符路径应转反斜杠: {out}");
    }

    #[test]
    fn non_findstr_command_also_normalized() {
        // 统一归一化：非 findstr 命令同样受益（python/cargo/type 一族全治）
        let out = super::normalize_drive_slash("python D:/x/y/z.py");
        assert_eq!(out, "python D:\\x\\y\\z.py", "盘符斜杠应统一转反斜杠: {out}");
    }

    #[test]
    fn unquoted_url_untouched() {
        let out = super::normalize_drive_slash("curl http://example.com/a");
        assert_eq!(out, "curl http://example.com/a", "无盘符冒号斜杠模式不应被改: {out}");
    }

    #[test]
    fn quoted_drive_path_converted() {
        let cmd = "cmd /c \"dir /b /s D:/project-b/server & dir /b /s D:/project-b\"";
        let out = super::normalize_drive_slash(cmd);
        assert!(out.contains("D:\\project-b\\server"), "引号内盘符路径必须转反斜杠: {out}");
        assert!(out.contains("D:\\project-b"), "第二个也要转: {out}");
        assert!(!out.contains("D:/project-b"), "不得残留正斜杠盘符: {out}");
    }

    #[test]
    fn findstr_pattern_quote_untouched() {
        let cmd = "findstr /i /c:\"D:/x.txt\" D:/log/app.log";
        let out = super::normalize_drive_slash(cmd);
        assert!(out.contains("/c:\"D:/x.txt\""), "findstr 模式串不得被改: {out}");
        assert!(out.contains("D:\\log\\app.log"), "文件参数仍应转反斜杠: {out}");
    }

    #[test]
    fn quoted_url_untouched() {
        let out = super::normalize_drive_slash("curl \"https://example.com/a/b\"");
        assert!(out.contains("https://example.com/a/b"), "引号内 URL 不得被改: {out}");
    }

    // ── 走 Git Bash 的反斜杠路径归一（bash 把反斜杠当转义符，必须转正斜杠）──

    #[test]
    fn bash_normalize_drive_unc_relative_bare() {
        let f = super::normalize_backslash_paths_for_bash;
        assert_eq!(
            f("grep -n x D:\\proj\\server\\src"),
            "grep -n x D:/proj/server/src",
            "① 盘符"
        );
        assert_eq!(f("cat .\\build\\x.log"), "cat ./build/x.log", "③ 相对 .\\");
        assert_eq!(f("cat ..\\src\\a.rs"), "cat ../src/a.rs", "③ 相对 ..\\");
        assert_eq!(f("ls \\tmp\\a.log"), "ls /tmp/a.log", "④ 根相对");
        assert_eq!(f("cat build\\out.txt"), "cat build/out.txt", "⑤ 裸相对");
        assert_eq!(
            f("run --path=D:\\x\\y.rs"),
            "run --path=D:/x/y.rs",
            "选项=路径"
        );
        assert_eq!(
            f("cp \\\\srv\\share\\f.txt ."),
            "cp //srv/share/f.txt .",
            "② UNC"
        );
    }

    #[test]
    fn bash_normalize_keeps_regex_and_escapes() {
        let f = super::normalize_backslash_paths_for_bash;
        // 单引号内（grep/sed 正则）原样不动 —— bash 不在其中解析反斜杠
        assert_eq!(f("grep -P '\\d+\\s' f"), "grep -P '\\d+\\s' f");
        assert_eq!(f("sed 's/\\n//g' f"), "sed 's/\\n//g' f");
        // 双引号内的转义序列（非路径前缀）不动
        assert_eq!(f("printf \"a\\tb\""), "printf \"a\\tb\"");
        // 无引号的正则样式（含 | 或 \/）不动
        assert_eq!(f("grep a\\|b f"), "grep a\\|b f");
        assert_eq!(f("sed s/a\\/b/c/ f"), "sed s/a\\/b/c/ f");
        // 双引号内的绝对路径（前缀形态）仍转
        assert_eq!(f("cmd \"dir D:\\x\""), "cmd \"dir D:/x\"");
    }
