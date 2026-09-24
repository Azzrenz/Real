use crate::path::path_exists_cached;

/// 事实性引用的类型（判定方式由类型决定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactKind {
    /// 绝对路径（`X:/...` 或 `X:\...`）
    Path,
    /// 环境变量名（大写字母/数字/下划线，≥4 位）
    EnvVar,
    /// 目录名形态但不是绝对路径（如 `src/agent`）——判不了
    RelativePath,
    /// 其余（URL、模型名、服务名、自然语言）——判不了
    Other,
}

/// 事实的存活状态（三态，唯一语义）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Valid,
    Stale,
    Unknown,
}

/// 事实类型表（表驱动：新增一类事实只加一条分支）
pub fn classify(fact: &str) -> FactKind {
    let t = fact.trim();
    let b = t.as_bytes();
    let is_abs = b.len() > 2
        && (b[0] as char).is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'/' || b[2] == b'\\');
    if is_abs {
        return FactKind::Path;
    }
    let looks_env = t.len() >= 4
        && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && t.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false);
    if looks_env {
        return FactKind::EnvVar;
    }
    if t.contains('/') || t.contains('\\') {
        return FactKind::RelativePath;
    }
    FactKind::Other
}

/// 单条事实的存活判定（唯一判定实现）
pub fn liveness(fact: &str) -> Liveness {
    match classify(fact) {
        FactKind::Path => path_verdict(fact.trim()),
        FactKind::EnvVar => match std::env::var(fact.trim()) {
            Err(_) => Liveness::Stale,
            Ok(v) => {
                let v = v.trim();
                // 变量值若是路径形态，再核一次值本身（判定同样走三态，不误杀）
                if matches!(classify(v), FactKind::Path) {
                    path_verdict(v)
                } else {
                    Liveness::Valid
                }
            }
        },
        // 判不了的一律放行（不猜、不假红）
        FactKind::RelativePath | FactKind::Other => Liveness::Unknown,
    }
}

fn path_verdict(p: &str) -> Liveness {
    if path_exists_cached(p) {
        return Liveness::Valid;
    }
    let mut cur = std::path::Path::new(p).parent();
    let mut hops = 0;
    while let Some(c) = cur {
        if hops > 8 || c.as_os_str().is_empty() {
            break;
        }
        if c.is_dir() {
            return Liveness::Stale;
        }
        cur = c.parent();
        hops += 1;
    }
    Liveness::Unknown
}

/// 写时判定（落库/写配置前）：返回逐条结果，供调用方告警与留痕。
pub fn judge_on_write(facts: &[String]) -> Vec<(String, Liveness)> {
    facts.iter().map(|f| (f.clone(), liveness(f))).collect()
}

/// 读时判定（注入/使用前·**唯一出口**）：只放行 Valid 与 Unknown；
pub fn filter_for_injection(facts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in facts {
        let alive = liveness(f);
        if alive != Liveness::Stale {
            if !out.iter().any(|x| x == f) {
                out.push(f.clone());
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod facts_tests;
