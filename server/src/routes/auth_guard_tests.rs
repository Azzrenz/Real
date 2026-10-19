//! auth_guard.rs 的测试外置（测试不进核心文件）。
//! 覆盖：回环判定 / 鉴权覆盖范围 / 豁免名单 / 仅本机接口判定 / 请求头取 token / 票据与会话 id 解析 / 判定矩阵。

use super::*;
use axum::http::HeaderValue;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            HeaderValue::from_str(v).unwrap(),
        );
    }
    h
}

fn addr(s: &str) -> Option<SocketAddr> {
    Some(s.parse().unwrap())
}

// ── 回环判定（桌面零变化的关键） ────────────────────────────────────────

#[test]
fn 回环地址_判定() {
    assert!(is_loopback(addr("127.0.0.1:1234")), "127.0.0.1 必须算回环");
    assert!(is_loopback(addr("127.5.6.7:80")), "整个 127/8 都是回环");
    assert!(is_loopback(addr("[::1]:80")), "IPv6 ::1 必须算回环");
    assert!(!is_loopback(addr("192.168.1.5:80")), "局域网地址不是回环");
    assert!(!is_loopback(addr("10.0.0.9:80")), "局域网地址不是回环");
    assert!(!is_loopback(addr("0.0.0.0:80")), "通配地址不是回环");
    assert!(!is_loopback(addr("8.8.8.8:80")), "公网地址不是回环");
}

#[test]
fn 缺失对端地址_不当回环_失败关闭() {
    // 取不到真实来源时不能默认放行（宁可拒，也不能把整个服务裸奔）
    assert!(!is_loopback(None));
}

// ── 鉴权覆盖范围 ────────────────────────────────────────────────────────

#[test]
fn 鉴权覆盖_api下全部_含_sse端点() {
    for p in [
        "/api/sessions",
        "/api/sessions/abc/events",
        "/api/sessions/abc/confirm",
        "/api/settings",
        "/api/mcp/servers/apply",
    ] {
        assert!(needs_auth(p), "{p} 必须被鉴权覆盖");
    }
    // 健康检查不在覆盖内（它们自己按鉴权结果决定详细程度）
    // 静态前端资源也不在覆盖内：手机必须能**免 token**拿到界面（`REAL_SERVE_WEB` 托管的前置条件）。
    // 端口被静态遮不住：只要 `/api/` 前缀仍在鉴权内即可。
    for p in [
        "/health",
        "/ready",
        "/",
        "/index.html",
        "/assets/index-abc.js",
        "/favicon.ico",
    ] {
        assert!(!needs_auth(p), "{p} 不应被鉴权覆盖");
    }
}

#[test]
fn 豁免名单_只有配对入口() {
    assert!(is_exempt_path("/api/auth/pair"));
    for p in [
        "/api/auth/pair/code",
        "/api/auth/sse-ticket",
        "/api/auth/devices",
        "/api/sessions",
    ] {
        assert!(!is_exempt_path(p), "{p} 不得豁免");
    }
}

// ── 仅本机接口 ──────────────────────────────────────────────────────────

#[test]
fn 仅本机接口_判定() {
    // MCP 写入面 = 直接 RCE，全部仅本机
    assert!(is_local_only(&Method::PUT, "/api/mcp/servers"));
    assert!(is_local_only(&Method::POST, "/api/mcp/servers/apply"));
    assert!(is_local_only(&Method::POST, "/api/mcp/servers/foo/reconnect"));
    // 配对码生成仅本机（否则远程能自配对）
    assert!(is_local_only(&Method::POST, "/api/auth/pair/code"));
}

#[test]
fn 仅本机接口_不误伤读与其它写() {
    // 读 MCP 配置不是仅本机（远程持 token 可读）
    assert!(!is_local_only(&Method::GET, "/api/mcp/servers"));
    // apply 只有 POST 才算；GET 不存在但不得误判为仅本机
    assert!(!is_local_only(&Method::GET, "/api/mcp/servers/apply"));
    // reconnect 只有 POST 才算
    assert!(!is_local_only(&Method::GET, "/api/mcp/servers/foo/reconnect"));
    // 配对本身不是仅本机（它就是要让手机能调）
    assert!(!is_local_only(&Method::POST, "/api/auth/pair"));
    // settings 是**字段级**收口（base_url），不能整端点锁死（否则手机改不了模型）
    assert!(!is_local_only(&Method::PUT, "/api/settings"));
    // 会话/聊天等正常远程可用
    assert!(!is_local_only(&Method::POST, "/api/sessions/abc/confirm"));
}

// ── 请求头取 token ──────────────────────────────────────────────────────

#[test]
fn 取token_支持_cookie与_bearer() {
    assert_eq!(
        device_token(&headers(&[("cookie", "real_device=tok1; other=1")])).as_deref(),
        Some("tok1")
    );
    assert_eq!(
        device_token(&headers(&[("cookie", "a=1; real_device=tok2")])).as_deref(),
        Some("tok2")
    );
    assert_eq!(
        device_token(&headers(&[("authorization", "Bearer tok3")])).as_deref(),
        Some("tok3")
    );
    assert_eq!(
        device_token(&headers(&[("authorization", "bearer tok4")])).as_deref(),
        Some("tok4")
    );
}

#[test]
fn 取token_缺失或空值返回_none() {
    assert!(device_token(&HeaderMap::new()).is_none());
    assert!(device_token(&headers(&[("cookie", "real_device=")])).is_none());
    assert!(device_token(&headers(&[("cookie", "other=1")])).is_none());
    assert!(device_token(&headers(&[("authorization", "Basic abc")])).is_none());
    assert!(device_token(&headers(&[("authorization", "Bearer   ")])).is_none());
}

// ── SSE 票据与会话 id 解析 ──────────────────────────────────────────────

#[test]
fn 从query取票据() {
    assert_eq!(
        ticket_from_query(Some("after=3&ticket=abc")).as_deref(),
        Some("abc")
    );
    assert_eq!(ticket_from_query(Some("ticket=xyz")).as_deref(), Some("xyz"));
    assert!(ticket_from_query(None).is_none());
    assert!(ticket_from_query(Some("after=1")).is_none());
    assert!(ticket_from_query(Some("ticket=")).is_none());
}

#[test]
fn 解析_sse路径的会话_id() {
    assert_eq!(
        events_session_id("/api/sessions/abc-123/events").as_deref(),
        Some("abc-123")
    );
    for p in [
        "/api/sessions/abc/events/before",
        "/api/sessions/abc/confirm",
        "/api/sessions",
        "/api/sessions/abc",
        "/health",
        "/api/sessions/abc/events/extra",
    ] {
        assert!(events_session_id(p).is_none(), "{p} 不该解析出会话 id");
    }
}

// ── 判定矩阵（穷举） ────────────────────────────────────────────────────

#[test]
fn 回环来源_一律放行() {
    // ① 回环放行优先于一切（包括"仅本机"接口）——桌面壳零变化
    for exempt in [false, true] {
        for authed in [false, true] {
            for local_only in [false, true] {
                assert_eq!(
                    verdict(true, exempt, authed, local_only),
                    Verdict::Allow,
                    "回环必须放行 exempt={exempt} authed={authed} local_only={local_only}"
                );
            }
        }
    }
}

#[test]
fn 非回环_未鉴权_统一404() {
    // ② 非回环 + 未鉴权 ⇒ 一律 404（不区分路径是否存在、不暴露服务存在）
    for local_only in [false, true] {
        assert_eq!(
            verdict(false, false, false, local_only),
            Verdict::NotFound,
            "非回环未鉴权必须 404 local_only={local_only}"
        );
    }
}

#[test]
fn 非回环_已鉴权_常规放行() {
    // ③ 有效 token ⇒ 常规接口放行
    assert_eq!(verdict(false, false, true, false), Verdict::Allow);
}

#[test]
fn 非回环_已鉴权_仅本机接口仍拒绝() {
    // ④ 有效 token 也进不了"仅本机"接口
    assert_eq!(verdict(false, false, true, true), Verdict::LocalOnly);
}

#[test]
fn 非回环_豁免路径_放行() {
    // ⑤ 唯一未鉴权入口（配对）
    assert_eq!(verdict(false, true, false, false), Verdict::Allow);
}

// ── 生效客户端地址：可信本机代理（REAL_TRUST_LOCAL_PROXY） ────────────────

#[test]
fn 开关_解析_默认关且容错() {
    // 未设置 / 空 / 0 / 乱值 ⇒ 关（默认关是硬要求：桌面零变化）
    for off in [None, Some(""), Some("  "), Some("0"), Some("false"), Some("no"), Some("off"), Some("2"), Some("x")] {
        assert!(!parse_bool_flag(off), "应判为关: {off:?}");
    }
    for on in ["1", "true", "TRUE", "Yes", " on ", "ON"] {
        assert!(parse_bool_flag(Some(on)), "应判为开: {on}");
    }
}

// ① 开关开 + 回环 peer + XFF 带非回环 ⇒ 判远程（tailscale serve 核心场景）
#[test]
fn 开关开_回环peer_xff非回环_判远程() {
    assert!(
        !client_is_loopback(addr("127.0.0.1:5555"), Some("192.168.1.90"), true),
        "开关开 + 回环 peer + XFF 非回环 ⇒ 必须走鉴权"
    );
    assert!(!client_is_loopback(addr("127.0.0.1:5555"), Some("::1, 100.64.0.9"), true));
    // 末尾非回环才采信（最左客户端可控）
    assert!(!client_is_loopback(addr("127.0.0.1:5555"), Some("127.0.0.1, 100.64.0.9"), true));
}

// ② 开关开 + 回环 peer + **无 XFF** ⇒ 本机直连的正常形态 ⇒ 回环（放行，桌面零变化）
#[test]
fn 开关开_回环peer_无xff_判回环() {
    for none in [None, Some(""), Some("   "), Some(",")] {
        assert!(
            client_is_loopback(addr("127.0.0.1:5555"), none, true),
            "无 XFF 必须判回环（本机直连）：{none:?}"
        );
    }
}

// ③ 【P0 修复】开关开 + 回环 peer + XFF 存在但**无非回环** ⇒ 失败关闭，判远程
#[test]
fn 开关开_回环peer_xff全回环_判远程_失败关闭() {
    for spoof in [
        Some("127.0.0.1"),
        Some("::1"),
        Some("127.0.0.1, ::1"),
        Some("127.0.0.1, ::1, 127.0.0.2"),
        Some("127.0.0.1,,::1"), // 含空段
        Some(" not-an-ip "),    // 有头但解析不出
        Some("127.0.0.1, garbage"),
    ] {
        assert!(
            !client_is_loopback(addr("127.0.0.1:5555"), spoof, true),
            "有 XFF 但无非回环 ⇒ 必须判远程（失败关闭），不得回退 peer：{spoof:?}"
        );
    }
}

// ④ 开关关 ⇒ 上述所有输入都回到「回环」（与不启用时逐字节一致，桌面零变化）
#[test]
fn 开关关_任何xff_一律判回环() {
    for xff in [
        None,
        Some(""),
        Some("192.168.1.90"),
        Some("127.0.0.1"),
        Some("::1, 127.0.0.1"),
        Some("not-an-ip"),
    ] {
        assert!(
            client_is_loopback(addr("127.0.0.1:5555"), xff, false),
            "开关关必须判回环：{xff:?}"
        );
    }
}

// ⑤ 防绕过：**非回环 peer 带任意 XFF ⇒ 一律判远程**（开关也救不了）
#[test]
fn 非回环peer_带xff_不读xff防绕过() {
    let peer = addr("203.0.113.7:443");
    for trust in [false, true] {
        for xff in [None, Some("127.0.0.1"), Some("127.0.0.1, ::1"), Some("10.0.0.1")] {
            assert!(
                !client_is_loopback(peer, xff, trust),
                "非回环 peer 必须判远程，绝不读 XFF：trust={trust} xff={xff:?}"
            );
        }
    }
}

// ⑥ 三态解析本身：Absent / Client（取最后一个非回环）/ Unusable
#[test]
fn 解析_xff_三态() {
    // Absent：无头 / 空头 / 只有分隔符
    for a in [None, Some(""), Some("   "), Some(","), Some(" , , ")] {
        assert_eq!(parse_xff(a), Xff::Absent, "应为 Absent：{a:?}");
    }
    let c = |s: &str| Xff::Client(s.parse::<SocketAddr>().unwrap());
    // Client：取**最后一个非回环**；最左客户端可控不采信
    assert_eq!(parse_xff(Some("1.2.3.4, 10.0.0.5")), c("10.0.0.5:0"));
    assert_eq!(parse_xff(Some("1.2.3.4, 10.0.0.5, 172.16.0.9")), c("172.16.0.9:0"));
    // 末尾是回环 ⇒ 取左边那个非回环（回环不采信）
    assert_eq!(parse_xff(Some("1.2.3.4, 127.0.0.1")), c("1.2.3.4:0"));
    // 带端口 / 带引号 / 带空段 / 混 IPv6
    assert_eq!(parse_xff(Some("\"1.2.3.4:5678\", , 10.0.0.5:99")), c("10.0.0.5:99"));
    assert_eq!(parse_xff(Some("[2001:db8::1]:80")), c("[2001:db8::1]:80"));
    // Unusable：有头但**无任何非回环**
    for u in [
        Some("127.0.0.1"),
        Some("::1"),
        Some("127.0.0.1, ::1, 127.0.0.2"),
        Some("not-an-ip"),
        Some("127.0.0.1, garbage"),
    ] {
        assert_eq!(parse_xff(u), Xff::Unusable, "应为 Unusable：{u:?}");
    }
}

#[test]
fn 缺peer_时一律不当回环_开关也不救() {
    assert!(!client_is_loopback(None, Some("1.2.3.4"), true));
    assert!(!client_is_loopback(None, None, true));
    assert!(!client_is_loopback(None, None, false));
}

#[test]
fn 从请求头读xff() {
    assert_eq!(
        forwarded_for(&headers(&[("x-forwarded-for", "1.2.3.4, 10.0.0.5")])),
        Some("1.2.3.4, 10.0.0.5")
    );
    assert!(forwarded_for(&HeaderMap::new()).is_none());
}

// ── 仅本机字段（data_dir） ──────────────────────────────────────────────

#[test]
fn 仅本机字段_远程改data_dir被拒_本机放行() {
    // 远程改 data_dir ⇒ 数据根被换 = 重启后数据丢失 ⇒ 403
    assert_eq!(local_only_field(true, true), Some("data_dir"));
    // 远程改其它字段 ⇒ 放行
    assert_eq!(local_only_field(true, false), None);
    // 本机改 data_dir ⇒ 放行（桌面体验零变化）
    assert_eq!(local_only_field(false, true), None);
    assert_eq!(local_only_field(false, false), None);
}

#[test]
fn 仅本机字段_开关开时的本机代理口径() {
    // 远程经本机代理进来：生效地址非回环 ⇒ remote=true ⇒ data_dir 收口生效
    let remote = !client_is_loopback(addr("127.0.0.1:5555"), Some("192.168.1.90"), true);
    assert!(remote);
    assert_eq!(local_only_field(remote, true), Some("data_dir"));
    // 真本机（无 XFF）：生效地址回环 ⇒ remote=false ⇒ 放行
    let local = !client_is_loopback(addr("127.0.0.1:5555"), None, true);
    assert!(!local);
    assert_eq!(local_only_field(local, true), None);
    // 【P0】本机代理若原样透传客户端的**全回环伪造头**：也判远程 ⇒ data_dir 同样收口
    let spoofed = !client_is_loopback(addr("127.0.0.1:5555"), Some("127.0.0.1"), true);
    assert!(spoofed, "有 XFF 但无非回环 ⇒ 必须判远程（失败关闭）");
    assert_eq!(local_only_field(spoofed, true), Some("data_dir"));
}

// ── base_url 真白名单（枚举式，不是排除式） ──────────────────────────────

/// 测试用白名单：只认档案里的这两条官方地址。
fn fake_catalog() -> impl Fn(&str) -> bool {
    |u: &str| u == "https://api.deepseek.com" || u == "https://open.bigmodel.cn/api/paas/v4"
}

#[test]
fn base_url_远程改成白名单内_放行() {
    // 手机跨厂商切模型：前端会把目标厂商档案里的 base_url 一起发上来 ⇒ 必须放行
    for ok in [
        "https://api.deepseek.com",
        "https://open.bigmodel.cn/api/paas/v4",
    ] {
        assert_eq!(
            check_base_url_change(true, Some(ok), fake_catalog()),
            Ok(()),
            "白名单内的地址应放行: {ok}"
        );
    }
}

#[test]
fn base_url_远程改成白名单外_一律拒() {
    // 攻击者域名（原漏洞的核心场景：既不是"别家官方 host"，格式也合法）
    // + 其它各种"看起来像"的变体，全部必须拒（枚举式，不做前缀/后缀/子串匹配）
    for bad in [
        "https://evil.tld/v1",
        "https://api.deepseek.com.evil.tld",
        "https://api.deepseek.com/sneaky",
        "https://api.deepseek.com:8443",
        "http://api.deepseek.com",
        "https://open.bigmodel.cn/api/paas/v4/../..",
        "not-a-url",
        "",
    ] {
        assert_eq!(
            check_base_url_change(true, Some(bad), fake_catalog()),
            Err(BaseUrlDeny::NotInCatalog),
            "白名单外的地址必须拒: {bad}"
        );
    }
}

#[test]
fn base_url_本机随便改_放行() {
    // 本机不受白名单限制（连攻击者域名都放行 —— 本机用户的自由，桌面零变化）
    for any in [
        "https://evil.tld/v1",
        "http://192.168.1.5:8080/v1",
        "https://my-own-proxy.internal/v4",
        "",
    ] {
        assert_eq!(
            check_base_url_change(false, Some(any), |_| false),
            Ok(()),
            "本机应放行: {any}"
        );
    }
}

#[test]
fn base_url_回显不算改动_放行() {
    // 前端把"当前" base_url 原样回显 ⇒ 不是新选择（尾 `/` 等价）
    assert!(is_base_url_echo("https://api.deepseek.com", "https://api.deepseek.com"));
    assert!(is_base_url_echo("https://api.deepseek.com/", "https://api.deepseek.com"));
    assert!(is_base_url_echo("https://api.deepseek.com", "https://api.deepseek.com/"));
    assert!(!is_base_url_echo("https://evil.tld/v1", "https://api.deepseek.com"));
    // 回显被上层滤成 None ⇒ 授权入口直接放行（哪怕白名单为空、来源是远程）
    assert_eq!(check_base_url_change(true, None, |_| false), Ok(()));
}

#[test]
fn base_url_归一化_尾斜杠与大小写视为同一地址() {
    assert_eq!(
        crate::model::catalog::normalize_base_url("HTTPS://API.DeepSeek.COM/"),
        "https://api.deepseek.com"
    );
    assert_eq!(
        crate::model::catalog::normalize_base_url("https://api.deepseek.com"),
        "https://api.deepseek.com"
    );
    assert_eq!(crate::model::catalog::normalize_base_url(""), "");
}

#[test]
fn base_url_白名单来源是厂商档案_且攻击者域名不在内() {
    // 集合来源 = 厂商档案（providers/*.json 经 catalog 加载的权威值）
    let known = crate::model::catalog::known_base_urls();
    for url in &known {
        assert!(
            crate::model::catalog::is_known_base_url(url),
            "档案里的 base_url 必须判为已知: {url}"
        );
        // 尾斜杠变体同样算已知（归一化口径）
        assert!(crate::model::catalog::is_known_base_url(&format!("{url}/")));
    }
    // 负例：攻击者域名 / 空值 / 前缀伪造
    for bad in [
        "https://evil.tld/v1",
        "",
        "https://not-a-provider.invalid",
    ] {
        assert!(
            !crate::model::catalog::is_known_base_url(bad),
            "非档案地址不得判为已知: {bad}"
        );
    }
    for url in &known {
        assert!(
            !crate::model::catalog::is_known_base_url(&format!("{url}/sneaky")),
            "前缀伪造不得判为已知: {url}/sneaky"
        );
    }
}

// ── 连接信息（ConnectInfo）真的被填充：桌面零变化的根 ───────────────────
//
// `is_loopback` 依赖 `ConnectInfo<SocketAddr>` 这个请求扩展。若 `main.rs` 的
// `axum::serve` 忘了 `into_make_service_with_connect_info`，扩展缺失 ⇒ 所有请求都被
// 当成"非回环" ⇒ 连桌面壳也一起被 404。本测试用一个真实 TCP 回环连接证明这条接线成立。
#[tokio::test]
async fn connect_info_被填充_回环客户端判定为回环() {
    use axum::routing::get;
    use axum::Router;
    use std::sync::{Arc, Mutex};

    let observed: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
    let observed_in = observed.clone();

    let app = Router::new()
        .route("/api/probe", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(
            move |req: Request, next: Next| {
                let observed = observed_in.clone();
                async move {
                    *observed.lock().unwrap() = Some(is_loopback(peer_addr(&req)));
                    next.run(req).await
                }
            },
        ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let resp = reqwest::get(format!("http://{addr}/api/probe"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");

    assert_eq!(
        *observed.lock().unwrap(),
        Some(true),
        "serve 带 connect info 时，回环客户端的来源必须判定为回环"
    );
    server.abort();
}
