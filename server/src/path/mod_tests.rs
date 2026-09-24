//! path/mod.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    // ── 黄金测试矩阵（路径处理架构规范 §7，历史 bug 固化）──

    #[test]
    fn g7_trailing_punct_cleaned() {
        assert_eq!(normalize(r#""D:\proj\file", "#), r"D:\proj\file");
        assert_eq!(normalize(r"D:\proj\file，"), r"D:\proj\file");
        assert_eq!(normalize(r"D:\proj\file  "), r"D:\proj\file");
    }

    #[test]
    fn g9_deep_anchor_upscoped() {
        assert_eq!(
            normalize_workspace(r"D:\proj\src-tauri\src\agent"),
            r"D:\proj"
        );
        assert_eq!(normalize_workspace(r"D:\proj\node_modules\pkg"), r"D:\proj");
    }

    // ── 命令字段占位符判定（误伤修复）──

    #[test]
    fn cmd_placeholder_still_rejects_intent_words() {
        // 占位意图词仍拦截
        assert!(looks_like_placeholder_cmd("待定位的文件"));
        assert!(looks_like_placeholder_cmd(
            "python tests/verifier.py 或等价验证命令"
        ));
    }

    #[test]
    fn cmd_placeholder_allows_code_with_angle_brackets() {
        // 误伤修复实证（4 假阳性场景固化）：纯 ASCII 尖括号内容是合法代码
        assert!(!looks_like_placeholder_cmd(
            r#"python -c "print('<div class=\"a\">x</div>')""#
        ));
        assert!(!looks_like_placeholder_cmd(
            "python -c \"if t.get('attempts',0) < 1: raise RetryableError\""
        ));
        assert!(!looks_like_placeholder_cmd(
            "python a.py # fatal error -> dead letter"
        ));
    }

    #[test]
    fn cmd_placeholder_rejects_cjk_in_angle_brackets() {
        // 真占位形态：<...> 内是中文描述 → 仍拦
        assert!(looks_like_placeholder_cmd("run <目标命令>"));
        assert!(looks_like_placeholder_cmd("cat <待写入的文件>"));
        // 不成对的孤立 < 不拦
        assert!(!looks_like_placeholder_cmd("python a.py < input.txt"));
    }

    #[test]
    fn quoted_title_in_command_is_not_placeholder() {
        let cmd = r#"W="/d/Divinci Doc/滇西潮音/_工作产物"; echo "=== 星级/筛选相关文件 ==="; ls -la "$W/artifacts" | awk '{print $5, $9}'; du -sh /u/x"#;
        assert!(!looks_like_placeholder_cmd(cmd), "引号里的中文标题不是占位符");
        assert!(!looks_like_placeholder_cmd(r#"echo "待确认清单""#));
        assert!(!looks_like_placeholder_cmd("ls -la /tmp # 相关文件"));
        // 不带引号的中文参数 + 复合命令结构 → 也不拦（结构证据优先于词表）
        assert!(!looks_like_placeholder_cmd("ls -la /d/x | grep 相关文件"));
        assert!(!looks_like_placeholder_cmd("du -sh /u/x; echo 待确认"));
        // 护栏不松：裸的意图句仍要拦
        assert!(looks_like_placeholder_cmd("查看相关文件"));
        assert!(looks_like_placeholder_cmd("待定位的文件"));
    }

    #[test]
    fn anchor_prefers_existing_dir_over_earlier_example_path() {
        // 正文里的示例路径（D:\x 不存在）不得抢走真实任务目录（真实项目路径存在）
        let dir = std::env::temp_dir().join(format!("anchor_real_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.to_string_lossy().replace('\\', "/");
        let input = format!("参考示例 `\"D:\\x\"` 的写法。\n你去审核一下 {real} 这个项目。");
        let got = extract_anchor_path(&input).expect("应提取到真实目录");
        assert_eq!(
            got,
            real.trim_end_matches('/'),
            "应取真实存在的目录而非示例，实际: {got}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn anchor_falls_back_to_last_candidate_when_none_exist() {
        // 都不存在时取最后一个候选（任务指令通常在消息末尾）
        let input = "参考 D:\\x 示例。审核 D:\\proj_x 项目。";
        let got = extract_anchor_path(input).expect("应提取到候选");
        assert!(got.contains("proj_x"), "应取最后一个候选，实际: {got}");
    }

    #[test]
    fn anchor_paths_collects_all_candidates_deduplicated() {
        // （多路径语义·设计决定：）："对比 A 和 B"必须收集两个候选，不猜
        let input = "对比 D:\\SampleProject 和 D:\\proj 两个项目的架构，D:\\SampleProject 是主项目。";
        let got = extract_anchor_paths(input);
        assert!(got.len() >= 2, "应收集全部候选: {got:?}");
        assert!(got.iter().any(|p| p.contains("SampleProject")), "应含 SampleProject: {got:?}");
        assert!(
            got.iter().any(|p| p.contains("proj")),
            "应含 proj: {got:?}"
        );
        // 去重：同一路径出现两次只留一次
        let dup = extract_anchor_paths("处理 D:\\SampleProject，再看 D:\\SampleProject 的日志");
        let dup_count = dup.iter().filter(|p| p.contains("SampleProject")).count();
        assert_eq!(dup_count, 1, "应去重: {dup:?}");
    }

    // ── WorkspacePath 守卫测试（原 tools/path_guard.rs）──

    #[test]
    fn relative_path_rejected() {
        // poka-yoke：相对路径直接报错，不静默拼接项目根（防 D:\proj\#E3 类错误）
        let err = WorkspacePath::new("README.md").unwrap_err();
        assert_eq!(err.code, "RELATIVE_PATH", "相对路径必须被拒绝");
        let err2 = WorkspacePath::new(r"..\..\Users\user\secret.txt").unwrap_err();
        assert_eq!(err2.code, "RELATIVE_PATH", "含 .. 的相对路径也必须被拒绝");
    }

    #[test]
    fn dot_resolves_to_root() {
        let p = WorkspacePath::new(".").expect("点应解析为项目根");
        assert_eq!(p.into_inner(), crate::tools::project_root());
    }

    #[test]
    fn any_absolute_path_allowed() {
        // 策略：用户给出的路径均可访问（全放开，不做工作区白名单）
        let p = WorkspacePath::new(r"C:\Windows\system32\drivers\etc\hosts")
            .expect("外部绝对路径应通过");
        assert!(p.into_inner().is_absolute());
        let q = WorkspacePath::new(r"D:\proj\docs\pitfalls.md").expect("绝对路径应通过");
        assert!(q.into_inner().is_absolute());
    }

    #[test]
    fn sensitive_files_detected() {
        assert!(is_sensitive(Path::new(r"D:\proj\server\real.db")));
        assert!(is_sensitive(Path::new(r"D:\proj\server\.env")));
        assert!(is_sensitive(Path::new(r"D:\proj\keys\api.pem")));
        assert!(!is_sensitive(Path::new(r"D:\proj\README.md")));
        assert!(!is_sensitive(Path::new(r"D:\proj\server\src\main.rs")));
    }

    #[test]
    fn normalize_mixed_slashes() {
        // 正斜杠 → 反斜杠
        assert_eq!(
            normalize_input_path(r"D:/proj/README.md"),
            r"D:\proj\README.md"
        );
    }

    #[test]
    fn normalize_strips_quotes_and_punct() {
        // 带引号 + 尾部中文逗号
        assert_eq!(
            normalize_input_path(r#""D:\proj\README.md"，"#),
            r"D:\proj\README.md"
        );
        // 尾部英文句号
        assert_eq!(
            normalize_input_path(r"D:\proj\README.md."),
            r"D:\proj\README.md"
        );
        // 尾部空白
        assert_eq!(
            normalize_input_path(r"D:\proj\README.md  "),
            r"D:\proj\README.md"
        );
    }

    #[test]
    fn normalize_cleans_dot_segments() {
        assert_eq!(
            normalize_input_path(r"D:\proj\.\README.md"),
            r"D:\proj\README.md"
        );
        // 尾随分隔符
        assert_eq!(normalize_input_path(r"D:\proj\docs\"), r"D:\proj\docs");
    }

    #[test]
    fn guarded_path_accepts_normalized() {
        // 带引号/正斜杠的绝对路径 → 清洗后应通过守卫
        let p = WorkspacePath::new(r#""D:/proj/README.md""#).expect("清洗后应通过守卫");
        assert!(p.into_inner().to_string_lossy().ends_with("README.md"));
    }

    // ── PathPolicy 准入测试（原 security/path_policy.rs）──

    #[test]
    fn write_sensitive_requires_confirm() {
        let pp = PathPolicy::new();
        assert!(matches!(
            pp.check_write(r"C:\Windows\System32\drivers\etc\hosts"),
            PathVerdict::RequireConfirm(_)
        ));
        assert!(matches!(
            pp.check_write(r"C:\Program Files\App\config.json"),
            PathVerdict::RequireConfirm(_)
        ));
    }

    #[test]
    fn write_normal_allowed() {
        let pp = PathPolicy::new();
        assert!(matches!(
            pp.check_write(r"D:\user-projects\my-app\src\main.rs"),
            PathVerdict::Allow
        ));
        // 临时目录放行（AppData 敏感区须排除 temp）
        let tmp = std::env::temp_dir().join("real_test_tmp.txt");
        assert!(matches!(
            pp.check_write(&tmp.to_string_lossy()),
            PathVerdict::Allow
        ));
    }

    #[test]
    fn drive_root_requires_confirm() {
        let pp = PathPolicy::new();
        assert!(matches!(
            pp.check_write(r"D:\"),
            PathVerdict::RequireConfirm(_)
        ));
    }

    // ── 路径纠正（实况固化：漂移无关 + 候选必须存在）──

    #[test]
    fn correct_path_full_drift_in_parent_dir() {
        // 实况：模型 list 后重生成路径，中间目录连字符漂移成下划线。
        let real = std::env::temp_dir().join(format!("drift_test_{}", std::process::id()));
        let script_dir = real.join("scripts");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(script_dir.join("run.sh"), "echo ok").unwrap();
        let good = script_dir.join("run.sh").to_string_lossy().replace('/', "\\");
        // 构造漂移目标：good 里的第一处下划线 → 连字符（模拟 LLM token 漂移）
        let target = good.replacen('_', "-", 1);
        assert_ne!(target, good);
        let got = correct_path(&target, &[good.clone()]);
        assert_eq!(got, Some((good, target)), "整路径漂移（中间目录连字符↔下划线）应被纠正");
        let _ = std::fs::remove_dir_all(&real);
    }

    #[test]
    fn correct_path_requires_existing_candidate() {
        // 账本记录时存在 ≠ 现在存在：候选不存在时不得纠正（改对必然更好原则）
        let got = correct_path(
            r"D:\nonexistent\data_quality-hard_time_leakage\app\audit_leakage.py",
            &[r"D:\nonexistent\data_quality-hard-time_leakage\app\audit_leakage.py".to_string()],
        );
        assert_eq!(got, None, "不存在的候选不纠正");
    }

    #[test]
    fn correct_path_basename_same_dir_tail() {
        // basename 兜底的**合法场景**：目录末段相同（少写/多写一层同名目录）
        let root = std::env::temp_dir().join(format!("drift_tail_{}", std::process::id()));
        let deep = root.join(".real").join("memory");
        std::fs::create_dir_all(&deep).unwrap();
        let good = deep.join("MEMORY.md").to_string_lossy().replace('/', "\\");
        std::fs::write(&good, "x").unwrap();
        // 目标父目录末段也是 "memory" ⇒ 同一目录层的另一种写法 ⇒ 可纠正
        let target = root.join("memory").join("MEMORY.md").to_string_lossy().to_string();
        let got = correct_path(&target, &[good.clone()]);
        assert_eq!(got, Some((good, target)), "末段相同应纠正");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn correct_path_rejects_same_name_in_unrelated_dir() {
        let root = std::env::temp_dir().join(format!("drift_hit_{}", std::process::id()));
        let archives = root.join("docs").join("archives");
        std::fs::create_dir_all(&archives).unwrap();
        let existed = archives.join("MEMORY.md").to_string_lossy().replace('/', "\\");
        std::fs::write(&existed, "老内容 260 行").unwrap();
        // 目标：另一个目录层的同名文件（父末段 "memory" ≠ 候选父末段 "archives"）
        let target = root
            .join(".real")
            .join("memory")
            .join("MEMORY.md")
            .to_string_lossy()
            .to_string();
        let got = correct_path(&target, &[existed]);
        assert_eq!(got, None, "目录末段不同 ⇒ 只是重名，不得纠正（否则覆盖别人的文件）");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn correct_path_no_false_positive_on_distinct_names() {
        // 两个真实存在但名字确实不同的文件（非漂移）→ 不纠正
        let real = std::env::temp_dir().join(format!("drift_neg_{}", std::process::id()));
        std::fs::create_dir_all(&real).unwrap();
        let a = real.join("config.toml");
        let b = real.join("confx.toml");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let got = correct_path(&b.to_string_lossy(), &[a.to_string_lossy().to_string()]);
        assert_eq!(got, None, "名字不同的真实文件不得互相当作漂移");
        let _ = std::fs::remove_dir_all(&real);
    }
}

#[cfg(test)]
mod write_policy_tests {
    use super::*;

    #[test]
    fn app_data_of_own_project_is_not_sensitive() {
        // AppData 下的**应用数据目录**（项目自身数据）不该反复确认——
        let g = PathPolicy::new();
        let p = "C:/Users/user/AppData/Roaming/AppX/data/config.json";
        assert!(
            matches!(g.check_write(p), PathVerdict::Allow),
            "自家应用数据目录应直接放行"
        );
    }

    #[test]
    fn windows_and_program_files_still_confirm() {
        // 系统命脉仍需确认（保命底线）
        let g = PathPolicy::new();
        assert!(matches!(g.check_write(r"C:\Windows\System32\drivers\etc\hosts"), PathVerdict::RequireConfirm(_)), "Windows 目录必须确认");
        assert!(matches!(g.check_write(r"C:\Program Files\App\x.exe"), PathVerdict::RequireConfirm(_)), "Program Files 必须确认");
        // 用户名不硬编码：该判定比对本机 USERPROFILE，动态构造才稳
        let up = std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\Default".into());
        let sys = format!(r"{up}\AppData\Roaming\Microsoft\Windows\x.json");
        assert!(matches!(g.check_write(&sys), PathVerdict::RequireConfirm(_)), "系统相关 AppData 子目录必须确认");
    }

    #[test]
    fn grant_scope_then_skip_confirm() {
        // 确认一次后同目录不再弹（设计决定："确认一遍就 OK"）
        let g = PathPolicy::new();
        let sid = "s_grant_scope";
        let target = "C:/Users/user/AppData/Roaming/AppX/data/config.json";
        // 首次：走确认（此处构造一个原本需确认的路径——盘符根便于验证授权前后差异）
        let root = "C:/";
        assert!(matches!(g.check_write_scoped(Some(sid), root), PathVerdict::RequireConfirm(_)), "未授权前应确认");
        grant_write_scope(sid, root);
        assert!(
            matches!(g.check_write_scoped(Some(sid), root), PathVerdict::Allow),
            "授权后同目录应放行"
        );
        // 同目录另一文件同样放行（授权按目录不按单文件）
        assert!(matches!(g.check_write_scoped(Some(sid), "C:/other.txt"), PathVerdict::Allow));
        // 别的会话不受影响
        assert!(matches!(g.check_write_scoped(Some("other_session"), root), PathVerdict::RequireConfirm(_)));
        clear_write_grants(sid);
        let _ = target;
    }
}

#[test]
fn invalid_cwd_is_corrected_and_reported() {
    let args = serde_json::json!({"cwd": "D:////project_a_no_such_dir_xyz"});
    let fixed = super::correct_invalid_cwd(&args, None);
    let (orig, used) = fixed.expect("无效 cwd 必须被纠正");
    assert!(orig.contains("project_a_no_such_dir_xyz"), "保留原值用于回灌: {orig}");
    assert!(!used.is_empty(), "必须有回退目录: {used}");
}

#[test]
fn valid_cwd_left_alone() {
    let tmp = std::env::temp_dir();
    let args = serde_json::json!({"cwd": tmp.display().to_string()});
    assert!(super::correct_invalid_cwd(&args, None).is_none(), "有效 cwd 不应被改");
}

#[test]
fn structured_paths_only_from_fields_no_substring_false_positive() {
    let v = serde_json::json!({
        "call_id": "call_8869912c48744ab0b46bc000",
        "cwd": "D:/project-a",
        "url": "https://example.com/a/project-a",
        "note": "日志里提到 project-a 和 D:/project-a 但这不是字段",
        "relative": "src/main.rs"
    });
    let out = super::structured_paths_from_args(&v);
    assert_eq!(out, vec!["D:/project-a".to_string()], "只认字段且只收绝对路径: {out:?}");
}

#[test]
fn structured_paths_collect_paths_array_and_dedup() {
    let v = serde_json::json!({
        "paths": ["D:/project-b/server/a.rs", "D:/project-b/server/a.rs", "C:/x/y.md"]
    });
    let out = super::structured_paths_from_args(&v);
    assert_eq!(out.len(), 2, "去重: {out:?}");
}

#[test]
fn live_block_lists_only_existing_paths_and_never_mentions_dead() {
    let paths = vec!["D:/project-a".to_string(), "D:/project-b".to_string()];
    let block = super::live_paths_block_with(&paths, 8, &|p| p == "D:/project-b");
    assert!(block.contains("D:/project-b"), "存活路径应列出: {block}");
    assert!(!block.contains("project-a"), "失效路径必须彻底不出现（遗忘）: {block}");
}

#[test]
fn path_facts_block_respects_limit() {
    let paths: Vec<String> = (0..20).map(|i| format!("D:/p{i}")).collect();
    let block = super::live_paths_block_with(&paths, 3, &|_| true);
    assert_eq!(block.matches("- D:/p").count(), 3, "限制条数: {block}");
}
