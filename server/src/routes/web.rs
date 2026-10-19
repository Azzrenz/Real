//! 可选的静态前端托管（`REAL_SERVE_WEB`，**默认关**）。
//!
//! 为什么需要它：桌面壳把前端打进 Tauri（`tauri.conf.json` 的 `frontendDist: "../dist"`），
//! 后端本身**不托管静态前端**。于是手机打开一个网址时，背后没有东西把界面发出去。
//! 本模块给后端一个"顺便把前端也发出去"的开关。
//!
//! 规则：
//! 1. `/api/*`（含 `/api`）的**未匹配**请求 ⇒ 走 API 的 **404**，绝不被 SPA 回落成 `index.html`
//!    （`/api/*` 里已注册的路由在路由阶段就命中，不会进到这里）。
//! 2. 其余 **GET/HEAD** ⇒ 从 web 根提供静态文件；找不到 ⇒ 回落 `index.html`（前端是 SPA）。
//! 3. 其余方法 ⇒ 404（与「没有静态托管」时的观感一致）。
//! 4. 路径穿越（`..` / 反斜杠 / 非法百分号编码 / 跳出根目录）⇒ **403**，且不做 SPA 回落。
//! 5. 静态资源**不要求 token**（否则手机连界面都拿不到）——这一点由 `auth_guard::needs_auth`
//!    天然保证（它只覆盖 `/api/` 前缀），见 `auth_guard.rs`；`/api/*` 的鉴权口径不变。
//!
//! ⚠️ 默认关闭时**不注册任何回落**，路由与行为与未加本模块时逐字节一致（桌面零变化）。
//!
//! 未用 `tower_http::services::ServeDir` 的原因：它要 `tower-http` 的 `fs` 特性，
//! 该特性依赖 `http-range-header`（当前 cargo 缓存里没有）⇒ 离线构建解析不了。
//! 本仓是离线构建，故按本仓习惯手写一份最小实现（与 `strip_html`/`hex` 等零依赖手写同风格）。

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use std::path::{Component, Path, PathBuf};

/// 开关的环境变量名（`1/true/yes/on` 为开，其余含未设置 = 关）
pub const SERVE_WEB_ENV: &str = "REAL_SERVE_WEB";
/// web 根目录的环境变量名（显式指定，最高优先）
pub const WEB_DIR_ENV: &str = "REAL_WEB_DIR";

/// 开关是否打开（复用鉴权层的布尔口径，避免两处各写一份）。
pub fn serve_web_enabled() -> bool {
    crate::routes::auth_guard::parse_bool_flag(std::env::var(SERVE_WEB_ENV).ok().as_deref())
}

/// 解析 web 根目录：
/// 1. `REAL_WEB_DIR`（显式，最高优先）；
/// 2. 缺省 `<项目根>/client/dist` —— 正是 vite `outDir` 与 Tauri `frontendDist` 的落点。
///
/// 取不到「存在的目录」⇒ `None`（调用方据此回落为「不托管」）。
/// 取舍：显式 env 覆盖「后端单独部署、前端目录另放」的场景；缺省约定路径覆盖「仓库内直跑」。
/// 不用 `include_dir!` 内嵌：那会把前端**编译期**绑进后端二进制（后端单独编译就要先有 dist），
/// 与「前端可独立重部署」冲突，且体积膨胀、离线构建徒增依赖。
pub fn resolve_web_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var(WEB_DIR_ENV) {
        let v = v.trim();
        if !v.is_empty() {
            let p = PathBuf::from(v);
            return p.is_dir().then_some(p);
        }
    }
    let p = crate::tools::fs_read::project_root()
        .join("client")
        .join("dist");
    p.is_dir().then_some(p)
}

/// 启动期解析：开关开 ⇒ 解析 web 根（解析失败则告警并视为未开启）；开关关 ⇒ `None`。
pub fn startup_web_root() -> Option<PathBuf> {
    if !serve_web_enabled() {
        return None;
    }
    match resolve_web_dir() {
        Some(d) => {
            tracing::info!(dir = %d.display(), "REAL_SERVE_WEB 已开启：后端将托管静态前端");
            Some(d)
        }
        None => {
            tracing::warn!(
                "REAL_SERVE_WEB 已开启，但未找到 web 目录（用 REAL_WEB_DIR 指定，或缺省 client/dist）；静态托管不生效"
            );
            None
        }
    }
}

/// 给路由装上静态回落。`None` ⇒ **不动路由**（保持 axum 默认 404，与未开启时逐字节一致）。
///
/// 泛型便于测试：`router()` 传 `Router<AppState>`，测试传 `Router<()>`，两边走同一段装配代码。
pub fn with_web_fallback<S>(router: Router<S>, web_root: Option<PathBuf>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    match web_root {
        Some(root) => router.fallback(move |req: Request| {
            let root = root.clone();
            async move { serve(root, req).await }
        }),
        None => router,
    }
}

/// 请求路径的解析结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebFile {
    /// 指向 web 根内一个**存在的文件**
    Serve(PathBuf),
    /// 合法路径但文件不存在（调用方决定是否 SPA 回落）
    NotFound,
    /// 路径穿越 / 非法（反斜杠、非法百分号编码、跳出根）⇒ 一律拒
    Forbidden,
}

/// 百分号解码（`%XX` → 字节，再按 UTF-8 校验）。非法编码 / 非 UTF-8 ⇒ `None`。
///
/// 必须先解码再判 `..`：否则 `%2e%2e%2f` 会绕过朴素检查。
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = (*bytes.get(i + 1)? as char).to_digit(16)?;
            let lo = (*bytes.get(i + 2)? as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 把请求路径安全地映射到 web 根下的文件（**纯函数**，便于穷举测试）。
pub fn resolve_request(root: &Path, req_path: &str) -> WebFile {
    // ① 解码（`%2e%2e` → `..` 必须能在解码后被抓到）
    let Some(decoded) = percent_decode(req_path) else {
        return WebFile::Forbidden;
    };
    // ② 反斜杠一律拒：Windows 上 `\` 是分隔符，`..\..\x` 可穿越；NUL 也拒
    if decoded.contains('\\') || decoded.contains('\0') {
        return WebFile::Forbidden;
    }
    // ③ 逐段拼装：`..` 与非 Normal 成分（盘符/根/前缀）一律拒
    let mut out = root.to_path_buf();
    for seg in decoded.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return WebFile::Forbidden;
        }
        if Path::new(seg)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return WebFile::Forbidden;
        }
        out.push(seg);
    }
    // ④ 存在性判定；存在时再验一次「规范化后仍在根内」（防根内符号链接外逃）
    if !out.is_file() {
        return WebFile::NotFound;
    }
    match (root.canonicalize(), out.canonicalize()) {
        (Ok(r), Ok(o)) if o.starts_with(&r) => WebFile::Serve(o),
        _ => WebFile::Forbidden,
    }
}

/// 扩展名 → `Content-Type`（够 SPA 用即可；未知 ⇒ `application/octet-stream`）。
pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 静态回落处理器本体（`with_web_fallback` 注册它）。
pub async fn serve(root: PathBuf, req: Request) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // ① `/api/*` 的未匹配请求：走 API 的 404，**绝不** SPA 回落（否则 API 错误被伪装成 200 页面）
    if path == "/api" || path.starts_with("/api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    // ② 静态只服务 GET/HEAD
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let head_only = method == Method::HEAD;

    match resolve_request(&root, &path) {
        WebFile::Serve(file) => file_response(&file, head_only).await,
        WebFile::Forbidden => {
            tracing::warn!(%path, "静态托管：路径穿越/非法路径，已拒绝");
            StatusCode::FORBIDDEN.into_response()
        }
        // ③ SPA 回落：找不到 ⇒ index.html（前端路由自己处理）
        WebFile::NotFound => {
            let index = root.join("index.html");
            if index.is_file() {
                file_response(&index, head_only).await
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

/// 读文件 → 响应（带 Content-Type）。HEAD 只回头不带体；`index.html` 附 `no-cache`
/// （避免旧页引用已被替换的 hash 资源）。读失败 ⇒ 404（不泄露路径细节）。
async fn file_response(path: &Path, head_only: bool) -> Response {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(file = %path.display(), error = %e, "静态托管：读文件失败");
            return StatusCode::NOT_FOUND.into_response();
        }
    };
    let body = if head_only {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut resp = Response::new(body);
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime_for(path)));
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("index.html"))
        .unwrap_or(false)
    {
        resp.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    resp
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod web_tests;
