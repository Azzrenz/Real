//! `cmd_bash` 的判据用例：**收谁、不收谁**是这条通道唯一的决策点，钉死它。

use super::*;

#[test]
fn unix_commands_route_to_bash() {
    for c in [
        "grep -rn \"x\" .",
        "find . -name \"*.rs\"",
        "sed -n '1,3p' f.txt",
        "awk '{print $1}' f",
        "ls -la server",
        "cat file.md",
        "head -20 log.txt",
        "tail -f x.log",
        "wc -l file",
        "mkdir -p a/b/c",
        "rm -f tmp.txt",
        "which cargo",
    ] {
        assert!(wants_unix_shell(c), "应改道 Git Bash：{c}");
    }
}

#[test]
fn cmd_commands_stay_on_cmd() {
    // 这些是模型写 cmd/PS 的常态，**不得**被改道（否则 cmd 习惯全线失效）
    for c in [
        "dir /b /s *.rs",
        "type file.md",
        "findstr /n /i \"pat\" file.txt",
        "cargo build --release",
        "python script.py",
        "git status --short",
        "git commit -m \"x\"",
        "echo hello",
        "sort file.txt",
        "more file.txt",
        "where git",
        "Remove-Item -Force 'D:/x.tmp'",
        "powershell -NoProfile -Command \"Get-ChildItem\"",
        "build-run.bat",
    ] {
        assert!(!wants_unix_shell(c), "不应改道（cmd 有同名/同义）：{c}");
    }
}

#[test]
fn chain_with_any_unix_segment_routes_to_bash() {
    // **改道保留**：翻译层给不出等价 ⇒ bash 是唯一能跑通的壳
    for c in [
        "cargo build 2>&1 | grep -iE \"error|错误\"",
        // tail 带非行数参数 ⇒ 翻译层显式拒绝（见 cmd_translate_tests 的 tail/head 用例）
        "echo x | tail -c 1200",
    ] {
        assert!(wants_unix_shell(c), "翻译层给不出等价 ⇒ 改道 Git Bash：{c}");
    }
    // **不再改道**：翻译层能给出等价 ⇒ 走 cmd 翻译，命令体的 Windows 语义得以保留
    for c in [
        "cargo test | tail -40",
        "python x.py | head -5",
        "git status | wc -l",
        "python C:\\Users\\a\\scripts\\srf_extract.py \"C:\\Program Files\\Cubase 15\\Skins\\skin.srf\" --list 2>&1 | tail -40",
    ] {
        assert!(
            !wants_unix_shell(c),
            "管道段可翻译 ⇒ 不得改道（改道会让未加引号的 Windows 路径被吃）：{c}"
        );
    }
    // 边界一：**含 cmd 专有语法不改道** —— `dir /b`、`2>nul` 在 bash 里语义不同，
    for c in ["cargo build 2>nul | tail -5", "dir /b | grep x"] {
        assert!(!wants_unix_shell(c), "cmd 专有语法不得改道：{c}");
    }
    // 边界二：**首段是 cmd 内建不改道** —— 得先由 `rewrite_cmd_builtin` 做等价映射
    for c in ["type build_out.txt | grep -i error", "findstr /n x f | head -5"] {
        assert!(!wants_unix_shell(c), "首段是 cmd 内建 ⇒ 交给内建映射路径：{c}");
    }
}

#[test]
fn semicolon_chain_with_unix_segment_routes_to_bash() {
    for c in [
        "tasklist /FI \"PID eq 7644\" | findstr /i msbuild; echo ---; tail -c 1200 \"f.log\"",
        "echo a; grep x f.txt",
        "dir; ls",
        "cargo build && grep -i error out.log",
    ] {
        assert!(wants_unix_shell(c), "语句首有 Unix 命令 ⇒ 整条改道 Git Bash：{c}");
    }
    // 边界不变：纯 cmd 且语句首无 Unix → 仍走 cmd
    for c in ["dir /b; echo done", "findstr /n \"pat\" f.txt"] {
        assert!(!wants_unix_shell(c), "语句首无 Unix 命令不得改道：{c}");
    }
    // 两级判据的分界：**管道段**里的 Unix 命令不改变"首段是 cmd 内建"的归属 ——
    for c in ["type build_out.txt | grep -i error", "findstr /n x f | head -5"] {
        assert!(!wants_unix_shell(c), "管道段有 Unix、但语句首是 cmd 内建 ⇒ 仍走映射路径：{c}");
    }
}

#[test]
fn unix_native_shapes_route_to_bash() {
    assert!(
        wants_unix_shell(
            "\"/c/Program Files/Microsoft Visual Studio/2022/Community/MSBuild/Current/Bin/MSBuild.exe\" \
             \"Builds/VisualStudio2022/MyApp.sln\" -p:Configuration=Release -m -v:m"
        ),
        "bash 风格盘符路径 ⇒ 必须走 Git Bash"
    );
    // ① 转义引号
    assert!(
        wants_unix_shell("python -c \"print(\\\"x\\\")\""),
        "含转义引号 ⇒ 走 Git Bash（cmd 里 \\ 不转义）"
    );
    // 边界：Windows 风格路径 / URL 不得误判（误判的代价是"多走一次 bash 并报错"，但仍要避免）
    for c in [
        "cargo build --release",
        "D:/proj/server/build.bat",
        "curl https://example.com/a/b",
        "powershell -NoProfile -Command \"Get-ChildItem\"",
    ] {
        assert!(!wants_unix_shell(c), "不得误判为 bash 原生写法：{c}");
    }
}

#[test]
fn judge_is_case_and_suffix_insensitive() {
    assert!(wants_unix_shell("GREP -n x f"), "大写应命中");
    assert!(wants_unix_shell("grep.exe -n x f"), "带扩展名应命中");
    assert!(wants_unix_shell("/usr/bin/grep -n x f"), "带路径前缀应命中");
    assert!(wants_unix_shell("   grep x"), "前导空格应命中");
    assert!(!wants_unix_shell(""), "空命令不命中");
    assert!(!wants_unix_shell("   "), "纯空白不命中");
}

#[test]
fn locate_is_stable_across_calls() {
    // 缓存的是同一个结果（缓存错了会让首条命令与其余命令走不同通道）
    assert_eq!(locate(), locate());
}

// ── 第二类路由：cmd 内建 → Unix 等价 ──

#[test]
fn cmd_builtin_type_rewrites_to_cat() {
    assert_eq!(
        rewrite_cmd_builtin("type D:/x.log").as_deref(),
        Some("cat D:/x.log"),
        "type f 与 cat f 完全等价"
    );
    // 无参不映射：cmd 的 `type`（无参）显示"命令类型"，与 cat 不等价
    assert_eq!(rewrite_cmd_builtin("type"), None);
}

#[test]
fn real_incident_chain_rewrites_whole() {
    let got = rewrite_cmd_builtin(
        r#"type "C:/a/b.log" | findstr /v "replaced by CRLF" | findstr /v "^warning:""#,
    );
    assert_eq!(
        got.as_deref(),
        Some(r#"cat "C:/a/b.log" | grep -v "replaced by CRLF" | grep -v "^warning:""#),
        "内建链应整体等价改写"
    );
}

#[test]
fn findstr_flag_table_is_closed() {
    assert_eq!(
        rewrite_cmd_builtin("findstr /i /n error f.txt").as_deref(),
        Some("grep -i -n error f.txt")
    );
    assert_eq!(
        rewrite_cmd_builtin(r#"findstr /C:"test result" f"#).as_deref(),
        Some(r#"grep -F -e "test result" f"#),
        "/C: 是字面串 ⇒ -F（且带空格的模式不能被拆断）"
    );
    assert_eq!(
        rewrite_cmd_builtin("findstr /s /v x").as_deref(),
        Some("grep -r -v x")
    );
    // 表外 flag → 整体不改（不猜：静默改变语义比拒绝更糟）
    assert_eq!(rewrite_cmd_builtin("findstr /o x f"), None, "表外 flag 不映射");
}

#[test]
fn unknown_commands_are_never_rewritten() {
    for c in [
        "dir /b",
        "cargo test | findstr error",
        "tasklist",
        "python x.py",
        "Get-ChildItem",
    ] {
        assert_eq!(rewrite_cmd_builtin(c), None, "不该改写: {c}");
    }
}

#[test]
fn unix_segments_pass_through_untouched() {
    // 链上本来就是 Unix 命令的段原样保留（与 wants_unix_shell 共用同一张表）
    assert_eq!(
        rewrite_cmd_builtin("type a.log | grep x").as_deref(),
        Some("cat a.log | grep x")
    );
}

#[test]
fn file_op_builtins_rewrite_by_table() {
    assert_eq!(
        rewrite_cmd_builtin("copy /y a.txt b.txt").as_deref(),
        Some("cp -f a.txt b.txt")
    );
    assert_eq!(
        rewrite_cmd_builtin("del /f /q \"C:/x y/z.log\"").as_deref(),
        Some("rm -f \"C:/x y/z.log\""),
        "带空格的路径要整体保留（引号感知分词）"
    );
    assert_eq!(rewrite_cmd_builtin("md newdir").as_deref(), Some("mkdir newdir"));
    assert_eq!(rewrite_cmd_builtin("move a b").as_deref(), Some("mv a b"));
    // 表外 flag 不改：`del /s` 与 `rm -r` 语义有差（rm -r 连目录本身一起删）
    assert_eq!(rewrite_cmd_builtin("del /s /q dir"), None);
    assert_eq!(rewrite_cmd_builtin("copy /z a b"), None);
    // 无参数不改
    assert_eq!(rewrite_cmd_builtin("del"), None);
}

// ── 第三类判定：混血命令（首词属 PS 壳 + 命令体是 cmd 语法）──

const PS: &[&str] = &["Remove-Item", "Get-ChildItem", "Get-Item", "Select-String"];

#[test]
fn mixed_shell_detected_on_real_incident() {
    let cmd = r#"Remove-Item -Force 'D:\project-b\server\src\prompts.rs.pre_cut' & echo == 复查 == & dir /b /s "D:\project-b\server\src\*.pre_*" 2>nul & echo (无输出=干净)"#;
    let got = detect_mixed_shell(cmd, PS);
    assert!(got.is_some(), "混血命令必须被拒");
    assert!(
        got.as_deref().unwrap().contains('&'),
        "应指名 `&`（本次事故的直接原因）：{got:?}"
    );
    // 首词大小写不敏感（PS 的 cmdlet 名本身就不敏感）
    assert!(detect_mixed_shell("remove-item -Force 'x' & echo hi", PS).is_some());
}

#[test]
fn mixed_shell_detects_cmd_redirect_and_dir_flags() {
    assert!(
        detect_mixed_shell("Remove-Item -Force 'x' 2>nul", PS).is_some(),
        "2>nul 是 cmd 的空设备重定向"
    );
    assert!(
        detect_mixed_shell("Get-ChildItem x | dir /b", PS).is_some(),
        "dir /b 是 cmd 参数（PS 的 dir 别名不认）"
    );
}

#[test]
fn pure_ps_and_ps_redirects_pass() {
    for c in [
        "Remove-Item -Force 'D:/x.tmp'",
        "Get-ChildItem -Recurse -Filter *.rs",
        "Remove-Item a.txt; Get-ChildItem .",
        "Get-Item x 2>&1",
        "Select-String -Path f.txt -Pattern x",
    ] {
        assert_eq!(detect_mixed_shell(c, PS), None, "不该判为混血: {c}");
    }
}

#[test]
fn non_ps_first_word_is_never_mixed() {
    // 首词不属 PS 壳 ⇒ 这层不管（别把 Unix 命令的病因误报成混血）
    assert_eq!(detect_mixed_shell("cargo test & echo done", PS), None);
    assert_eq!(detect_mixed_shell("grep x f & ls", PS), None);
    assert_eq!(detect_mixed_shell("", PS), None);
    assert_eq!(detect_mixed_shell("   ", PS), None);
}
