//! Call activity time-series endpoint for dashboard charts.
//!
//! Returns hourly call counts for the past 24 hours.
//! In this initial implementation, we use a simple in-memory counter
//! that is updated by the event pipeline.

use axum::{Router, routing::get, extract::State, Json};
use serde::Serialize;
use chrono::{Utc, Timelike};

use crate::error::{ApiResponse, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct HourlyActivity {
    pub hour: u32,
    pub calls: u64,
    pub queued: u64,
}

#[derive(Debug, Serialize)]
pub struct ActivityResponse {
    pub hours: Vec<HourlyActivity>,
}

async fn get_activity(
    State(state): State<AppState>,
) -> ConsoleResult<Json<ApiResponse<ActivityResponse>>> {
    let tracker = state.activity_tracker.read();
    let current_hour = Utc::now().hour();

    let hours: Vec<HourlyActivity> = (0..24u32)
        .map(|offset| {
            let hour = (current_hour + 1 + offset) % 24;
            let idx = hour as usize;
            HourlyActivity {
                hour,
                calls: tracker.calls[idx],
                queued: tracker.queued[idx],
            }
        })
        .collect();

    Ok(Json(ApiResponse::success(
        ActivityResponse { hours },
        uuid::Uuid::new_v4().to_string(),
    )))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_activity))
}
