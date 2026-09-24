//! 路由装配

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

use crate::state::AppState;
use axum::http::{header, Method};
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub fn router() -> Router<AppState> {
    // CORS：来源按 origin_guard 白名单判定，方法/头收窄到实际使用集合。
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |origin: &axum::http::HeaderValue, _parts| {
                origin_guard::is_allowed_origin(origin.to_str().ok())
            },
        ))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT]);

    Router::new()
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
        .route("/api/sessions/{id}/chat", post(chat::chat))
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
        .merge(crate::sse::routes())
        .layer(cors)
        // 最后挂的层最外层：Origin 守卫先于路由与 CORS 执行，非白名单来源直接 403。
        .layer(axum::middleware::from_fn(origin_guard::guard))
}
