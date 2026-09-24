//! agent/execution/prepare_args.rs 的测试外置（部门盘查：核心文件测试全部移出，此文件单管）
use super::*;

#[cfg(test)]
mod sanitize_tests {
    use super::{prepare_tool_args, sanitize_path_args};
    use serde_json::json;

    #[test]
    fn path_backslash_normalized_to_slash() {
        // 路径字段反斜杠→正斜杠（模型 JSON 参数里 D:\proj\bench 的 \b \r
        let mut a = json!({"file": "D:\\proj\\bench\\x.rs", "cwd": "D:\\ws"});
        sanitize_path_args(&mut a);
        assert_eq!(a["file"], "D:/proj/bench/x.rs");
        assert_eq!(a["cwd"], "D:/ws");
    }

    #[test]
    fn command_slash_direction_left_to_execution_layer() {
        // 装配层**不决定斜杠方向**：无论模型写正斜杠还是反斜杠，都原样透传，
        use std::collections::HashMap;
        for raw in [
            "grep -rn 'fn main' D:/proj/server/src",
            "dir D:/proj/server/src",
        ] {
            let out = prepare_tool_args(
                "run",
                &json!({"command": raw}),
                &HashMap::new(),
                "s_slash_keep",
                &std::sync::Mutex::new(HashMap::new()),
            )
            .unwrap();
            let cmd = out["command"].as_str().unwrap();
            assert!(cmd.contains("D:/proj/server/src"), "装配层应保留原文: {cmd}");
        }
    }

    #[test]
    fn read_paths_array_normalized() {
        let mut a = json!({"paths": ["D:\\a\\b.rs", "D:\\c\\d.rs"]});
        sanitize_path_args(&mut a);
        assert_eq!(a["paths"][0], "D:/a/b.rs");
        assert_eq!(a["paths"][1], "D:/c/d.rs");
    }

    #[test]
    fn command_keeps_backslash_until_execution_layer() {
        // 装配层**不决定斜杠方向** —— 保留模型原文，方向交给执行层（cmd_tools::run 按壳归一）。
        let mut a = json!({"command": "python D:\\proj\\run.py -c \"print('a\\nb')\""});
        sanitize_path_args(&mut a);
        let cmd = a["command"].as_str().unwrap();
        assert!(
            cmd.starts_with("python D:\\proj\\run.py"),
            "command 斜杠应保留原文: {cmd}"
        );
        assert!(cmd.contains("\\n"), "命令内 \\n 转义应保留: {cmd}");
    }

    // write 的 content 不经过 #En 展开，
    #[test]
    fn write_content_skips_en_expansion_but_path_expands() {
        use std::collections::HashMap;
        let evidence: HashMap<String, String> = [("#E2".to_string(), "ENVELOPE_JSON".to_string())]
            .into_iter()
            .collect();
        // content 含 #e 颜色 + 真 #E2 引用形态 → 原样写盘
        let args = json!({
            "path": "D:/tmp/x.html",
            "content": "<style>body{background:#e0e0e0;color:#e1a2b3}</style>\n参考 #E2 的内容"
        });
        let out = prepare_tool_args(
            "write",
            &args,
            &evidence,
            "s1",
            &std::sync::Mutex::new(HashMap::new()),
        )
        .unwrap();
        assert_eq!(
            out["content"],
            "<style>body{background:#e0e0e0;color:#e1a2b3}</style>\n参考 #E2 的内容",
            "write.content 的 #En 文本必须原样保留（不展开）"
        );
        // path 字段残留 #En → find_unresolved_placeholder 拦截（不再展开）
        let args2 = json!({"path": "#E2", "content": "x"});
        let err = prepare_tool_args(
            "write",
            &args2,
            &evidence,
            "s1",
            &std::sync::Mutex::new(HashMap::new()),
        )
        .expect_err("path 残留 #E2 应被拦截，不再展开");
        assert!(err.contains("占位符"), "应提示占位符未解析: {err}");
    }

    #[test]
    fn write_empty_content_teaching_error() {
        // P1b：write content 为空 → 教学错误（而非"长度 ≥ 1"）
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        for bad in [
            json!({"path": "D:/x/a.py", "content": ""}),
            json!({"path": "D:/x/a.py"}),
        ] {
            let err = super::prepare_tool_args("write", &bad, &HashMap::new(), "s_test", &alloc)
                .expect_err("write content 缺失/为空必须报教学错误");
            assert!(
                err.contains("完整可写的最终文件内容"),
                "应报契约错误码 EMPTY_CONTENT，实际: {err}"
            );
        }
    }

    #[test]
    fn search_paths_plural_normalized_to_path() {
        // P1-1：search 收到 paths（复数）→ path 取首项（模型按 read 习惯混用不丢参数）
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let out = super::prepare_tool_args(
            "search",
            &json!({"paths": ["D:/proj/src"], "pattern": "fn main"}),
            &HashMap::new(),
            "s1",
            &alloc,
        )
        .unwrap();
        assert_eq!(out["path"], "D:/proj/src", "paths 复数应归一到 path: {out}");
        assert_eq!(out["pattern"], "fn main");
    }

    #[test]
    fn reread_full_same_file_served_directly() {
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let sid = "s_reread_test";
        // 先标记已读（给了全文）
        let out = super::prepare_tool_args(
            "read",
            &json!({"paths": ["D:/x/repeated.py"]}),
            &HashMap::new(),
            sid,
            &alloc,
        )
        .expect("重复 full 读应直接放行（服务而非拒绝）");
        assert!(out["paths"][0].as_str().unwrap().contains("repeated.py"), "路径应保留");
        // lines 分段不拦（合理细化）
        let ok = super::prepare_tool_args(
            "read",
            &json!({"paths": ["D:/x/repeated.py"], "mode": "lines", "start_line": 1, "end_line": 10}),
            &HashMap::new(),
            sid,
            &alloc,
        );
        assert!(ok.is_ok(), "lines 分段读应放行: {ok:?}");
        // fresh=true 显式声明 = 确需重读最新内容 → 豁免
        let ok_fresh = super::prepare_tool_args(
            "read",
            &json!({"paths": ["D:/x/repeated.py"], "fresh": true}),
            &HashMap::new(),
            sid,
            &alloc,
        );
        assert!(
            ok_fresh.is_ok(),
            "fresh=true 应放行（确需重读）: {ok_fresh:?}"
        );
    }

    #[test]
    fn partial_batch_read_then_full_reread_is_allowed() {
        // 批量读登记为 partial
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let sid = "s_partial_test";
        // 模拟批量读登记（partial：没给全文）
        let out = super::prepare_tool_args(
            "read",
            &json!({"paths": ["D:/x/file_tools.rs"], "mode": "full"}),
            &HashMap::new(),
            sid,
            &alloc,
        )
        .expect("partial（批量读/摘要）后补读全文应放行");
        assert!(
            out["paths"][0].as_str().unwrap().contains("file_tools"),
            "补读请求应放行: {out}"
        );
        // 补读登记后变 full——再次 full 读仍放行（拦截已退役，stub 只留最近一次）
        let out2 = super::prepare_tool_args(
            "read",
            &json!({"paths": ["D:/x/file_tools.rs"]}),
            &HashMap::new(),
            sid,
            &alloc,
        )
        .expect("再次 full 读仍应放行（stub 只留最近一次，成本有界）");
        assert!(out2["paths"][0].as_str().unwrap().contains("file_tools"), "路径应保留");
    }

    #[test]
    fn line_mode_edit_without_old_does_not_require_read() {
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let sid = "s_line_edit_test";
        // 未 read 过目标文件
        let ok = super::prepare_tool_args(
            "edit",
            &json!({"file": "D:/x/_verify.js",
                    "replacements": [{"line": 12, "new": "const x = 1;"}]}),
            &HashMap::new(),
            sid,
            &alloc,
        )
        .expect("纯 line+new（old 缺省）未 read 应放行——后端自动取第 12 行原文，不存在拼错 old 风险");
        assert_eq!(ok["file"], "D:/x/_verify.js", "应放行: {ok}");
    }

    #[test]
    fn modify_placeholder_path_gives_teaching_error() {
        // modify 在占位检查列表（path 字段）——
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let err = super::prepare_tool_args(
            "modify",
            &json!({"path": "#E6 定位到的文件", "change_spec": {"description": "改白名单话术"}}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .expect_err("modify 占位路径必须被教学拦截");
        assert!(
            err.contains("占位符描述") || err.contains("占位符"),
            "应教学占位符，实际: {err}"
        );
        assert!(!err.contains("创建新文件"), "不得误报创建新文件: {err}");
    }

    #[test]
    fn write_missing_path_teaches_absolute_path() {
        // 模型写临时脚本只给 content 漏 path →
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let err = super::prepare_tool_args(
            "write",
            &json!({"content": "import os\nprint('hi')\n"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .expect_err("write 缺 path 必须被教学拦截");
        assert!(err.contains("write.path 缺失"), "应教学 path 缺失: {err}");
        assert!(err.contains("绝对路径"), "应明确绝对路径: {err}");
        // 带 path 的正常通过
        let ok = super::prepare_tool_args(
            "write",
            &json!({"path": "D:/x/tmp.py", "content": "print(1)\n"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(ok["path"], "D:/x/tmp.py");
    }

    #[test]
    fn modify_embedded_ref_resolved_from_evidence() {
        // modify 的 path="#E6 定位到的文件" 不从 evidence 提取路径；残留 #En 被统一拦截（引导绝对路径）。
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let mut evidence = HashMap::new();
        evidence.insert("#E6".into(), json!({
            "data": {"matches": [{"file": "D:/SampleProject/server/src/tools/cmd.rs", "line": 1, "text": "x"}]}
        }).to_string());
        let err = super::prepare_tool_args(
            "modify",
            &json!({"path": "#E6 定位到的文件", "change_spec": {"description": "改"}}),
            &evidence,
            "s_test",
            &alloc,
        )
        .expect_err("残留 #E6 应被拦截（不再从 evidence 展开）");
        assert!(
            err.contains("占位符"),
            "应提示占位符未解析: {err}"
        );
    }

    #[test]
    fn modify_file_path_normalized_to_file() {
        // 别名归一（prepare_args 别名表）：modify 传 file_path 自动归一为 file，不报 MISSING_PARAM
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let args =
            json!({"file_path": "D:/x/main.rs", "change_spec": {"find": "a", "replace": "b"}});
        let ok = super::prepare_tool_args("modify", &args, &HashMap::new(), "s_test", &alloc)
            .expect("modify 契约齐备（file 归一 + change_spec 定位）应通过预处理");
        assert_eq!(
            ok["file"], "D:/x/main.rs",
            "file_path 应被归一为 file，实际: {ok}"
        );
    }

    #[test]
    fn read_paths_placeholder_fallback_to_evidence() {
        // production evidence 无内容（executor 传空表），
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let mut evidence = HashMap::new();
        evidence.insert(
            "#E1".into(),
            json!({"data": {"files": [{"path": "D:/x/real.py"}, {"path": "D:/x/other.py"}]}})
                .to_string(),
        );
        let args = json!({"paths": ["#E1"], "mode": "auto"});
        let err = super::prepare_tool_args("read", &args, &evidence, "s_test", &alloc)
            .expect_err("残留 #E1 应被拦截，不再解析");
        assert!(
            err.contains("占位符"),
            "应提示占位符未解析，实际: {err}"
        );
        assert!(
            err.contains("字面值"),
            "应引导写真实路径（字面值），实际: {err}"
        );
        assert!(
            err.contains("D:/proj/server/src/tools/modify.rs"),
            "必须给出可照抄的正例（教对，而不是只说错），实际: {err}"
        );
    }
    #[test]
    fn placeholder_desc_detected() {
        // 归一化层：占位符描述文本必须被识别
        assert!(super::looks_like_placeholder("待定位的解压源码文件"));
        assert!(super::looks_like_placeholder(
            "python tests/verifier.py 或等价验证命令"
        ));
        assert!(super::looks_like_placeholder("待修改的目标文件"));
        // 真实路径不应误判
        assert!(!super::looks_like_placeholder(
            "D:/proj/bench/runs/x/workspace/archive_like/extract.py"
        ));
        assert!(!super::looks_like_placeholder("extract.py"));
        assert!(!super::looks_like_placeholder(
            "python -m pytest tests/verifier.py -v"
        ));
    }

    #[test]
    fn run_args_array_merged_into_command() {
        // 归一化：run 的 args 数组自动拼进 command，不再报错
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let args = json!({"command": "python", "args": ["D:/x/tests/verifier.py", "-v"]});
        let resolved = super::prepare_tool_args("run", &args, &HashMap::new(), "s_test", &alloc)
            .expect("args 数组应自动归一化，不报错");
        assert_eq!(
            resolved["command"], "python \"D:/x/tests/verifier.py\" -v",
            "路径 token 应自动加引号（保留原文斜杠；方向由执行层按壳归一）: {resolved}"
        );
        assert!(
            resolved.get("args").is_none(),
            "args 字段应被移除: {resolved}"
        );
    }

    #[test]
    fn placeholder_path_gives_teaching_error() {
        // 归一化：read 路径是占位符描述 → 教学错误（非"NOT_FOUND"）
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let args = json!({"paths": ["待定位的解压源码文件"]});
        let err = super::prepare_tool_args("read", &args, &HashMap::new(), "s_test", &alloc)
            .expect_err("占位符路径必须报教学错误");
        assert!(err.contains("占位符描述"), "应教学'占位符描述': {err}");
    }

    #[test]
    fn control_chars_in_path_stripped_via_clean() {
        // 控制字符（退格/回车）若混入路径字段，清洗后不应保留
        let mut a = json!({"file": "D:\\proj\u{0008}ench\u{000d}uns\\x.rs"});
        sanitize_path_args(&mut a);
        let f = a["file"].as_str().unwrap();
        assert!(
            !f.contains('\u{0008}') && !f.contains('\u{000d}'),
            "控制字符应被处理: {f:?}"
        );
        assert!(f.starts_with("D:/proj"), "路径前缀应保留: {f}");
    }

    #[test]
    fn modify_empty_change_spec_teaching_error() {
        // 全空参数在 prepare 层
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        for bad in [
            json!({"file": "D:/x/a.py", "change_spec": {}}),
            json!({"file": "D:/x/a.py", "change_spec": {"find": "", "replace": ""}}),
            json!({"path": "D:/x/a.py", "find": "", "change": ""}),
        ] {
            let err = super::prepare_tool_args("modify", &bad, &HashMap::new(), "s_test", &alloc)
                .expect_err("modify 全空参数必须报教学错误");
            assert!(
                err.contains("缺少修改内容"),
                "应教学'缺少修改内容'，实际: {err}"
            );
        }
    }

    #[test]
    fn modify_with_find_or_change_passes() {
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        // 有 find/replace 正常通过
        let ok = super::prepare_tool_args(
            "modify",
            &json!({"file": "D:/x/a.py", "change_spec": {"find": "old", "replace": "new"}}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .expect("find/replace 应通过");
        assert_eq!(ok["change_spec"]["find"], "old");
        // 顶层 change 描述正常通过
        let ok2 = super::prepare_tool_args(
            "modify",
            &json!({"path": "D:/x/a.py", "change": "把 target_path 改为解码后校验"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .expect("change 描述应通过");
        assert!(ok2["change"].as_str().unwrap().contains("target_path"));
    }

    #[test]
    fn read_mode_alias_normalized() {
        // 原子 read 直调 mode 非枚举
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        for bad in ["head", "preview", "summary"] {
            let args = json!({"paths": ["D:/x/a.py"], "mode": bad});
            let resolved =
                super::prepare_tool_args("read", &args, &HashMap::new(), "s_test", &alloc)
                    .expect("read 应通过");
            assert_eq!(resolved["mode"], "auto", "{bad} 应归一为 auto: {resolved}");
        }
        // segment + 行区间 → lines（addca5ec 现场回归：mode=segment 撞枚举 3 次）
        let seg = super::prepare_tool_args(
            "read",
            &json!({
                "paths": ["D:/x/a.py"], "mode": "segment", "start_line": 390, "end_line": 430
            }),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(seg["mode"], "lines", "segment+行区间应归一 lines: {seg}");
        assert_eq!(seg["start_line"], 390, "行区间应保留: {seg}");
        // 合法 mode 不受影响
        let ok = super::prepare_tool_args(
            "read",
            &json!({"paths": ["D:/x/a.py"], "mode": "lines"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(ok["mode"], "lines");
    }

    #[test]
    fn enum_fields_normalized() {
        // audit.scope / search.output_mode 未知值确定性归一
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        // audit.scope：未知值 → 语义推导（path 文件→file）
        let d = std::env::temp_dir();
        let f = d.join("scope_probe.rs");
        std::fs::write(&f, "x").unwrap();
        let r = super::prepare_tool_args(
            "audit",
            &json!({"path": f.display().to_string(), "scope": "module"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(
            r["scope"], "file",
            "scope=module+文件 path 应推导为 file: {r}"
        );
        // audit.scope 缺 path → project
        let r2 = super::prepare_tool_args(
            "audit",
            &json!({"scope": "all"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(
            r2["scope"], "project",
            "scope=all 缺 path 应推导 project: {r2}"
        );
        // audit.scope 合法值不动
        let r3 = super::prepare_tool_args(
            "audit",
            &json!({"path": "D:/x", "scope": "dir"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(r3["scope"], "dir");
        // scope=file（合法值）+
        let dir = std::env::temp_dir().join(format!("audit_comb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r6 = super::prepare_tool_args(
            "audit",
            &json!({"path": dir.display().to_string(), "scope": "file", "focus": ["cmd.rs"]}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(
            r6["scope"], "dir",
            "scope=file+目录 path 应升级 dir（focus 才能生效）: {r6}"
        );
        // scope=dir/project + **文件** path → 降级 file
        let rf = dir.join("x.rs");
        std::fs::write(&rf, "fn x() {}\n").unwrap();
        let r7 = super::prepare_tool_args(
            "audit",
            &json!({"path": rf.display().to_string(), "scope": "dir"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(r7["scope"], "file", "scope=dir+文件 path 应降级 file: {r7}");
        let _ = std::fs::remove_dir_all(&dir);
        // search.output_mode 未知 → content
        let r4 = super::prepare_tool_args(
            "search",
            &json!({"pattern": "x", "output_mode": "lines"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(
            r4["output_mode"], "content",
            "output_mode=lines 应归一 content: {r4}"
        );
        // search 合法值不动
        let r5 = super::prepare_tool_args(
            "search",
            &json!({"pattern": "x", "output_mode": "count"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(r5["output_mode"], "count");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn audit_missing_path_keeps_ok_without_workspace() {
        // 缺 path 时 prepare 层
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let ok = super::prepare_tool_args(
            "audit",
            &json!({"scope": "project"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .expect("audit 缺 path 在无 workspace 时应原样返回（registry 兜底）");
        assert_eq!(ok["scope"], "project");
        assert!(ok.get("path").is_none(), "无 workspace 不应注入 path");
        // 有 path 时不受影响
        let ok2 = super::prepare_tool_args(
            "audit",
            &json!({"scope": "dir", "path": "D:/x"}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .expect("有 path 应通过");
        assert_eq!(ok2["path"], "D:/x");
    }

    #[test]
    fn search_context_bool_normalized_to_int() {
        // 模型传 context:true（布尔）被 INVALID_PARAM 拦。
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let r = super::prepare_tool_args(
            "search",
            &json!({"path": "D:/x", "pattern": "foo", "context": true}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(r["context"], 3, "context:true 应归一为 3: {r}");
        let r2 = super::prepare_tool_args(
            "search",
            &json!({"path": "D:/x", "pattern": "foo", "context": false}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(r2["context"], 0, "context:false 应归一为 0: {r2}");
        // 整数原样保留
        let r3 = super::prepare_tool_args(
            "search",
            &json!({"path": "D:/x", "pattern": "foo", "context": 5}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        assert_eq!(r3["context"], 5, "整数 context 不应改动: {r3}");
    }

    #[test]
    fn modify_missing_file_teaches_clear_path() {
        // modify 缺 file → 给明确教学而非裸 MISSING_PARAM，
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let err = super::prepare_tool_args(
            "modify",
            &json!({"change_spec": {"find": "old", "replace": "new"}}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap_err();
        assert!(
            err.contains("modify.file 缺失"),
            "应给 file 教学而非裸 MISSING_PARAM: {err}"
        );
        assert!(err.contains("绝对路径"), "应提到绝对路径: {err}");
    }

    #[test]
    fn read_singular_path_wrapped_into_paths_array() {
        // read 契约是 paths 数组，模型写 path 单数 → 包装成数组，
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let sid = "s_read_wrap_test";
        let r = super::prepare_tool_args(
            "read",
            &json!({"path": "D:/x/a.py", "numbered": true}),
            &HashMap::new(),
            sid,
            &alloc,
        )
        .unwrap();
        let paths = r["paths"].as_array().expect("paths 应为数组");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], "D:/x/a.py");
    }

    #[test]
    fn modify_path_alias_normalized_to_file() {
        // modify 的 path/file_path/target 别名都归一到 file——
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let d = std::env::temp_dir().join(format!("alias_probe_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.py");
        std::fs::write(&f, "old").unwrap();
        let r = super::prepare_tool_args("modify",
            &json!({"path": f.display().to_string(), "change_spec": {"find": "old", "replace": "new"}}),
            &HashMap::new(), "s_test", &alloc).unwrap();
        let got = r["file"].as_str().unwrap();
        let want = f.to_string_lossy().replace('\\', "/");
        assert_eq!(got.replace('\\', "/"), want, "path 应归一为 file: {r}");
    }

    #[test]
    fn command_multiline_preserved() {
        // python -c 多行命令的真实换行曾被剥成一行
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let multi = "python -c \"import sys\nsys.path.insert(0,'x')\nprint(1)\"";
        let r = super::prepare_tool_args(
            "run",
            &json!({"command": multi}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        let cmd = r["command"].as_str().unwrap();
        assert!(cmd.contains('\n'), "命令真实换行应保留: {:?}", cmd);
        assert!(cmd.contains("sys.path"), "多行 python 应完整: {:?}", cmd);
    }

    #[test]
    fn command_trailing_paren_not_stripped() {
        // 命令尾部 `)`/`'` 曾被"剥尾部噪音"误删
        use std::collections::HashMap;
        let alloc = std::sync::Mutex::new(HashMap::new());
        let cmd = r#"python -c "import sys; print(Serializer('x'))""#;
        let r = super::prepare_tool_args(
            "run",
            &json!({"command": cmd}),
            &HashMap::new(),
            "s_test",
            &alloc,
        )
        .unwrap();
        let c = r["command"].as_str().unwrap();
        assert!(
            c.ends_with("))\"") || c.contains("Serializer('x')"),
            "命令尾部括号应保留: {c}"
        );
        assert!(c.contains("'x'"), "内部引号应保留: {c}");
    }
}
