//! Unified API gateway for the web console.
//!
//! Aggregates endpoints from call-engine, registrar-core, and system health
//! into a single Axum router under `/api/v1/`.

pub mod auth;
pub mod users;
pub mod dashboard;
pub mod calls;
pub mod agents;
pub mod queues;
pub mod registrations;
pub mod activity;
pub mod routing;
pub mod system;
pub mod system_config;
pub mod presence;
pub mod monitoring;
pub mod departments;
pub mod extensions;
pub mod skills;
pub mod phone_lists;
pub mod ivr;
pub mod trunks;
pub mod schedules;
pub mod reports;
pub mod quality;
pub mod knowledge;
pub mod softphone;
pub mod ai;

use axum::Router;
use crate::server::AppState;

/// Build the complete API router.
///
/// All routes are nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/dashboard", dashboard::router())
        .nest("/dashboard/activity", activity::router())
        .nest("/calls", calls::router())
        .nest("/agents", agents::router())
        .nest("/queues", queues::router())
        .nest("/users", users::router())
        .nest("/routing", routing::router())
        .nest("/registrations", registrations::router())
        .nest("/system", system::router())
        .nest("/system/config", system_config::config_router())
        .nest("/system/audit", system_config::audit_router())
        .nest("/presence", presence::router())
        .nest("/monitoring", monitoring::router())
        .nest("/departments", departments::router())
        .nest("/extensions", extensions::router())
        .nest("/skills", skills::router())
        .nest("/phone-lists", phone_lists::router())
        .nest("/ivr", ivr::router())
        .nest("/trunks", trunks::router())
        .nest("/schedules", schedules::router())
        .nest("/reports", reports::router())
        .nest("/quality", quality::router())
        .nest("/knowledge", knowledge::router())
        .nest("/softphone", softphone::router())
        .nest("/ai", ai::router())
}
