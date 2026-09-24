//! tools/fs_read.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod edit_tool_tests {
    use super::*;
    use crate::tools::fs_write::EditTool;

    fn tmp_file(tag: &str, content: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("real_edit_{}_{}", tag, std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("main.rs");
        std::fs::write(&p, content).unwrap();
        // edit 前置守卫要求"先读后改"：测试夹具等价于已读
        p
    }

    #[tokio::test]
    async fn list_returns_lines_null_for_unreadable_file() {
        let d = std::env::temp_dir().join(format!("real_list_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let f = d.join("ok.txt");
        std::fs::write(&f, "line1\nline2\n").unwrap();
        // 目录路径指向一个不可作为 UTF-8 文本读取的文件（二进制）
        let bin = d.join("bin.dat");
        std::fs::write(&bin, [0u8, 159, 146, 150, 255, 0, 1, 2, 3, 4, 5, 6]).unwrap();
        let tool = ListTool;
        let args = json!({"path": d.to_string_lossy(), "recursive": false, "detail": true});
        let res = tool.run(args).await.unwrap();
        let v: Value = serde_json::from_str(&res).unwrap();
        let entries = v["data"]["entries"].as_array().unwrap();
        let ok_entry = entries
            .iter()
            .find(|e| e["name"] == "ok.txt")
            .expect("应列出 ok.txt");
        assert_eq!(ok_entry["lines"], 2, "文本文件行数应为 2");
        let bin_entry = entries
            .iter()
            .find(|e| e["name"] == "bin.dat")
            .expect("应列出 bin.dat");
        assert!(
            bin_entry["lines"].is_null(),
            "二进制文件读失败 lines 应为 null，实际: {}",
            bin_entry["lines"]
        );
    }

    #[tokio::test]
    async fn empty_replacement_rejected() {
        let p = tmp_file("empty_repl", "fn main() {\n    let x = 1;\n}\n");
        let tool = EditTool;
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 2, "old": "    let x = 1;", "new": "    let x = 1;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_err(), "old==new 空替换必须报错");
        let err = res.unwrap_err();
        assert!(
            err.contains("EMPTY_REPLACEMENT") || err.contains("空替换"),
            "错误应含 EMPTY_REPLACEMENT: {err}"
        );
        // 文件未被改动
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("let x = 1;"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn real_replacement_ok() {
        let p = tmp_file("real_repl", "fn main() {\n    let x = 1;\n}\n");
        let tool = EditTool;
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 2, "old": "    let x = 1;", "new": "    let x = 2;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_ok(), "真实替换应成功: {:?}", res.err());
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("let x = 2;"), "文件必须被真实修改");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn old_optional_backend_reads_line() {
        // 模型零记忆负担，精确性由后端保证（消灭 old 复制失配）。
        let p = tmp_file(
            "old_opt",
            "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
        );
        let tool = EditTool;
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 2, "new": "    let x = 42;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_ok(), "old 缺省应成功（后端取原文）: {:?}", res.err());
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("let x = 42;"), "文件必须被修改: {after}");
        assert!(after.contains("let y = 2;"), "其它行不受影响: {after}");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn old_optional_empty_new_rejected() {
        // old 缺省 + new 与该行原文相同 → EMPTY_REPLACEMENT（防止空替换误判成功）
        let p = tmp_file("old_opt_empty", "fn main() {\n    let x = 1;\n}\n");
        let tool = EditTool;
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 2, "new": "    let x = 1;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_err(), "new 与原文相同必须报空替换");
        let err = res.unwrap_err();
        assert!(
            err.contains("EMPTY_REPLACEMENT"),
            "错误应含 EMPTY_REPLACEMENT: {err}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn missing_replacements_guides() {
        let p = tmp_file("no_repl", "fn main() {\n}\n");
        let tool = EditTool;
        let args = json!({"file": p.to_string_lossy()});
        let res = tool.run(args).await;
        assert!(res.is_err(), "无 replacements 必须引导");
        let err = res.unwrap_err();
        assert!(
            err.contains("REPLACEMENTS_REQUIRED"),
            "错误应含 REPLACEMENTS_REQUIRED: {err}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn line_offset_corrected_by_unique_old() {
        let content = "// 注释行\n// BUG: 空文件时 panic\n    let x = 1;\n";
        let p = tmp_file("line_corr", content);
        let tool = EditTool;
        // 给 line=2 但 old 是第 3 行的内容 → 应校正到第 3 行并成功
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 2, "old": "    let x = 1;", "new": "    let x = 2;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_ok(), "唯一匹配应自动校正行号: {:?}", res.err());
        let out = serde_json::from_str::<serde_json::Value>(&res.unwrap()).unwrap();
        let changes = out["data"]["changes"].as_array().unwrap();
        assert_eq!(
            changes[0]["line"], 3,
            "行号应被校正为真实行 3，实际: {changes:?}"
        );
        assert_eq!(changes[0]["corrected"], true, "应标记 corrected");
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("let x = 2;"), "第 3 行必须被真实修改");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn scan_issues_detects_common_code_smells() {
        let content = "\
pub(crate) fn main() {
    // TODO: 实现错误处理
    let v = maybe().unwrap();
    panic!(\"boom\");
    console.log(\"debug\");
}
";
        let issues = scan_issues(content);
        let kinds: Vec<String> = issues
            .iter()
            .map(|i| i["kind"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(kinds.contains(&"TODO".into()), "应检出 TODO: {kinds:?}");
        assert!(
            kinds.contains(&"PANIC_RISK".into()),
            "应检出 unwrap: {kinds:?}"
        );
        assert!(kinds.contains(&"PANIC".into()), "应检出 panic!: {kinds:?}");
        assert!(
            kinds.contains(&"DEBUG_RESIDUE".into()),
            "应检出 console.log: {kinds:?}"
        );
        // 行号定位正确（TODO 在第 2 行）
        let todo = issues.iter().find(|i| i["kind"] == "TODO").unwrap();
        assert_eq!(todo["line"], 2, "TODO 应在第 2 行: {todo:?}");
    }

    #[test]
    fn scan_issues_clean_code_no_false_positive() {
        let content = "fn ok() -> i32 {\n    let x = 1;\n    x + 1\n}\n";
        assert!(
            scan_issues(content).is_empty(),
            "干净代码不应误报: {:?}",
            scan_issues(content)
        );
    }

    #[tokio::test]
    async fn ambiguous_old_reports_lines() {
        let content = "    let x = 1;\n    let y = 1;\n    let x = 1;\n";
        let p = tmp_file("ambig_old", content);
        let tool = EditTool;
        // line=2 是 y 行（不匹配 old），old 在第 1/3 行出现两次 → 应报错列出行号
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 2, "old": "    let x = 1;", "new": "    let x = 9;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_err(), "多处匹配必须报错");
        let err = res.unwrap_err();
        assert!(
            err.contains("OLD_TEXT_MISMATCH") && err.contains("1, 3"),
            "错误应列出行号: {err}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn absent_old_reports_real_line() {
        let content = "    let a = 1;\n";
        let p = tmp_file("absent_old", content);
        let tool = EditTool;
        let args = json!({
            "file": p.to_string_lossy(),
            "replacements": [{"line": 1, "old": "    let b = 2;", "new": "    let b = 3;"}]
        });
        let res = tool.run(args).await;
        assert!(res.is_err(), "old 不存在必须报错");
        let err = res.unwrap_err();
        assert!(
            err.contains("OLD_TEXT_MISMATCH") && err.contains("let a = 1"),
            "错误应含实际行内容: {err}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}

#[cfg(test)]
mod structure_tests {
    use crate::tools::fs_common::extract_structure;

    #[test]
    fn extracts_rust_fns_and_impls() {
        let src = "use std::fmt;\n\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nimpl fmt::Display for Add {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {\n        write!(f, \"add\")\n    }\n}\n";
        let s = extract_structure(src);
        let names: Vec<String> = s.iter().map(|(_, _, n)| n.clone()).collect();
        assert!(
            names.contains(&"add".to_string()),
            "应提取 fn add，实际 {names:?}"
        );
        assert!(
            s.iter().any(|(_, k, _)| k == "impl"),
            "应提取 impl，实际 {s:?}"
        );
    }

    #[test]
    fn extracts_python_defs_and_classes() {
        let src = "import os\n\nclass Calculator:\n    def add(self, a, b):\n        return a + b\n\ndef main():\n    print('hi')\n";
        let s = extract_structure(src);
        let names: Vec<String> = s.iter().map(|(_, _, n)| n.clone()).collect();
        assert!(
            names.contains(&"Calculator".to_string()),
            "应提取类，实际 {names:?}"
        );
        assert!(
            names.contains(&"main".to_string()),
            "应提取 def，实际 {names:?}"
        );
    }

    #[tokio::test]
    async fn full_mode_also_carries_structure() {
        // （函数/类+行号）——模型拿结果即知结构，不依赖 auto 模式。
        let dir = std::env::temp_dir().join(format!("read_struct_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.py");
        std::fs::write(&f, "import os\n\ndef alpha():\n    return 1\n\nclass Beta:\n    def m(self):\n        pass\n").unwrap();
        let r = super::read_one(&f, "full", 0, 0, false).await.unwrap();
        let s = r["structure"].as_array().cloned().unwrap_or_default();
        let names: Vec<&str> = s.iter().filter_map(|x| x["name"].as_str()).collect();
        assert!(
            names.contains(&"alpha"),
            "full 模式应含函数 alpha: {names:?}"
        );
        assert!(names.contains(&"Beta"), "full 模式应含类 Beta: {names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn skips_comments_and_empty() {
        let src = "// comment\n\n# python comment\nfn real() {}\n";
        let s = extract_structure(src);
        assert_eq!(s.len(), 1, "只应提取 real，实际 {s:?}");
        assert_eq!(s[0].2, "real");
    }

    #[test]
    fn caps_at_60_definitions() {
        // 截断是调用方职责（fs_read 摘要 take 60 / file_store 地图 take 120）。
        let mut src = String::new();
        for i in 0..80 {
            src.push_str(&format!("fn f{i}() {{}}\n"));
        }
        let s = extract_structure(&src);
        assert_eq!(s.len(), 80, "extract_structure 应返回全部（截断归调用方）");
        let capped = s.iter().take(60).count();
        assert_eq!(capped, 60, "调用方 take(60) 应生效");
    }

    fn write_tmp(tag: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("read_auto_{}_{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("m.py");
        std::fs::write(&f, content).unwrap();
        f
    }

    #[tokio::test]
    async fn auto_small_file_full_content() {
        // <100 行小文件 → 全文（不截断）
        let f = write_tmp("small", "a = 1\nb = 2\nc = 3\n");
        let r = super::read_one(&f, "auto", 0, 0, false).await.unwrap();
        let c = r["content"].as_str().unwrap_or("");
        assert!(c.contains("c = 3"), "小文件应全文返回: {c}");
        assert_eq!(r["truncated"], false, "小文件不应截断");
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[tokio::test]
    async fn auto_large_file_has_structure_and_segments() {
        let mut src = String::new();
        for i in 0..40 {
            src.push_str(&format!("def fn_{i}(x):\n    return x + {i}\n\n"));
        }
        for i in 0..600 {
            src.push_str(&format!("line_{i}\n"));
        }
        let f = write_tmp("large", &src);
        let r = super::read_one(&f, "auto", 0, 0, false).await.unwrap();
        let c = r["content"].as_str().unwrap_or("");
        // 分级策略：大文件（500-2000 行）→ 函数列表关键段 + 开头，不应只给头部截断
        assert!(
            c.contains("// def fn_0"),
            "大文件应含函数关键段: …{:?}",
            c.chars().take(200).collect::<String>()
        );
        let expl = r["explanation"].as_str().unwrap_or("");
        assert!(expl.contains("结构摘要"), "auto 应带结构摘要: {expl}");
        // 720 行文件走 500-2000 分级：关键段拼接可能接近全文（truncated 不必然 true），
        assert!(c.contains("// def fn_0"), "大文件应含函数关键段标记");
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }
}

#[cfg(test)]
mod output_contract_tests {
    use super::*;
    use crate::mcp::registry::BuiltinTool;
    use crate::tools::base::validate_output;

    #[tokio::test]
    async fn real_read_output_passes_its_own_output_contract() {
        // output_schema——防止"输出结构与声明漂移"（changed:0 一类"消费方读错字段"的根）。
        let dir = std::env::temp_dir().join(format!("read_oc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.rs");
        std::fs::write(
            &f,
            "fn main() {}
",
        )
        .unwrap();

        let tool = ReadTool;
        let out = tool
            .run(json!({"paths": [f.to_str().unwrap()], "mode": "full"}))
            .await
            .unwrap();
        let schema = tool.output_schema();
        let v = validate_output("read", &schema, &out).expect("真实 read 输出必须符合自身输出契约");
        assert_eq!(v["kind"], "read_result");
        assert_eq!(v["data"]["files"][0]["total_lines"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn real_read_output_rejects_contract_violation() {
        // 反向：声明了 object 契约的工具若返回非 JSON，必须被闸门拦截（不进模型）
        let err =
            validate_output("read", &ReadTool.output_schema(), "not json at all").unwrap_err();
        assert_eq!(err.code, "OUTPUT_INVALID");
    }

    #[test]
    fn read_schema_declares_fresh_parameter() {
        // 引导模型"加 fresh=true 就放行"，registry 缓存层也认 fresh——但 input_schema
        let schema = ReadTool.input_schema();
        let fresh = schema
            .pointer("/properties/fresh")
            .expect("read 契约必须声明 fresh 参数（教学出口可达性）");
        assert_eq!(fresh["type"], "boolean", "fresh 必须是 boolean: {fresh}");
        // description 双面声明：描述必须提及 fresh 用途（防单面声明）
        let desc = ReadTool.description();
        assert!(
            desc.contains("fresh=true"),
            "description 必须声明 fresh=true 出口: {desc}"
        );
    }
}

#[cfg(test)]
mod full_text_allowed_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_mode_allowed() {
        let t = ReadTool;
        assert!(t.full_text_allowed(&json!({"mode": "full", "paths": ["D:/x/a.rs"]})));
    }

    #[test]
    fn lines_with_complete_range_allowed() {
        // 就是要拿完整区间（478 行 ≈ 2.6 万字符），截断到 8017 后半段全丢。
        let t = ReadTool;
        assert!(t.full_text_allowed(
            &json!({"mode": "lines", "start_line": 1, "end_line": 478, "paths": ["D:/x/a.rs"]})
        ));
    }

    #[test]
    fn auto_mode_not_allowed() {
        // auto（预览）应截断——防上下文膨胀
        let t = ReadTool;
        assert!(!t.full_text_allowed(&json!({"mode": "auto", "paths": ["D:/x/a.rs"]})));
    }

    #[test]
    fn missing_mode_allowed() {
        //（小文件 full / 大文件 auto）。full 需全文直通（截断丢中段），auto 本身
        let t = ReadTool;
        assert!(t.full_text_allowed(&json!({"paths": ["D:/x/a.rs"]})));
    }
}

    fn search_tmp_file(tag: &str, content: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("real_search_{}_{}.txt", tag, std::process::id()));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[tokio::test]
    async fn search_single_file_returns_matches_with_line_numbers() {
        let p = search_tmp_file(
            "hit",
            "fn main() {\n    let project_version = \"1.0.0\";\n    println!(\"hello\");\n}\n",
        );
        let tool = SearchTool;
        let res = tool
            .run(json!({"pattern": "project_version", "path": p.to_string_lossy()}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&res).unwrap();
        assert_eq!(v["data"]["total"], 1, "单文件搜索应命中 1 行: {res}");
        assert_eq!(v["data"]["files_scanned"], 1, "files_scanned 应为 1");
        assert_eq!(v["data"]["matches"][0]["line"], 2, "行号必须正确（索引/精读闭环的锚）");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn search_miss_returns_zero_not_error() {
        let p = search_tmp_file("miss", "fn main() { println!(\"hello\"); }\n");
        let tool = SearchTool;
        let res = tool
            .run(json!({"pattern": "no_such_symbol_xyz", "path": p.to_string_lossy()}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&res).unwrap();
        assert_eq!(v["data"]["total"], 0, "未命中=0 而非错误");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn search_chinese_pattern_utf8_exact() {
        let p = search_tmp_file("cjk", "条目 1：水晶怀表的登记编号为WT-2049-C。\n其他内容。\n");
        let tool = SearchTool;
        let res = tool
            .run(json!({"pattern": "水晶怀表的登记编号", "path": p.to_string_lossy()}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&res).unwrap();
        assert_eq!(v["data"]["total"], 1, "UTF-8 中文检索必须精确命中: {res}");
        assert!(
            v["data"]["matches"][0]["text"]
                .as_str()
                .unwrap()
                .contains("WT-2049-C"),
            "命中行应含完整内容"
        );
        let _ = std::fs::remove_file(&p);
    }

#[cfg(test)]
mod batch_semantics_tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fs_read_batch_{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// **部分成功必须返回成功**（本轮修的核心行为）。
    #[tokio::test]
    async fn partial_success_returns_content_not_error() {
        let dir = scratch("partial");
        let good = dir.join("good.txt");
        std::fs::write(&good, "alpha\nbeta\n").unwrap();
        let missing = dir.join("nope.txt");

        let out = ReadTool
            .run(json!({
                "paths": [good.to_str().unwrap(), missing.to_str().unwrap()],
                "mode": "full"
            }))
            .await
            .expect("部分成功必须返回 Ok，而不是整批失败");
        let v: Value = serde_json::from_str(&out).unwrap();

        let files = v["data"]["files"].as_array().unwrap();
        assert_eq!(files.len(), 2, "两个路径都要有条目（成功项 + 失败占位）");

        let top = files
            .iter()
            .find(|f| f["path"] == json!(good.to_str().unwrap()))
            .expect("成功的文件必须在 files 里");
        assert!(
            top["content"].as_str().unwrap_or("").contains("alpha"),
            "成功的文件**必须给出内容**（这是本修复的全部意义）"
        );

        let bad = files
            .iter()
            .find(|f| f["error"].is_string())
            .expect("失败项必须带 error");
        assert_eq!(
            bad["path"],
            json!(missing.to_str().unwrap()),
            "失败项必须带**真实路径** —— 旧版写死 \"?\"，批量里模型不知道是哪个失败了"
        );

        let warnings = v["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.as_str().unwrap_or("").contains("已成功返回")),
            "warnings 必须告知模型'其余已成功返回'，否则它会以为整批无效：{warnings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 反向：**全失败**仍返回 Err（不静默、不假装成功）。
    #[tokio::test]
    async fn all_failed_still_returns_error() {
        let dir = scratch("allfail");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");

        let err = ReadTool
            .run(json!({
                "paths": [a.to_str().unwrap(), b.to_str().unwrap()],
                "mode": "full"
            }))
            .await
            .expect_err("全部失败必须返回 Err");
        assert!(
            err.contains("NOT_FOUND") || err.contains("不存在") || err.contains("失败"),
            "错误信息应可读: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod search_gate_tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fs_search_gate_{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// **二进制资产按扩展名跳过，且跳过必须留痕**。
    #[tokio::test]
    async fn binary_assets_skipped_and_reported() {
        let dir = scratch("binary");
        std::fs::write(dir.join("note.txt"), "needle here\n").unwrap();
        std::fs::write(dir.join("hit.rs"), "let needle = 1;\n").unwrap();
        // 真二进制：判据是**内容里的 NUL 字节**，不是扩展名
        std::fs::write(
            dir.join("Qwen3-8B-Q4_K_M.gguf"),
            b"needle\x00\x01GGUF payload\n".as_slice(),
        )
        .unwrap();
        std::fs::write(dir.join("lib.dll"), b"needle\x00MZ\x90\x00stub\n".as_slice()).unwrap();
        // ⚠️ **反向用例**：扩展名像二进制、内容却是纯文本 ⇒ **必须被搜到**。
        std::fs::write(dir.join("actually-text.dll"), "needle in plain text\n").unwrap();

        let out = SearchTool
            .run(json!({"pattern": "needle", "path": dir.to_string_lossy()}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(
            v["data"]["total"].as_u64().unwrap_or(0),
            3,
            "该命中 .txt / .rs / 纯文本的 .dll 共 3 个；两个真二进制不搜: {out}"
        );

        let bin = v["data"]["skipped_binary"]
            .as_object()
            .expect("必须给出 skipped_binary 明细");
        assert_eq!(bin.get("gguf").and_then(|x| x.as_u64()), Some(1), "gguf 应计入跳过");
        assert_eq!(bin.get("dll").and_then(|x| x.as_u64()), Some(1), "dll 应计入跳过");

        let warnings = v["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.as_str().unwrap_or("").contains("已跳过") && w.as_str().unwrap_or("").contains("二进制文件")),
            "warnings 必须说明跳过了二进制（否则模型把零命中读成不存在）: {warnings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 二进制**不因 file_pattern 豁免** —— 它是**内容属性**，与扩展名、与显式意图都无关。
    #[tokio::test]
    async fn binary_is_not_exempted_by_file_pattern() {
        let dir = scratch("override");
        std::fs::write(dir.join("weights.gguf"), b"needle\x00payload\n".as_slice()).unwrap();

        let out = SearchTool
            .run(json!({
                "pattern": "needle",
                "path": dir.to_string_lossy(),
                "file_pattern": "*.gguf"
            }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["data"]["total"].as_u64().unwrap_or(0),
            0,
            "二进制不因显式 file_pattern 而被搜索: {out}"
        );
        assert!(
            v["data"]["skipped_binary"]
                .as_object()
                .map(|o| !o.is_empty())
                .unwrap_or(false),
            "必须记入 skipped_binary: {out}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **体积闸兜底**：扩展名正常但体积过大的文件（`MAX_SEARCH_FILE_BYTES` = 4 MB），
    #[tokio::test]
    async fn oversize_file_skipped_before_read() {
        use std::io::Write;

        let dir = scratch("oversize");
        std::fs::write(dir.join("small.txt"), "needle\n").unwrap();
        // .bin 不在二进制名单（故意留作兜底覆盖）⇒ 只能靠体积闸拦
        let big = dir.join("dump.bin");
        let mut f = std::fs::File::create(&big).unwrap();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..5 {
            f.write_all(&chunk).unwrap();
        }
        drop(f);

        let out = SearchTool
            .run(json!({"pattern": "needle", "path": dir.to_string_lossy()}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["data"]["total"].as_u64().unwrap_or(0),
            1,
            "只该命中 small.txt: {out}"
        );
        assert!(
            v["data"]["skipped_oversize"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "超大文件必须记入 skipped_oversize: {out}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod binary_gate_tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("real_bingate_{}_{}", tag, std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[tokio::test]
    async fn read_image_returns_visual_attachment() {
        let d = scratch("png");
        let f = d.join("shot.png");
        // PNG 魔数 + NUL：与真图片同一"内容属性"
        std::fs::write(
            &f,
            [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D],
        )
        .unwrap();
        let args = json!({"paths": [f.to_string_lossy()]});

        let out = ReadTool
            .run(args.clone())
            .await
            .expect("图片必须被 read 接受（旧契约把它拒成乱码指引，模型只能干瞪眼）");
        assert!(!out.contains("GBK"), "不得走文本解码路径: {out}");

        let parts = ReadTool
            .content_parts(&args, &out)
            .expect("图片必须走非文本产出通道（image_ref）");
        assert_eq!(parts.len(), 2, "一句带路径的说明 + 一张图");
        assert_eq!(parts[0].type_, "text");
        let note = parts[0].text.as_deref().unwrap_or("");
        assert!(
            note.contains("shot.png"),
            "说明必须带原路径（base64 剥离后模型靠它回取）: {note}"
        );
        assert_eq!(parts[1].type_, "image_ref", "必须是 image_ref 附件");
        assert_eq!(parts[1].mime_type.as_deref(), Some("image/png"));
        let uri = parts[1].uri.as_deref().unwrap_or("");
        assert!(
            uri.starts_with("data:image/png;base64,"),
            "必须是内联 data_url（下游统一图片通道直接回灌）: {uri}"
        );

        let _ = std::fs::remove_dir_all(&d);
    }

    /// **反向用例**：含 NUL 但**不是图片**的二进制仍走拒绝闸（闸没被图片分支放水）。
    #[tokio::test]
    async fn read_binary_non_image_still_rejected() {
        let d = scratch("bin");
        let f = d.join("payload.bin");
        std::fs::write(&f, [0x00u8, 0x01, 0x02, 0x03, 0x00, 0xFF, 0xFE]).unwrap();

        let err = ReadTool
            .run(json!({"paths": [f.to_string_lossy()]}))
            .await
            .expect_err("非图片的二进制必须被 read 拒绝（原先会返回乱码 content）");
        assert!(err.contains("BINARY_FILE"), "错误应带 BINARY_FILE 码: {err}");
        assert!(!err.contains("GBK"), "不得再出现误导性的编码提示: {err}");

        let _ = std::fs::remove_dir_all(&d);
    }

    /// **反向用例**：纯文本（含中文/Tab，但无 NUL）绝不能被这道闸误伤。
    #[tokio::test]
    async fn text_without_nul_still_readable() {
        let d = scratch("txt");
        let f = d.join("note.md");
        std::fs::write(&f, "# 标题\n正文 with tab\tand 中文\n").unwrap();

        let out = ReadTool
            .run(json!({"paths": [f.to_string_lossy()]}))
            .await
            .expect("纯文本必须能读");
        assert!(out.contains("标题"), "内容应原样返回: {out}");

        let _ = std::fs::remove_dir_all(&d);
    }
}
