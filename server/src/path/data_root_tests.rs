//! `data_root` 的落点与命名契约测试（规范见 `docs/20260915-数据落盘与命名规范.md`）。

use super::*;

/// 文件名首段 = 统一时间戳（`YYYYMMDDTHHMMSSmmm+0800`，23 字符）。
fn stamp_head_ok(name: &str) -> bool {
    match name.split('-').next() {
        Some(h) if h.len() == 23 => h
            .chars()
            .all(|c| c.is_ascii_digit() || c == 'T' || c == '+'),
        _ => false,
    }
}

/// 统一时间戳的形态契约：秒级 20 字符 / 毫秒级 23 字符，且只含数字 / `T` / `+`。
#[test]
fn stamps_are_iso8601_basic_local() {
    let s = stamp_secs();
    let m = stamp_ms();
    assert_eq!(s.len(), 20, "秒级应为 YYYYMMDDTHHMMSS+0800: {s}");
    assert_eq!(m.len(), 23, "毫秒级应为 YYYYMMDDTHHMMSSmmm+0800: {m}");
    assert_eq!(&s[8..9], "T", "日期与时刻之间必须是 ISO 8601 的 T: {s}");
    // 两档必须出自同一时钟：秒级(20) 与毫秒级(23) 的日期段一致。
    assert_eq!(&s[..8], &m[..8], "两档日期段应同源: {s} / {m}");
    for t in [&s, &m] {
        assert!(
            t.chars().all(|c| c.is_ascii_digit() || c == 'T' || c == '+'),
            "时间戳只允许数字 / T / 加号（文件名安全）: {t}"
        );
    }
}

/// 回归：**spill 文件名里的 tag 必须能被模型从回执头部逐字节复现**。
#[test]
fn spill_file_name_keeps_call_id_verbatim() {
    let call_id = "call_02_dm9egAGHDu1yvqE2giMu6217";
    let n = spill_file_name(call_id);
    assert!(
        n.ends_with("-call02dm9egAGHDu1yvqE2giMu6217.txt"),
        "tag 去符号但不得截断；扩展名按内容用 .txt: {n}"
    );
    assert!(stamp_head_ok(&n), "文件名首段必须是统一时间戳: {n}");
}

/// 去符号后为空（tag 全是符号）时也要产出合法文件名，不能 panic、不能漏出非法字符。
#[test]
fn spill_file_name_survives_symbol_only_tag() {
    let n = spill_file_name("---___---");
    assert!(
        n.ends_with("-.txt"),
        "全符号 tag 退化成「时间戳 + 连字符」: {n}"
    );
    assert!(stamp_head_ok(&n), "文件名首段必须是统一时间戳: {n}");
    assert!(
        n.chars().all(|c| !"\\/:*?\"<>|".contains(c)),
        "文件名不得含 Windows 非法字符: {n}"
    );
}

// ── 默认落点契约：D 盘优先，不再默认 C 盘用户目录 ──────────────────────────
#[cfg(windows)]
mod default_root_tests {
    use super::*;

    /// 出厂默认根必须是 `X:\RealData` 形态。
    #[test]
    fn default_root_leaf_is_realdata() {
        let hint = default_data_root_hint();
        assert!(
            hint.ends_with(DEFAULT_ROOT_LEAF),
            "默认根最后一段必须是 {DEFAULT_ROOT_LEAF}: {hint}"
        );
        assert!(
            !hint.to_lowercase().contains("appdata"),
            "默认根不该再落在 %APPDATA%（重装系统即丢）: {hint}"
        );
    }

    /// 选盘 → 该盘下的落点：`E:` / `E:\` / `e:/` 都归一到 `X:\RealData`。
    #[test]
    fn root_for_drive_normalizes_slashes() {
        assert_eq!(root_for_drive("E:"), "E:\\RealData");
        assert_eq!(root_for_drive("E:\\"), "E:\\RealData");
        assert_eq!(root_for_drive("E:/"), "E:\\RealData");
        assert_eq!(root_for_drive("  D:  "), "D:\\RealData");
    }

    /// 盘符列表只给**真实存在**的盘（设置面板据此出按钮，给个不存在的盘就是死按钮）。
    #[test]
    fn available_drives_are_real() {
        let drives = available_drives();
        assert!(!drives.is_empty(), "至少要能列出系统盘");
        for d in &drives {
            assert!(
                std::path::Path::new(&format!("{d}\\")).is_dir(),
                "列出的盘必须真实存在: {d}"
            );
        }
        assert!(
            drives.iter().any(|d| d.eq_ignore_ascii_case("C:")),
            "系统盘 C: 必须在列: {drives:?}"
        );
    }

    /// 有 D 盘时，生效根不得再是 C 盘用户目录（除非用户显式设了标记/环境变量）。
    #[test]
    fn data_root_prefers_default_over_appdata() {
        if std::env::var("REAL_DATA_DIR").map(|v| !v.trim().is_empty()).unwrap_or(false) {
            return; // 测试机显式指定了根，不参与本判据
        }
        if get_data_dir_setting().is_some() {
            return; // 用户选过目录 —— 以标记为准
        }
        if !std::path::Path::new("D:\\").is_dir() {
            return; // 无 D 盘：回落 C 盘是正确行为
        }
        assert_ne!(
            data_root(),
            legacy_root(),
            "有 D 盘时默认根不该还是 %APPDATA%\\real-agent"
        );
    }
}
