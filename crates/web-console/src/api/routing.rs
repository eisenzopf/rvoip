//! Routing configuration and overflow policy endpoints.
//!
//! Routing config is read-only at runtime (immutable in call-engine config).
//! Overflow policies are persisted to PostgreSQL.

use axum::{Router, routing::{get, put, post, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::AdminApi;
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RoutingConfigView {
    pub default_strategy: String,
    pub enable_load_balancing: bool,
    pub load_balance_strategy: String,
    pub enable_geographic_routing: bool,
    pub enable_time_based_routing: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoutingBody {
    pub default_strategy: Option<String>,
    pub enable_load_balancing: Option<bool>,
    pub load_balance_strategy: Option<String>,
    pub enable_geographic_routing: Option<bool>,
    pub enable_time_based_routing: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverflowPolicyView {
    pub id: String,
    pub name: String,
    pub condition_type: String,
    pub condition_value: String,
    pub action_type: String,
    pub action_value: String,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateOverflowPolicyBody {
    pub name: String,
    pub condition_type: String,
    pub condition_value: String,
    pub action_type: String,
    pub action_value: String,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_priority() -> i32 { 5 }
fn default_true() -> bool { true }
fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database helpers ---------------------------------------------------------

/// Ensure the overflow_policies table exists and seed defaults.
pub async fn init_overflow_policies_table(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()), // no DB configured — skip
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS overflow_policies (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL, \
            condition_type TEXT NOT NULL, \
            condition_value TEXT NOT NULL, \
            action_type TEXT NOT NULL, \
            action_value TEXT NOT NULL, \
            priority INTEGER NOT NULL DEFAULT 5, \
            enabled BOOLEAN NOT NULL DEFAULT TRUE, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create overflow_policies table: {e}"))?;

    // Seed default policies (idempotent)
    let defaults = [
        ("policy-1", "High Queue Size", "QueueSize", "20", "RouteToQueue", "overflow", 1, true),
        ("policy-2", "Long Wait Time", "WaitTime", "300", "EnableCallbacks", "", 2, true),
        ("policy-3", "After Hours", "AfterHours", "", "ForwardToVoicemail", "", 3, false),
    ];

    for (id, name, ctype, cval, atype, aval, prio, enabled) in &defaults {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO overflow_policies (id, name, condition_type, condition_value, \
             action_type, action_value, priority, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(ctype)
        .bind(cval)
        .bind(atype)
        .bind(aval)
        .bind(*prio)
        .bind(*enabled)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

fn row_to_policy(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> OverflowPolicyView {
    OverflowPolicyView {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        condition_type: row.try_get("condition_type").unwrap_or_default(),
        condition_value: row.try_get("condition_value").unwrap_or_default(),
        action_type: row.try_get("action_type").unwrap_or_default(),
        action_value: row.try_get("action_value").unwrap_or_default(),
        priority: row.try_get("priority").unwrap_or(5),
        enabled: row.try_get("enabled").unwrap_or(true),
    }
}

// -- Routing config handlers --------------------------------------------------

async fn get_routing_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<RoutingConfigView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let admin = AdminApi::new(state.engine.clone());
    let config = admin.get_config();
    let r = &config.routing;

    Ok(Json(ApiResponse::success(RoutingConfigView {
        default_strategy: format!("{:?}", r.default_strategy),
        enable_load_balancing: r.enable_load_balancing,
        load_balance_strategy: format!("{:?}", r.load_balance_strategy),
        enable_geographic_routing: r.enable_geographic_routing,
        enable_time_based_routing: r.enable_time_based_routing,
    }, rid())))
}

/// Routing config is immutable at runtime in call-engine.
/// This endpoint acknowledges the request and returns the current config.
async fn update_routing_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(_body): Json<UpdateRoutingBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let admin = AdminApi::new(state.engine.clone());
    let current = admin.get_config().routing.clone();
    admin.update_routing_config(current).await?;

    Ok(Json(ApiResponse::success(
        "routing config is immutable at runtime — restart required for changes".into(),
        rid(),
    )))
}

// -- Overflow policy CRUD (PostgreSQL-backed) ---------------------------------

async fn list_overflow_policies(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<OverflowPolicyView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, condition_type, condition_value, action_type, action_value, \
         priority, enabled FROM overflow_policies ORDER BY priority ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("overflow_policies query failed: {e}")))?;

    let policies: Vec<OverflowPolicyView> = rows.iter().map(row_to_policy).collect();
    Ok(Json(ApiResponse::success(policies, rid())))
}

async fn create_overflow_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateOverflowPolicyBody>,
) -> ConsoleResult<Json<ApiResponse<OverflowPolicyView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let id = format!("policy-{}", uuid::Uuid::new_v4());

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO overflow_policies (id, name, condition_type, condition_value, \
         action_type, action_value, priority, enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.condition_type)
    .bind(&body.condition_value)
    .bind(&body.action_type)
    .bind(&body.action_value)
    .bind(body.priority)
    .bind(body.enabled)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert overflow_policy failed: {e}")))?;

    let policy = OverflowPolicyView {
        id,
        name: body.name,
        condition_type: body.condition_type,
        condition_value: body.condition_value,
        action_type: body.action_type,
        action_value: body.action_value,
        priority: body.priority,
        enabled: body.enabled,
    };

    Ok(Json(ApiResponse::success(policy, rid())))
}

async fn update_overflow_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(policy_id): Path<String>,
    Json(body): Json<CreateOverflowPolicyBody>,
) -> ConsoleResult<Json<ApiResponse<OverflowPolicyView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "UPDATE overflow_policies SET name = $1, condition_type = $2, condition_value = $3, \
         action_type = $4, action_value = $5, priority = $6, enabled = $7 \
         WHERE id = $8",
    )
    .bind(&body.name)
    .bind(&body.condition_type)
    .bind(&body.condition_value)
    .bind(&body.action_type)
    .bind(&body.action_value)
    .bind(body.priority)
    .bind(body.enabled)
    .bind(&policy_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update overflow_policy failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("policy {policy_id}")));
    }

    let updated = OverflowPolicyView {
        id: policy_id,
        name: body.name,
        condition_type: body.condition_type,
        condition_value: body.condition_value,
        action_type: body.action_type,
        action_value: body.action_value,
        priority: body.priority,
        enabled: body.enabled,
    };

    Ok(Json(ApiResponse::success(updated, rid())))
}

async fn delete_overflow_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(policy_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM overflow_policies WHERE id = $1",
    )
    .bind(&policy_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete overflow_policy failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("policy {policy_id}")));
    }

    Ok(Json(ApiResponse::success(format!("policy {policy_id} deleted"), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/config", get(get_routing_config).put(update_routing_config))
        .route("/overflow/policies", get(list_overflow_policies).post(create_overflow_policy))
        .route("/overflow/policies/{policy_id}", put(update_overflow_policy).delete(delete_overflow_policy))
}
