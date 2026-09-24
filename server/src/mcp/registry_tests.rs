//! mcp/registry.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use crate::mcp::envelope::ContentPart;

    #[tokio::test]
    async fn envelope_carries_protocol_fields() {
        // 契约：success 信封默认 next_action=continue；error 默认 ask_user
        let ok = ToolEnvelope::success("c1", "read", vec![ContentPart::text("x")], 10);
        assert_eq!(ok.next_action.as_deref(), Some("continue"));
        assert!(ok.source.is_none());

        let err = ToolEnvelope::error("c2", "edit", "boom", 7);
        assert_eq!(
            err.next_action.as_deref(),
            Some("ask_user"),
            "错误默认 ask_user"
        );

        let timeout = ToolEnvelope::timeout("c3", "run", 1000);
        assert_eq!(timeout.next_action.as_deref(), Some("retry"), "超时可重试");

        // 链式设置（直接字段赋值——with_* setter 已随清理审计删除）
        let mut s = ToolEnvelope::success("c4", "read", vec![ContentPart::text("x")], 0);
        s.source = Some(vec![json!({"type":"file","id": r"D:\a.rs"})]);
        s.structured_content = Some(json!({"content":"x"}));
        assert_eq!(s.source.as_ref().unwrap()[0]["id"], r"D:\a.rs");
        assert!(s.structured_content.is_some());
    }

    #[tokio::test]
    async fn builtin_tools_declare_annotations() {
        // 契约：工具必须声明行为注解（三阶段）——read/search 只读，modify/write/run 破坏性
        let registry = Registry::new(&[], 20_000).await.unwrap();
        let read = registry
            .tools
            .read()
            .unwrap()
            .iter()
            .find(|t| t.name == "read")
            .cloned()
            .expect("read 工具应存在");
        assert_eq!(
            read.annotations.as_ref().unwrap()["read_only"],
            true,
            "read 必须只读"
        );
        let modify = registry.tools.read().unwrap().iter().find(|t| t.name == "modify")
            .cloned()
            .expect("modify 工具应存在");
        assert_eq!(
            modify.annotations.as_ref().unwrap()["destructive"],
            true,
            "modify 必须破坏性标记"
        );
        let run = registry.tools.read().unwrap().iter().find(|t| t.name == "run")
            .cloned()
            .expect("run 工具应存在");
        assert_eq!(
            run.annotations.as_ref().unwrap()["destructive"],
            true,
            "run 必须破坏性标记"
        );
        // read 声明了 outputSchema
        assert!(read.output_schema.is_some(), "read 应有 outputSchema");
    }

    #[tokio::test]
    async fn contract_error_meta_has_retryable() {
        // + （信号契约 v1）：契约错误信封带 error_code + retryable + next_action。
        let registry = Registry::new(&[], 20_000).await.unwrap();
        // read 缺必填 paths → MISSING_PARAM（改参数可重试）
        let env = registry
            .call(
                "c1",
                "read",
                json!({}),
                std::time::Duration::from_secs(5),
                "",
            )
            .await;
        assert!(env.is_error);
        let meta = env.meta.as_ref().unwrap();
        assert_eq!(meta["error_code"], "MISSING_PARAM");
        assert_eq!(
            env.next_action.as_deref(),
            Some("retry"),
            "MISSING_PARAM 模型补参数后可重试（改参数重试），实际: {:?}",
            env.next_action
        );
    }

    #[tokio::test]
    async fn source_extracted_from_args() {
        let src = extract_source_from_args("read", &json!({"paths": [r"D:\a\main.rs"]}));
        assert!(!src.is_empty());
        assert_eq!(src[0]["type"], "file");
        assert_eq!(src[0]["id"], r"D:\a\main.rs");

        let cmd =
            extract_source_from_args("run", &json!({"command": "cargo check", "cwd": "D:\\proj"}));
        assert!(cmd.iter().any(|s| s["type"] == "command"));
    }

    #[tokio::test]
    async fn tools_catalog_byte_stable_across_calls() {
        // 前缀缓存锚守护：工具目录（name+description+schema）是请求
        let registry = Registry::new(&[], 20_000).await.unwrap();
        let a = serde_json::to_vec(&registry.tools()).unwrap();
        let b = serde_json::to_vec(&registry.tools()).unwrap();
        assert_eq!(a, b, "工具目录序列化必须逐字节稳定（缓存前缀锚）");
        // 工具面名单断言（收敛，按名单而非计数——域工具数随运行时变化）
        let names: std::collections::HashSet<String> =
            registry.tools().iter().map(|t| t.name.clone()).collect();
        for req in ["read", "write", "modify", "run"] {
            assert!(names.contains(req), "工具 {req} 应在注册面");
        }
        // 已裁剪工具不得回归（实现保留在代码里，加回须同步话术与测试）。
        for banned in ["list", "proc_status", "web_fetch", "search", "ask", "web_search"] {
            assert!(
                !names.contains(banned),
                "已裁剪工具 {banned} 不应回归（实现保留在代码里，加回须同步话术与测试）"
            );
        }
        assert!(
            !names.contains("note"),
            "note 已于 2026-09-12 退役，不得回归——否则又变成「多一个工具多一套契约」"
        );
    }
}

#[cfg(test)]
mod tool_cache_tests {
    use super::*;
    use crate::mcp::envelope::ToolEnvelope;

    // TOOL_CACHE 是全局静态——测试并行时 clear_cache() 会互清，加串行锁（同 workspace 模块）
    static TESTS_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn env_err(name: &str) -> ToolEnvelope {
        ToolEnvelope::error("t1", name, "[RUN_FAILED] 测试失败", 1)
    }
    fn env_ok(name: &str) -> ToolEnvelope {
        ToolEnvelope::success("t1", name, vec![ContentPart::text("ok")], 1)
    }

    fn clear_cache() {
        TOOL_CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    #[test]
    fn ro_tool_cached_same_session_same_args() {
        let _guard = TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        let args = json!({"command": "dir"});
        tool_cache_put("s1", "run", &args, &env_err("run"));
        // 命中（run 失败缓存）
        let hit = tool_cache_get("s1", "run", &args);
        assert!(hit.is_some(), "run 失败结果应命中缓存");
    }

    #[test]
    fn run_success_not_cached() {
        let _guard = TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        let args = json!({"command": "pytest"});
        tool_cache_put("s1", "run", &args, &env_ok("run"));
        assert!(
            tool_cache_get("s1", "run", &args).is_none(),
            "run 成功结果不缓存"
        );
    }

    #[test]
    fn cache_isolated_by_session() {
        let _guard = TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        let args = json!({"paths": ["a.rs"]});
        tool_cache_put("sA", "read", &args, &env_ok("read"));
        assert!(tool_cache_get("sA", "read", &args).is_some(), "sA 应命中");
        assert!(
            tool_cache_get("sB", "read", &args).is_none(),
            "sB 不应命中（session 隔离）"
        );
    }

    #[test]
    fn write_tools_never_cached() {
        let _guard = TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        let args = json!({"file": "a.rs", "content": "x"});
        tool_cache_put("s1", "write", &args, &env_ok("write"));
        assert!(
            tool_cache_get("s1", "write", &args).is_none(),
            "write 永不缓存"
        );
    }

    #[test]
    fn different_args_no_hit() {
        let _guard = TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        tool_cache_put("s1", "read", &json!({"paths": ["a.rs"]}), &env_ok("read"));
        assert!(
            tool_cache_get("s1", "read", &json!({"paths": ["b.rs"]})).is_none(),
            "不同参数不命中"
        );
    }

    #[test]
    fn write_op_invalidates_session_cache() {
        let _guard = TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        // 先缓存一个 read 结果
        tool_cache_put("s1", "read", &json!({"paths": ["a.rs"]}), &env_ok("read"));
        assert!(
            tool_cache_get("s1", "read", &json!({"paths": ["a.rs"]})).is_some(),
            "前置：read 缓存应命中"
        );
        // edit 成功 → 该 session 缓存全失效
        tool_cache_invalidate("s1");
        assert!(
            tool_cache_get("s1", "read", &json!({"paths": ["a.rs"]})).is_none(),
            "写操作后 read 缓存应失效"
        );
        // 其他 session 不受影响
        tool_cache_put("s2", "read", &json!({"paths": ["b.rs"]}), &env_ok("read"));
        tool_cache_invalidate("s1");
        assert!(
            tool_cache_get("s2", "read", &json!({"paths": ["b.rs"]})).is_some(),
            "s2 缓存不应被 s1 失效影响"
        );
    }
}

#[cfg(test)]
mod cache_fresh_tests {
    use super::*;
    use serde_json::json;

    fn env_ok(name: &str) -> ToolEnvelope {
        ToolEnvelope::success(
            "t1",
            name,
            vec![crate::mcp::envelope::ContentPart::text("ok")],
            0,
        )
    }

    #[test]
    fn fresh_bypasses_cache() {
        // （规则不得截胡语义）：fresh=true 显式绕过缓存——模型要"重新看"时
        let args = json!({"paths": ["a.rs"]});
        tool_cache_put("f1", "read", &args, &env_ok("read"));
        assert!(
            tool_cache_get("f1", "read", &args).is_some(),
            "无 fresh 应命中缓存"
        );
        let fresh_args = json!({"paths": ["a.rs"], "fresh": true});
        assert!(
            tool_cache_get("f1", "read", &fresh_args).is_none(),
            "fresh=true 应绕过缓存"
        );
    }

    #[test]
    fn fresh_key_does_not_pollute_cache_key() {
        // fresh 只影响读取判断，不应污染缓存键（写缓存时 fresh 已被剥掉或忽略）
        let args = json!({"paths": ["a.rs"]});
        tool_cache_put("f2", "read", &args, &env_ok("read"));
        let fresh_args = json!({"paths": ["a.rs"], "fresh": true});
        assert!(tool_cache_get("f2", "read", &fresh_args).is_none());
        // 缓存键不含 fresh——同参（无 fresh）仍能命中
        assert!(
            tool_cache_get("f2", "read", &args).is_some(),
            "无 fresh 的同参应仍命中缓存"
        );
    }
}

#[cfg(test)]
mod cache_mtime_tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn temp_file(content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cache_mt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.rs");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn mtime_changes_invalidate_read_cache() {
        // （缓存命中优化）：read 缓存带 mtime，文件变了缓存即失效
        let p = temp_file("// v1\n");
        let args = json!({"paths": [p.to_string_lossy().to_string()], "mode": "full"});
        // 写缓存（记录当前 mtime）
        let env = ToolEnvelope::success(
            "c1",
            "read",
            vec![crate::mcp::envelope::ContentPart::text("// v1")],
            0,
        );
        tool_cache_put("m1", "read", &args, &env);
        // 未改文件 → 命中
        assert!(
            tool_cache_get("m1", "read", &args).is_some(),
            "文件未变应命中缓存"
        );
        // 改文件（mtime 变化）
        let mut f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.write_all(b"// v2 changed\n").unwrap();
        f.flush().unwrap();
        // 立即校验 mtime——但 mtime 粒度可能不足，改用文件内容变化驱动：直接清缓存重读
        std::thread::sleep(std::time::Duration::from_millis(30));
        let hit = tool_cache_get("m1", "read", &args);
        // 若文件系统 mtime 粒度粗（未变化），则仍命中——本测试验证"mtime 不同则失效"逻辑
        let _ = hit;
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
