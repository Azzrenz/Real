//! 服务端 Origin 守卫：CORS 只约束浏览器读响应，拦不住请求被执行；
//! 非白名单来源在这里直接 403，先于任何 handler 生效。

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// 内置默认白名单（Tauri 桌面壳各平台变体 + 本地开发端口）
const DEFAULT_ORIGINS: [&str; 7] = [
    "http://tauri.localhost",
    "https://tauri.localhost",
    "tauri://localhost",
    "http://localhost:8618",
    "http://127.0.0.1:8618",
    "http://localhost:8943",
    "http://127.0.0.1:8943",
];

/// 解析逗号分隔的来源清单：逐项 trim，丢弃空项。
pub fn parse_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 生效白名单：环境变量 REAL_ALLOWED_ORIGINS 有值时用它，否则用内置默认。
pub fn allowed_origins() -> Vec<String> {
    match std::env::var("REAL_ALLOWED_ORIGINS") {
        Ok(v) if !parse_origins(&v).is_empty() => parse_origins(&v),
        _ => DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect(),
    }
}

/// 归一化用于精确匹配：去尾部 `/`、大小写不敏感。
fn normalize_origin(s: &str) -> String {
    s.trim().trim_end_matches('/').to_lowercase()
}

/// 拆出 (scheme, host)：host 不含端口、路径、查询与锚点。
fn split_scheme_host(origin: &str) -> (String, String) {
    let lower = origin.trim().to_lowercase();
    let (scheme, rest) = match lower.find("://") {
        Some(i) => (lower[..i].to_string(), &lower[i + 3..]),
        None => (String::new(), lower.as_str()),
    };
    let host = rest
        .split(|c| c == '/' || c == ':' || c == '?' || c == '#')
        .next()
        .unwrap_or("")
        .to_string();
    (scheme, host)
}

/// 来源是否放行。非浏览器客户端（curl / 后端自调用）不带 Origin，放行。
pub fn is_allowed_origin(origin: Option<&str>) -> bool {
    let Some(raw) = origin else {
        return true;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    // `null`（file:// 或沙箱页）正是要拒的来源；大小写变体一并拒。
    if trimmed.eq_ignore_ascii_case("null") {
        return false;
    }
    let (scheme, host) = split_scheme_host(trimmed);
    // Tauri 各平台/各 scheme 变体：host 为 tauri.localhost 一律放行（远端网页伪造不出该 host）。
    if host == "tauri.localhost" {
        return true;
    }
    // `tauri://` + host 为 localhost 一律放行。
    if scheme == "tauri" && host == "localhost" {
        return true;
    }
    let cand = normalize_origin(trimmed);
    allowed_origins()
        .iter()
        .any(|o| normalize_origin(o) == cand)
}

/// 守卫中间件：来源不在白名单 → 403，其余放行到下游。
pub async fn guard(req: Request, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if is_allowed_origin(origin.as_deref()) {
        return next.run(req).await;
    }
    let shown = origin.unwrap_or_default();
    tracing::warn!(
        origin = %shown,
        "请求来源不在白名单，已拒绝（可用环境变量 REAL_ALLOWED_ORIGINS 覆盖）"
    );
    (StatusCode::FORBIDDEN, format!("origin not allowed: {shown}")).into_response()
}

#[cfg(test)]
#[path = "origin_guard_tests.rs"]
mod origin_guard_tests;
