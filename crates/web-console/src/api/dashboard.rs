//! Dashboard aggregation endpoint.

use axum::{Router, routing::get, extract::State, Json};
use serde::Serialize;
use rvoip_call_engine::SupervisorApi;

use crate::error::{ApiResponse, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct DashboardMetrics {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub available_agents: usize,
    pub busy_agents: usize,
    pub queued_calls: usize,
    pub total_calls_handled: u64,
    pub sip_registrations: usize,
}

async fn get_dashboard(
    State(state): State<AppState>,
) -> ConsoleResult<Json<ApiResponse<DashboardMetrics>>> {
    let supervisor = SupervisorApi::new(state.engine.clone());
    let stats = supervisor.get_stats().await;

    let sip_registrations = if let Some(ref registrar) = state.registrar {
        registrar.list_registered_users().await.len()
    } else {
        0
    };

    let metrics = DashboardMetrics {
        active_calls: stats.active_calls,
        active_bridges: stats.active_bridges,
        available_agents: stats.available_agents,
        busy_agents: stats.busy_agents,
        queued_calls: stats.queued_calls,
        total_calls_handled: stats.total_calls_handled,
        sip_registrations,
    };

    Ok(Json(ApiResponse::success(
        metrics,
        uuid::Uuid::new_v4().to_string(),
    )))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_dashboard))
}
