//! Phone blacklist/whitelist/VIP management endpoints.
//!
//! Persisted to PostgreSQL. CRUD available to admin+ roles.
//! A quick check endpoint is also provided for call routing integration.

use axum::{Router, routing::{get, put, post, delete}, extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PhoneListEntry {
    pub id: String,
    pub number: String,
    pub list_type: String,
    pub reason: Option<String>,
    pub customer_name: Option<String>,
    pub vip_level: Option<i32>,
    pub expires_at: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePhoneListBody {
    pub number: String,
    pub list_type: String,
    pub reason: Option<String>,
    pub customer_name: Option<String>,
    pub vip_level: Option<i32>,
    pub expires_at: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePhoneListBody {
    pub number: Option<String>,
    pub list_type: Option<String>,
    pub reason: Option<String>,
    pub customer_name: Option<String>,
    pub vip_level: Option<i32>,
    pub expires_at: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(rename = "type")]
    pub list_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub number: String,
    pub entries: Vec<PhoneListEntry>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

const VALID_TYPES: &[&str] = &["blacklist", "whitelist", "vip"];

// -- Database init ------------------------------------------------------------

/// Ensure the phone_lists table exists and seed sample data.
pub async fn init_phone_lists_table(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS phone_lists (\
            id TEXT PRIMARY KEY, \
            number TEXT NOT NULL, \
            list_type TEXT NOT NULL CHECK (list_type IN ('blacklist', 'whitelist', 'vip')), \
            reason TEXT, \
            customer_name TEXT, \
            vip_level INTEGER, \
            expires_at TIMESTAMPTZ, \
            created_by TEXT, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create phone_lists table: {e}"))?;

    // Create indexes
    let _ = rvoip_call_engine::database::sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_phone_lists_number ON phone_lists(number)",
    )
    .execute(db.pool())
    .await;

    let _ = rvoip_call_engine::database::sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_phone_lists_type ON phone_lists(list_type)",
    )
    .execute(db.pool())
    .await;

    // Seed default entries (idempotent)
    let defaults: &[(&str, &str, &str, Option<&str>, Option<&str>, Option<i32>, Option<&str>)] = &[
        ("PL-seed-001", "+8613800001111", "blacklist", Some("Spam caller"), None, None, Some("system")),
        ("PL-seed-002", "+8613800002222", "blacklist", Some("Harassment"), None, None, Some("system")),
        ("PL-seed-003", "+8613900001111", "whitelist", Some("Partner company"), Some("ABC Corp"), None, Some("admin")),
        ("PL-seed-004", "+8613900002222", "whitelist", Some("Authorized vendor"), Some("XYZ Ltd"), None, Some("admin")),
        ("PL-seed-005", "+8618000001111", "vip", Some("Gold member"), Some("Zhang Wei"), Some(5), Some("admin")),
        ("PL-seed-006", "+8618000002222", "vip", Some("Silver member"), Some("Li Na"), Some(3), Some("admin")),
        ("PL-seed-007", "+8618000003333", "vip", Some("Bronze member"), Some("Wang Fang"), Some(1), Some("admin")),
    ];

    for (id, number, list_type, reason, customer_name, vip_level, created_by) in defaults {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO phone_lists (id, number, list_type, reason, customer_name, vip_level, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(number)
        .bind(list_type)
        .bind(reason)
        .bind(customer_name)
        .bind(vip_level)
        .bind(created_by)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

// -- Handlers -----------------------------------------------------------------

fn row_to_entry(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> PhoneListEntry {
    let expires_at: Option<chrono::DateTime<chrono::Utc>> = row.try_get("expires_at").unwrap_or_default();
    let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at").unwrap_or_default();

    PhoneListEntry {
        id: row.try_get("id").unwrap_or_default(),
        number: row.try_get("number").unwrap_or_default(),
        list_type: row.try_get("list_type").unwrap_or_default(),
        reason: row.try_get("reason").unwrap_or_default(),
        customer_name: row.try_get("customer_name").unwrap_or_default(),
        vip_level: row.try_get("vip_level").unwrap_or_default(),
        expires_at: expires_at.map(|dt| dt.to_rfc3339()),
        created_by: row.try_get("created_by").unwrap_or_default(),
        created_at: created_at.to_rfc3339(),
    }
}

async fn list_phone_lists(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<PhoneListEntry>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = if let Some(ref list_type) = query.list_type {
        if !VALID_TYPES.contains(&list_type.as_str()) {
            return Err(ConsoleError::BadRequest(format!(
                "invalid list_type: {list_type}; must be one of blacklist, whitelist, vip"
            )));
        }
        rvoip_call_engine::database::sqlx::query(
            "SELECT id, number, list_type, reason, customer_name, vip_level, expires_at, \
             created_by, created_at FROM phone_lists WHERE list_type = $1 ORDER BY created_at DESC",
        )
        .bind(list_type)
        .fetch_all(db.pool())
        .await
    } else {
        rvoip_call_engine::database::sqlx::query(
            "SELECT id, number, list_type, reason, customer_name, vip_level, expires_at, \
             created_by, created_at FROM phone_lists ORDER BY created_at DESC",
        )
        .fetch_all(db.pool())
        .await
    }
    .map_err(|e| ConsoleError::Internal(format!("phone_lists query failed: {e}")))?;

    let entries: Vec<PhoneListEntry> = rows.iter().map(row_to_entry).collect();
    Ok(Json(ApiResponse::success(entries, rid())))
}

async fn create_phone_list(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreatePhoneListBody>,
) -> ConsoleResult<Json<ApiResponse<PhoneListEntry>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.number.is_empty() {
        return Err(ConsoleError::BadRequest("number is required".into()));
    }
    if !VALID_TYPES.contains(&body.list_type.as_str()) {
        return Err(ConsoleError::BadRequest(format!(
            "invalid list_type: {}; must be one of blacklist, whitelist, vip",
            body.list_type
        )));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let id = format!("PL-{}", uuid::Uuid::new_v4());

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO phone_lists (id, number, list_type, reason, customer_name, vip_level, \
         expires_at, created_by) VALUES ($1, $2, $3, $4, $5, $6, \
         CASE WHEN $7::TEXT IS NOT NULL AND $7::TEXT != '' THEN $7::TIMESTAMPTZ ELSE NULL END, $8)",
    )
    .bind(&id)
    .bind(&body.number)
    .bind(&body.list_type)
    .bind(&body.reason)
    .bind(&body.customer_name)
    .bind(body.vip_level)
    .bind(&body.expires_at)
    .bind(&body.created_by)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert phone_list failed: {e}")))?;

    let entry = PhoneListEntry {
        id,
        number: body.number,
        list_type: body.list_type,
        reason: body.reason,
        customer_name: body.customer_name,
        vip_level: body.vip_level,
        expires_at: body.expires_at,
        created_by: body.created_by,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(entry, rid())))
}

async fn update_phone_list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(entry_id): Path<String>,
    Json(body): Json<UpdatePhoneListBody>,
) -> ConsoleResult<Json<ApiResponse<PhoneListEntry>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if let Some(ref lt) = body.list_type {
        if !VALID_TYPES.contains(&lt.as_str()) {
            return Err(ConsoleError::BadRequest(format!(
                "invalid list_type: {lt}; must be one of blacklist, whitelist, vip"
            )));
        }
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Fetch current entry
    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, number, list_type, reason, customer_name, vip_level, expires_at, \
         created_by, created_at FROM phone_lists WHERE id = $1",
    )
    .bind(&entry_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch phone_list failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("phone_list entry {entry_id}")))?;

    let number: String = body.number.unwrap_or_else(|| current.try_get("number").unwrap_or_default());
    let list_type: String = body.list_type.unwrap_or_else(|| current.try_get("list_type").unwrap_or_default());
    let reason: Option<String> = body.reason.or_else(|| current.try_get("reason").unwrap_or_default());
    let customer_name: Option<String> = body.customer_name.or_else(|| current.try_get("customer_name").unwrap_or_default());
    let vip_level: Option<i32> = body.vip_level.or_else(|| current.try_get("vip_level").unwrap_or_default());
    let created_by: Option<String> = body.created_by.or_else(|| current.try_get("created_by").unwrap_or_default());

    // Handle expires_at: if provided in body, use it; otherwise keep current
    let expires_at_str: Option<String> = if body.expires_at.is_some() {
        body.expires_at
    } else {
        let current_dt: Option<chrono::DateTime<chrono::Utc>> = current.try_get("expires_at").unwrap_or_default();
        current_dt.map(|dt| dt.to_rfc3339())
    };

    rvoip_call_engine::database::sqlx::query(
        "UPDATE phone_lists SET number = $1, list_type = $2, reason = $3, customer_name = $4, \
         vip_level = $5, expires_at = CASE WHEN $6::TEXT IS NOT NULL AND $6::TEXT != '' \
         THEN $6::TIMESTAMPTZ ELSE NULL END, created_by = $7 WHERE id = $8",
    )
    .bind(&number)
    .bind(&list_type)
    .bind(&reason)
    .bind(&customer_name)
    .bind(vip_level)
    .bind(&expires_at_str)
    .bind(&created_by)
    .bind(&entry_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update phone_list failed: {e}")))?;

    let created_at: chrono::DateTime<chrono::Utc> = current.try_get("created_at").unwrap_or_default();

    let entry = PhoneListEntry {
        id: entry_id,
        number,
        list_type,
        reason,
        customer_name,
        vip_level,
        expires_at: expires_at_str,
        created_by,
        created_at: created_at.to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(entry, rid())))
}

async fn delete_phone_list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(entry_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM phone_lists WHERE id = $1",
    )
    .bind(&entry_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete phone_list failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("phone_list entry {entry_id}")));
    }

    Ok(Json(ApiResponse::success(format!("entry {entry_id} deleted"), rid())))
}

async fn check_number(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(number): Path<String>,
) -> ConsoleResult<Json<ApiResponse<CheckResult>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(
            CheckResult { number: number.clone(), entries: Vec::new() },
            rid(),
        ))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, number, list_type, reason, customer_name, vip_level, expires_at, \
         created_by, created_at FROM phone_lists WHERE number = $1 \
         AND (expires_at IS NULL OR expires_at > NOW()) ORDER BY created_at DESC",
    )
    .bind(&number)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("phone_lists check failed: {e}")))?;

    let entries: Vec<PhoneListEntry> = rows.iter().map(row_to_entry).collect();
    let result = CheckResult { number, entries };
    Ok(Json(ApiResponse::success(result, rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_phone_lists).post(create_phone_list))
        .route("/{id}", put(update_phone_list).delete(delete_phone_list))
        .route("/check/{number}", get(check_number))
}
