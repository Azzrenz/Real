//! settings.rs 的测试外置（测试不进核心文件）。
//! 覆盖：本机路径类字段的远程收敛（`local_paths_view`）——远程不下发 `data_dir`/`data_root_effective`。

use super::*;

#[test]
fn 本机_路径字段原样下发() {
    let (d, r) = local_paths_view(false, Some("D:\\data".into()), "D:\\real".into());
    assert_eq!(d, Value::String("D:\\data".into()));
    assert_eq!(r, Value::String("D:\\real".into()));
}

#[test]
fn 本机_未设自定义目录_data_dir为_null_但生效根仍下发() {
    let (d, r) = local_paths_view(
        false,
        None,
        "C:\\Users\\x\\AppData\\Roaming\\real-agent".into(),
    );
    assert_eq!(d, Value::Null);
    assert_eq!(
        r,
        Value::String("C:\\Users\\x\\AppData\\Roaming\\real-agent".into())
    );
}

#[test]
fn 远程_两个路径字段一律收敛为_null() {
    // 无论是否设了自定义目录，远程都不得拿到任何本机绝对路径
    for dir in [None, Some("D:\\data".to_string())] {
        let (d, r) = local_paths_view(true, dir, "D:\\real".into());
        assert_eq!(d, Value::Null, "远程不得下发 data_dir");
        assert_eq!(r, Value::Null, "远程不得下发 data_root_effective");
    }
}
