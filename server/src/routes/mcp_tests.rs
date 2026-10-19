//! mcp.rs 的测试外置（测试不进核心文件）。
//! 覆盖：远程视图下 MCP 配置 `headers` 的脱敏（`redact_mcp_headers`）——只留键名、抹掉值。

use super::*;

fn cfg(name: &str, headers: Value) -> Value {
    json!({
        "name": name,
        "transport": "http",
        "command": null,
        "args": [],
        "url": "https://example.com/mcp",
        "headers": headers,
    })
}

#[test]
fn 本机_配置原样返回_含请求头值() {
    // 本机（回环）必须逐字节原样返回——`McpSection.tsx:82` 靠它回填界面，改了就是桌面行为变化
    let saved = json!([cfg("a", json!({ "Authorization": "Bearer SECRET" }))]);
    assert_eq!(
        redact_mcp_headers(saved.clone(), false),
        saved,
        "本机必须原样返回（桌面零变化）"
    );
}

#[test]
fn 远程_只留请求头键名_抹掉值() {
    let saved = json!([
        cfg(
            "a",
            json!({ "Authorization": "Bearer SECRET", "X-Api-Key": "sk-live-123" })
        ),
        cfg("b", json!({})),
    ]);
    let got = redact_mcp_headers(saved, true);
    let arr = got.as_array().unwrap();
    let h = arr[0].get("headers").unwrap().as_object().unwrap();
    assert!(h.contains_key("Authorization"), "键名必须保留");
    assert!(h.contains_key("X-Api-Key"), "键名必须保留");
    for v in h.values() {
        assert_eq!(
            v,
            &Value::String(String::new()),
            "远程不得下发任何请求头**值**"
        );
    }
    // 同条目的其它字段不受影响
    assert_eq!(arr[0].get("name").unwrap().as_str(), Some("a"));
    // 无 headers 的条目原样（空对象）
    assert_eq!(arr[1].get("headers").unwrap(), &json!({}));
}

#[test]
fn 远程_非数组结构原样返回_不崩() {
    for v in [json!({}), json!([]), json!("x"), Value::Null] {
        assert_eq!(redact_mcp_headers(v.clone(), true), v);
    }
}

#[test]
fn 远程_非对象条目原样保留() {
    let saved = json!(["junk", cfg("a", json!({ "Authorization": "SECRET" }))]);
    let got = redact_mcp_headers(saved, true);
    let arr = got.as_array().unwrap();
    assert_eq!(arr[0].as_str(), Some("junk"), "非对象条目应原样保留");
    let h = arr[1].get("headers").unwrap().as_object().unwrap();
    assert_eq!(
        h.get("Authorization").unwrap(),
        &Value::String(String::new())
    );
}
