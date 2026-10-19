//! web.rs 的测试外置（测试不进核心文件）。
//! 覆盖：百分号解码 / MIME 映射 / 路径解析与穿越防护 / `/api/*` 优先级 /
//! SPA 回落 / 非 GET / HEAD / 真实 TCP 下「开 vs 关」的行为。

use super::*;
use axum::routing::get;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// 造一个临时 web 根：`index.html` + `assets/app.js`；每个用例独立目录（防并行互踩）。
fn tmp_web_root(tag: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!(
        "real_web_test_{}_{}_{}",
        tag,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("assets")).unwrap();
    std::fs::write(d.join("index.html"), "<!doctype html><div id=\"root\"></div>").unwrap();
    std::fs::write(d.join("assets").join("app.js"), "console.log(1)").unwrap();
    d
}

fn req(method: &str, uri: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn read_body(resp: Response) -> String {
    let b = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(b.to_vec()).unwrap()
}

fn content_type(resp: &Response) -> String {
    resp.headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

// ── 百分号解码 ──────────────────────────────────────────────────────────

#[test]
fn 百分号解码_合法与非法() {
    assert_eq!(percent_decode("/a%20b").as_deref(), Some("/a b"));
    assert_eq!(percent_decode("/%2e%2e/").as_deref(), Some("/../"));
    assert_eq!(percent_decode("/ok").as_deref(), Some("/ok"));
    assert_eq!(percent_decode("").as_deref(), Some(""));
    // 非法：截断 / 非十六进制 / 非 UTF-8 字节
    assert!(percent_decode("/%").is_none());
    assert!(percent_decode("/%2").is_none());
    assert!(percent_decode("/%zz").is_none());
    assert!(percent_decode("/%ff").is_none());
}

// ── MIME 映射 ───────────────────────────────────────────────────────────

#[test]
fn mime_映射() {
    assert_eq!(mime_for(Path::new("a/index.html")), "text/html; charset=utf-8");
    assert_eq!(
        mime_for(Path::new("a/app.js")),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(mime_for(Path::new("a/app.css")), "text/css; charset=utf-8");
    assert_eq!(mime_for(Path::new("a/x.svg")), "image/svg+xml");
    assert_eq!(mime_for(Path::new("a/x.woff2")), "font/woff2");
    // 未知扩展名 / 无扩展名 ⇒ 兜底
    assert_eq!(mime_for(Path::new("a/x.bin")), "application/octet-stream");
    assert_eq!(mime_for(Path::new("a/LICENSE")), "application/octet-stream");
    // 大小写不敏感
    assert_eq!(mime_for(Path::new("a/X.JS")), "text/javascript; charset=utf-8");
}

// ── 路径解析 ────────────────────────────────────────────────────────────

#[test]
fn 静态解析_正常文件与缺失() {
    let root = tmp_web_root("resolve");
    match resolve_request(&root, "/index.html") {
        WebFile::Serve(p) => assert!(p.ends_with("index.html"), "应指向 index.html: {p:?}"),
        other => panic!("应 Serve，得到 {other:?}"),
    }
    assert!(matches!(
        resolve_request(&root, "/assets/app.js"),
        WebFile::Serve(_)
    ));
    // 合法但不存在 ⇒ NotFound（调用方决定 SPA 回落）
    assert!(matches!(
        resolve_request(&root, "/assets/nope.js"),
        WebFile::NotFound
    ));
    // 根 / 目录 ⇒ NotFound（不是文件）
    assert!(matches!(resolve_request(&root, "/"), WebFile::NotFound));
    assert!(matches!(resolve_request(&root, "/assets"), WebFile::NotFound));
}

#[test]
fn 静态解析_路径穿越一律拒绝() {
    let root = tmp_web_root("trav");
    for bad in [
        "/../Cargo.toml",
        "/../../server/src/main.rs",
        "/assets/../../Cargo.toml",
        "/assets/./../../Cargo.toml",
        "/..%2fCargo.toml",
        "/%2e%2e/%2e%2e/server/src/main.rs",
        "/..\\..\\server\\src\\main.rs",
        "/%zz",
        "/%",
        "/%2",
    ] {
        assert!(
            matches!(resolve_request(&root, bad), WebFile::Forbidden),
            "必须拒绝: {bad}"
        );
    }
}

#[cfg(windows)]
#[test]
fn 静态解析_windows盘符前缀被拒() {
    let root = tmp_web_root("drive");
    for bad in ["/C:/Windows/win.ini", "/C:\\Windows\\win.ini"] {
        assert!(
            matches!(resolve_request(&root, bad), WebFile::Forbidden),
            "必须拒绝: {bad}"
        );
    }
}

// ── 处理器（直接调 serve，不经网络） ────────────────────────────────────

#[tokio::test]
async fn 处理_api路径绝不被spa回落() {
    let root = tmp_web_root("apiguard");
    for p in ["/api", "/api/", "/api/nope", "/api/mcp/nope"] {
        let resp = serve(root.clone(), req("GET", p)).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{p} 必须走 API 404");
        let body = read_body(resp).await;
        assert!(
            !body.contains("root"),
            "{p} 的 404 不得回落到 index.html: {body}"
        );
    }
}

#[tokio::test]
async fn 处理_非get一律404() {
    let root = tmp_web_root("method");
    for m in ["POST", "PUT", "DELETE", "PATCH"] {
        let resp = serve(root.clone(), req(m, "/")).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{m} / 应 404");
    }
}

#[tokio::test]
async fn 处理_静态资源与spa回落() {
    let root = tmp_web_root("serve");
    // 存在的文件 ⇒ 200 + 正确 MIME
    let resp = serve(root.clone(), req("GET", "/assets/app.js")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "text/javascript; charset=utf-8");
    assert_eq!(read_body(resp).await, "console.log(1)");

    // index.html ⇒ text/html + no-cache
    let resp = serve(root.clone(), req("GET", "/index.html")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "text/html; charset=utf-8");
    assert_eq!(
        resp.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );

    // 不存在的 SPA 深链 ⇒ 回落 index.html
    let resp = serve(root.clone(), req("GET", "/settings/deep/link")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "text/html; charset=utf-8");
    assert!(read_body(resp).await.contains("id=\"root\""));

    // 根路径 ⇒ index.html
    let resp = serve(root.clone(), req("GET", "/")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "text/html; charset=utf-8");

    // 穿越 ⇒ 403（不是 200/404）
    let resp = serve(root.clone(), req("GET", "/../Cargo.toml")).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn 处理_head只回头不带体() {
    let root = tmp_web_root("head");
    let resp = serve(root.clone(), req("HEAD", "/index.html")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "text/html; charset=utf-8");
    assert!(read_body(resp).await.is_empty(), "HEAD 不得带体");
}

#[tokio::test]
async fn 处理_缺index时不存在路径仍404() {
    // 目录里没有 index.html ⇒ SPA 回落无处可落 ⇒ 404
    let mut d = std::env::temp_dir();
    d.push(format!("real_web_noindex_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let resp = serve(d, req("GET", "/whatever")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── 真实 TCP：验证 `with_web_fallback` 的装配（开 / 关） ────────────────

#[tokio::test]
async fn 真实tcp_托管开启_api优先_spa回落() {
    let root = tmp_web_root("http_on");
    let app = with_web_fallback(
        Router::new().route("/api/known", get(|| async { "known" })),
        Some(root.clone()),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // 静态首页
    let r = reqwest::get(format!("http://{addr}/")).await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );

    // 静态资源（绝对路径 `/assets/...`，与 dist/index.html 的引用一致）
    let r = reqwest::get(format!("http://{addr}/assets/app.js"))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "console.log(1)");

    // SPA 深链回落
    let r = reqwest::get(format!("http://{addr}/some/spa/route"))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.text().await.unwrap().contains("id=\"root\""));

    // `/api/*` 已注册路由照常
    let r = reqwest::get(format!("http://{addr}/api/known")).await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "known");

    // `/api/*` 未注册 ⇒ API 404，**不是** index.html
    let r = reqwest::get(format!("http://{addr}/api/definitely-missing"))
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert!(!r.text().await.unwrap().contains("id=\"root\""));

    server.abort();
}

#[tokio::test]
async fn 真实tcp_托管关闭_默认404() {
    // 关闭 = 不注册回落 ⇒ 非 /api 路径走 axum 默认 404（与未加本模块时一致）
    let app = with_web_fallback(
        Router::new().route("/api/known", get(|| async { "known" })),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let r = reqwest::get(format!("http://{addr}/")).await.unwrap();
    assert_eq!(r.status(), 404, "默认关：/ 必须 404（与现状一致）");
    let r = reqwest::get(format!("http://{addr}/index.html")).await.unwrap();
    assert_eq!(r.status(), 404, "默认关：静态路径必须 404");
    // 已有的 API 路由不受影响
    let r = reqwest::get(format!("http://{addr}/api/known")).await.unwrap();
    assert_eq!(r.status(), 200);

    server.abort();
}
