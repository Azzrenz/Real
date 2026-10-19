//! 路由装配

pub mod auth;
pub mod auth_guard;
pub mod chat;
pub mod feedback;
pub mod health;
pub mod mcp;
pub mod memory;
pub mod origin_guard;
pub mod reveal;
pub mod schedule;
pub mod self_improve;
pub mod sessions;
pub mod settings;
pub mod skills;
pub mod web;

use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::http::{header, Method};
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};

/// 装配路由并落到具体状态（`with_state`）：层序是本函数的一部分，必须能被一眼看全，
/// 故 `state` 作为参数传入、在这里统一收口（鉴权层需要投影 `AppState`）。
pub fn router(state: AppState) -> Router {
    // CORS：来源按 origin_guard 白名单判定，方法/头收窄到实际使用集合。
    // `allow_credentials(true)`：手机端走 HttpOnly Cookie 鉴权，跨源时必须允许带凭据。
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |origin: &axum::http::HeaderValue, _parts| {
                origin_guard::is_allowed_origin(origin.to_str().ok())
            },
        ))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT]);

    let web_root = web::startup_web_root();

    let app = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/api/sessions", get(sessions::list).post(sessions::create))
        .route("/api/feedback", post(feedback::rate))
        .route(
            "/api/sessions/{id}",
            get(sessions::get)
                .patch(sessions::update)
                .delete(sessions::delete),
        )
        // ⚠️ 附件（图片/文档）以 base64 内联在 JSON 里，体积天然是原文件的约 1.37 倍，
        // 而 axum 默认的 `DefaultBodyLimit` 只有 2MB —— 结果是**超过约 1.4MB 的原图
        // 在进入 handler 之前就被 413 挡掉**（`Failed to buffer the request body`），
        // 业务侧连一条日志都不会有，表现为「附件发不出去」。
        // 前端单文件上限 8MB（`attachments.ts: MAX_FILE_BYTES`）⇒ base64 ≈10.7MB，
        // 取 16MB 留出 JSON 包装与多附件的余量。**只放开这一条路由**，其余接口维持默认。
        .route(
            "/api/sessions/{id}/chat",
            post(chat::chat).layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route("/api/sessions/{id}/interject", post(sessions::interject))
        .route("/api/sessions/{id}/interjections", get(sessions::interjections))
        .route("/api/skills", get(skills::list_skills).post(skills::create_skill))
        .route("/api/skills/{name}", delete(skills::delete_skill))
        .route("/api/skills/{name}/rename", post(skills::rename_skill))
        .route("/api/skills/absorb", post(skills::absorb_candidate))
        .route("/api/skills/{name}/open", post(skills::open_skill_dir))
        .route("/api/reveal", post(reveal::reveal_path))
        .route("/api/skills/{name}/raw", get(skills::skill_raw))
        .route("/api/sessions/{id}/thinking", get(sessions::thinking))
        .route("/api/sessions/{id}/events/before", get(sessions::events_before))
        .route("/api/sessions/{id}/cancel", post(chat::cancel))
        .route("/api/sessions/{id}/confirm", post(chat::confirm))
        .route("/api/self-improve/events", post(self_improve::push_event))
        .route("/api/self-improve/status", post(self_improve::push_status))
        .route("/api/settings", get(settings::get).put(settings::put))
        .route("/api/mcp/servers", get(mcp::list).put(mcp::save))
        .route("/api/mcp/servers/apply", post(mcp::apply))
        .route("/api/mcp/servers/{name}/reconnect", post(mcp::reconnect))
        // 设备鉴权：配对（唯一未鉴权入口 / 仅本机的码生成）与设备管理
        .route("/api/auth/pair/code", post(auth::pair_code))
        .route("/api/auth/pair", post(auth::pair))
        .route("/api/auth/sse-ticket", post(auth::sse_ticket))
        .route("/api/auth/devices", get(auth::list_devices))
        .route("/api/auth/devices/{id}", delete(auth::delete_device))
        .route("/api/settings/test", post(settings::test))
        .route("/api/memory/persona", get(memory::persona_get).put(memory::persona_put))
        .route(
            "/api/memory/preferences",
            get(memory::prefs_get).post(memory::prefs_post).delete(memory::prefs_delete),
        )
        .route("/api/memory/themes", get(memory::themes_get))
        .route("/api/memory/themes/pin", post(memory::theme_pin))
        .route("/api/memory/themes/{id}", delete(memory::theme_delete))
        .route("/api/memory/digests", get(memory::digests_get))
        .route("/api/memory/files", get(memory::files_get))
        .route("/api/memory/stats", get(memory::stats_get))
        .route("/api/schedules", get(schedule::list).post(schedule::create))
        .route("/api/schedules/{id}", delete(schedule::delete))
        .route("/api/schedules/{id}/toggle", post(schedule::toggle))
        .route("/api/schedules/{id}/run", post(schedule::run_now))
        .merge(crate::sse::routes());
    // 可选静态前端托管（`REAL_SERVE_WEB`，默认关）：`None` ⇒ 不注册回落，
    // 路由与行为与未加本模块时逐字节一致（桌面零变化）。开启时才挂 SPA 回落。
    web::with_web_fallback(app, web_root)
        .layer(cors)
        // Origin 守卫只防浏览器（不带 Origin 直接放行），因此它**不能**当鉴权，只做第一道收窄。
        .layer(axum::middleware::from_fn(origin_guard::guard))
        // 鉴权在最外层（最后挂的层最外层）：未鉴权请求在极早处被拒，
        // 且回环来源直接放行 ⇒ 桌面 Tauri 壳（打 127.0.0.1）体验零变化。
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_guard::guard,
        ))
        .with_state(state)
}
