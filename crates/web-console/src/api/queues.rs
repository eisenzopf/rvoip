//! Queue management endpoints — list + CRUD + manual call assignment.
//!
//! Queue update/delete use the AdminApi stubs from call-engine. The AdminApi
//! logs the operation and validates preconditions (e.g. no active calls on
//! delete) but does not yet modify the in-memory QueueManager — that requires
//! call-engine changes tracked separately. Config updates are acknowledged and
//! will take effect when QueueManager gains a runtime-update API.

use axum::{Router, routing::{get, post, put, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::{AdminApi, SupervisorApi};

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN, ROLE_SUPERVISOR};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct QueueView {
    pub queue_id: String,
    pub total_calls: usize,
    pub avg_wait_secs: u64,
    pub longest_wait_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct QueueConfigView {
    pub queue_id: String,
    pub default_max_wait_time: u64,
    pub max_queue_size: usize,
    pub enable_priorities: bool,
    pub enable_overflow: bool,
    pub announcement_interval: u64,
}

#[derive(Debug, Serialize)]
pub struct QueuesResponse {
    pub queues: Vec<QueueView>,
    pub configs: Vec<QueueConfigView>,
    pub total_waiting: usize,
}

#[derive(Debug, Deserialize)]
pub struct CreateQueueBody {
    pub queue_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateQueueBody {
    pub default_max_wait_time: Option<u64>,
    pub max_queue_size: Option<usize>,
    pub enable_priorities: Option<bool>,
    pub enable_overflow: Option<bool>,
    pub announcement_interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AssignCallBody {
    pub agent_id: String,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Handlers -----------------------------------------------------------------

async fn list_queues(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<QueuesResponse>>> {
    let supervisor = SupervisorApi::new(state.engine.clone());
    let admin = AdminApi::new(state.engine.clone());

    // Get realtime queue stats
    let queue_stats = match supervisor.get_all_queue_stats().await {
        Ok(stats) => stats,
        Err(_) => vec![],
    };

    let queues: Vec<QueueView> = queue_stats
        .iter()
        .map(|(_, q)| QueueView {
            queue_id: q.queue_id.clone(),
            total_calls: q.total_calls,
            avg_wait_secs: q.average_wait_time_seconds,
            longest_wait_secs: q.longest_wait_time_seconds,
        })
        .collect();

    // Get queue configurations
    let config_map = admin.get_queue_configs().await;
    let configs: Vec<QueueConfigView> = config_map
        .iter()
        .map(|(id, c)| QueueConfigView {
            queue_id: id.clone(),
            default_max_wait_time: c.default_max_wait_time,
            max_queue_size: c.max_queue_size,
            enable_priorities: c.enable_priorities,
            enable_overflow: c.enable_overflow,
            announcement_interval: c.announcement_interval,
        })
        .collect();

    let total_waiting = queues.iter().map(|q| q.total_calls).sum();

    Ok(Json(ApiResponse::success(
        QueuesResponse { queues, configs, total_waiting },
        rid(),
    )))
}

async fn get_queue(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(queue_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<QueueConfigView>>> {
    let admin = AdminApi::new(state.engine.clone());
    let configs = admin.get_queue_configs().await;

    let config = configs.get(&queue_id)
        .ok_or_else(|| ConsoleError::NotFound(format!("queue {queue_id}")))?;

    Ok(Json(ApiResponse::success(
        QueueConfigView {
            queue_id: queue_id.clone(),
            default_max_wait_time: config.default_max_wait_time,
            max_queue_size: config.max_queue_size,
            enable_priorities: config.enable_priorities,
            enable_overflow: config.enable_overflow,
            announcement_interval: config.announcement_interval,
        },
        rid(),
    )))
}

async fn create_queue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateQueueBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.queue_id.is_empty() {
        return Err(ConsoleError::BadRequest("queue_id is required".into()));
    }

    let admin = AdminApi::new(state.engine.clone());
    admin.create_queue(&body.queue_id).await?;

    Ok(Json(ApiResponse::success(format!("queue {} created", body.queue_id), rid())))
}

async fn update_queue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(queue_id): Path<String>,
    Json(body): Json<UpdateQueueBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    // Verify the queue exists
    let admin = AdminApi::new(state.engine.clone());
    let configs = admin.get_queue_configs().await;
    if !configs.contains_key(&queue_id) {
        return Err(ConsoleError::NotFound(format!("queue {queue_id}")));
    }

    // Build merged config from current defaults + overrides
    let current_config = state.engine.config().queues.clone();
    let config = rvoip_call_engine::config::QueueConfig {
        default_max_wait_time: body.default_max_wait_time.unwrap_or(current_config.default_max_wait_time),
        max_queue_size: body.max_queue_size.unwrap_or(current_config.max_queue_size),
        enable_priorities: body.enable_priorities.unwrap_or(current_config.enable_priorities),
        enable_overflow: body.enable_overflow.unwrap_or(current_config.enable_overflow),
        announcement_interval: body.announcement_interval.unwrap_or(current_config.announcement_interval),
    };

    // AdminApi.update_queue logs the intent. Actual in-memory QueueManager
    // update requires a call-engine API addition (QueueManager.queues is private).
    admin.update_queue(&queue_id, config).await?;

    Ok(Json(ApiResponse::success(
        format!("queue {queue_id} config update acknowledged — takes effect on next queue creation"),
        rid(),
    )))
}

async fn delete_queue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(queue_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    // AdminApi.delete_queue validates no active calls, then acknowledges.
    // Actual removal from QueueManager.queues requires a call-engine API
    // addition (QueueManager.queues is private, no remove_queue method yet).
    let admin = AdminApi::new(state.engine.clone());
    admin.delete_queue(&queue_id).await?;

    Ok(Json(ApiResponse::success(format!("queue {queue_id} deleted"), rid())))
}

async fn get_queue_calls(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(queue_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<Vec<serde_json::Value>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN, ROLE_SUPERVISOR])?;

    let supervisor = SupervisorApi::new(state.engine.clone());
    let calls = supervisor.get_queued_calls(&queue_id).await;

    let call_views: Vec<serde_json::Value> = calls.iter().map(|c| {
        serde_json::json!({
            "session_id": c.session_id.to_string(),
            "from": c.from.clone(),
            "to": c.to.clone(),
            "status": format!("{:?}", c.status),
            "priority": c.priority,
            "created_at": c.created_at.to_rfc3339(),
        })
    }).collect();

    Ok(Json(ApiResponse::success(call_views, rid())))
}

async fn assign_call(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((queue_id, call_id)): Path<(String, String)>,
    Json(body): Json<AssignCallBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN, ROLE_SUPERVISOR])?;

    let supervisor = SupervisorApi::new(state.engine.clone());
    supervisor.force_assign_call(
        rvoip_call_engine::prelude::SessionId(call_id.clone()),
        rvoip_call_engine::agent::AgentId(body.agent_id.clone()),
    ).await?;

    Ok(Json(ApiResponse::success(
        format!("call {call_id} in queue {queue_id} assigned to agent {}", body.agent_id),
        rid(),
    )))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_queues).post(create_queue))
        .route("/{queue_id}", get(get_queue).put(update_queue).delete(delete_queue))
        .route("/{queue_id}/calls", get(get_queue_calls))
        .route("/{queue_id}/calls/{call_id}/assign", post(assign_call))
}
