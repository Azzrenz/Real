//! mcp/spill.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod spill_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn code_outline_extracts_structure_points() {
        let content = "    1\tuse crate::memtable::MemTable;\n    2\t\n    3\tpub struct EngineConfig {\n    4\t    pub path: String,\n    5\t}\n    6\t\n    7\timpl EngineConfig {\n    8\t    pub fn load_config(&self) -> Result<()> {\n    9\t        Ok(())\n   10\t    }\n   11\t}\n   12\t\n   13\tpub async fn open(&self) -> Engine {\n   14\t    Engine::new()\n   15\t}\n   16\t\n   17\t// 注释不该算结构点\n   18\tfn helper() {}\n";
        let outline = code_outline(content);
        assert!(
            outline.contains("L3    pub struct EngineConfig"),
            "应提取 struct: {outline}"
        );
        assert!(
            outline.contains("L7    impl EngineConfig"),
            "应提取 impl: {outline}"
        );
        assert!(
            outline.contains("L8    pub fn load_config"),
            "应提取 fn: {outline}"
        );
        assert!(
            outline.contains("L13   pub async fn open"),
            "应提取 async fn: {outline}"
        );
        assert!(
            outline.contains("L18   fn helper"),
            "结构点应带行号: {outline}"
        );
        assert!(
            !outline.contains("use crate"),
            "use 行不计入结构点: {outline}"
        );
        assert!(
            !outline.contains("MemTable;"),
            "不该包含 use 行正文: {outline}"
        );
        assert!(outline.contains("约 18 行"), "应含总行数: {outline}");
    }

    #[test]
    fn code_outline_empty_for_non_code() {
        assert_eq!(
            code_outline("    1\t这是一段中文文本\n    2\t没有代码结构\n"),
            ""
        );
        assert_eq!(code_outline(""), "");
        assert_eq!(code_outline("   12\t  // 只有注释\n"), "");
    }

    fn no_read() -> (Option<&'static str>, Vec<String>) {
        (None, Vec::new())
    }

    #[tokio::test]
    async fn small_result_no_spill() {
        let (m, p) = no_read();
        assert!(maybe_spill("read", "s1", "small text", m, &p)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn large_search_spills() {
        let big = "x".repeat(20_000);
        let (m, p) = no_read();
        let r = maybe_spill("search", "s1", &big, m, &p).await;
        assert!(r.is_some(), "大 search 结果应触发 spill");
        let (locator, preview) = r.unwrap();
        assert!(
            locator.contains("spill"),
            "locator 应含 spill 路径: {locator}"
        );
        assert!(preview.contains("正文未进上下文"), "预览应含正文未读强制提示");
        assert!(preview.len() < 4_000, "预览应远小于原文");
    }

    #[tokio::test]
    async fn read_small_not_spilled() {
        // read 中小文件全量直通（模型改代码要看到内容），低于 READ 阈值不 spill
        let small = "y".repeat(5_000);
        let (m, p) = no_read();
        assert!(
            maybe_spill("read", "s1", &small, m, &p).await.is_none(),
            "小 read 不应 spill"
        );
    }

    // （结构感知预览）——audit 大结果 spill 后预览用工具摘要层
    #[tokio::test]
    async fn audit_large_spills_with_summary_preview() {
        let summary = json!({
            "scope": "project",
            "root": "D:/x",
            "file_count": 3,
            "deep_count": 3,
            "issue_files": 2,
            "total_issues": 2,
            "tech_stack": {"languages": ["Rust"]},
            "high_risk": [{"path": "D:/x/server/src/mod.rs", "issue_count": 2, "issues": [{"line": 1, "kind": "TODO"}]}]
        });
        let mut data =
            json!({"deep_read": [{"path": "D:/x/a.rs", "preview": ""}], "deep_count": 1});
        data["summary"] = summary.clone();
        // padding 放 JSON 字符串值内部（保持 JSON 合法——真实 audit 794KB 是合法 JSON）
        data["deep_read"][0]["preview"] = json!("y".repeat(12_000));
        let audit_text = json!({"ok": true, "kind": "audit_result", "data": data}).to_string();
        assert!(
            audit_text.len() > 8_000,
            "测试 JSON 应超 spill 阈值: {}",
            audit_text.len()
        );

        let (m, p) = no_read();
        let r = maybe_spill("audit", "s1", &audit_text, m, &p).await;
        assert!(r.is_some(), "大 audit 应触发 spill");
        let (locator, preview) = r.unwrap();
        assert!(
            locator.contains("spill"),
            "locator 应含 spill 路径: {locator}"
        );
        assert!(
            preview.contains("工具摘要"),
            "audit 预览应用摘要层: {preview:.100}"
        );
        assert!(
            preview.contains("total_issues"),
            "摘要应含问题统计: {preview:.100}"
        );
        assert!(
            preview.contains("high_risk"),
            "摘要应含高危文件: {preview:.100}"
        );
        assert!(preview.len() < 4_000, "摘要预览应远小于原文");
        // 反向：无 summary 的 audit（旧结构）走结构概览（key 清单），不 panic
        let legacy = json!({"ok": true, "kind": "audit_result", "data": {"deep_read": [{"preview": "z".repeat(12_000)}]}}).to_string();
        let r2 = maybe_spill("audit", "s1", &legacy, m, &p).await.unwrap();
        assert!(
            r2.1.contains("全文层"),
            "无 summary 的 audit 仍含 spill 强制提示（契约式回取尾 [全文层 · 未读]）"
        );
        assert!(
            r2.1.contains("deep_read"),
            "无 summary 的 audit 应给结构概览: {:.100}",
            r2.1
        );
    }

    #[tokio::test]
    async fn spill_receipt_states_no_second_spill() {
        let text = json!({
            "ok": true,
            "data": {"lines": (0..900).map(|i| format!("L{i} {}", "x".repeat(60))).collect::<Vec<_>>()}
        })
        .to_string();
        let r = maybe_spill("read", "s1", &text, None, &[]).await;
        let (locator, preview) = r.expect("大 read 结果应触发 spill");
        assert!(
            preview.contains("正文未进上下文"),
            "回执必须明说正文未进上下文，否则模型以为已看到全文: {preview:.200}"
        );
        assert!(
            preview.contains(locator.as_str()),
            "回执必须给出 locator——它是回取的唯一入口: {preview:.200}"
        );
        assert!(
            preview.contains("永不二次落盘"),
            "回执必须明说落盘快照豁免二次落盘，否则模型只能试错: {preview:.200}"
        );
        assert!(
            !preview.contains("凡答案需要引述原文"),
            "契约本体（判定标准 / 取段线）不该再逐条重贴——它由工具描述常驻承担: {preview:.200}"
        );
        assert!(
            preview.len() < 900,
            "尾注应收敛（原 ~420 字符契约尾现只留 locator + 豁免声明）: {}",
            preview.len()
        );
    }

    // （结构感知预览覆盖所有 JSON 工具）——search 大结果 spill 后
    #[tokio::test]
    async fn search_large_spills_with_structured_preview() {
        let mut matches: Vec<serde_json::Value> = Vec::new();
        for i in 0..200 {
            matches.push(json!({"file": format!("D:/x/src/file_{i}.rs"), "line": i, "text": "fn foo() {".to_string()}));
        }
        let search_text = json!({"ok": true, "kind": "search_result", "data": {"total": 200, "matches": matches}}).to_string();
        assert!(
            search_text.len() > 8_000,
            "测试 JSON 应超 spill 阈值: {}",
            search_text.len()
        );

        let (m, p) = no_read();
        let r = maybe_spill("search", "s1", &search_text, m, &p).await;
        assert!(r.is_some(), "大 search 应触发 spill");
        let (locator, preview) = r.unwrap();
        assert!(
            locator.contains("spill"),
            "locator 应含 spill 路径: {locator}"
        );
        assert!(
            preview.contains("matches"),
            "search 预览应含 matches 结构概览: {preview:.100}"
        );
        assert!(
            preview.contains("200 项") || preview.contains("200 条"),
            "应显示命中总数: {preview:.100}"
        );
        assert!(
            preview.contains("file_0"),
            "应含前几条命中（路径不丢）: {preview:.100}"
        );
        assert!(preview.len() < 4_000, "结构预览应远小于原文");
    }

    // 非 JSON 文本流（read 全文）仍走 head/tail——阅读流头尾最有价值
    #[tokio::test]
    async fn plain_text_still_head_tail() {
        let big = "y".repeat(30_000);
        let (_, p) = no_read();
        let r = maybe_spill("read", "s1", &big, Some("full"), &p).await;
        assert!(r.is_some(), "大 read 应触发 spill");
        let (_, preview) = r.unwrap();
        assert!(
            preview.contains("中间省略"),
            "文本流应 head/tail 截断: {preview:.80}"
        );
        assert!(preview.contains("正文未进上下文"), "应含正文未读强制提示");
    }

    #[tokio::test]
    async fn read_large_spills() {
        // full 模式大文件全读（>20k 字符 ≈ 500 行）→ spill 防上下文爆炸
        let big = "y".repeat(30_000);
        let (_, p) = no_read();
        let r = maybe_spill("read", "s1", &big, Some("full"), &p).await;
        assert!(r.is_some(), "大 read 应触发 spill");
        let (locator, preview) = r.unwrap();
        assert!(
            locator.contains("spill"),
            "locator 应含 spill 路径: {locator}"
        );
        assert!(preview.contains("正文未进上下文"), "预览应含正文未读强制提示");
        assert!(preview.len() < 4_000, "预览应远小于原文");
    }

    /// `read mode=lines` 的落盘线 —— **被反转过两次，来历就是它的价值**。
    #[tokio::test]
    async fn read_lines_named_segment_is_not_spilled() {
        // 点名精读 30k：在 40,000 线之下 ⇒ 不落盘（模型要什么给什么）
        let big = "y".repeat(30_000);
        let (_, p) = no_read();
        assert!(
            maybe_spill("read", "s1", &big, Some("lines"), &p)
                .await
                .is_none(),
            "lines 精读 30k 不该落盘（阈值 40,000）——\
             体量由模型参数决定，累积量交给 L1/L2，入库层不抢"
        );
        // 兜底不丢：超过 read 工具上限（40k）仍会落盘
        let huge = "y".repeat(50_000);
        assert!(
            maybe_spill("read", "s1", &huge, Some("lines"), &p)
                .await
                .is_some(),
            "lines 精读 50k 超过工具上限，仍应落盘（兜底不丢）"
        );
        // 短段更不该落盘（落盘只会白多一次回读）
        let small = "y".repeat(3_000);
        assert!(
            maybe_spill("read", "s1", &small, Some("lines"), &p)
                .await
                .is_none(),
            "lines 精读 3k 不该落盘（短段落盘只会白多一次回读）"
        );
    }

    #[tokio::test]
    async fn read_spill_locator_exempt() {
        // （上下文预算）：read spill 文件（模型按 locator 回取全文）→ 豁免，
        let dir = spill_dir("s_spill_exempt");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let locator = dir.join("read_x.json").display().to_string();
        let big = "y".repeat(30_000);
        let paths = vec![locator.clone()];
        let r = maybe_spill("read", "s_spill_exempt", &big, Some("full"), &paths).await;
        assert!(r.is_none(), "read spill 文件应豁免 spill: {r:?}");
        // 非 spill 路径不受影响（仍 spill）
        let normal = vec!["D:/proj/big.rs".to_string()];
        let r2 = maybe_spill("read", "s_spill_exempt", &big, Some("full"), &normal).await;
        assert!(r2.is_some(), "普通大文件仍应 spill");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn non_content_tool_threshold_by_size() {
        let small = "y".repeat(1_000);
        let big = "y".repeat(20_000);
        let (m, p) = no_read();
        assert!(
            maybe_spill("done", "s1", &small, m, &p).await.is_none(),
            "小回执不 spill"
        );
        assert!(
            maybe_spill("done", "s1", &big, m, &p).await.is_some(),
            "超阈值回执一律 spill（不再限定工具名单）"
        );
    }

    // ── 启动清理（spill 只保留最近 7 天）──

    #[test]
    fn cleanup_removes_only_expired_dirs() {
        // 用可控 cutoff 覆盖"删/不删"两条路径，不依赖真实 mtime 拨动（Windows 目录 mtime 难写）
        let tmp = std::env::temp_dir().join(format!("spill_cleanup_test_{}", std::process::id()));
        let old = tmp.join("old_dir");
        let fresh = tmp.join("fresh_dir");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        // 拨动 old 目录 mtime 到 30 天前（filetime 跨平台稳定 API）
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 24 * 3600);
        filetime::set_file_mtime(&old, filetime::FileTime::from_system_time(past)).unwrap();
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
        let removed = scan_and_cleanup(&tmp, cutoff, cutoff, &std::collections::HashSet::new());
        assert_eq!(removed, 1, "只应删过期目录: old_dir");
        assert!(!old.exists(), "old_dir 应被删");
        assert!(fresh.exists(), "fresh_dir 应保留");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 真会话目录走长保留期，同名孤儿目录走短保留期 —— 两者在同一轮里必须被区别对待。
    #[test]
    fn cleanup_keeps_known_session_but_drops_orphan() {
        let tmp = std::env::temp_dir().join(format!("spill_dual_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let known = tmp.join("87485b26-260e-4300-abde-e3c7ec029399");
        let orphan_dir = tmp.join("s1");
        let orphan_file = tmp.join("1789472369737601400-callplain.out");
        for p in [&known, &orphan_dir] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(&orphan_file, b"x").unwrap();
        // 三个条目都拨到 3 天前：超过孤儿档(1 天)、未超真会话档(7 天)
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
        for p in [&known, &orphan_dir, &orphan_file] {
            filetime::set_file_mtime(p, filetime::FileTime::from_system_time(past)).unwrap();
        }
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(7);
        let orphan_cutoff = now - chrono::Duration::days(1);
        let known_set: std::collections::HashSet<String> =
            ["87485b26-260e-4300-abde-e3c7ec029399".to_string()]
                .into_iter()
                .collect();

        let removed = scan_and_cleanup(&tmp, cutoff, orphan_cutoff, &known_set);
        assert_eq!(
            removed, 2,
            "应只删孤儿目录与散文件（真会话目录 3 天前仍在 7 天档内）"
        );
        assert!(known.exists(), "在册会话目录必须保留（3 天 < 7 天档）");
        assert!(!orphan_dir.exists(), "不在册目录应被删（3 天 > 1 天档）");
        assert!(!orphan_file.exists(), "根下平铺散文件应被删");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 空名单 = 保守档（全部按孤儿处理）：真会话目录 3 天前也会被删。
    #[test]
    fn cleanup_with_empty_roster_treats_all_as_orphan() {
        let tmp = std::env::temp_dir().join(format!("spill_empty_roster_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let d = tmp.join("some-session-dir");
        std::fs::create_dir_all(&d).unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
        filetime::set_file_mtime(&d, filetime::FileTime::from_system_time(past)).unwrap();
        let now = chrono::Utc::now();
        let removed = scan_and_cleanup(
            &tmp,
            now - chrono::Duration::days(7),
            now - chrono::Duration::days(1),
            &std::collections::HashSet::new(),
        );
        assert_eq!(removed, 1, "空名单时应按孤儿档处理");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 造一个真实形态的载荷：信封 JSON，正文放在 `content[].text`，另带一份不同的 `render_full`。
    fn envelope_payload(name: &str, body: &str, render_full: Option<&str>) -> String {
        let mut env = crate::mcp::envelope::ToolEnvelope::success(
            "call_test_1",
            name,
            vec![crate::mcp::envelope::ContentPart::text(body)],
            1,
        );
        env.render_full = render_full.map(|s| s.to_string());
        serde_json::to_string(&env).unwrap()
    }

    #[test]
    fn spilled_form_drops_duplicate_copy_and_keeps_locator() {
        let mut env = crate::mcp::envelope::ToolEnvelope::success(
            "call_x",
            "run",
            vec![crate::mcp::envelope::ContentPart::text("正文预览")],
            1,
        );
        env.render_full = Some("r".repeat(20_000));
        mark_spilled(&mut env, "C:/spill/s/1-run.json", "【预览】");
        assert!(
            env.render_full.is_none(),
            "重复副本必须去掉：全量已在落盘文件里，留着等于模型的上下文付双份"
        );
        assert_eq!(env.content.len(), 1, "正文应只剩预览这一条");
        assert!(env.content[0].text.as_deref().unwrap_or("").contains("预览"));
        let meta = env.meta.as_ref().expect("必须写 meta");
        assert_eq!(meta["spilled"], json!(true));
        assert!(
            meta["spill_locator"].as_str().unwrap_or("").contains("1-run.json"),
            "locator 必须留下，模型靠它回取全文"
        );
    }

    #[tokio::test]
    async fn read_payload_measured_by_full_size_not_render_full() {
        let body = "z".repeat(64_624);
        let rf = "r".repeat(6_115);
        assert!(
            rf.chars().count() < READ_SPILL_THRESHOLD_CHARS,
            "前提：render_full 本身低于 read 门槛（旧写法因此漏判）"
        );
        let payload = envelope_payload("read", &body, Some(&rf));
        let (_, paths) = no_read();
        let r = maybe_spill("read", "s_measure", &payload, None, &paths).await;
        assert!(r.is_some(), "计量按完整载荷算 ⇒ 该 read 必须 spill");
        let _ = std::fs::remove_file(r.unwrap().0);
    }

    #[tokio::test]
    async fn spill_file_holds_the_whole_payload() {
        let body = "z".repeat(64_624);
        let payload = envelope_payload("read", &body, Some(&"r".repeat(6_115)));
        let (_, paths) = no_read();
        let (locator, _) = maybe_spill("read", "s_whole", &payload, None, &paths)
            .await
            .expect("应 spill");
        let saved = std::fs::read_to_string(&locator).expect("落盘文件应可读");
        assert!(
            saved.chars().count() >= body.chars().count(),
            "落盘必须含全文（旧写法只落 render_full，被预览挤掉的正文永久丢失）：文件 {} 字符",
            saved.chars().count()
        );
        assert!(saved.contains(&body[..2_000]), "那段大正文必须在文件里");
        let _ = std::fs::remove_file(&locator);
    }

    #[tokio::test]
    async fn preview_samples_the_longest_body_not_field_names() {
        let body = format!("{}\n【正文尾标记】", "z".repeat(64_624));
        let payload = envelope_payload("read", &body, Some(&"r".repeat(6_115)));
        let (_, paths) = no_read();
        let (locator, preview) = maybe_spill("read", "s_preview", &payload, None, &paths)
            .await
            .expect("应 spill");
        assert!(
            preview.contains("zzz"),
            "预览要取样自最长正文，不能是信封的字段清单：{preview:.160}"
        );
        assert!(
            preview.contains("【正文尾标记】"),
            "头尾预览必须带尾部（结论/报错常在尾部）：{preview:.240}"
        );
        assert!(
            !preview.contains("tool_call_id"),
            "不该把信封字段名当预览内容：{preview:.160}"
        );
        let _ = std::fs::remove_file(&locator);
    }

    /// UTF-8 边界安全：按字节截断**不能 panic**，也不能切出半个汉字。
    #[test]
    fn truncate_utf8_bytes_never_splits_a_char() {
        // 3 字节汉字重复，强制让 max_bytes 落在汉字中间
        let s = "测".repeat(100);
        assert_eq!(s.len(), 300, "前提：3 字节 × 100");
        for cut in [1usize, 2, 4, 5, 298, 299] {
            let (out, truncated) = truncate_utf8_bytes(&s, cut);
            assert!(truncated, "cut={cut} 应判为截断");
            assert!(out.len() <= cut, "cut={cut} 结果不得超限: {}", out.len());
            // 能完整读回 = 没有半个字符
            assert_eq!(out.chars().count() * 3, out.len(), "cut={cut} 切出了半个汉字");
        }
        // 恰好整字边界
        let (out, t) = truncate_utf8_bytes(&s, 3);
        assert_eq!(out, "测");
        assert!(t);
        // 不超限则原样返回
        let (out, t) = truncate_utf8_bytes("短文", 100);
        assert_eq!(out, "短文");
        assert!(!t);
    }

    /// 超限 → 只落前 8 MB，且文件头有截断说明（模型回取时第一眼看到"不完整"）。
    #[tokio::test]
    async fn save_full_caps_oversized_payload() {
        // 构造 > 8 MB 的纯 ASCII 正文（比汉字省内存，够触发即可）
        let huge = "A".repeat(SPILL_MAX_FILE_BYTES + 4096);
        let path = save_full("run", "s_cap_test", &huge)
            .await
            .expect("应有落盘");
        let written = std::fs::read_to_string(&path).expect("文件应可读");
        assert!(
            written.len() < huge.len(),
            "超限必须截断：写入 {} 应小于原文 {}",
            written.len(),
            huge.len()
        );
        assert!(
            written.contains("【spill 截断】"),
            "必须带截断说明（否则模型会把缺尾当成结果本来如此）"
        );
        assert!(
            written.contains("不要指望从这份文件里拿到全文"),
            "截断说明要给动作指引：改用更精确的命令重跑"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// 未超限 → 原样落盘，不加任何截断噪声。
    #[tokio::test]
    async fn save_full_keeps_small_payload_intact() {
        let small = "正常内容，不大。";
        let path = save_full("run", "s_small_test", small)
            .await
            .expect("应有落盘");
        let written = std::fs::read_to_string(&path).expect("文件应可读");
        assert_eq!(written, small, "未超限必须逐字原样");
        assert!(!written.contains("【spill 截断】"), "不该加截断说明");
        let _ = std::fs::remove_file(&path);
    }

    /// 工具描述（spill_note.md）里写的字符上限必须与 th 常量一致 —— 文案与代码不许各说各话。
    #[test]
    fn spill_note_publishes_real_thresholds() {
        let note = include_str!("../../prompts/tools/spill_note.md");
        for (label, v) in [
            ("通用线", crate::agent::history::th::SPILL_THRESHOLD_CHARS),
            ("read 全文线", crate::agent::history::th::READ_SPILL_THRESHOLD_CHARS),
            ("read 精读线", crate::agent::history::th::READ_LINES_SPILL_THRESHOLD_CHARS),
            // 通用线已动态化：**锚点**（上面的通用线常量）与**浮动区间**两端都要同步，
            ("动态下限", crate::agent::history::th::MIN_DYNAMIC_SPILL_CHARS),
            ("动态上限", crate::agent::history::th::MAX_DYNAMIC_SPILL_CHARS),
        ] {
            // Rust 的 format! 不支持千分位分组，手写（2000 -> "2,000"）
            let digits = v.to_string();
            let mut want = String::new();
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i) % 3 == 0 {
                    want.push(',');
                }
                want.push(c);
            }
            assert!(
                note.contains(&want),
                "spill_note.md 未同步 {label}（常量 {want}）—— 改了阈值就要同步工具描述"
            );
        }
    }

    // ── 压力缓存的**有界性**（LRU by 装配时刻）─────────────────────────

    /// 淘汰必须命中**真正最旧**的那条（`seq` 最小），而不是随便删一个。
    #[test]
    fn evict_picks_the_stalest_session() {
        let mut m: std::collections::HashMap<String, PressureEntry> =
            std::collections::HashMap::new();
        m.insert("fresh".into(), PressureEntry { tokens: 1, seq: 30 });
        m.insert("oldest".into(), PressureEntry { tokens: 1, seq: 10 });
        m.insert("mid".into(), PressureEntry { tokens: 1, seq: 20 });

        assert_eq!(evict_stalest(&mut m).as_deref(), Some("oldest"));
        assert!(!m.contains_key("oldest"), "最久未装配的应当出局");
        assert!(
            m.contains_key("fresh") && m.contains_key("mid"),
            "较新的不许被误伤"
        );
        assert_eq!(m.len(), 2);

        // 空表 ⇒ None（不是 panic）
        assert_eq!(evict_stalest(&mut std::collections::HashMap::new()), None);
    }

    /// 反复登记**永不突破上限**，且最新的一批必须存活。
    #[test]
    fn cache_never_exceeds_cap_and_keeps_the_newest() {
        let mut m: std::collections::HashMap<String, PressureEntry> =
            std::collections::HashMap::new();
        let mut seq = 0u64;
        let total = PRESSURE_CACHE_MAX as u64 + 25;
        for i in 0..total {
            seq += 1;
            m.insert(format!("s{i}"), PressureEntry { tokens: i, seq });
            if m.len() > PRESSURE_CACHE_MAX {
                evict_stalest(&mut m);
            }
            assert!(
                m.len() <= PRESSURE_CACHE_MAX,
                "第 {i} 次登记后长度 {} 已破上限 {PRESSURE_CACHE_MAX}",
                m.len()
            );
        }
        assert_eq!(m.len(), PRESSURE_CACHE_MAX, "稳定后应恰好占满上限");
        assert!(
            m.contains_key(&format!("s{}", total - 1)),
            "最新登记的会话被淘汰了 —— 淘汰方向反了"
        );
        assert!(!m.contains_key("s0"), "最早的那批应当已出局");
    }

    /// 被淘汰 / 从未登记的会话**读回来是稳态水位**（不是 0、不 panic）。
    #[test]
    fn evicted_session_reads_back_as_steady_watermark() {
        assert_eq!(
            session_pressure("__never_registered__"),
            steady_watermark_tokens(),
            "未登记的会话必须回退稳态水位（令动态阈值等于原常量）"
        );
    }

    /// base64 体积不得参与计量 —— 否则一张图必然越过门槛、整个信封被预览替换掉。
    #[tokio::test]
    async fn inline_image_payload_is_not_spilled_by_its_base64() {
        // 一张 200 KB 的图 ≈ 26 万字符 base64，远超 read 门槛（12,000）
        let payload = serde_json::to_string(&crate::mcp::envelope::ToolEnvelope::success(
            "call_img",
            "read",
            vec![
                crate::mcp::envelope::ContentPart::text("已读取 shot.png —— 图片见附件"),
                crate::mcp::envelope::ContentPart::image_ref(
                    format!("data:image/png;base64,{}", "A".repeat(260_000)),
                    "image/png",
                ),
            ],
            1,
        ))
        .unwrap();
        let (mode, paths) = no_read();
        assert!(
            maybe_spill("read", "s1", &payload, mode, &paths).await.is_none(),
            "含内联图片的 read 结果不得 spill：一旦 spill，image_ref 会被预览替换、模型永远看不到图"
        );
    }

    /// 剥离只动 `data_url`：http(s) 图不占体积，原样留给回灌通道；文本不受影响。
    #[test]
    fn strip_removes_only_inline_base64() {
        let inline = "data:image/png;base64,AAAA";
        let payload = serde_json::to_string(&crate::mcp::envelope::ToolEnvelope::success(
            "call_img",
            "read",
            vec![
                crate::mcp::envelope::ContentPart::text("正文"),
                crate::mcp::envelope::ContentPart::image_ref(inline, "image/png"),
                crate::mcp::envelope::ContentPart::image_ref("https://x/y.png", "image/png"),
            ],
            1,
        ))
        .unwrap();
        let (out, stripped) = strip_image_payload(&payload);
        assert_eq!(stripped, inline.len(), "只统计被剥离的内联图");
        assert!(!out.contains("AAAA"), "base64 不得留在载荷里（不计量、不落盘）");
        assert!(out.contains("https://x/y.png"), "http 图原样保留");
        assert!(out.contains("正文"), "文本不受影响");
    }

    /// spill 之后图片部件必须还在 —— 下游 `workflow` 靠它把图当附件回灌。
    #[test]
    fn mark_spilled_keeps_image_parts_for_the_attachment_channel() {
        let mut env = crate::mcp::envelope::ToolEnvelope::success(
            "call_img",
            "read",
            vec![
                crate::mcp::envelope::ContentPart::text("z".repeat(50_000)),
                crate::mcp::envelope::ContentPart::image_ref("data:image/png;base64,BBBB", "image/png"),
            ],
            1,
        );
        env.render_full = Some("r".repeat(20_000));
        mark_spilled(&mut env, "C:/spill/s/1-read.json", "【预览】");
        let imgs = env
            .content
            .iter()
            .filter(|p| p.reachable_image_uri().is_some())
            .count();
        assert_eq!(imgs, 1, "图片部件必须活过 spill（正文换预览，图不换）");
        assert!(
            env.content[0].text.as_deref().unwrap_or("").contains("预览"),
            "第一条仍是预览文本"
        );
        assert!(env.render_full.is_none());
        let meta = env.meta.as_ref().expect("必须写 meta");
        assert_eq!(meta["images_kept"], json!(1));
    }
}
