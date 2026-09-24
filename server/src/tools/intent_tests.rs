//! tools/intent.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod route_any_tool_tests {
    use super::*;
    use serde_json::json;

    fn route(name: &str, args: &Value) -> (String, Value) {
        route_any_tool(name, args).expect("路由成功")
    }

    #[test]
    fn atomic_shape_passthrough() {
        // 原子名 + 原子形态 → 原样透传（后端不翻译、不猜）
        let args = json!({"paths": ["D:/x/a.py"], "mode": "full"});
        let (n, out) = route("read", &args);
        assert_eq!(n, "read");
        assert_eq!(out, args);
    }

    #[test]
    fn list_single_path_passthrough() {
        // list 的 schema 就是单数字段 path——不得被误判成"意图形态"
        let args = json!({"path": "D:/x/src"});
        let (n, out) = route("list", &args);
        assert_eq!(n, "list");
        assert_eq!(out, args);
    }

    #[test]
    fn legacy_intent_shape_not_translated() {
        // 旧"意图形态"参数不再由后端翻译：原样透传，缺参由契约层（contract.rs）报
        let args = json!({"path": "D:/x/a.py", "change": "改 x"});
        let (n, out) = route("modify", &args);
        assert_eq!(n, "modify");
        assert_eq!(out, args, "不再做 change → change_spec 的翻译");
    }
}

#[cfg(test)]
mod verify_command_route_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verify_with_command_routes_run() {
        let (n, out) = route_any_tool(
            "verify",
            &json!({"command": "cargo test", "cwd": "D:/SampleProject/server"}),
        )
        .expect("应路由成功");
        assert_eq!(n, "run", "verify+command 应转 run，实际: {n}");
        assert_eq!(out["command"], "cargo test");
        assert_eq!(out["cwd"], "D:/SampleProject/server");
        assert!(out.get("mode").is_none(), "run 不应残留 mode");
    }

    #[test]
    fn verify_with_target_stays_verify() {
        // verify + target（无 command）→ 保持 verify（验证文件/项目路径）
        let (n, out) =
            route_any_tool("verify", &json!({"target": "D:/x/analysis.json"})).expect("应路由成功");
        assert_eq!(n, "verify");
        assert_eq!(out["target"], "D:/x/analysis.json");
    }

    #[test]
    fn verify_with_path_routes_via_intent_to_verify() {
        // verify + path 单数 → 意图形态 → intent_run → verify（path 归一 target）
        let (n, out) =
            route_any_tool("verify", &json!({"path": "D:/x/analysis.json"})).expect("应路由成功");
        assert_eq!(n, "verify");
        assert_eq!(out["target"], "D:/x/analysis.json");
    }
}
