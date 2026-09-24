//! 工具契约基础层（用户架构定调：契约收拢成基础，工具就是后端）

use serde_json::{json, Value};

use crate::tools::contract::{validate, ToolError};

/// 输出校验闸门：run() 返回文本 → 若声明了 output_schema，强制校验
pub fn validate_output(
    name: &str,
    output_schema: &Option<Value>,
    out: &str,
) -> Result<Value, ToolError> {
    let schema = match output_schema {
        None => return Ok(json!(out)),
        Some(s) => s,
    };
    // string 型输出契约：任意文本通过（纯文本工具）
    if schema.get("type").and_then(|v| v.as_str()) == Some("string") {
        return Ok(json!(out));
    }
    // object 型输出契约：必须可解析为 JSON 且符合 schema
    let parsed: Value = serde_json::from_str(out)
        .map_err(|_| ToolError::domain(
            "OUTPUT_INVALID",
            format!("工具 {name} 输出不符合声明的输出契约：无法解析为 JSON（output_schema 声明为 object）"),
            Some("请检查工具实现：声明了结构化输出就必须返回合法 JSON"),
        ))?;
    let violations = validate(schema, &parsed)?;
    Ok(violations)
}

/// base64 标准解码（自 fs_read 迁入：附件 data_url 解码属工具层基础能力，
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(t.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in t.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => return None,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// base64 标准编码（附件图降采样压缩后重建 data_url，与 base64_decode 对称）
pub fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
#[path = "base_tests.rs"]
mod base_tests;
