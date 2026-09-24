//! backbone 脏活层测试：InfoExtractor 依赖提取 + FileIndex lookup 主链路

use crate::backbone::file_index::FileIndex;

#[test]
fn file_index_lookup_resolves_basename() {
    let mut idx = FileIndex::new();
    let dir = std::env::temp_dir().join(format!("real_fi_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let f = dir.join("tmp_file.rs");
    std::fs::write(&f, "pub fn hello() {}\n").unwrap();

    idx.build(&dir);
    // 完整路径直接命中
    assert!(matches!(
        idx.lookup(&f.to_string_lossy()),
        crate::backbone::file_index::LookupResult::Exact(_)
    ));
    // basename 模糊匹配命中
    match idx.lookup("tmp_file.rs") {
        crate::backbone::file_index::LookupResult::Exact(p) => {
            assert!(p.contains("tmp_file.rs"), "p={p}")
        }
        other => panic!("basename 应命中 Exact, got {other:?}"),
    }
    // 不存在的名字 → NotFound
    assert_eq!(
        idx.lookup("no_such_file.rs"),
        crate::backbone::file_index::LookupResult::NotFound
    );

    let _ = std::fs::remove_dir_all(&dir);
}
