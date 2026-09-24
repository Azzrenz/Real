
use std::time::{Duration, SystemTime};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wt_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn finds_files_written_after_mark() {
    let d = tmp("finds");
    std::fs::write(d.join("a.rs"), "fn main() {}").unwrap();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/b.py"), "print(1)").unwrap();
    let files = super::written_since(&d, SystemTime::now() - Duration::from_secs(60)).unwrap();
    assert_eq!(files.len(), 2, "应找到两个源码文件: {files:?}");
    assert!(files.iter().any(|f| f.ends_with("a.rs")));
    assert!(files.iter().any(|f| f.ends_with("sub/b.py")), "子目录要递归: {files:?}");
}

#[test]
fn empty_when_nothing_written_since_mark() {
    let d = tmp("empty");
    std::fs::write(d.join("a.rs"), "x").unwrap();
    // 水位放到"未来" → 什么都还没写过（等价于"这段区间内无写入"）
    let files = super::written_since(&d, SystemTime::now() + Duration::from_secs(600)).unwrap();
    assert!(files.is_empty(), "水位在未来 → 应为空: {files:?}");
}

#[test]
fn skips_deps_products_and_non_source() {
    let d = tmp("skips");
    for dir in ["node_modules", "target", "dist", ".git"] {
        std::fs::create_dir_all(d.join(dir)).unwrap();
        std::fs::write(d.join(dir).join("x.rs"), "noise").unwrap();
    }
    std::fs::write(d.join("keep.rs"), "real").unwrap();
    std::fs::write(d.join("blob.bin"), "binary noise").unwrap();
    let files = super::written_since(&d, SystemTime::now() - Duration::from_secs(60)).unwrap();
    assert_eq!(files.len(), 1, "依赖/产物/非源码都要跳过: {files:?}");
    assert!(files[0].ends_with("keep.rs"));
}

#[test]
fn unreachable_root_is_none_not_empty() {
    // 判不出必须返回 None——**绝不能**退化成"没有改动"（那正是本次要修的假事实）
    assert!(super::written_since(std::path::Path::new("Z:/no_such_ws"), SystemTime::now()).is_none());
}
