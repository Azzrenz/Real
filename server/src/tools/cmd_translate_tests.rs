use super::{extract_inline_code, extract_tail_n, to_ps_pipeline, translate_semicolons, translate_unix_pipeline};

#[test]
fn multiline_inline_code_with_pipe_carried_by_powershell() {
    // 跨行内联代码 + 管道：原生不带管道、cmd 不支持跨行引号 → 整条交 PS 原生管道
    let cmd = "python -X utf8 -c \"import io\nfor i in range(3):\n    print(i)\" | head -5";
    let ps = to_ps_pipeline(cmd).expect("应能映射为 PS 原生管道");
    assert!(ps.starts_with("python -X utf8 -c \""), "首段原样保留: {ps}");
    assert!(ps.contains("for i in range(3)"), "多行代码须完整: {ps}");
    assert!(ps.ends_with("| Select-Object -First 5"), "head 映射: {ps}");
    assert!(!ps.contains("powershell"), "不得嵌套 powershell（$input 会被外层展开）: {ps}");
}

#[test]
fn ps_pipeline_refuses_unmappable_segment() {
    // 有段无法映射 → None（调用方走原路径，不生成半吊子命令）
    assert_eq!(to_ps_pipeline("python -c \"print(1)\" | sed s/a/b/"), None);
    assert_eq!(to_ps_pipeline("python -c \"print(1)\""), None);
}

#[test]
fn ps_pipeline_maps_tail_grep_wc_cat() {
    assert!(to_ps_pipeline("python -c \"a\nb\" | tail -3")
        .unwrap()
        .ends_with("Select-Object -Last 3"));
    assert!(to_ps_pipeline("python -c \"a\nb\" | grep -i ok")
        .unwrap()
        .contains("Select-String"));
    assert!(to_ps_pipeline("python -c \"a\nb\" | wc -l")
        .unwrap()
        .ends_with("Measure-Object -Line"));
}

#[test]
fn inline_code_keeps_multiline_and_drops_pipe_tail() {
    let cmd = "python -X utf8 -c \"import io\nfor i in range(3):\n    print(i)\" | powershell -NoProfile -Command \"$input | Select-Object -First 50\"";
    let code = extract_inline_code(cmd, " -c ");
    assert!(code.contains("for i in range(3)"), "多行代码应完整: {code}");
    assert!(code.contains('\n'), "换行须保留（原生执行合法）: {code}");
    assert!(
        !code.contains("Select-Object"),
        "管道段不得进代码（此前导致 SyntaxError）: {code}"
    );
}

#[test]
fn inline_code_single_quoted() {
    let code = extract_inline_code("python -c 'print(1); print(2)' | head -1", " -c ");
    assert_eq!(code, "print(1); print(2)");
}

#[test]
fn inline_code_node_e_flag() {
    let code = extract_inline_code("node -e \"console.log(1)\" | head -1", " -e ");
    assert_eq!(code, "console.log(1)");
}

#[test]
fn semicolons_translated_only_outside_single_quotes() {
    assert_eq!(
        translate_semicolons("python -c 'a;b'"),
        "python -c 'a;b'"
    );
    let out = translate_semicolons("echo a; echo b");
    assert!(out.contains(" & ") && !out.contains(';'), "引号外 ; 应转 &: {out}");
}

// 复合命令 & 结构翻译（修复：管道切分不得跨顶层 & / 括号）
#[test]
fn compound_paren_branches_each_tail_translated_and_structure_kept() {
    // 实盘翻车命令：多路 git clone 并列，每支 2>&1 | tail -n N
    let cmd = "(git clone --depth 1 https://github.com/a/b.git 2>&1 | tail -n 2) & (git clone --depth 1 https://github.com/c/d.git 2>&1 | tail -n 5)";
    let out = translate_unix_pipeline(cmd).unwrap();
    // 每支 tail 各自翻译成 PS 且行数正确（不再是默认 10）
    assert!(
        out.contains("Select-Object -Last 2"),
        "第一支应 -Last 2: {out}"
    );
    assert!(
        out.contains("Select-Object -Last 5"),
        "第二支应 -Last 5（此前误退 10）: {out}"
    );
    assert!(out.contains(" & "), "并列分隔符应保留: {out}");
    assert!(out.contains("github.com/a/b.git") && out.contains("github.com/c/d.git"), "两支 URL 都必须保留: {out}");
    assert_eq!(out.matches('(').count(), out.matches(')').count(), "括号须配平: {out}");
}

#[test]
fn ampersand_branch_no_parens_translates_tail_once() {
    let cmd = "cargo test 2>&1 | tail -n 6 & echo done";
    let out = translate_unix_pipeline(cmd).unwrap();
    assert!(out.contains("Select-Object -Last 6"), "tail 分支应翻译: {out}");
    assert!(out.contains("echo done"), "& 后续分支不得吞: {out}");
    assert_eq!(out.matches("powershell").count(), 1, "只译一次: {out}");
}

#[test]
fn redirect_ampersand_not_treated_as_branch_separator() {
    // 2>&1 里的 & 不是并列分隔符
    let cmd = "git clone https://github.com/x/y.git 2>&1";
    let out = translate_unix_pipeline(cmd).unwrap();
    assert!(!out.contains(" & "), "2>&1 不得被当成分支: {out}");
    assert!(out.contains("2>&1"), "重定向保留: {out}");
}

#[test]
fn simple_pipeline_still_translates() {
    let out = translate_unix_pipeline("cargo test 2>&1 | tail -n 15").unwrap();
    assert!(out.contains("Select-Object -Last 15"));
}

#[test]
fn tail_n_with_paren_tail_defensive() {
    assert_eq!(extract_tail_n("tail -n 2)"), 2);
    assert_eq!(extract_tail_n("tail -5"), 5);
}

    #[test]
    fn chain_with_grep_gets_translated() {
        let cmd = "cd /d D:/Phoenix/server && grep -n \"pub fn extract_error_meta\" src/tools/contract.rs";
        let out = crate::tools::cmd_translate::translate_single_unix_command(cmd)
            .expect("链式不应拒绝")
            .expect("含 grep 应翻译");
        assert!(out.contains("findstr"), "grep 段应译为 findstr: {out}");
        assert!(out.contains("/n"), "-n 应译为 findstr /n: {out}");
        assert!(out.starts_with("cd /d"), "cd 段原样保留: {out}");
        assert!(out.contains("&&"), "链算子保留: {out}");
    }

    #[test]
    fn grep_context_flags_deterministically_rejected() {
        let cmd = "grep -n -A 6 \"pub fn x\" src/lib.rs";
        let err = crate::tools::cmd_translate::translate_single_unix_command(cmd)
            .expect_err("上下文 flag 应确定性拒绝");
        assert!(err.contains("-A"), "教学应指明是哪个上下文 flag: {err}");
    }

    #[test]
    fn chain_preserves_non_unix_segments() {
        let cmd = "cargo check && dir /b";
        let out = crate::tools::cmd_translate::translate_single_unix_command(cmd)
            .expect("无 unix 段不拒绝")
            .expect("有 & 时返回重排后的整链");
        assert!(out.contains("cargo check") && out.contains("dir /b"), "段保留: {out}");
    }

    /// **任一段被拒 = 整链拒绝** —— 这是 run 里那句「整条命令未执行，零副作用」的**前提**。
    #[test]
    fn chain_rejects_whole_when_any_segment_untranslatable() {
        let cmd = "git checkout -- a.rs & find . -name \"*.rs\"";
        let err = crate::tools::cmd_translate::translate_single_unix_command(cmd)
            .expect_err("含 find 的链必须整条拒绝（否则『未执行』就是假话）");
        assert!(err.contains("find"), "应指明是哪一段不合规: {err}");
    }

    /// `2>&1` 的重定向 `&` **不是**链分隔符 —— 拆链必须原样保留。
    #[test]
    fn chain_keeps_redirect_amp_intact() {
        let cmd = "dir /b D:/x 2>&1 & echo == y ==";
        let out = crate::tools::cmd_translate::translate_single_unix_command(cmd)
            .expect("链里有重定向不该被拒")
            .expect("有 & 应返回重排后的整链");
        assert!(out.contains("2>&1"), "重定向必须完整保留: {out}");
        assert!(!out.contains("2> & 1"), "不得把 2>&1 拆成 2> & 1: {out}");
        // 反向重定向 `1>&2` 同理
        let out2 = crate::tools::cmd_translate::translate_single_unix_command("echo a 1>&2 & echo b")
            .expect("不拒绝")
            .expect("有 & 返回整链");
        assert!(out2.contains("1>&2"), "反向重定向同样保留: {out2}");
    }

#[test]
fn tail_or_head_with_file_is_refused_not_silently_emptied() {
    // ① 带 `-c`（按字节）：Select-Object 只按行，无等价
    let e1 = translate_unix_pipeline("echo x | tail -c 1200").unwrap_err();
    assert!(e1.contains("按字节"), "应说明是字节语义无等价: {e1}");

    // ② 管道位置但带文件参数：文件名不能丢
    let e2 = translate_unix_pipeline("echo x | tail \"a.log\"").unwrap_err();
    assert!(e2.contains("文件参数"), "应指明是文件参数形态: {e2}");

    // ③ 落在语句首（前面没有上游）：没有输入源可读
    let e3 = translate_unix_pipeline("echo y | findstr z & tail -40").unwrap_err();
    assert!(e3.contains("首段"), "应指明语句首无上游: {e3}");

    // 反向：真管道位置 + 不带文件 → **保持原有能力**，别一并砍掉
    let ok = translate_unix_pipeline("cargo test 2>&1 | tail -n 15").unwrap();
    assert!(ok.contains("Select-Object -Last 15"), "正常管道仍应翻译: {ok}");
}

#[test]
fn redirect_suffix_is_not_a_file_arg() {
    // 正向：重定向后缀不得导致拒绝（本次修的就是这个）
    for c in [
        "dir /b D:/x | head -n 8 2>nul",
        "dir /b D:/x | tail -n 20 2>&1",
        "cargo test 2>&1 | tail -5 >out.txt",
    ] {
        let r = translate_unix_pipeline(c);
        assert!(r.is_ok(), "重定向后缀不该触发拒绝: {c} → {r:?}");
    }
    // 反向：真带文件名**仍然拒绝**（别把这道保护一起砍了）
    let e = translate_unix_pipeline("echo x | head -n 8 d:/x/a.log").unwrap_err();
    assert!(e.contains("文件参数"), "真文件名仍须拦: {e}");
}

#[test]
fn grep_flags_outside_whitelist_are_refused() {
    // 放行：-i / -n（含合并写法）
    for ok in [
        "echo x | grep -i pat",
        "echo x | grep -in pat",
        "echo x | grep -n pat",
    ] {
        assert!(translate_unix_pipeline(ok).is_ok(), "白名单内应放行: {ok}");
    }
    // 拒绝：递归 / 计数 / 只列名 / 上下文 / 未登记
    for (bad, hint) in [
        ("echo x | grep -r pat src", "递归"),
        ("echo x | grep -R pat", "递归"),
        ("echo x | grep -c pat", "计数"),
        ("echo x | grep -l pat f", "只列文件名"),
        ("echo x | grep -A 3 pat f", "上下文行"),
        ("echo x | grep -z pat f", "未登记"),
    ] {
        let e = translate_unix_pipeline(bad).unwrap_err();
        assert!(e.contains(hint), "应说明「{hint}」无等价，实际: {e}");
    }
}

/// `cat` 带 flag / `wc` 非 `-l`：确定性拒绝（旧实现会生成 `type -n file` 这种错命令）。
#[test]
fn cat_flags_and_wc_non_l_are_refused() {
    let e = translate_unix_pipeline("echo x | cat -n").unwrap_err();
    assert!(e.contains("read 工具"), "应引导到 read 工具: {e}");

    let e2 = translate_unix_pipeline("echo x | wc -w").unwrap_err();
    assert!(e2.contains("wc -l"), "应说明只支持 wc -l: {e2}");

    // 正向：无 flag 的 cat、带 -l 的 wc 仍要工作（别一并砍掉）
    assert!(translate_unix_pipeline("echo x | cat").is_ok(), "cat 无 flag 应放行");
    assert!(
        translate_unix_pipeline("echo x | wc -l").unwrap().contains("find /c /v"),
        "wc -l 仍应翻译"
    );
}
