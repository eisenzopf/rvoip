//! Active calls management endpoints.

use axum::{Router, routing::{get, post}, extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::SupervisorApi;
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct ActiveCall {
    pub call_id: String,
    pub from_uri: String,
    pub to_uri: String,
    pub caller_id: String,
    pub agent_id: Option<String>,
    pub queue_id: Option<String>,
    pub status: String,
    pub priority: u8,
    pub customer_type: String,
    pub required_skills: Vec<String>,
    pub created_at: String,
    pub queued_at: Option<String>,
    pub answered_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CallsResponse {
    pub calls: Vec<ActiveCall>,
    pub total: usize,
}

fn map_call(c: &rvoip_call_engine::prelude::CallInfo) -> ActiveCall {
    ActiveCall {
        call_id: c.session_id.to_string(),
        from_uri: c.from.clone(),
        to_uri: c.to.clone(),
        caller_id: c.caller_id.clone(),
        agent_id: c.agent_id.as_ref().map(|id| id.0.clone()),
        queue_id: c.queue_id.clone(),
        status: format!("{:?}", c.status),
        priority: c.priority,
        customer_type: format!("{:?}", c.customer_type),
        required_skills: c.required_skills.clone(),
        created_at: c.created_at.to_rfc3339(),
        queued_at: c.queued_at.map(|t| t.to_rfc3339()),
        answered_at: c.answered_at.map(|t| t.to_rfc3339()),
        ended_at: c.ended_at.map(|t| t.to_rfc3339()),
    }
}

async fn list_calls(
    State(state): State<AppState>,
) -> ConsoleResult<Json<ApiResponse<CallsResponse>>> {
    let supervisor = SupervisorApi::new(state.engine.clone());
    let active = supervisor.list_active_calls().await;

    let calls: Vec<ActiveCall> = active.iter().map(map_call).collect();
    let total = calls.len();

    Ok(Json(ApiResponse::success(
        CallsResponse { calls, total },
        rid(),
    )))
}

async fn get_call(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<ActiveCall>>> {
    let supervisor = SupervisorApi::new(state.engine.clone());
    let active = supervisor.list_active_calls().await;

    let call = active
        .iter()
        .find(|c| c.session_id.to_string() == call_id)
        .map(map_call)
        .ok_or_else(|| ConsoleError::NotFound(format!("call {call_id}")))?;

    Ok(Json(ApiResponse::success(call, rid())))
}

fn rid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Serialize)]
pub struct CallHistoryEntry {
    pub call_id: String,
    pub customer_number: Option<String>,
    pub agent_id: Option<String>,
    pub queue_name: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub duration_seconds: Option<i32>,
    pub disposition: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

async fn call_history(
    State(state): State<AppState>,
    Query(params): Query<HistoryQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<CallHistoryEntry>>>> {
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => {
            return Ok(Json(ApiResponse::success(Vec::new(), rid())));
        }
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT call_id, customer_number, agent_id, queue_name, start_time, end_time, \
         duration_seconds, disposition, notes FROM call_records ORDER BY start_time DESC \
         LIMIT $1::bigint OFFSET $2::bigint",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("database query failed: {e}")))?;

    let entries: Vec<CallHistoryEntry> = rows
        .iter()
        .map(|row| {
            // start_time and end_time are TIMESTAMPTZ — read as Option<DateTime<Utc>>
            let start_time: Option<String> = row
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("start_time")
                .unwrap_or(None)
                .map(|t| t.to_rfc3339());
            let end_time: Option<String> = row
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("end_time")
                .unwrap_or(None)
                .map(|t| t.to_rfc3339());

            CallHistoryEntry {
                call_id: row.try_get("call_id").unwrap_or_default(),
                customer_number: row.try_get("customer_number").ok(),
                agent_id: row.try_get("agent_id").ok(),
                queue_name: row.try_get("queue_name").ok(),
                start_time,
                end_time,
                duration_seconds: row.try_get("duration_seconds").ok(),
                disposition: row.try_get("disposition").ok(),
                notes: row.try_get("notes").ok(),
            }
        })
        .collect();

    Ok(Json(ApiResponse::success(entries, rid())))
}

async fn hangup_call(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(call_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    // Look up the call in active calls
    let supervisor = SupervisorApi::new(state.engine.clone());
    let active = supervisor.list_active_calls().await;
    let call = active.iter().find(|c| c.session_id.to_string() == call_id);

    // Write a call_record to the database if DB is available
    if let Some(db) = state.engine.database_manager() {
        let agent_id = call.and_then(|c| c.agent_id.as_ref()).map(|a| a.0.clone());
        let queue_name = call.and_then(|c| c.queue_id.clone());
        let customer_number = call.map(|c| c.from.clone());

        let pool = db.pool().clone();
        let cid = call_id.clone();
        tokio::spawn(async move {
            if let Err(e) = rvoip_call_engine::database::sqlx::query(
                "INSERT INTO call_records (call_id, customer_number, agent_id, queue_name, \
                 start_time, end_time, duration_seconds, disposition) \
                 VALUES ($1, $2, $3, $4, NOW(), NOW(), 0, $5) \
                 ON CONFLICT (call_id) DO UPDATE SET end_time = NOW(), disposition = $5",
            )
            .bind(&cid)
            .bind(customer_number.as_deref())
            .bind(agent_id.as_deref())
            .bind(queue_name.as_deref())
            .bind("hangup")
            .execute(&pool)
            .await
            {
                tracing::warn!("Failed to record hangup for call {}: {}", cid, e);
            }
        });
    }

    // Actual SIP session termination requires deep session-core integration.
    // The hangup is recorded in the database for audit purposes.
    Ok(Json(ApiResponse::success(
        format!("hangup recorded for call {call_id}"),
        rid(),
    )))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_calls))
        .route("/{call_id}", get(get_call))
        .route("/history", get(call_history))
        .route("/{call_id}/hangup", post(hangup_call))
}
