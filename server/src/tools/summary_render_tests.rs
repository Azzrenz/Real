//! tools/summary_render.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_shows_all_lines_when_small() {
        // full 精读：小文件全文直接给（含行号前缀），不折叠
        let raw = r#"{"data":{"files":[{"path":"/ws/src/lib.rs","total_lines":10,"mode":"full","content":"1\tfn a()\n2\tb\n3\tc\n4\td\n5\te\n6\tf\n7\tg\n8\th\n9\ti\n10\tj"}]}}"#;
        let s = clean("read", raw);
        assert!(s.contains("· 10 行"), "应含文件清单: {s}");
        assert!(s.contains("fn a()"), "应含首行: {s}");
        assert!(s.contains("10\tj"), "应含末行（原文，含行号前缀）: {s}");
        assert!(!s.contains("其余"), "精读不应有省略提示: {s}");
    }

    #[test]
    fn read_large_file_shows_structure_map() {
        // （读多少行较劲根治）：大文件（full 截断场景）渲染头部必须带
        let raw = r#"{"data":{"files":[{"path":"/ws/src/cmd.rs","total_lines":2013,"mode":"full","truncated":true,
            "structure_map":"fn main→1 · struct Args→45 · fn run_tools→390 · fn parse→512 · impl CmdTool→600",
            "content":"1\tfn main() {\n2\t// 截断后的前几行"}]}}"#;
        let s = clean("read", raw);
        assert!(s.contains("【结构地图】"), "精读应渲染结构地图头: {s}");
        assert!(
            s.contains("fn run_tools→390"),
            "地图应含目标函数行号（模型直接定位）: {s}"
        );
        assert!(
            s.contains("start_line/end_line") || s.contains("lines 分段"),
            "结构地图应自解释下一步动作: {s}"
        );
        assert!(s.contains("fn main()"), "仍应含正文: {s}");
    }

    #[test]
    fn auto_mode_keeps_full_preview_content() {
        // （auto 语义修正·不二次截断）：auto 预览**直给 read_one 的分级
        let mut lines = Vec::new();
        for i in 1..=70 {
            lines.push(format!("{i}\t代码行{i}"));
        }
        let content = lines.join("\n");
        let raw = serde_json::json!({
            "data": {"files": [{"path": "/ws/src/big.rs", "total_lines": 70, "mode": "auto", "content": content}]}
        }).to_string();
        let s = clean("read", &raw);
        assert!(s.contains("big.rs"), "应含文件路径: {s}");
        assert!(s.contains("代码行1"), "应含首行: {s}");
        assert!(
            s.contains("代码行35"),
            "auto 应保留中间内容（关键段不被二次截断）: {s}"
        );
        assert!(s.contains("代码行70"), "应含末行: {s}");
        assert!(!s.contains("省略"), "70 行未超长不应走兜底截断: {s}");
    }

    #[test]
    fn search_truncated_renders_scan_offset_hint() {
        let raw = r#"{"data":{"total":100,"has_more":true,"next_offset":34,
            "matches":[{"file":"/ws/src/a.rs","line":10,"text":"cache_file(..)"}]}}"#;
        let s = clean("search", raw);
        assert!(
            s.contains("scan_offset=34"),
            "截断时应渲染续扫断点值（模型据此续扫）: {s}"
        );
        assert!(s.contains("共 100 条"), "应给总数事实: {s}");
        assert!(!s.contains("缩小范围"), "不得再给搜法建议: {s}");
        assert!(s.contains("/ws/src/a.rs:10"), "应含匹配行: {s}");
    }

    #[test]
    fn search_complete_no_scan_hint() {
        // 未截断时不得出现续扫指引（无 next_offset=0）
        let raw = r#"{"data":{"total":2,"has_more":false,"next_offset":0,
            "matches":[{"file":"/ws/src/a.rs","line":1,"text":"x"},{"file":"/ws/src/b.rs","line":2,"text":"y"}]}}"#;
        let s = clean("search", raw);
        assert!(!s.contains("scan_offset"), "未截断不应有续扫指引: {s}");
        assert!(s.contains("2 处匹配"), "应显示总数: {s}");
    }

    #[test]
    fn search_renders_context_window_with_line_numbers() {
        // （智能投喂·免 read 确认）：search 命中自带前后文窗口（工具层 context 默认 5），
        let raw = r#"{"data":{"total":1,"has_more":false,"next_offset":0,
            "matches":[{"file":"/ws/src/chat.rs","line":120,"text":"fn handle(..) {",
            "context":[
                {"line":118,"text":"// 路由注册","match":false},
                {"line":119,"text":"router.get(\"/chat\")","match":false},
                {"line":120,"text":"fn handle(..) {","match":true},
                {"line":121,"text":"  let msg = ...","match":false}
            ]}]}}"#;
        let s = clean("search", raw);
        assert!(s.contains("/ws/src/chat.rs:120"), "应含命中坐标: {s}");
        assert!(s.contains("▶"), "命中行应有 ▶ 标记: {s}");
        assert!(s.contains("118"), "窗口应含前一行行号: {s}");
        assert!(s.contains("router.get"), "窗口应含命中前内容: {s}");
        assert!(s.contains("121"), "窗口应含后一行行号: {s}");
        assert!(
            !s.contains("未附窗口"),
            "全部命中带窗口时不应有剩余提示: {s}"
        );
    }

    #[test]
    fn search_window_cap_shows_remaining_hint() {
        // 命中过多时（窗口只给前 N 处）→ 渲染层必须自解释"还有 N 处未附窗口 + 下一步"
        let mut matches = Vec::new();
        for i in 0..3 {
            matches.push(serde_json::json!({"file": "/ws/src/a.rs", "line": i + 1, "text": "hit"}));
        }
        matches[0]["context"] = serde_json::json!([
            {"line": 1, "text": "hit", "match": true}
        ]);
        let raw = serde_json::json!({"data": {"total": 3, "matches": matches}}).to_string();
        let s = clean("search", &raw);
        assert!(
            s.contains("未附窗口"),
            "部分命中未带窗口应自解释剩余: {s}"
        );
        assert!(
            s.contains("start_line/end_line"),
            "应给下一步动作（read 指定行号）: {s}"
        );
    }

    #[test]
    fn find_files_truncated_shows_remaining() {
        let mut files = Vec::new();
        for i in 0..10 {
            files.push(serde_json::json!({"path": format!("/ws/src/f{i}.rs"), "size": 100}));
        }
        let raw = serde_json::json!({"data": {"total": 77, "files": files}}).to_string();
        let s = clean("find_files", &raw);
        assert!(s.contains("77 个文件"), "应显示总数: {s}");
        assert!(
            s.contains("还有 67 个未显示"),
            "截断应显式说明剩余: {s}"
        );
        // 截断只陈述事实（总数 + 未显示数），不再给搜法建议
        assert!(!s.contains("缩小范围"), "不得再给搜法建议: {s}");
    }

    #[test]
    fn list_shows_all_entries_no_window() {
        // （设计决定："全部都得给"）：list 不再人为砍窗口——模型一轮看到
        let mut entries = Vec::new();
        for i in 0..20 {
            entries.push(serde_json::json!({"type": "file", "path": format!("/ws/f{i}.py")}));
        }
        let raw = serde_json::json!({"data": {"entries": entries, "dirs": 0, "files": 20}}).to_string();
        let s = clean("list", &raw);
        assert!(s.contains("0 目录 · 20 文件"), "应显示计数: {s}");
        assert!(s.contains("/ws/f0.py"), "应含首条: {s}");
        assert!(s.contains("/ws/f19.py"), "应含末条（完整不截断）: {s}");
        assert!(
            !s.contains("还有"),
            "不再人为砍窗口、不再出现\"还有 N 条未显示\": {s}"
        );
    }

    #[test]
    fn auto_huge_content_falls_back_to_linenums_truncation() {
        // 超长 content（>4000 字符）→ read_with_linenums 兜底：首尾连续行号 + 明确省略标注
        let mut lines = Vec::new();
        for i in 1..=600 {
            lines.push(format!("{i}\t代码行{i}"));
        }
        let content = lines.join("\n");
        let raw = serde_json::json!({
            "data": {"files": [{"path": "/ws/src/huge.rs", "total_lines": 600, "mode": "auto", "content": content}]}
        }).to_string();
        let s = clean("read", &raw);
        assert!(
            s.contains("省略"),
            "超长应走兜底截断（连续行号省略标注）: {s}"
        );
        assert!(s.contains("代码行1"), "应含首行: {s}");
        assert!(s.contains("代码行600"), "应含末行: {s}");
    }

    #[test]
    fn lines_mode_keeps_full_content() {
        // （上下文投喂 P0）：lines/full 精读必须给**完整内容**——
        let mut lines = Vec::new();
        for i in 1..=100 {
            lines.push(format!("{i}\t代码行{i}"));
        }
        let raw = serde_json::json!({
            "data": {"files": [{"path": "/ws/src/mid.rs", "total_lines": 100, "mode": "lines", "content": lines.join("\n")}]}
        }).to_string();
        let s = clean("read", &raw);
        assert!(
            s.contains("代码行50"),
            "lines 精读应含中间内容（第 50 行）: {s}"
        );
        assert!(s.contains("代码行99"), "lines 精读应含尾段内容: {s}");
        assert!(!s.contains("其余"), "lines 精读不应折叠: {s}");
    }

    #[test]
    fn read_multi_file_parallel_each_previewed() {
        // v11（用户"并行读取怎么排"）：2-5 个文件并行 → 每个文件一段（path · N 行 +
        let raw = serde_json::json!({
            "data": {"files": [
                {"path": "/ws/src/a.rs", "total_lines": 3, "mode": "full", "content": "1\tfn a1()\n2\ta2\n3\ta3"},
                {"path": "/ws/src/b.rs", "total_lines": 3, "mode": "full", "content": "1\tfn b1()\n2\tb2\n3\tb3"}
            ]}
        }).to_string();
        let s = clean("read", &raw);
        assert!(s.contains("/ws/src/a.rs · 3 行"), "应含文件 a 段标识: {s}");
        assert!(s.contains("/ws/src/b.rs · 3 行"), "应含文件 b 段标识: {s}");
        assert!(s.contains("fn a1()"), "应含 a 内容: {s}");
        assert!(s.contains("fn b1()"), "应含 b 内容: {s}");
        assert!(s.contains("\n\n"), "文件间应有空行分隔: {s}");
        assert!(s.matches(" 行").count() >= 2, "应至少含两段行数标识: {s}");
    }

    #[test]
    fn modify_renders_block_diff_with_counts() {
        // （modify 渲染缺失 P0）：modify 结果必须是干净文本（文件 + 处数 +
        let raw = r#"{"ok":true,"kind":"modify_result","data":{"changed":1,"changes":[{"find":"    if price >= min_spend:\n        return price - reduce_amount\n    return price","replace":"    price = max(price, 0)\n    if price >= min_spend:\n        return max(price - reduce_amount, 0)\n    return price"}],"file":"D:/ws/prices.py","mode":"block","verified":{"find_gone":true,"note":"find 已消失、replace 已出现","replace_present":true,"verified":true}},"error":null,"warnings":[]}"#;
        let s = clean("modify", raw);
        assert!(s.contains("prices.py"), "应含文件路径: {s}");
        assert!(s.contains("修改 1 处"), "应含修改处数: {s}");
        assert!(s.contains("\n- "), "应有删除行标记（find 块）: {s}");
        assert!(s.contains("\n+ "), "应有新增行标记（replace 块）: {s}");
        assert!(s.contains("校验: find 已消失"), "应含 verified 校验行: {s}");
        assert!(!s.contains("\"data\""), "不应暴露原始 JSON 字段: {s}");
    }

    #[test]
    fn diagnose_renders_conclusion_and_hit_files() {
        // （契约链路闭环·下游投喂）：diagnose 结论必须在投喂中完整可见，
        let raw = r#"{"ok":true,"kind":"diagnose_result","data":{"issue":"login 报错","candidates":[{"file":"D:/x/login.rs","matches":[{"line":12,"text":"let user = auth()"}],"total_lines":120,"issues":[]},{"file":"D:/x/auth.rs","matches":[],"total_lines":60,"issues":[]}],"conclusion":"定位完成：先 read 确认命中文件上下文，再 edit 修复"}}"#;
        let s = clean("diagnose", raw);
        assert!(s.contains("诊断结论：定位完成"), "应含定位结论: {s}");
        assert!(s.contains("命中 2 个文件"), "应含命中文件数: {s}");
        assert!(s.contains("D:/x/login.rs"), "应含首个命中文件: {s}");
        assert!(s.contains("D:/x/auth.rs"), "应含第二个命中文件: {s}");
        assert!(!s.contains("\"candidates\""), "不应暴露原始 JSON 字段: {s}");
    }

    #[test]
    fn run_shows_command_exit_and_stdout() {
        let raw = r#"{"data":{"command":"cargo check","exit_code":0,"cwd":"/ws","stdout":"   Compiling ...\n   Finished\n"}}"#;
        let s = clean("run", raw);
        assert!(s.contains("cargo check"), "应含命令: {s}");
        assert!(s.contains("exit 0"), "应含退出码: {s}");
        assert!(s.contains("Finished"), "应含 stdout: {s}");
        assert!(!s.contains("exit_code"), "不应暴露原始字段名: {s}");
    }

    #[test]
    fn error_returns_full_text() {
        let raw = r#"{"status":"error","is_error":true,"content":[{"type":"text","text":"[INVALID_PARAM] 参数 path 不合法：请检查…"}]}"#;
        let s = clean("modify", raw);
        assert!(s.starts_with("[INVALID_PARAM]"), "失败应显示完整错误: {s}");
    }

    #[test]
    fn run_failure_keeps_stdout_stderr() {
        let raw = r#"{"ok":false,"kind":"run_result","data":{"command":"cargo check","exit_code":1,"cwd":"D:/x","stdout":"error[E0308]: mismatched types\n  --> src/main.rs:5","stderr":"warning: unused"},"warnings":[],"error":{"code":"RUN_FAILED","message":"命令退出码 1（非零）","suggestion":"请查看输出"}}"#;
        let s = clean("run", raw);
        assert!(s.contains("error[E0308]"), "应保留 stdout 失败证据: {s}");
        assert!(s.contains("退出码"), "应含退出码信息: {s}");
    }

    #[test]
    fn verify_failure_keeps_errors_array() {
        // 同断层：verify 失败（ok:false）的 errors 数组（构建/测试输出）不得被丢。
        let raw = r#"{"ok":false,"kind":"verify_result","data":{"target":"D:/x","command":"pytest -q","exit_code":1,"output":"FAILED tests/test_a.py","passed":false,"errors":["FAILED tests/test_a.py::test_validate - AssertionError"]},"warnings":[],"error":{"code":"VERIFY_FAILED","message":"构建/测试失败（exit=1）","suggestion":"查看 errors"}}"#;
        let s = clean("verify", raw);
        assert!(s.contains("test_validate"), "应保留 errors 失败证据: {s}");
    }

    /// 正文里带引号与换行（JSON/代码文件就是这种），摊平后必须**逐字**出现，
    #[test]
    fn read_full_text_flattens_and_keeps_body_verbatim() {
        let body = "    1\t{\n    2\t  \"meta\": {\n    3\t    \"project\": \"X\"\n    4\t  }\n";
        let raw = serde_json::json!({
            "data": {"files": [{"path": "D:/w/board.json", "total_lines": 4u64,
                               "mode": "full", "content": body}]},
            "error": null
        })
        .to_string();
        let out = read_full_text(&raw).expect("read 结果应能摊平");
        assert!(out.contains(body), "正文必须逐字保留（含真实换行与引号）: {out}");
        assert!(out.contains("D:/w/board.json"), "路径头要留下");
        assert!(out.contains("· 4 行"), "行数要留下: {out}");
        assert!(
            out.len() < raw.len(),
            "摊平后必须更短（raw {} → out {}）",
            raw.len(),
            out.len()
        );
    }

    #[test]
    fn read_full_text_keeps_structure_map_and_truncated_flag() {
        let raw = serde_json::json!({
            "data": {"files": [{
                "path": "D:/w/big.rs", "total_lines": 9000u64, "mode": "full",
                "truncated": true,
                "structure_map": "L12   pub fn a()\nL88   pub struct B",
                "content": "    1\tuse x;"
            }]}
        })
        .to_string();
        let out = read_full_text(&raw).unwrap();
        assert!(out.contains("L88   pub struct B"), "结构地图不能丢: {out}");
        assert!(out.contains("已截断"), "截断标记不能丢: {out}");
        assert!(out.contains("use x;"), "正文不能丢: {out}");
    }

    /// 认不出形状一律 `None` —— **宁可不省，不可改错**（调用方会退回原 JSON）。
    #[test]
    fn read_full_text_refuses_unknown_shapes() {
        for raw in [
            "纯文本，不是 JSON",
            r#"{"data":{"matches":[{"file":"a.py"}]}}"#,
            r#"{"data":{"files":[]}}"#,
            r#"{"data":{"files":[{"path":"a","content":"x"}]"#,
        ] {
            assert!(
                read_full_text(raw).is_none(),
                "不该认的形状必须返回 None: {raw}"
            );
        }
    }
}
