//! tools/modify.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod locate_intent_tests {
    use super::{indexed_structure, locate_intent_target};

    fn write_temp(tag: &str, content: &str) -> String {
        // 每个测试独立目录：cargo test 并行跑，共享路径会互相覆盖导致定位错乱
        let dir = std::env::temp_dir().join(format!("modify_loc_{}_{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("routing.py");
        std::fs::write(&f, content).unwrap();
        f.display().to_string()
    }

    #[test]
    fn locates_class_from_intent() {
        let f = write_temp("cls", "class WebSocketRoute:\n    def __init__(self, app):\n        pass\n\nclass HttpRoute:\n    pass\n");
        let r = locate_intent_target(&f, "修改 WebSocketRoute 类，使其接收 dependencies 参数");
        let (kind, name, line, _) = r.expect("应定位到 WebSocketRoute");
        assert_eq!(kind, "class");
        assert!(name.contains("WebSocketRoute"), "name={name}");
        assert_eq!(line, 1);
        let _ = std::fs::remove_dir_all(std::path::Path::new(&f).parent().unwrap());
    }

    #[test]
    fn locates_method_from_intent() {
        let f = write_temp("mth", "class WebSocketRoute:\n    def __init__(self, app):\n        pass\n    def get_route_handler(self):\n        return self\n");
        let r = locate_intent_target(
            &f,
            "在 get_route_handler 中把 dependencies 合并后调用 solve_dependencies",
        );
        let (_, name, line, _) = r.expect("应定位到 get_route_handler");
        assert!(name.contains("get_route_handler"), "name={name}");
        assert!(line >= 4, "line={line}");
        let _ = std::fs::remove_dir_all(std::path::Path::new(&f).parent().unwrap());
    }

    #[test]
    fn no_match_returns_none() {
        let f = write_temp("nom", "class WebSocketRoute:\n    pass\n");
        assert!(locate_intent_target(&f, "修改不存在的 FooBar 类").is_none());
        let _ = std::fs::remove_dir_all(std::path::Path::new(&f).parent().unwrap());
    }

    #[test]
    fn line_range_locates_by_line_number() {
        let content = (1..=20)
            .map(|i| format!("line_{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let f = write_temp("ln", &content);
        let r = locate_intent_target(&f, "在 L5-8 的 logic 中修改 xx").expect("应行号定位");
        let (kind, _name, line, snippet) = r;
        assert_eq!(kind, "lines");
        assert_eq!(line, 5, "起始行应为 5（line 字段保持原始目标行）");
        // （FIND_AMBIGUOUS 根治）：snippet 前后扩展 8 行上下文——
        assert!(
            snippet.lines().count() > 4,
            "应返回带上下文的扩展 snippet（>4 行），实际 {}",
            snippet.lines().count()
        );
        assert!(
            snippet.contains("line_5") && snippet.contains("line_8"),
            "snippet 应含目标行"
        );
        assert!(
            snippet.contains("line_1"),
            "snippet 应含前扩上下文（line_1）"
        );
        let _ = std::fs::remove_dir_all(std::path::Path::new(&f).parent().unwrap());
    }

    #[test]
    fn line_range_variants() {
        // 多种行号写法归一
        let f = write_temp("lnv", "a\nb\nc\nd\ne\n");
        let r1 = locate_intent_target(&f, "改 2-3 行").expect("2-3 应定位");
        assert_eq!(r1.2, 2);
        let r2 = locate_intent_target(&f, "第4行的逻辑").expect("第4行应定位");
        assert_eq!(r2.2, 4);
        let _ = std::fs::remove_dir_all(std::path::Path::new(&f).parent().unwrap());
    }

    #[test]
    fn line_range_snippet_has_context() {
        // （FIND_AMBIGUOUS 根治）：行号定位 snippet 前后扩展 8 行——
        let content = "fn f() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    let e = 5;\n    let g = 6;\n    let h = 7;\n    let i = 8;\n    let j = 9;\n    let k = 10;\n}\n";
        let f = write_temp("lnctx", content);
        let r = locate_intent_target(&f, "改 L6-7 的 logic").expect("L6-7 应定位");
        assert_eq!(r.2, 6, "line 字段应保持原始目标起始行: {}", r.2);
        let lines: Vec<&str> = r.3.lines().collect();
        assert!(
            lines.len() >= 10,
            "snippet 应有上下文（≥10 行），实际 {}: {:?}",
            lines.len(),
            r.3
        );
        assert!(
            r.3.contains("fn f()"),
            "snippet 应含函数开头（前扩上下文）: {}",
            r.3
        );
        assert!(r.3.contains("let g = 6"), "snippet 应含目标行: {}", r.3);
        let _ = std::fs::remove_dir_all(std::path::Path::new(&f).parent().unwrap());
    }

    #[test]
    fn indexed_structure_caches_and_invalidates() {
        // 用户方案 #1：文件索引缓存——同文件重复定位复用 structure，
        let dir = std::env::temp_dir().join(format!("idx_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("idx.py");
        std::fs::write(&f, "def alpha():\n    pass\n").unwrap();
        let fp = f.display().to_string();

        // 首次：建索引
        let s1 = indexed_structure(&fp).expect("首次应建索引");
        assert!(
            s1.iter().any(|(_, _, n)| n == "alpha"),
            "索引应含 alpha: {s1:?}"
        );

        // 未修改：命中缓存（structure 相同）
        let s2 = indexed_structure(&fp).expect("缓存应命中");
        assert_eq!(s1, s2, "未修改应命中缓存");

        // 修改文件（mtime 变化）→ 失效重扫
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "def beta():\n    pass\n").unwrap();
        let s3 = indexed_structure(&fp).expect("修改后应重扫");
        assert!(
            s3.iter().any(|(_, _, n)| n == "beta"),
            "重扫后应含 beta: {s3:?}"
        );
        assert!(
            !s3.iter().any(|(_, _, n)| n == "alpha"),
            "alpha 应消失: {s3:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn acceptance_append_style_matches() {
        // （机制修复）：追加型修改（replace 含 find，如插入代码块保留 find 原文）
        let content = "if CWD_TOOLS.contains(&tool) {\n    insert_new();\n    do_it();\n}\n";
        let find = "if CWD_TOOLS.contains(&tool) {";
        let replace = "if CWD_TOOLS.contains(&tool) {\n    insert_new();\n";
        let (fc, rc, matched) = super::acceptance_check(content, find, replace);
        assert_eq!(fc, 1, "find 仍在（追加型必然）");
        assert_eq!(rc, 1, "replace 应出现");
        assert!(matched, "追加型应判匹配: fc={fc} rc={rc}");
    }

    #[test]
    fn acceptance_replace_style_requires_find_gone() {
        // 替换型（replace 不含 find）→ 必须 find 消失 + replace 出现
        let content = "let a = 10;\n";
        let (fc, rc, matched) = super::acceptance_check(content, "let a = 1;", "let a = 10;");
        assert_eq!(fc, 0);
        assert_eq!(rc, 1);
        assert!(matched);
    }

    #[test]
    fn acceptance_delete_style_matches_when_find_gone() {
        // content 传的是**删除之后**的文件内容（apply 的结果），find 已不在其中。
        let content = "let b = 2;\n";
        let (fc, rc, matched) = super::acceptance_check(content, "keep();\n", "");
        assert_eq!(fc, 0, "删完 find 应消失");
        assert_eq!(rc, 0, "空 replace 无法计数");
        assert!(matched, "删除成功必须判通过: fc={fc} rc={rc}");
    }

    /// 删除语义**不能放水**：find 还在 ⇒ 没删掉 ⇒ 必须判失败
    #[test]
    fn acceptance_delete_still_fails_when_find_remains() {
        let content = "keep();\nlet b = 2;\n";
        let (fc, _rc, matched) = super::acceptance_check(content, "let b = 2;\n", "");
        assert_eq!(fc, 1, "find 仍在文件里");
        assert!(!matched, "find 未消失 ⇒ 删除没生效 ⇒ 必须判失败");
    }

    #[test]
    fn brace_balance_detects_extra_close() {
        // 结构护栏：多余 } 应检测为不平衡
        assert!(crate::tools::fs_common::brace_balance_ok("fn f() {\n    let x = 1;\n}\n"));
        assert!(
            !crate::tools::fs_common::brace_balance_ok("fn f() {\n    let x = 1;\n}\n}\n"),
            "多余 }} 应不平衡"
        );
        assert!(
            crate::tools::fs_common::brace_balance_ok("fn f() {\n    let s = format!(\"{{}}\", x);\n}\n"),
            "字符串内括号应忽略"
        );
        assert!(
            crate::tools::fs_common::brace_balance_ok("// } 注释里的括号\nfn f() {}\n"),
            "注释内括号应忽略"
        );
    }
}

#[cfg(test)]
mod brace_guard_tests {
    // 判据本体已搬到 `tools/fs_common.rs`（modify 与 run 两条通道共用），
    use crate::tools::fs_common::{brace_balance_ok, brace_diagnose, brace_guard_should_block};

    #[test]
    fn lifetime_quotes_do_not_break_balance() {
        // 误判回归：Rust 生命周期 'a 遍地，旧扫描器把 ' 当字符字面量起点
        let code = "fn foo<'a>(x: &'a str) -> &'a str {\n    let y = 'x';\n    y\n}\n";
        assert!(brace_balance_ok(code), "含生命周期+字符字面量的合法代码应判平衡");
    }

    #[test]
    fn loop_label_and_char_literal_ok() {
        let code = "'outer: loop {\n    let c = '}';\n    break 'outer;\n}\n";
        assert!(brace_balance_ok(code), "标签 + 花括号字符字面量应判平衡");
    }

    #[test]
    fn raw_string_content_braces_are_ignored() {
        let code =
            "fn f() {\n    let args = r##\"{\"objective\":\"x\",\"steps\":[{\"step_id\":\"#E1\"}]}\"##;\n}\n";
        assert!(brace_balance_ok(code), "raw string 里的花括号是内容，应忽略");
        // 各级 # 深度与字节 raw string 都要认
        assert!(brace_balance_ok("fn f() { let s = r\"}\"; }\n"), "r\"\" 应忽略");
        assert!(brace_balance_ok("fn f() { let s = r###\"}\"###; }\n"), "r###\"\"### 应忽略");
        assert!(brace_balance_ok("fn f() { let s = br#\"}\"#; }\n"), "字节 raw string 应忽略");
        // 跨行 raw string（JSON 常多行）要整体吞掉
        let multiline = "fn f() {\n    let j = r#\"\n{\n  \"a\": 1\n}\n\"#;\n}\n";
        assert!(brace_balance_ok(multiline), "跨行 raw string 应整体吞掉");
        // 但**真的**多一个闭合仍要检出 —— 别为了剥字面量把护栏弄哑
        assert!(
            !brace_balance_ok("fn f() {\n    let args = r#\"{}\"#;\n}\n}\n"),
            "raw string 之外的多余闭合必须仍检出"
        );
    }

    #[test]
    fn real_unbalanced_still_detected() {
        assert!(!brace_balance_ok("fn f() {\n    let x = 1;\n"), "缺闭合应检出");
        assert!(!brace_balance_ok("fn f() {\n}\n}\n"), "多余闭合应检出");
    }

    #[test]
    fn guard_blocks_only_when_edit_breaks_balance() {
        // 防误伤：修改前就不平衡（文件自身问题）→ 不拦，交给编译验证
        let before = "fn f() {\n    let x = 1;\n";
        let after_same = "fn f() {\n    let x = 2;\n";
        assert!(!brace_guard_should_block(before, after_same), "原本不平衡的改动不该拦（防回滚好改动）");
        // 改前平衡、改后不平衡 → 拦
        let good = "fn f() {\n    let x = 1;\n}\n";
        let broken = "fn f() {\n    let x = 1;\n";
        assert!(brace_guard_should_block(good, broken), "把平衡改坏必须拦");
        // 改前改后都平衡 → 不拦
        assert!(!brace_guard_should_block(good, "fn f() {\n    let x = 9;\n}\n"), "平衡改动放行");
    }

    #[test]
    fn diagnose_reports_line_and_delta() {
        let d = brace_diagnose("fn f() {\n    let x = 1;\n}\n}\n");
        assert!(d.contains("净差"), "应报净差: {d}");
        assert!(d.contains("第 4 行"), "应报多余闭合行号: {d}");
        let d2 = brace_diagnose("fn f() {\n    let x = 1;\n");
        assert!(d2.contains("未闭合"), "应报未闭合开口: {d2}");
    }
}

// （度量衡统一·回归）模型 find/replace 带 \r\n、落盘为 LF 时，校验必须与匹配侧同尺——
#[test]
fn acceptance_check_crlf_normalized() {
    // 落盘内容 = apply 后的 LF 文本（EditTool 匹配侧归一化写入）；模型 find/replace 带 \r\n
    let content = "def a():\n    return 2\n";
    let find = "def a():\r\n    return 1\r\n";
    let replace = "def a():\r\n    return 2\r\n";
    let norm = |s: &str| s.replace("\r\n", "\n").replace('\r', "\n");
    let (_f, r, matched) = super::acceptance_check(&norm(content), &norm(find), &norm(replace));
    assert!(matched, "归一化后应通过（replace 出现 {r} 次）");
}

#[test]
fn acceptance_check_still_rejects_real_failure() {
    // 真失败（replace 从未落盘）必须继续拦截——修 bug 不许拆护栏
    let content = "def a():\n    return 1\n";
    let (_f, _r, matched) =
        super::acceptance_check(content, "def a():", "def a():\n    return 2\n");
    assert!(!matched, "replace 未出现应判 false");
}

#[test]
fn insert_after_merge_keeps_its_own_line() {
    let cs = serde_json::json!({
        "mode": "block",
        "find": "    A;\n    B;",
        "insert_after": "    C;"
    });
    let (_, args) = super::compile_change_spec("x.cpp", &cs).expect("应能编译");
    let rep = args["replacements"][0]["replace"].as_str().unwrap_or("");
    assert_eq!(rep, "    A;\n    B;\n    C;", "新增内容必须独占一行");
    // find 末尾已有换行时不得重复加
    let cs2 = serde_json::json!({"mode":"block","find":"    A;\n","insert_after":"    C;\n"});
    let (_, args2) = super::compile_change_spec("x.cpp", &cs2).expect("应能编译");
    assert_eq!(
        args2["replacements"][0]["replace"].as_str().unwrap_or(""),
        "    A;\n    C;\n"
    );
}

#[test]
fn verify_uses_compiled_spec_for_insert_after() {
    // 执行认 insert_after、校验也必须认（单一真值源）。否则校验侧 replace 落到空串 ⇒
    let cs = serde_json::json!({
        "mode": "block",
        "find": "    A;\n    B;",
        "insert_after": "    C;"
    });
    let (_, args) = super::compile_change_spec("x.cpp", &cs).expect("应能编译");
    let v = super::verify_spec_from(&args);
    assert_eq!(v["find"], serde_json::json!("    A;\n    B;"), "find 应与执行一致");
    assert_eq!(
        v["replace"],
        serde_json::json!("    A;\n    B;\n    C;"),
        "replace 应是合并后的内容"
    );
    // 拿它校验一份"已正确插入"的文件：追加型 ⇒ 只看 replace 出现
    let content = "head\n    A;\n    B;\n    C;\ntail\n";
    let (_f, _r, matched) = super::acceptance_check(
        content,
        v["find"].as_str().unwrap_or(""),
        v["replace"].as_str().unwrap_or(""),
    );
    assert!(matched, "插入成功的文件必须判通过（原文仍在属追加型）");
    // line 模式底层用 new，也要映射到 replace
    let cs3 = serde_json::json!({"mode":"line","line":5,"replace":"    Z;"});
    let (_, args3) = super::compile_change_spec("x.cpp", &cs3).expect("应能编译");
    let v3 = super::verify_spec_from(&args3);
    assert_eq!(v3["replace"], serde_json::json!("    Z;"), "line 模式的 new 要映射成 replace");
}

#[test]
fn acceptance_empty_replace_is_not_present() {
    // 空 replace 不能算"已出现"：`matches("")` 返回 len+1，会把"没给内容"骗成"内容出现了"
    let (_f, r, matched) = super::acceptance_check("abc", "abc", "");
    assert_eq!(r, 0, "空 replace 计数必须为 0");
    assert!(!matched, "空 replace 不得判通过");
}

#[test]
fn doc_pair_steers_away_from_fragmented_reads() {
    let r = <crate::tools::fs_read::ReadTool as crate::mcp::registry::BuiltinTool>::description(
        &crate::tools::fs_read::ReadTool,
    );
    let e = crate::agent::specs::match_specs("改文件 修改 替换 按行");
    assert!(
        e.contains("SP-EDITSAFE"),
        "改文件类任务必须命中 SP-EDITSAFE：{e}"
    );
    assert!(
        e.contains("先一次 `read` 拿全貌与行号"),
        "SP-EDITSAFE 必须给出「一次读全」的次序引导"
    );
    assert!(
        r.contains("一次 `mode=full` 读全") || e.contains("SP-EDITSAFE"),
        "「要改就一次读全」的引导必须可达模型（read 描述或 SP-EDITSAFE 任一）"
    );
    let m = <ModifyTool as crate::mcp::registry::BuiltinTool>::description(&ModifyTool);
    assert!(
        m.contains("已知行号就优先用它"),
        "modify 侧缺「line 模式优先」的引导：有行号就不必读正文、不必贴 find"
    );
}

#[test]
fn nearest_actual_hands_back_real_lines_with_numbers() {
    // 校验没过时唯一的兜底信息源：告诉模型"文件里现在长什么样"（带行号），
    let content = "fn a() {\n    let x = 1;\n    let y = 2;\n}\n";
    let got = crate::tools::modify::nearest_actual(content, "let y = 2;").expect("应能找到锚");
    assert!(got.contains("3:     let y = 2;"), "应带行号与实际内容: {got}");

    // 锚 trim 后比较 ⇒ 缩进不同也能定位（模型猜错缩进正是最常见的失败）
    let got2 = crate::tools::modify::nearest_actual(content, "        let y = 2;")
        .expect("缩进差异应仍能定位");
    assert!(got2.contains("let y = 2;"), "{got2}");

    // 压根找不到 → None：**不编造**（宁可不给，也不给假位置）
    assert!(crate::tools::modify::nearest_actual(content, "这个词根本不在文件里").is_none());
}

#[test]
fn verify_fail_message_carries_actual_content() {
    let e = crate::tools::contract::ToolError::modify_verify_fail(
        true,
        false,
        Some("42:     let z = 3;"),
    );
    assert!(e.message.contains("替换后该处实际内容"), "缺少实况块: {}", e.message);
    assert!(e.message.contains("42:     let z = 3;"), "缺少实际行: {}", e.message);
    // 拿不到实况时也必须能正常出错误（不能空消息、不能 panic）
    let e2 = crate::tools::contract::ToolError::modify_verify_fail(true, false, None);
    assert!(e2.message.contains("MODIFY_VERIFY_FAIL") || e2.message.contains("校验未通过"));
    assert_eq!(e2.code, "MODIFY_VERIFY_FAIL");
}

// 三类坏调用取证：文案必须与**真实原因**对得上
#[cfg(test)]
mod bad_call_probe {
    use super::ModifyTool;
    use crate::mcp::registry::BuiltinTool;
    use serde_json::json;

    const SRC: &str = "fn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n}\n";

    fn tmp(tag: &str, content: &str) -> String {
        let dir = std::env::temp_dir().join(format!("modify_bad_{}_{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("t.rs");
        std::fs::write(&f, content).unwrap();
        f.display().to_string()
    }

    /// 坏调用①：**参数没填实**（缺失 / null / 空串）——三种写法一个码。
    #[tokio::test]
    async fn probe_1_param_unfilled() {
        let f = tmp("p1", SRC);
        let cases = [
            ("键缺失", json!({"find": "let x = 1;", "description": "改成 2"})),
            ("值 null", json!({"find": "let x = 1;", "replace": null})),
            ("空字符串", json!({"find": "let x = 1;", "replace": ""})),
            ("只含空白", json!({"find": "let x = 1;", "replace": "   \n  "})),
        ];
        for (label, cs) in cases {
            let e = ModifyTool
                .run(json!({"file": f, "change_spec": cs}))
                .await
                .unwrap_err();
            println!("\n[① 参数没填实 · {}]\n{}\n", label, e.chars().take(400).collect::<String>());
            assert!(
                e.contains("MODIFY_PARAM_UNFILLED"),
                "①{label} 没自证为参数问题（模型会被支使去翻文件系统）: {e}"
            );
            // 反向守住：不得再退回"意图定位失败"那种把人引向文件系统的说法
            assert!(
                !e.contains("MODIFY_INTENT_LOCATE_FAILED"),
                "①{label} 又退回意图分支了: {e}"
            );
        }
    }

    /// 坏调用①b：合法写法不得被闸门误伤。
    #[tokio::test]
    async fn probe_1b_legit_forms_pass_the_gate() {
        let f = tmp("p1b", SRC);
        // 别名兜底（content）应被视为"填实了"。
        match ModifyTool
            .run(json!({"file": f, "change_spec": {"find": "let x = 1;", "content": "let x = 9;"}}))
            .await
        {
            Ok(_) => println!("\n[①b 别名 content] 真的改成功了（未被误拦）\n"),
            Err(e) => {
                println!("\n[①b 别名 content]\n{}\n", e.chars().take(300).collect::<String>());
                assert!(
                    !e.contains("MODIFY_PARAM_UNFILLED"),
                    "别名 content 被误判成没填实: {e}"
                );
            }
        }
        // find + insert_after（追加正道）里 replace 本就该缺 —— 不得拦
        let f2 = tmp("p1b2", SRC);
        match ModifyTool
            .run(json!({"file": f2, "change_spec": {"find": "fn beta() {", "insert_after": "    // 新增注释"}}))
            .await
        {
            Ok(_) => println!("\n[①b insert_after] 真的改成功了（未被误拦）\n"),
            Err(e2) => {
                println!("\n[①b insert_after]\n{}\n", e2.chars().take(300).collect::<String>());
                assert!(
                    !e2.contains("MODIFY_PARAM_UNFILLED"),
                    "insert_after 追加用法被误判成没填实: {e2}"
                );
            }
        }
    }

    /// 坏调用②：**路径不存在** —— 这才该引导去查文件系统。
    #[tokio::test]
    async fn probe_2_path_missing() {
        let e = ModifyTool
            .run(json!({
                "file": "D:/__no_such_dir_9f8d__/ghost.rs",
                "change_spec": {"find": "let x = 1;", "replace": "let x = 2;"}
            }))
            .await
            .unwrap_err();
        println!("\n[② 路径不存在]\n{}\n", e.chars().take(600).collect::<String>());
    }

    /// 坏调用③：**锚点问题** —— 零命中与多义是两种修法，必须分开报。
    #[tokio::test]
    async fn probe_3_anchor() {
        let f = tmp("p3", SRC);
        let zero = ModifyTool
            .run(json!({
                "file": f,
                "change_spec": {"find": "这段锚点文件里根本没有_9f8d", "replace": "x"}
            }))
            .await
            .unwrap_err();
        println!("\n[③ 零命中]\n{}\n", zero.chars().take(600).collect::<String>());

        let dup = ModifyTool
            .run(json!({
                "file": f,
                "change_spec": {"find": "let ", "replace": "let  "}
            }))
            .await
            .unwrap_err();
        println!("\n[③ 多义（let 出现两次）]\n{}\n", dup.chars().take(600).collect::<String>());
    }

    /// 归因必须按**码**走，不能被文本子串带跑偏。
    #[test]
    fn probe_4_attribution_follows_code_not_substring() {
        use crate::tools::contract::classify_failure;

        // ① 锚点零命中：**绝不能**归成「路径」
        let a = classify_failure("[FIND_NOT_FOUND] find 文本在文件中未找到：xxx");
        assert_eq!(a.category, "锚点未命中", "锚点零命中被误归因: {a:?}");
        assert!(!a.advice.contains("路径拼写"), "给的是路径建议: {}", a.advice);

        // ② 锚点多义
        let b = classify_failure("[FIND_AMBIGUOUS] find 在文件中出现 2 处，无法唯一定位");
        assert_eq!(b.category, "锚点多义", "{b:?}");

        // ③ 真·路径不存在：仍须归「路径」
        let c = classify_failure("[NOT_FOUND] 文件不存在: D:\\x\\y.rs");
        assert_eq!(c.category, "路径", "{c:?}");

        // ④ 经 modify 包装后仍由**内层码**说了算（包装码不得干扰）
        let d = classify_failure(
            "[MODIFY_ANCHOR_NOT_FOUND] [FIND_NOT_FOUND] find 文本在文件中未找到：zzz\n\n\
             【后端已自动读取目标文件】...",
        );
        assert_eq!(d.category, "锚点未命中", "包装后丢了归因: {d:?}");

        // ⑤ 参数未填实：不得被引去翻文件系统
        let e = classify_failure("[MODIFY_PARAM_UNFILLED] change_spec.replace 没有填实");
        assert_eq!(e.category, "参数未填实", "{e:?}");

        // ⑥ run：命令失败的文本里**必然夹杂命令输出**（完全不可信的内容）。
        let f = classify_failure(
            "[RUN_FAILED] 命令退出码 101（非零）：error[E0308]: mismatched types\n\
             note: no such file or directory / not_found / 不存在",
        );
        assert_eq!(f.category, "命令失败", "命令输出劫持了归因: {f:?}");
        assert!(!f.advice.contains("路径拼写"), "指错方向: {}", f.advice);
    }

    /// next_action 必须**按码分流**——三类错因的处置各不相同。
    #[test]
    fn probe_5_next_action_is_per_cause() {
        use crate::tools::contract::{code_next_action, code_retryable_with_change};

        // 模型自己就能修好的：重试，不惊动用户
        assert_eq!(code_next_action("MODIFY_PARAM_UNFILLED"), "retry");
        assert_eq!(code_next_action("MODIFY_ANCHOR_NOT_FOUND"), "retry");
        assert_eq!(code_next_action("MODIFY_ANCHOR_AMBIGUOUS"), "retry");
        // 路径不存在：模型无从凭空得知，与既有 NOT_FOUND 同处置
        assert_eq!(code_next_action("MODIFY_PATH_NOT_FOUND"), "ask_user");

        // 被压平的证据：老包装码对这些一无所知
        assert_eq!(code_next_action("TOOL_ERROR_WITH_PREVIEW"), "ask_user");

        assert!(code_retryable_with_change("MODIFY_ANCHOR_AMBIGUOUS"));
        assert!(code_retryable_with_change("MODIFY_PATH_NOT_FOUND"));
    }
}
