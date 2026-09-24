//! 意图层（12 抽屉）：intent_read/search/edit/run/audit/query/web_fetch/diagnose/self_heal/list/doc_gen/finish，

use serde_json::{json, Value};

/// 工具路由（收敛）：**只做一件小事**——verify 名的参数形态归一。
pub fn route_any_tool(name: &str, args: &Value) -> Result<(String, Value), String> {
    
    if name == "verify" && args.get("target").is_none() {
        let has_cmd = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if has_cmd {
            return Ok(("run".into(), {
                let mut a = args.clone();
                if let Some(obj) = a.as_object_mut() {
                    obj.remove("mode");
                }
                a
            }));
        }
        if let Some(p) = args
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            let mut a = args.clone();
            if let Some(obj) = a.as_object_mut() {
                obj.remove("path");
                obj.insert("target".to_string(), json!(p));
            }
            return Ok(("verify".into(), a));
        }
    }
    Ok((name.to_string(), args.clone()))
}

#[cfg(test)]
#[path = "intent_tests.rs"]
mod intent_tests;
