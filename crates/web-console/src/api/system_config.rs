//! System configuration and audit log endpoints.

use axum::{Router, routing::{get, post, put}, extract::{State, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::{AdminApi, CallCenterConfig};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

fn rid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub user_id: String,
    pub username: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub details: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct AuditQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ImportConfigBody {
    pub config_json: String,
}

// ---------------------------------------------------------------------------
// Handlers — config
// ---------------------------------------------------------------------------

async fn get_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<CallCenterConfig>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let admin = AdminApi::new(state.engine.clone());
    let config = admin.get_config().clone();

    Ok(Json(ApiResponse::success(config, rid())))
}

async fn update_config(
    State(_state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<serde_json::Value>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    tracing::info!(
        user = %auth.username,
        "config update requested (hot-reload not yet implemented)"
    );
    let _body = body; // acknowledged but not applied

    Ok(Json(ApiResponse::success(
        "config update acknowledged (hot-reload pending)".to_string(),
        rid(),
    )))
}

async fn export_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let admin = AdminApi::new(state.engine.clone());
    let json_str = admin.export_config().await?;

    Ok(Json(ApiResponse::success(json_str, rid())))
}

async fn import_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ImportConfigBody>,
) -> ConsoleResult<Json<ApiResponse<CallCenterConfig>>> {
    require_role(&auth, &[ROLE_SUPER_ADMIN])?;

    let admin = AdminApi::new(state.engine.clone());
    let parsed = admin.import_config(&body.config_json).await?;

    Ok(Json(ApiResponse::success(parsed, rid())))
}

async fn optimize_database(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_SUPER_ADMIN])?;

    let admin = AdminApi::new(state.engine.clone());
    admin.optimize_database().await?;

    Ok(Json(ApiResponse::success(
        "database optimization completed".to_string(),
        rid(),
    )))
}

// ---------------------------------------------------------------------------
// Handlers — audit log
// ---------------------------------------------------------------------------

async fn audit_log(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<AuditQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<AuditLogEntry>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => {
            return Ok(Json(ApiResponse::success(Vec::new(), rid())));
        }
    };

    // Ensure table exists
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS audit_log (
            id BIGSERIAL PRIMARY KEY,
            user_id TEXT NOT NULL,
            username TEXT NOT NULL,
            action TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id TEXT,
            details JSONB,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("failed to ensure audit_log table: {e}")))?;

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, user_id, username, action, resource_type, resource_id, details, \
         created_at FROM audit_log ORDER BY created_at DESC LIMIT $1::bigint OFFSET $2::bigint",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("audit log query failed: {e}")))?;

    let entries: Vec<AuditLogEntry> = rows
        .iter()
        .map(|row| {
            let created: Option<chrono::DateTime<chrono::Utc>> = row.try_get("created_at").ok();
            AuditLogEntry {
                id: row.try_get("id").unwrap_or_default(),
                user_id: row.try_get("user_id").unwrap_or_default(),
                username: row.try_get("username").unwrap_or_default(),
                action: row.try_get("action").unwrap_or_default(),
                resource_type: row.try_get("resource_type").unwrap_or_default(),
                resource_id: row.try_get("resource_id").ok(),
                details: row.try_get("details").ok(),
                created_at: created
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default(),
            }
        })
        .collect();

    Ok(Json(ApiResponse::success(entries, rid())))
}

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

/// Router for `/system/config` paths.
pub fn config_router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_config).put(update_config))
        .route("/export", post(export_config))
        .route("/import", post(import_config))
        .route("/db/optimize", post(optimize_database))
}

/// Router for `/system/audit` paths.
pub fn audit_router() -> Router<AppState> {
    Router::new()
        .route("/log", get(audit_log))
}
