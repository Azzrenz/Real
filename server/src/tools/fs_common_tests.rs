//! tools/fs_common.rs 的测试外置（部门纪律：核心文件测试移出）

use super::*;
use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fs_common_guard_{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 只有"这次把平衡改坏"才拦；原本就不平衡的文件不拦（否则会把好改动回滚掉）。
#[test]
fn guard_blocks_only_new_breakage() {
    let good = "fn f() {\n    let x = 1;\n}\n";
    let broken = "fn f() {\n    let x = 1;\n";
    let already_broken = "fn f() {\n";

    assert!(brace_guard_should_block(good, broken), "把平衡改坏必须拦");
    assert!(
        !brace_guard_should_block(already_broken, "fn f() {\n    let x = 1;\n"),
        "原本不平衡 → 不拦（防回滚好改动）"
    );
    assert!(
        !brace_guard_should_block(good, "fn f() {\n    let x = 9;\n}\n"),
        "平衡改动放行"
    );
}

/// 快照只认工作区内**已存在**的 `.rs`：不存在（新建）跳过，工作区外跳过。
#[test]
fn snapshot_only_takes_existing_rs_inside_workdir() {
    let wd = temp_dir("snap");
    std::fs::create_dir_all(wd.join("src")).unwrap();
    std::fs::write(wd.join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(wd.join("note.md"), "# 无关\n").unwrap();

    let cmd = format!(
        "python -c \"open('{}/src/a.rs','w')\" && type src/missing.rs && dir note.md",
        wd.display().to_string().replace('\\', "/")
    );
    let got = snapshot_rs_targets(&cmd, &wd);
    assert_eq!(got.len(), 1, "只应收录 src/a.rs：{got:?}");
    assert!(got[0].0.ends_with("a.rs"));
    assert_eq!(got[0].1, "fn a() {}\n", "必须存下改前原文");

    // 工作区外的 .rs 不收（防越界回滚别人的文件）
    let outside = temp_dir("snap_outside");
    std::fs::write(outside.join("b.rs"), "fn b() {}\n").unwrap();
    let cmd2 = format!("python -c \"x\" {}", outside.join("b.rs").display());
    assert!(
        snapshot_rs_targets(&cmd2, &wd).is_empty(),
        "工作区外的文件不该进护栏"
    );
}

/// 端到端：命令把 .rs 的括号改坏 → 回滚原文 + 给出可读 warning。
#[test]
fn guard_rolls_back_broken_rs_and_reports() {
    let wd = temp_dir("roll");
    let file = wd.join("worker.rs");
    let before = "fn a() {\n    let x = 1;\n}\n\nfn b() {\n    let y = 2;\n}\n";
    std::fs::write(&file, before).unwrap();

    let guards = snapshot_rs_targets(&format!("python patch.py {}", file.display()), &wd);
    assert_eq!(guards.len(), 1, "应快照到 worker.rs");

    // 模拟"括号匹配吞掉后面函数"：删掉中间的闭合，把 b 一起吞了
    std::fs::write(&file, "fn a() {\n    let x = 1;\n\nfn b() {\n    let y = 2;\n}\n").unwrap();
    let warns = guard_rs_targets(&guards);

    assert_eq!(warns.len(), 1, "应报一条 warning：{warns:?}");
    assert!(warns[0].contains("回滚"), "要说明已回滚：{}", warns[0]);
    assert!(warns[0].contains("worker.rs"), "要点名文件：{}", warns[0]);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        before,
        "★ 文件必须恢复成执行前原文"
    );

    // 幂等：已经回滚过（内容与 before 相同）→ 再校验不再报
    assert!(guard_rs_targets(&guards).is_empty(), "回滚后不该重复报");

    let _ = std::fs::remove_dir_all(&wd);
}

/// 合法改动不该被误伤（护栏只在结构被破坏时动手）。
#[test]
fn guard_leaves_legit_change_alone() {
    let wd = temp_dir("ok");
    let file = wd.join("m.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    let guards = snapshot_rs_targets(&format!("python p.py {}", file.display()), &wd);

    std::fs::write(&file, "fn a() {}\n\nfn b() {\n    let z = 3;\n}\n").unwrap();
    assert!(guard_rs_targets(&guards).is_empty(), "正常新增函数不该报");
    assert!(
        std::fs::read_to_string(&file).unwrap().contains("fn b"),
        "合法改动必须保留"
    );

    let _ = std::fs::remove_dir_all(&wd);
}

// ── 判据二：结构护栏（定义点成批消失）────────────────────────────────────────

#[test]
fn structure_guard_blocks_bulk_deletion() {
    let mut before = String::new();
    for i in 0..6 {
        before.push_str(&format!("fn f{i}() {{\n    let x = {i};\n}}\n\n"));
    }
    // 删掉后 4 个，括号仍然平衡
    let after = "fn f0() {\n    let x = 0;\n}\n\nfn f1() {\n    let x = 1;\n}\n\n";
    let lost = structure_guard_should_block(&before, after);
    assert_eq!(lost.len(), 4, "应报 4 个定义点消失：{lost:?}");
    assert!(lost.iter().any(|s| s.contains("f2")), "要点名：{lost:?}");
    assert!(
        brace_balance_ok(after),
        "本条判据的前提：括号仍平衡（判据一看不见 => 必须由这条兜）"
    );
}

/// 端到端：脚本把 5 个函数删成 2 个 → **直接回滚**（不是"预警让模型再跑一轮"）。
#[test]
fn guard_rolls_back_bulk_deletion_in_run_channel() {
    let wd = temp_dir("bulk");
    let file = wd.join("svc.rs");
    let mut before = String::new();
    for i in 0..5 {
        before.push_str(&format!("fn s{i}() {{\n    let v = {i};\n}}\n\n"));
    }
    std::fs::write(&file, &before).unwrap();
    let guards = snapshot_rs_targets(&format!("python patch.py {}", file.display()), &wd);
    assert_eq!(guards.len(), 1, "应快照到 svc.rs");

    let after = "fn s0() {\n    let v = 0;\n}\n\nfn s1() {\n    let v = 1;\n}\n\n";
    std::fs::write(&file, after).unwrap();

    let warns = guard_rs_targets(&guards);
    assert_eq!(warns.len(), 1, "应拦并报道：{warns:?}");
    assert!(
        warns[0].contains("定义点"),
        "要说明是定义点消失：{}",
        warns[0]
    );
    assert!(warns[0].contains("回滚"), "要说明已回滚：{}", warns[0]);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        before,
        "★ 必须恢复成执行前原文"
    );

    let _ = std::fs::remove_dir_all(&wd);
}

/// 防误伤：删 1 个不算"成批消失"（单个废弃函数是本意）；改名（数量不变）更不该拦。
#[test]
fn structure_guard_tolerates_small_or_shape_preserving_change() {
    let three = "fn a() {}\n\nfn b() {}\n\nfn c() {}\n";
    let two = "fn a() {}\n\nfn b() {}\n";
    assert!(
        structure_guard_should_block(three, two).is_empty(),
        "只减 1 个不拦（阈值 3）"
    );
    let renamed = "fn a() {}\n\nfn b2() {}\n\nfn c() {}\n";
    assert!(
        structure_guard_should_block(three, renamed).is_empty(),
        "改名不改变数量 ⇒ 不拦（防误伤重构）"
    );
    assert!(
        structure_guard_should_block("随便一段文字\n", "另一段\n").is_empty(),
        "改前没有可识别定义点 ⇒ 无从判起，不拦"
    );
}

/// UTF-8 原样通过，且**不打标记**（纯 UTF-8 不该有噪音）。
#[test]
fn decode_text_passes_utf8_untouched() {
    let (s, non_utf8) = decode_text("中文 UTF-8 内容".as_bytes());
    assert_eq!(s, "中文 UTF-8 内容");
    assert!(!non_utf8, "纯 UTF-8 不得被标成非 UTF-8");
}

#[test]
fn decode_text_falls_back_to_gbk() {
    let gbk: &[u8] = &[0xc8, 0xce, 0xce, 0xf1];
    let (s, non_utf8) = decode_text(gbk);
    assert_eq!(s, "任务", "GBK 必须被正确解出（否则中文全变替换符，等于没读）");
    assert!(non_utf8, "非 UTF-8 **必须留痕** —— 调用方据此标注编码");
}

#[test]
fn decode_text_keeps_ascii_intact_in_mixed_content() {
    let mut bytes = b"Traceback (most recent call last):\n  File \"C:\\x\\".to_vec();
    bytes.extend_from_slice(&[0xc8, 0xce, 0xce, 0xf1]);
    bytes.extend_from_slice(b"\\skin\\0000.xml'\nFileNotFoundError\n");
    let (s, non_utf8) = decode_text(&bytes);
    assert!(non_utf8);
    assert!(s.contains("Traceback (most recent call last)"), "ASCII 主体必须完好");
    assert!(s.contains("FileNotFoundError"), "英文报错名必须完好（它是排查的抓手）");
    assert!(s.contains("任务"), "夹在中间的 GBK 中文也要解出来");
    assert!(s.contains("skin\\0000.xml"), "路径尾部必须完好");
}

/// 空输入不 panic（后台日志刚创建时是 0 字节，这条路径会真的走到）。
#[test]
fn decode_text_handles_empty() {
    let (s, non_utf8) = decode_text(&[]);
    assert_eq!(s, "");
    assert!(!non_utf8);
}
