//! 信息提取器：原始文本 → 结构化事实（纯规则零 LLM）


/// 依赖提取器（无状态，全部方法静态）
pub struct InfoExtractor;

impl InfoExtractor {
    /// 从代码提取 import/use/require 依赖
    pub fn extract_imports(content: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in content.lines() {
            let t = line.trim();
            let dep: Option<String> = if t.starts_with("from ") {
                // Python: from a.b import c → a.b
                let body = t.trim_start_matches("from ");
                Some(
                    body.split(" import ")
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                )
            } else if t.starts_with("import ") {
                if t.contains(" from ") || t.contains('}') {
                    // ES module: import x from 'y' / import {a} from 'y' → from 后路径
                    Some(
                        t.split(" from ")
                            .nth(1)
                            .unwrap_or("")
                            .trim_matches(['"', '\'', ';'])
                            .trim()
                            .to_string(),
                    )
                } else if t.starts_with("import (") {
                    // Go 多包 import ( ... 单行简化
                    Some(
                        t.trim_start_matches("import (")
                            .trim_end_matches(')')
                            .trim_matches(['"', ';'])
                            .trim()
                            .to_string(),
                    )
                } else {
                    // Python: import a.b as x / import a, b → 第一个模块
                    let first = t
                        .trim_start_matches("import ")
                        .split(',')
                        .next()
                        .unwrap_or("")
                        .trim();
                    Some(
                        first
                            .split(" as ")
                            .next()
                            .unwrap_or(first)
                            .trim()
                            .to_string(),
                    )
                }
            } else if t.starts_with("use ")
                && !t.starts_with("use std")
                && !t.starts_with("use crate")
            {
                // Rust: use a::b::c → 外部 crate 名
                Some(
                    t.trim_start_matches("use ")
                        .split("::")
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                )
            } else if t.starts_with("require(") {
                // JS CommonJS: require('x')
                Some(
                    t.split(['(', ')', '"', '\''])
                        .nth(1)
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                )
            } else if t.starts_with("using ") {
                // C#: using System;
                Some(
                    t.trim_start_matches("using ")
                        .trim_end_matches(';')
                        .trim()
                        .to_string(),
                )
            } else {
                None
            };
            if let Some(d) = dep {
                if !d.is_empty()
                    && d != "{"
                    && !d.starts_with('"')
                    && !d.starts_with("self")
                    && !out.iter().any(|x| x == &d)
                {
                    out.push(d);
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "info_extractor_tests.rs"]
mod info_extractor_tests;
