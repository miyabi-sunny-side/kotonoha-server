use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::llm::LlmError;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Llm(LlmError),
    Internal(String),
}

impl From<LlmError> for AppError {
    fn from(e: LlmError) -> Self {
        AppError::Llm(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Llm(e) => (StatusCode::BAD_GATEWAY, e.to_string()),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        if status.is_server_error() {
            tracing::warn!(%status, %message, "request failed");
        }
        (status, Json(json!({ "error": message }))).into_response()
    }
}
