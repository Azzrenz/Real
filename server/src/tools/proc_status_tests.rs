//! tools/proc_status.rs 的测试外置（随函数迁出，mod 壳保留）

#[cfg(test)]
mod proc_status_tests {
    use crate::mcp::registry::BuiltinTool;
    use crate::tools::proc_status::ProcStatusTool;
    use serde_json::{json, Value};

    /// 行为级验证（设计决定："判断进程而非靠时间等）
    #[tokio::test]
    async fn proc_status_observes_own_process() {
        let t = ProcStatusTool;
        let me = std::process::id() as i64;
        let out = t.run(json!({"pid": me})).await.expect("proc_status 应成功");
        let d: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(d["kind"], "proc_status_result", "kind 应正确: {out}");
        let obs = d["data"]["observed"].as_array().unwrap();
        assert!(!obs.is_empty(), "自身进程应被观察到: {out}");
        let st = obs[0]["state"].as_str().unwrap();
        assert!(st == "active" || st == "idle", "state 应为 active/idle: {out}");
        assert!(obs[0]["pid"].as_i64() == Some(me), "pid 应匹配自身: {out}");
    }

    #[tokio::test]
    async fn proc_status_missing_pid_reports_exited() {
        let t = ProcStatusTool;
        let out = t.run(json!({"pid": 4_0000_0000})).await.expect("proc_status 应成功");
        let d: Value = serde_json::from_str(&out).unwrap();
        assert!(d["data"]["note"].as_str().unwrap().contains("不存在或已退出"), "应判 exited: {out}");
    }
}
