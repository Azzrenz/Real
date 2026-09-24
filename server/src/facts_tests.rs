
use super::{classify, filter_for_injection, liveness, FactKind, Liveness};

/// 测试用「一定存在的路径」：编译期注入的 crate 目录。
fn existing_path() -> String {
    env!("CARGO_MANIFEST_DIR").to_string()
}

/// 测试用「一定不存在、但父目录可达的路径」：临时目录下一个不存在的子目录。
fn ghost_path(name: &str) -> String {
    std::env::temp_dir().join(name).display().to_string()
}

#[test]
fn absolute_path_valid_vs_stale() {
    // 存在 → Valid；不存在但父目录可达 → Stale
    assert_eq!(
        liveness(&format!("{}/Cargo.toml", existing_path())),
        Liveness::Valid
    );
    assert_eq!(
        liveness(&ghost_path("real_facts_ghost_dir")),
        Liveness::Stale
    );
    let block = classify(r"D:\no_such_dir_xyz");
    assert_eq!(block, FactKind::Path);
}

#[test]
fn env_var_absent_is_stale_present_path_is_valid() {
    // 未设置的变量名 → Stale（REAL_PROJECT_ROOT 被清掉的情形）
    assert_eq!(liveness("REAL_NOT_SET_VAR_XYZ"), Liveness::Stale);
    // 设一个指向真实目录的变量 → Valid
    std::env::set_var("REAL_FACTS_TEST_DIR", existing_path());
    assert_eq!(liveness("REAL_FACTS_TEST_DIR"), Liveness::Valid);
    // 变量在、但值指向幽灵目录 → Stale
    std::env::set_var("REAL_FACTS_TEST_GHOST", ghost_path("real_facts_ghost_var"));
    assert_eq!(liveness("REAL_FACTS_TEST_GHOST"), Liveness::Stale);
}

#[test]
fn urls_and_models_are_unknown_not_guessed() {
    assert_eq!(liveness("https://api.example.com/v1"), Liveness::Unknown);
    assert_eq!(liveness("deepseek-flash"), Liveness::Unknown);
    assert_eq!(classify("src/agent"), FactKind::RelativePath);
    assert_eq!(liveness("src/agent"), Liveness::Unknown);
}

#[test]
fn injection_filter_drops_stale_entirely() {
    // "不要提它"：失效事实不得出现在注入面（连标注都没有）
    let ghost = ghost_path("real_facts_ghost_filter");
    let alive = existing_path();
    let facts = vec![
        ghost.clone(),
        alive.clone(),
        "https://api.example.com".to_string(),
    ];
    let out = filter_for_injection(&facts);
    assert!(
        !out.iter().any(|f| f == &ghost),
        "失效事实必须彻底消失: {out:?}"
    );
    assert!(out.iter().any(|f| f == &alive), "存活事实保留: {out:?}");
    assert!(
        out.iter().any(|f| f.contains("example.com")),
        "未知事实放行: {out:?}"
    );
}

#[test]
fn injection_filter_dedups_but_keeps_order() {
    let alive = existing_path();
    let sub = format!("{alive}/src");
    let facts = vec![alive.clone(), alive.clone(), sub.clone()];
    let out = filter_for_injection(&facts);
    assert_eq!(out, vec![alive, sub]);
}

#[test]
fn unreachable_drive_is_unknown_not_stale() {
    // 不可达盘符（本机无 Z 盘）→ Unknown，不得判失效
    let l = liveness("Z:/no_such_project/src/main.rs");
    assert_ne!(l, Liveness::Stale, "不可达盘不得判失效（防误杀）: {l:?}");
    // 可达盘上的幽灵路径 → 仍应判 Stale（该杀就杀）
    assert_eq!(
        liveness(&ghost_path("real_facts_ghost_drive")),
        Liveness::Stale
    );
}
