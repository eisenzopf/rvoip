//! Extension pool management endpoints — ranges + individual extension CRUD.

use axum::{Router, routing::{get, post, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ExtensionRangeView {
    pub id: String,
    pub range_start: i32,
    pub range_end: i32,
    pub department_id: Option<String>,
    pub description: Option<String>,
    pub total: i64,
    pub assigned: i64,
    pub available: i64,
}

#[derive(Debug, Serialize)]
pub struct ExtensionView {
    pub number: i32,
    pub range_id: Option<String>,
    pub agent_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRangeBody {
    pub range_start: i32,
    pub range_end: i32,
    pub department_id: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssignExtensionBody {
    pub agent_id: String,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database init ------------------------------------------------------------

/// Ensure extension tables exist and seed defaults.
pub async fn init_extensions_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    // Create extension_ranges table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS extension_ranges (\
            id TEXT PRIMARY KEY, \
            range_start INTEGER NOT NULL, \
            range_end INTEGER NOT NULL, \
            department_id TEXT, \
            description TEXT, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create extension_ranges table: {e}"))?;

    // Create extensions table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS extensions (\
            number INTEGER PRIMARY KEY, \
            range_id TEXT REFERENCES extension_ranges(id), \
            agent_id TEXT, \
            status TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available', 'assigned', 'reserved')), \
            assigned_at TIMESTAMPTZ\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create extensions table: {e}"))?;

    // Seed default ranges (idempotent)
    let defaults = [
        ("EXT-GEN-001", 1001, 1099, None, Some("General pool")),
        ("EXT-SAL-001", 1100, 1199, Some("sales"), Some("Sales department")),
    ];

    for (id, start, end, dept, desc) in &defaults {
        let inserted = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO extension_ranges (id, range_start, range_end, department_id, description) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(start)
        .bind(end)
        .bind(dept)
        .bind(desc)
        .execute(db.pool())
        .await
        .map_err(|e| format!("failed to seed extension range: {e}"))?;

        // Populate individual extension numbers if the range was newly inserted
        if inserted.rows_affected() > 0 {
            for num in *start..=*end {
                let _ = rvoip_call_engine::database::sqlx::query(
                    "INSERT INTO extensions (number, range_id, status) \
                     VALUES ($1, $2, 'available') \
                     ON CONFLICT (number) DO NOTHING",
                )
                .bind(num)
                .bind(id)
                .execute(db.pool())
                .await;
            }
        }
    }

    Ok(())
}

// -- Handlers -----------------------------------------------------------------

async fn list_extension_ranges(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<ExtensionRangeView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT r.id, r.range_start, r.range_end, r.department_id, r.description, \
         COUNT(e.number) AS total, \
         COUNT(CASE WHEN e.status = 'assigned' THEN 1 END) AS assigned, \
         COUNT(CASE WHEN e.status = 'available' THEN 1 END) AS available \
         FROM extension_ranges r \
         LEFT JOIN extensions e ON e.range_id = r.id \
         GROUP BY r.id, r.range_start, r.range_end, r.department_id, r.description \
         ORDER BY r.range_start ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("extension_ranges query failed: {e}")))?;

    let ranges: Vec<ExtensionRangeView> = rows.iter().map(|row| ExtensionRangeView {
        id: row.try_get("id").unwrap_or_default(),
        range_start: row.try_get("range_start").unwrap_or_default(),
        range_end: row.try_get("range_end").unwrap_or_default(),
        department_id: row.try_get("department_id").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        total: row.try_get("total").unwrap_or_default(),
        assigned: row.try_get("assigned").unwrap_or_default(),
        available: row.try_get("available").unwrap_or_default(),
    }).collect();

    Ok(Json(ApiResponse::success(ranges, rid())))
}

async fn create_extension_range(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRangeBody>,
) -> ConsoleResult<Json<ApiResponse<ExtensionRangeView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.range_start >= body.range_end {
        return Err(ConsoleError::BadRequest("range_start must be less than range_end".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let id = format!("EXT-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO extension_ranges (id, range_start, range_end, department_id, description) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(body.range_start)
    .bind(body.range_end)
    .bind(&body.department_id)
    .bind(&body.description)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert extension_range failed: {e}")))?;

    // Populate individual extensions
    for num in body.range_start..=body.range_end {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO extensions (number, range_id, status) \
             VALUES ($1, $2, 'available') \
             ON CONFLICT (number) DO NOTHING",
        )
        .bind(num)
        .bind(&id)
        .execute(db.pool())
        .await;
    }

    let total = (body.range_end - body.range_start + 1) as i64;

    let view = ExtensionRangeView {
        id,
        range_start: body.range_start,
        range_end: body.range_end,
        department_id: body.department_id,
        description: body.description,
        total,
        assigned: 0,
        available: total,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_extension_range(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(range_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Check if any extensions are assigned
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM extensions WHERE range_id = $1 AND status = 'assigned'",
    )
    .bind(&range_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("check assigned extensions failed: {e}")))?;

    let assigned: i64 = row.try_get("cnt").unwrap_or_default();
    if assigned > 0 {
        return Err(ConsoleError::BadRequest(
            format!("cannot delete range with {assigned} assigned extensions"),
        ));
    }

    // Delete extensions first, then the range
    rvoip_call_engine::database::sqlx::query(
        "DELETE FROM extensions WHERE range_id = $1",
    )
    .bind(&range_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete extensions failed: {e}")))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM extension_ranges WHERE id = $1",
    )
    .bind(&range_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete extension_range failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("range {range_id}")));
    }

    Ok(Json(ApiResponse::success(format!("range {range_id} deleted"), rid())))
}

async fn list_available_extensions(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<ExtensionView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT number, range_id, agent_id, status FROM extensions \
         WHERE status = 'available' ORDER BY number ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("available extensions query failed: {e}")))?;

    let exts: Vec<ExtensionView> = rows.iter().map(|row| ExtensionView {
        number: row.try_get("number").unwrap_or_default(),
        range_id: row.try_get("range_id").unwrap_or_default(),
        agent_id: row.try_get("agent_id").unwrap_or_default(),
        status: row.try_get("status").unwrap_or_default(),
    }).collect();

    Ok(Json(ApiResponse::success(exts, rid())))
}

async fn assign_extension(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(number): Path<i32>,
    Json(body): Json<AssignExtensionBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "UPDATE extensions SET agent_id = $1, status = 'assigned', assigned_at = NOW() \
         WHERE number = $2 AND status = 'available'",
    )
    .bind(&body.agent_id)
    .bind(number)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("assign extension failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::BadRequest(
            format!("extension {number} is not available or does not exist"),
        ));
    }

    Ok(Json(ApiResponse::success(format!("extension {number} assigned to {}", body.agent_id), rid())))
}

async fn release_extension(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(number): Path<i32>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "UPDATE extensions SET agent_id = NULL, status = 'available', assigned_at = NULL \
         WHERE number = $1",
    )
    .bind(number)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("release extension failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("extension {number}")));
    }

    Ok(Json(ApiResponse::success(format!("extension {number} released"), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_extension_ranges))
        .route("/ranges", post(create_extension_range))
        .route("/ranges/{id}", delete(delete_extension_range))
        .route("/available", get(list_available_extensions))
        .route("/{number}/assign", post(assign_extension))
        .route("/{number}/release", post(release_extension))
}
