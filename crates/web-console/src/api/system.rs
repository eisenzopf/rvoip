//! System health and status endpoints.

use axum::{Router, routing::get, Json};
use serde::Serialize;

use crate::error::{ApiResponse, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub uptime_secs: u64,
    pub version: String,
}

async fn health() -> ConsoleResult<Json<ApiResponse<HealthStatus>>> {
    let status = HealthStatus {
        status: "healthy".to_string(),
        uptime_secs: 0, // TODO: track actual uptime
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    Ok(Json(ApiResponse::success(
        status,
        uuid::Uuid::new_v4().to_string(),
    )))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
}
