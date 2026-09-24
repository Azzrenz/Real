//! #En 残留检测与 JSON 宽松解析

use serde_json::Value;

/// 解析 "#E12" → 12（None=非占位符编号格式）
pub fn parse_step_number(id: &str) -> Option<usize> {
    let t = id.trim();
    if t.starts_with("#E") && t.len() > 2 && t[2..].chars().all(|c| c.is_ascii_digit()) {
        t[2..].parse().ok()
    } else {
        None
    }
}

/// 解析 "#E1" 形式占位符（整体引用形态；parse_step_number 的字符串视图）
pub(crate) fn parse_placeholder(s: &str) -> Option<String> {
    parse_step_number(s).map(|_| s.trim().to_string())
}

/// 收集 JSON 中所有"整体引用"形态的 #En（trim 后完全等于 #En）。
pub fn collect_placeholders(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                if let Some(ph) = parse_placeholder(s) {
                    out.push(ph);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => m.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(v, &mut out);
    out
}

/// 递归检测 JSON 中是否含 "#NEW" 字面（insert 占位符残留 = 决策不完整）
pub(crate) fn contains_new_placeholder(v: &Value) -> bool {
    match v {
        Value::String(s) => s.contains("#NEW"),
        Value::Array(arr) => arr.iter().any(contains_new_placeholder),
        Value::Object(map) => map.values().any(contains_new_placeholder),
        _ => false,
    }
}

/// 检测值中是否残留未解析占位符（#E\d+ 或 #NEW 字面）。
pub fn find_unresolved_placeholder(v: &Value) -> Option<String> {
    if let Some(ph) = collect_placeholders(v).into_iter().next() {
        return Some(ph);
    }
    if contains_new_placeholder(v) {
        return Some("#NEW".into());
    }
    None
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// 宽松 JSON 解析（模型输出兜底）：仅修复非法反斜杠转义（Windows 路径 D://proj 的单反斜杠），
pub fn extract_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let re = regex::Regex::new(r"[A-Za-z]:[\\/][^\s\x22]+").unwrap();
    for cap in re.find_iter(text) {
        let p = cap.as_str().trim_end_matches(['.', ',', ';', ')']);
        if !out.contains(&p.to_string()) {
            out.push(p.to_string());
        }
    }
    out
}

pub fn parse_json_lenient(s: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
        return Some(v);
    }
    let fixed = fix_illegal_escapes(s);
    serde_json::from_str::<serde_json::Value>(&fixed).ok()
}

/// 修复非法反斜杠转义：`D://proj` 的 `\E` 等非法转义补一个反斜杠保字面（合法转义不动）。
pub fn fix_illegal_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            let legal = match next {
                '"' | '\\' | '/' | 'n' | 'r' | 't' => true,
                'u' => {
                    i + 6 <= chars.len()
                        && chars[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit())
                }
                _ => false,
            };
            out.push('\\');
            if !legal {
                out.push('\\');
            }
            out.push(next);
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
#[path = "placeholder_tests.rs"]
mod placeholder_tests;
