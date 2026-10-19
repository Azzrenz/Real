//! 鉴权中间件：非回环来源必须持有效设备 token（或一次性 SSE 票据）。
//!
//! 为什么必须有它：`origin_guard` 对**不带 `Origin` 头**的请求直接放行（`origin_guard.rs:58-61`），
//! 因此 Origin 守卫只能防浏览器，**不能当鉴权**——`curl`/脚本不带 Origin 即可打穿。
//!
//! 挂载位置：`routes/mod.rs` 里**最后一个 `.layer`（最外层）**，使未鉴权请求在极早处被拒。
//!
//! 放行规则（按序）：
//! 1. CORS 预检 `OPTIONS` ⇒ 交给 `CorsLayer`（预检不带凭据，不能在这里拒）；
//! 2. 非 `/api/*`（`/health`、`/ready`）不在此列（它们自己按鉴权结果决定详细程度）；
//! 3. 来源回环（`127.0.0.1` / `::1`）⇒ 放行（**桌面零变化的关键**）；
//! 4. 路径在豁免名单（`/api/auth/pair`，唯一未鉴权入口）⇒ 放行；
//! 5. 设备 token 有效 或 SSE 一次性票据有效 ⇒ 放行；
//! 6. 否则 ⇒ **统一 404**（不暴露服务存在，也不区分「路径不存在」与「未授权」）。
//!
//! 另有一类**仅本机接口**（MCP 写/apply/reconnect、配对码生成）：远程即使持有有效 token 也拒绝（403）。
//!
//! # 生效客户端地址（`REAL_TRUST_LOCAL_PROXY`）
//!
//! ⚠️ 单靠 peer 判定回环会被**本机反向代理**打穿：`tailscale serve` 自己起在 tailnet 上，
//! 再把请求转发到 `http://127.0.0.1:8943`，于是后端看到的 peer 恒为 `127.0.0.1` ⇒ 手机来的
//! 请求全部命中「回环免鉴权」⇒ 整套鉴权空转。故引入开关 `REAL_TRUST_LOCAL_PROXY`（默认 `0` 关）：
//! **仅当**（peer 是回环）**且**（开关打开）时，才按 `X-Forwarded-For` **最后一个非回环地址**
//! 还原真实客户端（最左边那个是客户端可控的，不采信）。非回环 peer **绝不读 XFF**（否则远程
//! 攻击者自己加个头就能伪装）。
//!
//! ⚠️ XFF 必须**三态**处理，不能"没有非回环就回退到 peer"（那是**失败开放**）：
//! - **无 XFF 头**（或头为空）⇒ 本机直连的**正常**形态 ⇒ 回环（放行，桌面零变化）；
//! - **有 XFF 头但取不到任何非回环地址**（全回环 / 全垃圾）⇒ 经过了代理却没拿到真实客户端
//!   ⇒ **失败关闭，判远程**（必须走鉴权）。若此处回退 peer 就成后门：本机代理若原样透传
//!   客户端的伪造头，绕过"最后一个非回环"就成了免鉴权。
//! - 有 XFF 且含非回环 ⇒ 判远程（取最后一个非回环）。
//!
//! 开关关闭时行为与不启用时**逐字节一致**（桌面零变化）。
//! 信任面没有变大：本机进程本来就被完全信任（旧的「只绑回环」模型同样如此），而"信任本机代理
//! 写的 XFF"最多只能**加重**本机调用者自己的鉴权负担 —— 远程攻击者无法把自己的 peer 变成回环。

use crate::auth;
use crate::state::AppState;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::{IpAddr, SocketAddr};

/// 可信本机代理开关的环境变量名
pub const TRUST_LOCAL_PROXY_ENV: &str = "REAL_TRUST_LOCAL_PROXY";

/// 仅本机可调的接口：返回 true ⇒ 非回环来源（即便已鉴权）也拒绝。
///
/// 覆盖 MCP 写入面（`PUT /api/mcp/servers`、`POST /api/mcp/servers/apply`、
/// `POST /api/mcp/servers/{name}/reconnect`）——这三条最终都会
/// `Command::new(command).args(args)`（`mcp/client.rs:103-128`），是直接 RCE，必须锁死本机。
/// 以及配对码生成（`POST /api/auth/pair/code`）——否则远程攻击者能自己配对。
pub fn is_local_only(method: &Method, path: &str) -> bool {
    match (method, path) {
        (&Method::PUT, "/api/mcp/servers") => true,
        (&Method::POST, "/api/mcp/servers/apply") => true,
        (&Method::POST, "/api/auth/pair/code") => true,
        // POST /api/mcp/servers/{name}/reconnect
        _ => {
            method == Method::POST
                && path.starts_with("/api/mcp/servers/")
                && path.ends_with("/reconnect")
        }
    }
}

/// 一律仅本机可改的**字段**（端点级收口见 `is_local_only`）。远程请求带了这些字段 ⇒ 返回字段名，
/// 由 handler 回 403。集中放在这里，是为了让「仅本机」这份策略只有一个可审计的落点。
///
/// `data_dir`：改数据根 = 重启后换库 ⇒ 等于数据丢失。
///
/// ⚠️ `base_url` **不在**这里：它走**枚举式白名单**（见 `check_base_url_change`），
/// 否则远程跨厂商切模型会被一律拒掉。
pub fn local_only_field(remote: bool, data_dir_provided: bool) -> Option<&'static str> {
    if remote && data_dir_provided {
        return Some("data_dir");
    }
    None
}

/// `base_url` 改动的拒绝原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseUrlDeny {
    /// 远程把 `base_url` 改成了内置厂商档案里没有的地址
    NotInCatalog,
}

impl BaseUrlDeny {
    pub fn message(self) -> &'static str {
        match self {
            BaseUrlDeny::NotInCatalog => "base_url 远程只能改为内置厂商档案中登记的地址（本机不受此限）",
        }
    }
}

/// 「前端回显」判定：切模型时前端会把**当前** `base_url` 原样带上来，那不是新选择。
/// 语义与既有实现逐字一致（只忽略尾部 `/`，不 trim 空白）——改这里等于改写入行为，别顺手"优化"。
pub fn is_base_url_echo(requested: &str, current: &str) -> bool {
    requested.trim_end_matches('/') == current.trim_end_matches('/')
}

/// `base_url` 改动的**唯一授权入口**（纯函数；白名单来源由调用方注入，便于穷举测试）：
/// - 没带新值（`None`，含前端回显被上层滤掉的情形）⇒ 不算改动 ⇒ 放行；
/// - **本机** ⇒ 放行（不受此限，桌面体验零变化）；
/// - **远程** ⇒ 仅当新值**等于**内置厂商档案里登记的 `base_url`（枚举式，见
///   `catalog::is_known_base_url`）。格式合法但不在档案里的地址（含攻击者域名）一律拒。
pub fn check_base_url_change(
    remote: bool,
    new_value: Option<&str>,
    in_catalog: impl Fn(&str) -> bool,
) -> Result<(), BaseUrlDeny> {
    let Some(url) = new_value else {
        return Ok(());
    };
    if !remote || in_catalog(url) {
        return Ok(());
    }
    Err(BaseUrlDeny::NotInCatalog)
}

/// 唯一未鉴权入口：用配对码换 token。自保靠「配对码 + 尝试上限 + 限速」（`auth::AuthStore`）。
pub fn is_exempt_path(path: &str) -> bool {
    path == "/api/auth/pair"
}

/// 鉴权覆盖范围：`/api/*` 全部（含 SSE 端点 `/api/sessions/{id}/events`）。
pub fn needs_auth(path: &str) -> bool {
    path.starts_with("/api/")
}

/// 取对端地址（`axum::serve` 必须用 `into_make_service_with_connect_info` 才会有）。
pub fn peer_addr(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
}

/// 回环判定：`127.0.0.0/8` 与 `::1`。缺失地址 ⇒ **不当回环**（失败关闭）。
pub fn is_loopback(addr: Option<SocketAddr>) -> bool {
    addr.map(|a| a.ip().is_loopback()).unwrap_or(false)
}

// ── 生效客户端地址（可信本机代理还原，见模块头注释） ──────────────────────

/// 布尔开关解析：`1/true/yes/on`（大小写不敏感、允许首尾空白）为真，其余（含未设置）为假。
pub fn parse_bool_flag(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// 是否启用「信任本机代理」（`REAL_TRUST_LOCAL_PROXY`，默认关）。
pub fn trust_local_proxy_enabled() -> bool {
    parse_bool_flag(std::env::var(TRUST_LOCAL_PROXY_ENV).ok().as_deref())
}

/// 请求头里的 `X-Forwarded-For` 原值。
pub fn forwarded_for(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
}

/// 单段 XFF 解析：接受裸 IP、`1.2.3.4:5678`、`[::1]:80`（引号由调用方剥掉）。
fn parse_xff_addr(seg: &str) -> Option<SocketAddr> {
    if let Ok(sa) = seg.parse::<SocketAddr>() {
        return Some(sa);
    }
    seg.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, 0))
}

/// `X-Forwarded-For` 的**三态**解析结果。
///
/// ⚠️ 必须区分「头不存在」与「头存在但拿不到真实客户端」：
/// 前者是本机直连的**正常**形态（合法、放行）；后者说明**经过了代理但代理没追加真实客户端**
/// （或客户端自带了一个全是回环的伪造头）——此时若按回环放行，等于给攻击者开了免鉴权后门。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xff {
    /// 请求没带 `X-Forwarded-For`，或头内容为空（无任何条目）——本机直连的正常形态。
    Absent,
    /// 带了头，且取到了**最后一个非回环**地址（其右侧才是紧邻服务端的可信代理写入的；
    /// 最左边那个客户端可控，不采信）。⇒ 判远程。
    Client(SocketAddr),
    /// 带了头，但里面**没有任何非回环地址**（全是回环 / 全是无法解析的内容）。
    /// ⇒ 经过代理却没拿到真实客户端 ⇒ **失败关闭，判远程**（必须走鉴权）。
    Unusable,
}

/// 解析 `X-Forwarded-For`（纯函数，便于穷举测试）。
pub fn parse_xff(xff: Option<&str>) -> Xff {
    let Some(raw) = xff else {
        return Xff::Absent;
    };
    let mut found: Option<SocketAddr> = None;
    let mut entries = 0usize;
    for seg in raw.split(',') {
        let seg = seg.trim().trim_matches('"');
        if seg.is_empty() {
            continue;
        }
        entries += 1;
        if let Some(addr) = parse_xff_addr(seg) {
            if !addr.ip().is_loopback() {
                found = Some(addr);
            }
        }
    }
    if let Some(addr) = found {
        return Xff::Client(addr);
    }
    if entries == 0 {
        // 头为空/只有分隔符：什么都没表达 ⇒ 与"没带头"同义（本机直连）
        Xff::Absent
    } else {
        Xff::Unusable
    }
}

/// **生效客户端是否回环**（纯函数，唯一判据）：
/// - 取不到 peer ⇒ `false`（失败关闭）；
/// - **非回环 peer ⇒ 直接判远程，绝不读 XFF**（否则远程攻击者自己加个头就能伪装）；
/// - 回环 peer + 开关**关** ⇒ `true`（与不启用时逐字节一致）；
/// - 回环 peer + 开关**开**：
///   - 无 XFF（本机直连）⇒ `true`；
///   - XFF 有非回环 ⇒ `false`（按真实客户端判远程）；
///   - XFF 存在但无任何非回环地址（全回环/全垃圾）⇒ `false`（**失败关闭**，不按回环放行）。
pub fn client_is_loopback(
    peer: Option<SocketAddr>,
    xff: Option<&str>,
    trust_local_proxy: bool,
) -> bool {
    // 取不到 peer / peer 非回环 ⇒ 直接判远程（`is_loopback` 对 None 也返回 false，即失败关闭）
    if !is_loopback(peer) {
        return false;
    }
    if !trust_local_proxy {
        return true;
    }
    match parse_xff(xff) {
        Xff::Absent => true,
        Xff::Client(_) | Xff::Unusable => false,
    }
}

/// 该请求是否来自回环（生效判据；开关按 env 实际取值）。
pub fn is_loopback_request(peer: Option<SocketAddr>, headers: &HeaderMap) -> bool {
    client_is_loopback(
        peer,
        forwarded_for(headers),
        trust_local_proxy_enabled(),
    )
}

/// 从 `Cookie: real_device=...` 或 `Authorization: Bearer ...` 取设备 token。
pub fn device_token(headers: &HeaderMap) -> Option<String> {
    if let Some(raw) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let raw = raw.trim();
        let bearer = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "));
        if let Some(tok) = bearer.map(str::trim).filter(|t| !t.is_empty()) {
            return Some(tok.to_string());
        }
    }
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())?;
    let prefix = format!("{}=", auth::DEVICE_COOKIE);
    for part in cookies.split(';') {
        let part = part.trim();
        if let Some(tok) = part.strip_prefix(&prefix) {
            let tok = tok.trim().trim_matches('"');
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    None
}

/// 从 query 串取 `ticket=`（SSE 一次性票据；`EventSource` 不能自定义 header）。
pub fn ticket_from_query(query: Option<&str>) -> Option<String> {
    for pair in query.unwrap_or("").split('&') {
        if let Some(v) = pair.strip_prefix("ticket=") {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// `/api/sessions/{id}/events` → `Some(id)`；其它路径 → `None`。
/// 会话 id 由 `Uuid::new_v4().to_string()` 生成（URL 安全字符），无需百分号解码。
pub fn events_session_id(path: &str) -> Option<String> {
    let mut segs = path.split('/').filter(|s| !s.is_empty());
    if segs.next()? != "api" {
        return None;
    }
    if segs.next()? != "sessions" {
        return None;
    }
    let id = segs.next()?;
    if segs.next()? != "events" {
        return None;
    }
    if segs.next().is_some() {
        return None;
    }
    Some(id.to_string())
}

/// 鉴权结论（纯逻辑，便于穷举测试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// 统一 404：不暴露服务存在，也不区分「路径不存在」与「未授权」
    NotFound,
    /// 403：接口存在但仅本机可调（调用方已鉴权，说明原因更友好）
    LocalOnly,
}

/// 判定核心。顺序即优先级：回环/豁免 ⇒ 放行；已鉴权 ⇒ 看是否仅本机；否则 ⇒ 404。
pub fn verdict(peer_loopback: bool, exempt: bool, authed: bool, local_only: bool) -> Verdict {
    if peer_loopback || exempt {
        return Verdict::Allow;
    }
    if authed {
        return if local_only {
            Verdict::LocalOnly
        } else {
            Verdict::Allow
        };
    }
    Verdict::NotFound
}

/// 请求是否已鉴权（回环 或 有效设备 token）。
/// `/health`、`/ready` 不在鉴权覆盖内，但用本函数决定「是否给出详细信息」。
/// `peer` 是 TCP 对端**原始**地址；是否按 XFF 还原由开关决定（见 `client_is_loopback`）。
pub async fn is_authenticated(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> bool {
    if is_loopback_request(peer, headers) {
        return true;
    }
    match device_token(headers) {
        Some(tok) => auth::verify_token(&state.pool, &tok)
            .await
            .map(|d| d.is_some())
            .unwrap_or(false),
        None => false,
    }
}

/// 守卫中间件本体。
pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // ① CORS 预检不带凭据，必须交给 CorsLayer
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    // ② /health /ready 等非 /api 路径不在鉴权覆盖内
    if !needs_auth(&path) {
        return next.run(req).await;
    }

    let method = req.method().clone();
    // 回环判定走**生效客户端地址**（可信本机代理还原），不是裸 peer
    let loopback = is_loopback_request(peer_addr(&req), req.headers());
    let exempt = is_exempt_path(&path);
    let local_only = is_local_only(&method, &path);

    let mut authed = false;
    if !loopback && !exempt {
        // ③ 设备 token（Cookie / Bearer）
        if let Some(token) = device_token(req.headers()) {
            if let Ok(Some(dev)) = auth::verify_token(&state.pool, &token).await {
                let _ = auth::touch_device(&state.pool, &dev.id, auth::now_ms()).await;
                authed = true;
            }
        }
        // ④ SSE 一次性票据（绑定会话 id，用后即弃）
        if !authed {
            if let (Some(ticket), Some(sid)) =
                (ticket_from_query(req.uri().query()), events_session_id(&path))
            {
                authed = state
                    .auth
                    .consume_sse_ticket(&ticket, &sid, auth::now_secs());
            }
        }
    }

    match verdict(loopback, exempt, authed, local_only) {
        Verdict::Allow => next.run(req).await,
        Verdict::LocalOnly => {
            tracing::warn!(path = %path, "远程请求命中仅本机接口，已拒绝");
            // 与 `error::AppError` 同一个响应信封：客户端统一读 `detail` 显示原因。
            // （404 那支刻意保持**空体**：不暴露服务存在。）
            (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({
                    "title": "error",
                    "status": StatusCode::FORBIDDEN.as_u16(),
                    "detail": "该接口仅限本机调用",
                })),
            )
                .into_response()
        }
        Verdict::NotFound => {
            tracing::warn!(path = %path, "未鉴权请求，按统一 404 拒绝");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

#[cfg(test)]
#[path = "auth_guard_tests.rs"]
mod auth_guard_tests;
