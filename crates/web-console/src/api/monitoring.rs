//! Real-time monitoring and alerting endpoints.
//!
//! Alerts are computed from live engine stats — no mock data.

use axum::{Router, routing::get, extract::{State, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::SupervisorApi;

use crate::auth::{AuthUser, require_role, ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleResult};
use crate::server::AppState;

fn rid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RealtimeStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub available_agents: usize,
    pub busy_agents: usize,
    pub queued_calls: usize,
    pub total_calls_handled: u64,
    pub routing_stats: RoutingStatsView,
}

#[derive(Debug, Serialize)]
pub struct RoutingStatsView {
    pub calls_routed_directly: u64,
    pub calls_queued: u64,
    pub calls_rejected: u64,
}

#[derive(Debug, Serialize)]
pub struct AlertView {
    pub id: String,
    pub severity: String,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct EventsQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct EventView {
    pub event_type: String,
    pub timestamp: String,
    pub data: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn realtime_stats(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<RealtimeStats>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let supervisor = SupervisorApi::new(state.engine.clone());
    let stats = supervisor.get_stats().await;

    let view = RealtimeStats {
        active_calls: stats.active_calls,
        active_bridges: stats.active_bridges,
        available_agents: stats.available_agents,
        busy_agents: stats.busy_agents,
        queued_calls: stats.queued_calls,
        total_calls_handled: stats.total_calls_handled,
        routing_stats: RoutingStatsView {
            calls_routed_directly: stats.routing_stats.calls_routed_directly,
            calls_queued: stats.routing_stats.calls_queued,
            calls_rejected: stats.routing_stats.calls_rejected,
        },
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn alerts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<AlertView>>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let supervisor = SupervisorApi::new(state.engine.clone());
    let stats = supervisor.get_stats().await;
    let config = state.engine.config();
    let now = chrono::Utc::now().to_rfc3339();

    let mut live_alerts: Vec<AlertView> = Vec::new();

    // Alert: high queue depth (>10 queued calls)
    if stats.queued_calls > 10 {
        live_alerts.push(AlertView {
            id: rid(),
            severity: "warning".to_string(),
            message: format!("High queue depth: {} calls waiting", stats.queued_calls),
            timestamp: now.clone(),
        });
    }

    // Alert: no available agents
    if stats.available_agents == 0 && stats.busy_agents > 0 {
        live_alerts.push(AlertView {
            id: rid(),
            severity: "critical".to_string(),
            message: "No agents available — all agents are busy".to_string(),
            timestamp: now.clone(),
        });
    } else if stats.available_agents == 0 && stats.busy_agents == 0 {
        live_alerts.push(AlertView {
            id: rid(),
            severity: "critical".to_string(),
            message: "No agents online".to_string(),
            timestamp: now.clone(),
        });
    }

    // Alert: approaching system capacity (>80% of max_concurrent_calls)
    let max_calls = config.general.max_concurrent_calls;
    if max_calls > 0 {
        let threshold = (max_calls as f64 * 0.8) as usize;
        if stats.active_calls > threshold {
            live_alerts.push(AlertView {
                id: rid(),
                severity: "warning".to_string(),
                message: format!(
                    "System capacity warning: {}/{} concurrent calls ({}%)",
                    stats.active_calls,
                    max_calls,
                    (stats.active_calls * 100) / max_calls,
                ),
                timestamp: now.clone(),
            });
        }
    }

    // If nothing is alarming, emit an informational status
    if live_alerts.is_empty() {
        live_alerts.push(AlertView {
            id: rid(),
            severity: "info".to_string(),
            message: "System operating normally".to_string(),
            timestamp: now,
        });
    }

    Ok(Json(ApiResponse::success(live_alerts, rid())))
}

async fn events(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<EventsQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<EventView>>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let _limit = params.limit.unwrap_or(50);

    // Read recent events from the activity tracker as a lightweight proxy
    let tracker = state.activity_tracker.read();
    let mut event_views = Vec::new();

    // Produce a summary event per hour that had activity
    for (hour, &count) in tracker.calls.iter().enumerate() {
        if count > 0 {
            event_views.push(EventView {
                event_type: "call_activity".to_string(),
                timestamp: format!("{:02}:00", hour),
                data: serde_json::json!({ "calls": count }),
            });
        }
    }

    Ok(Json(ApiResponse::success(event_views, rid())))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/realtime", get(realtime_stats))
        .route("/alerts", get(alerts))
        .route("/events", get(events))
}
