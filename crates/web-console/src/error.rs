use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub type ConsoleResult<T> = std::result::Result<T, ConsoleError>;

#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("call engine error: {0}")]
    CallEngine(#[from] rvoip_call_engine::CallCenterError),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// Unified JSON response envelope per prx-sip spec
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: u16,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    pub request_id: String,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T, request_id: String) -> Self {
        Self {
            code: 200,
            message: "ok".to_string(),
            data: Some(data),
            request_id,
        }
    }
}

impl ApiResponse<()> {
    pub fn error(code: u16, message: String, request_id: String) -> Self {
        Self {
            code,
            message,
            data: None,
            request_id,
        }
    }
}

impl IntoResponse for ConsoleError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ConsoleError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ConsoleError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ConsoleError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            ConsoleError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            ConsoleError::CallEngine(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            ConsoleError::Anyhow(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };

        let body = ApiResponse::<()>::error(
            status.as_u16(),
            message,
            uuid::Uuid::new_v4().to_string(),
        );

        (status, axum::Json(body)).into_response()
    }
}
