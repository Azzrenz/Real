//! backbone/info_extractor.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod imports_tests {
    use super::InfoExtractor;

    #[test]
    fn extract_imports_python() {
        let deps = InfoExtractor::extract_imports(
            "import os\nfrom fastapi import FastAPI\nimport numpy as np\n# import 注释\n",
        );
        assert!(deps.contains(&"os".to_string()));
        assert!(deps.contains(&"fastapi".to_string()), "deps={deps:?}");
        assert!(deps.contains(&"numpy".to_string()));
        assert_eq!(deps.len(), 3, "去重+跳过注释: {deps:?}");
    }

    #[test]
    fn extract_imports_rust_js() {
        let deps = InfoExtractor::extract_imports("use serde::Serialize;\nuse std::collections::HashMap;\nimport React from 'react';\nimport { useState } from 'react';\n");
        // std/crate 跳过；serde 保留；JS from 后路径
        assert!(deps.contains(&"serde".to_string()));
        assert!(!deps.contains(&"std".to_string()));
        assert!(deps.contains(&"react".to_string()), "deps={deps:?}");
        assert!(
            !deps.contains(&"React".to_string()),
            "ES import 应取 from 路径而非名字: {deps:?}"
        );
    }

    #[test]
    fn extract_imports_skips_noise() {
        let deps = InfoExtractor::extract_imports("x = 1\ny = 2\n# comment\n// js comment\n");
        assert!(deps.is_empty(), "无 import 应返回空: {deps:?}");
    }
}
