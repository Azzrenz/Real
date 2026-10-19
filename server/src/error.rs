//! 类型化错误层级（工程纪律 #5）：全部错误有类型、有日志、有统一响应格式

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("数据库错误: {0}")]
    Db(#[from] sqlx::Error),

    #[error("LLM 调用失败: {0}")]
    Llm(String),

    #[error("工具调用失败: {0}")]
    Tool(String),

    #[error("资源未找到: {0}")]
    NotFound(String),

    #[error("参数校验失败: {0}")]
    Validation(String),

    #[error("未授权: {0}")]
    Unauthorized(String),

    #[error("禁止访问: {0}")]
    Forbidden(String),

    #[error("请求过于频繁: {0}")]
    TooManyRequests(String),

    #[error("HTTP 请求失败: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("内部错误: {0}")]
    Internal(String),
}

/// 传输类错误判定：SSE 读取/流式网络错误可重试；
///
/// 从 `agent/orchestration/workflow.rs` 归位——它只认 `AppError`，属错误域know-how，
/// 不该由主循环随身携带（调用点保持自由函数形态，故原样搬家）。
pub fn is_transport_error(e: &AppError) -> bool {
    let s = e.to_string();
    // "流异常早停"判为可重试：偶发抖动自愈，持续早停（重试 3 次）才上抛
    s.contains("SSE 读取失败") || s.contains("流式网络请求失败") || s.contains("SSE 解析失败")
        || s.contains("流异常早停")
        || is_upstream_unavailable(e)
}

/// 上游不可用（HTTP 5xx / 408 / 网络层失败）——与"模型答错了"不是一回事
pub fn is_upstream_unavailable(e: &AppError) -> bool {
    let s = e.to_string();
    if s.contains("LLM 流式 HTTP 5") || s.contains("LLM 流式 HTTP 408") || s.contains("上游繁忙") {
        return true;
    }
    matches!(e, AppError::Http(_)) || s.contains("流式网络请求失败")
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Llm(_) => StatusCode::BAD_GATEWAY,
            AppError::Tool(_) => StatusCode::BAD_GATEWAY,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            AppError::Http(_) => StatusCode::BAD_GATEWAY,
            AppError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// 统一错误响应：{title, status, detail, request_id?}（对齐 fullstack-dev 全局错误处理器）
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let detail = self.to_string();

        // 编程/未知错误：记日志，客户端只拿通用信息
        if matches!(
            self,
            AppError::Db(_) | AppError::Internal(_) | AppError::Config(_)
        ) {
            tracing::error!(error = %detail, "internal server error");
        } else {
            tracing::warn!(error = %detail, "operational error");
        }

        let body = Json(json!({
            "title": "error",
            "status": status.as_u16(),
            "detail": detail,
        }));
        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
