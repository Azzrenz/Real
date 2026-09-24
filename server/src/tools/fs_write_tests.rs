//! tools/fs_write.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod import_verify_tests {
    use super::*;

    /// 子模块式包回归：`from X import Y` 里 Y 是子模块时应判合法。
    #[tokio::test]
    async fn stdlib_submodule_import_not_false_positive() {
        let dir = std::env::temp_dir().join("real_import_check_reg");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("uses_stdlib_submodule.py");
        std::fs::write(&f, "# -*- coding: utf-8 -*
from xml import etree

def grab() -> None:
    return None
").unwrap();
        let r = check_python_imports(&f).await;
        assert!(!r.is_err(), "xml.etree 子模块导入不应判硬错: {:?}", r);
        let _ = std::fs::remove_file(&f);
    }

    /// 反向对照：模块存在但符号不存在仍应硬错回滚。
    #[tokio::test]
    async fn truly_missing_symbol_still_hard_error() {
        let dir = std::env::temp_dir().join("real_import_check_reg");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("bad_symbol.py");
        std::fs::write(&f, "# -*- coding: utf-8 -*
from os import NoSuchSymbol

def f() -> None:
    return None
").unwrap();
        let r = check_python_imports(&f).await;
        assert!(r.is_err(), "真缺符号应硬错: {:?}", r);
        assert!(r.unwrap_err().contains("NoSuchSymbol"), "错误应指名符号");
        let _ = std::fs::remove_file(&f);
    }
}

#[cfg(test)]
mod syntax_tests {
    use super::*;
    use serde_json::json;

    // broken edit（产生 IndentationError）必须被语法护栏拦截，并回滚到修改前
    #[tokio::test]
    async fn edit_producing_syntax_error_is_rejected_and_rolled_back() {
        let dir = std::env::temp_dir().join(format!("real_syn_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("broken.py");
        std::fs::write(&p, "def f():\n    x = 1\n    return x\n").unwrap();
        let original = std::fs::read_to_string(&p).unwrap();
        let t = EditTool;
        // old 缺省→按行定位；new="try:" 使函数体缩进非法 → IndentationError
        let args =
            json!({"file": p.display().to_string(), "replacements":[{"line":2,"new":"try:"}]});
        let err = t.run(args).await.unwrap_err();
        assert!(
            err.contains("SYNTAX_ERROR"),
            "broken edit 必须被语法护栏拦截: {err}"
        );
        let restored = std::fs::read_to_string(&p).unwrap();
        assert_eq!(restored, original, "必须回滚到修改前内容");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // （契约漏洞回归）：单行模式只传 line 缺 new → 必须返回 INVALID_PARAM
    #[tokio::test]
    async fn edit_single_line_missing_new_returns_invalid_param() {
        let dir = std::env::temp_dir().join(format!("real_edit_missing_new_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("x.py");
        std::fs::write(&p, "def f():\n    x = 1\n    return x\n").unwrap();
        let t = EditTool;
        // 只给 line、漏给 new（契约上 new 非必填，执行层必须容错报错而非崩溃）
        let args = json!({"file": p.display().to_string(), "replacements":[{"line":2}]});
        let err = t.run(args).await.unwrap_err();
        assert!(
            err.contains("INVALID_PARAM"),
            "缺 new 必须报 INVALID_PARAM 回灌模型: {err}"
        );
        // new 传非字符串（对象）同样必须报 INVALID_PARAM
        let args2 = json!({"file": p.display().to_string(), "replacements":[{"line":2, "new": {"bad": 1}}]});
        let err2 = t.run(args2).await.unwrap_err();
        assert!(
            err2.contains("INVALID_PARAM"),
            "new 非字符串必须报 INVALID_PARAM: {err2}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 合法编辑不应触发语法护栏
    #[tokio::test]
    async fn valid_edit_passes_syntax_check() {
        let dir = std::env::temp_dir().join(format!("real_syn_ok_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("ok.py");
        std::fs::write(&p, "def f():\n    x = 1\n    return x\n").unwrap();
        let t = EditTool;
        let args =
            json!({"file": p.display().to_string(), "replacements":[{"line":2,"new":"    y = 2"}]});
        let out = t.run(args).await.unwrap();
        assert!(out.contains("edit_result"), "合法编辑应通过: {out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // （后端兜底·写后语法检查）：json 用 Rust 内解析
    #[test]
    fn syntax_check_json_valid_and_invalid() {
        let p = std::path::Path::new("x.json");
        assert_eq!(
            syntax_check_result(p, "{\"a\":1}").as_deref(),
            Some("JSON_SYNTAX_OK")
        );
        let r = syntax_check_result(p, "{\"a\":1,}").unwrap();
        assert!(r.starts_with("JSON_SYNTAX_ERROR"), "坏 json 必须报错: {r}");
    }

    // js 走 node --check（node 缺失时跳过返回 None，不阻塞写入）
    #[test]
    fn syntax_check_js_via_node() {
        let p = std::path::Path::new("x.js");
        match syntax_check_result(p, "const a = 1; console.log(a);") {
            Some(r) => assert!(
                r.starts_with("JS_SYNTAX_"),
                "js 结果必须带 JS_SYNTAX_ 前缀: {r}"
            ),
            None => eprintln!("node 不在 PATH，跳过（优雅降级）"),
        }
    }

    // html 提取内嵌 script 检查；无 script / node 缺失 → None 跳过
    #[test]
    fn syntax_check_html_extracts_inline_script() {
        let p = std::path::Path::new("x.html");
        match syntax_check_result(
            p,
            "<!DOCTYPE html><html><body><script>const a=1;</script></body></html>",
        ) {
            Some(r) => assert!(
                r.starts_with("HTML_JS_SYNTAX_"),
                "html 结果必须带 HTML_JS_SYNTAX_ 前缀: {r}"
            ),
            None => eprintln!("node 不在 PATH，跳过（优雅降级）"),
        }
        // 非代码类型 → None
        assert_eq!(
            syntax_check_result(std::path::Path::new("x.md"), "# hello"),
            None
        );
        assert_eq!(
            syntax_check_result(std::path::Path::new("x.txt"), "plain"),
            None
        );
    }

    // 增量2：语法合法但符号不存在的坏 edit（如 `from poetry.json import ValidationError`）
    #[tokio::test]
    async fn edit_producing_invalid_import_is_rejected_and_rolled_back() {
        let dir = std::env::temp_dir().join(format!("real_imp_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("mod.py");
        // 基线：仅含合法 import
        std::fs::write(&p, "import os\n\nx = os.getpid()\n").unwrap();
        let original = std::fs::read_to_string(&p).unwrap();
        let t = EditTool;
        // 单行 line=1，new 含一个不存在的符号导入 → 导入校验应失败
        let args = json!({"file": p.display().to_string(),
            "replacements":[{"line":1,"new":"import os\nfrom os import ThisSymbolSurelyDoesNotExistXYZ"}]});
        let err = t.run(args).await.unwrap_err();
        assert!(
            err.contains("IMPORT_ERROR"),
            "坏 import 必须被导入护栏拦截: {err}"
        );
        assert!(
            err.contains("ThisSymbolSurelyDoesNotExistXYZ"),
            "错误应点名缺失符号: {err}"
        );
        let restored = std::fs::read_to_string(&p).unwrap();
        assert_eq!(restored, original, "坏 import 必须回滚到修改前内容");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 合法 import 的编辑不应被导入护栏误伤
    #[tokio::test]
    async fn valid_import_edit_passes_import_check() {
        let dir = std::env::temp_dir().join(format!("real_imp_ok_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("mod.py");
        std::fs::write(&p, "import os\n\nx = os.getpid()\n").unwrap();
        let t = EditTool;
        let args = json!({"file": p.display().to_string(),
            "replacements":[{"line":1,"new":"import os\nfrom os import path"}]});
        let out = t.run(args).await.unwrap();
        assert!(out.contains("edit_result"), "合法 import 编辑应通过: {out}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 工具层同步护栏：系统敏感区写入必须被直接拒绝（fail-fast，不依赖交互确认）
    #[tokio::test]
    async fn write_to_system_dir_requires_confirm() {
        let t = WriteTool;
        let args = json!({"path": "C:\\Windows\\System32\\drivers\\etc\\hosts", "content": "x=1"});
        let err = t.run(args).await.unwrap_err();
        assert!(
            err.contains("WRITE_REQUIRES_CONFIRM"),
            "系统区写入必须被护栏拒绝: {err}"
        );
    }

    #[tokio::test]
    async fn edit_to_system_dir_requires_confirm() {
        let t = EditTool;
        let args = json!({"file": "C:\\Windows\\System32\\drivers\\etc\\hosts", "replacements": [{"line": 1, "new": "x=1"}]});
        let err = t.run(args).await.unwrap_err();
        assert!(
            err.contains("EDIT_REQUIRES_CONFIRM"),
            "系统区编辑必须被护栏拒绝: {err}"
        );
    }

    #[tokio::test]
    async fn edit_returns_changed_field() {
        // （机制 bug 回归）：modify 依赖 fs_write 的 changed 字段判定
        let dir = std::env::temp_dir().join(format!("edit_changed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.txt");
        std::fs::write(&p, "let a = 1;\nlet b = 2;\n").unwrap();
        let t = EditTool;
        let args = json!({"file": p.display().to_string(),
            "replacements":[{"find":"let a = 1;","replace":"let a = 10;"}]});
        let out = t.run(args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["data"]["changed"], 1, "data.changed 应为 1: {out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_relative_path_rejected() {
        let t = WriteTool;
        let args = json!({"path": "README.md", "content": "x"});
        let err = t.run(args).await.unwrap_err();
        assert!(err.contains("RELATIVE_PATH"), "相对路径必须被拒绝: {err}");
    }

    #[test]
    fn ambiguous_hint_lists_match_lines_and_window() {
        // find 出现多处 → 列出匹配行号 + 第一个匹配附近真实内容（不是文件开头）
        let mut content = String::new();
        for i in 1..=25 {
            if i % 3 == 0 {
                content.push_str(&format!("fn f{i}() {{}}\n"));
            } else {
                content.push_str(&format!("let v{i} = {i};\n"));
            }
        }
        // 让 fn e() 出现在末尾（L25 附近），远离第一个匹配（L3）
        content.push_str("fn e() {}\n");
        let hint = find_ambiguous_hint(&content, "fn f", 8);
        assert!(hint.contains("8 处"), "应说明匹配数: {hint}");
        assert!(hint.contains("3"), "应列出匹配行号: {hint}");
        assert!(
            hint.contains("第一个匹配位置附近的真实内容"),
            "应回灌匹配附近: {hint}"
        );
        assert!(hint.contains("fn f3"), "第一个匹配附近应含真实代码: {hint}");
        // 不应只给文件开头全部内容（L25 的 fn e 不在第一个匹配窗口内）
        assert!(!hint.contains("fn e()"), "不应只给文件开头全部内容: {hint}");
    }

    #[test]
    fn ambiguous_hint_single_char_find() {
        // 极端：find 是单个字符（如 "}"），匹配极多——仍能列出位置并给窗口
        let content = "{\n}\nfn f() {\n    let x = 1;\n}\n";
        let hint = find_ambiguous_hint(content, "}", 2);
        assert!(hint.contains("2 处"), "应说明匹配数: {hint}");
        assert!(hint.contains("真实内容"), "应回灌内容: {hint}");
    }

    // （self_heal 放开自改）：携带有效一次性 token 时，对自身目录的写入应被放行
    #[tokio::test]
    async fn write_to_real_self_dir_with_valid_token_is_allowed() {
        // 复刻 PathPolicy::detect_root：从测试 exe 向上找 Cargo.toml，确定 Real 自身根
        let mut p = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|x| x.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("D:\\proj"));
        let real_root = loop {
            if p.join("Cargo.toml").exists() {
                break p;
            }
            match p.parent() {
                Some(parent) => p = parent.to_path_buf(),
                None => break std::path::PathBuf::from("D:\\proj"),
            }
        };
        let probe = real_root.join(format!("__self_heal_selftest_{}.tmp", std::process::id()));
        let token = issue_self_edit_token();
        let t = WriteTool;
        let args = json!({
            "path": probe.display().to_string(),
            "content": "x = 1\n",
            "__real_self_edit_token": token
        });
        let out = t.run(args).await.unwrap();
        assert!(
            out.contains("write_result"),
            "携带有效 token 对自身目录写入应成功: {out}"
        );
        // 清理探针文件
        let _ = std::fs::remove_file(&probe);
    }
}

    #[test]
    fn fuzzy_fills_missing_line_prefix() {
        // 真实文件三行：首行行首带 xy 前缀，其余两行与模型 find 完全相同
        let content = "xy  A\nB\nC\n";
        // 模型 find 漏了行首 xy（只写核心，没带前缀）
        let find = "A\nB\nC\n";
        let replace = "X\nB\nC\n";
        let (fv, _rv, why) = find_fuzzy_variants(content, find, replace).expect("应能纠偏成功");
        assert!(content.matches(&fv).count() == 1, "纠偏后应唯一匹配: {fv}");
        assert!(fv.starts_with("xy"), "应补上真实行首前缀: {fv}");
        assert!(!why.is_empty());
    }

    #[test]
    fn fuzzy_straightens_curly_quotes() {
        let content = "return \"hello\";\n";
        // 模型把直引号写成了弯引号
        let find = "return \u{201c}hello\u{201d};";
        let replace = "return \u{201c}你好\u{201d};";
        let (fv, rv, _why) = find_fuzzy_variants(content, find, replace).expect("弯引号应被转直");
        assert_eq!(fv, "return \"hello\";");
        assert_eq!(rv, "return \"你好\";");
    }
