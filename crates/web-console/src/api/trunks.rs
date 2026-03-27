//! SIP Trunk and DID number management endpoints.

use axum::{Router, routing::{get, put, post, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct TrunkView {
    pub id: String,
    pub name: String,
    pub provider: Option<String>,
    pub host: String,
    pub port: i32,
    pub transport: String,
    pub username: Option<String>,
    pub max_channels: i32,
    pub active_channels: i32,
    pub registration_required: bool,
    pub status: String,
    pub did_count: i64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct DidNumberView {
    pub id: String,
    pub number: String,
    pub trunk_id: Option<String>,
    pub trunk_name: Option<String>,
    pub assigned_to: Option<String>,
    pub assigned_type: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTrunkBody {
    pub name: String,
    pub provider: Option<String>,
    pub host: String,
    pub port: Option<i32>,
    pub transport: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub max_channels: Option<i32>,
    pub registration_required: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTrunkBody {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub host: Option<String>,
    pub port: Option<i32>,
    pub transport: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub max_channels: Option<i32>,
    pub registration_required: Option<bool>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDidBody {
    pub number: String,
    pub trunk_id: Option<String>,
    pub assigned_to: Option<String>,
    pub assigned_type: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDidBody {
    pub trunk_id: Option<String>,
    pub assigned_to: Option<String>,
    pub assigned_type: Option<String>,
    pub description: Option<String>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database init ------------------------------------------------------------

/// Ensure trunk/DID tables exist and seed defaults.
pub async fn init_trunks_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    // Create sip_trunks table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS sip_trunks (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL, \
            provider TEXT, \
            host TEXT NOT NULL, \
            port INTEGER DEFAULT 5060, \
            transport TEXT DEFAULT 'UDP', \
            username TEXT, \
            password_encrypted TEXT, \
            max_channels INTEGER DEFAULT 30, \
            active_channels INTEGER DEFAULT 0, \
            registration_required BOOLEAN DEFAULT FALSE, \
            status TEXT DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'error')), \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create sip_trunks table: {e}"))?;

    // Create did_numbers table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS did_numbers (\
            id TEXT PRIMARY KEY, \
            number TEXT NOT NULL UNIQUE, \
            trunk_id TEXT REFERENCES sip_trunks(id), \
            assigned_to TEXT, \
            assigned_type TEXT CHECK (assigned_type IN ('queue', 'agent', 'ivr', 'unassigned')), \
            description TEXT, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create did_numbers table: {e}"))?;

    // Seed default trunk (idempotent)
    let _ = rvoip_call_engine::database::sqlx::query(
        "INSERT INTO sip_trunks (id, name, provider, host, port, transport, max_channels, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind("TRK-001")
    .bind("Local Test Trunk")
    .bind("localhost")
    .bind("127.0.0.1")
    .bind(5060_i32)
    .bind("UDP")
    .bind(30_i32)
    .bind("active")
    .execute(db.pool())
    .await;

    // Seed default DID numbers (idempotent)
    let dids = [
        ("DID-001", "+1-800-555-0001", Some("TRK-001"), None::<&str>, Some("unassigned"), Some("Main line")),
        ("DID-002", "+1-800-555-0002", Some("TRK-001"), None::<&str>, Some("unassigned"), Some("Sales line")),
        ("DID-003", "+1-800-555-0003", Some("TRK-001"), None::<&str>, Some("unassigned"), Some("Support line")),
    ];

    for (id, number, trunk_id, assigned_to, assigned_type, desc) in &dids {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO did_numbers (id, number, trunk_id, assigned_to, assigned_type, description) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(number)
        .bind(trunk_id)
        .bind(assigned_to)
        .bind(assigned_type)
        .bind(desc)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

// -- Trunk Handlers -----------------------------------------------------------

async fn list_trunks(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<TrunkView>>>> {
    require_role(&auth, &[ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT t.id, t.name, t.provider, t.host, t.port, t.transport, \
         t.username, t.max_channels, t.active_channels, t.registration_required, \
         t.status, t.created_at, \
         COUNT(d.id) AS did_count \
         FROM sip_trunks t \
         LEFT JOIN did_numbers d ON d.trunk_id = t.id \
         GROUP BY t.id, t.name, t.provider, t.host, t.port, t.transport, \
         t.username, t.max_channels, t.active_channels, t.registration_required, \
         t.status, t.created_at \
         ORDER BY t.name ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("sip_trunks query failed: {e}")))?;

    let trunks: Vec<TrunkView> = rows.iter().map(|row| {
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at").unwrap_or_default();
        TrunkView {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            provider: row.try_get("provider").unwrap_or_default(),
            host: row.try_get("host").unwrap_or_default(),
            port: row.try_get("port").unwrap_or(5060),
            transport: row.try_get("transport").unwrap_or_default(),
            username: row.try_get("username").unwrap_or_default(),
            max_channels: row.try_get("max_channels").unwrap_or(30),
            active_channels: row.try_get("active_channels").unwrap_or(0),
            registration_required: row.try_get("registration_required").unwrap_or(false),
            status: row.try_get("status").unwrap_or_default(),
            did_count: row.try_get("did_count").unwrap_or(0),
            created_at: created.to_rfc3339(),
        }
    }).collect();

    Ok(Json(ApiResponse::success(trunks, rid())))
}

async fn create_trunk(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateTrunkBody>,
) -> ConsoleResult<Json<ApiResponse<TrunkView>>> {
    require_role(&auth, &[ROLE_SUPER_ADMIN])?;

    if body.name.is_empty() || body.host.is_empty() {
        return Err(ConsoleError::BadRequest("name and host are required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Generate next TRK-NNN ID
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM sip_trunks",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count trunks failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("TRK-{:03}", count + 1);

    let port = body.port.unwrap_or(5060);
    let transport = body.transport.unwrap_or_else(|| "UDP".to_string());
    let max_channels = body.max_channels.unwrap_or(30);
    let registration_required = body.registration_required.unwrap_or(false);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO sip_trunks (id, name, provider, host, port, transport, username, password_encrypted, max_channels, registration_required) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.provider)
    .bind(&body.host)
    .bind(port)
    .bind(&transport)
    .bind(&body.username)
    .bind(&body.password)
    .bind(max_channels)
    .bind(registration_required)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert trunk failed: {e}")))?;

    let view = TrunkView {
        id,
        name: body.name,
        provider: body.provider,
        host: body.host,
        port,
        transport,
        username: body.username,
        max_channels,
        active_channels: 0,
        registration_required,
        status: "active".to_string(),
        did_count: 0,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_trunk(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trunk_id): Path<String>,
    Json(body): Json<UpdateTrunkBody>,
) -> ConsoleResult<Json<ApiResponse<TrunkView>>> {
    require_role(&auth, &[ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Fetch current
    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, provider, host, port, transport, username, \
         max_channels, active_channels, registration_required, status, created_at \
         FROM sip_trunks WHERE id = $1",
    )
    .bind(&trunk_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch trunk failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("trunk {trunk_id}")))?;

    let name: String = body.name.unwrap_or_else(|| current.try_get("name").unwrap_or_default());
    let provider: Option<String> = body.provider.or_else(|| current.try_get("provider").unwrap_or_default());
    let host: String = body.host.unwrap_or_else(|| current.try_get("host").unwrap_or_default());
    let port: i32 = body.port.unwrap_or_else(|| current.try_get("port").unwrap_or(5060));
    let transport: String = body.transport.unwrap_or_else(|| current.try_get("transport").unwrap_or_default());
    let username: Option<String> = body.username.or_else(|| current.try_get("username").unwrap_or_default());
    let max_channels: i32 = body.max_channels.unwrap_or_else(|| current.try_get("max_channels").unwrap_or(30));
    let registration_required: bool = body.registration_required.unwrap_or_else(|| current.try_get("registration_required").unwrap_or(false));
    let status: String = body.status.unwrap_or_else(|| current.try_get("status").unwrap_or_default());

    // Validate status
    if !["active", "inactive", "error"].contains(&status.as_str()) {
        return Err(ConsoleError::BadRequest("status must be active, inactive, or error".into()));
    }

    // If password provided, update it too
    if let Some(ref pwd) = body.password {
        rvoip_call_engine::database::sqlx::query(
            "UPDATE sip_trunks SET name=$1, provider=$2, host=$3, port=$4, transport=$5, \
             username=$6, password_encrypted=$7, max_channels=$8, registration_required=$9, status=$10 \
             WHERE id=$11",
        )
        .bind(&name).bind(&provider).bind(&host).bind(port).bind(&transport)
        .bind(&username).bind(pwd).bind(max_channels).bind(registration_required).bind(&status)
        .bind(&trunk_id)
        .execute(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("update trunk failed: {e}")))?;
    } else {
        rvoip_call_engine::database::sqlx::query(
            "UPDATE sip_trunks SET name=$1, provider=$2, host=$3, port=$4, transport=$5, \
             username=$6, max_channels=$7, registration_required=$8, status=$9 \
             WHERE id=$10",
        )
        .bind(&name).bind(&provider).bind(&host).bind(port).bind(&transport)
        .bind(&username).bind(max_channels).bind(registration_required).bind(&status)
        .bind(&trunk_id)
        .execute(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("update trunk failed: {e}")))?;
    }

    // Get DID count
    let count_row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM did_numbers WHERE trunk_id = $1",
    )
    .bind(&trunk_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count DIDs failed: {e}")))?;
    let did_count: i64 = count_row.try_get("cnt").unwrap_or_default();

    let active_channels: i32 = current.try_get("active_channels").unwrap_or(0);
    let created: chrono::DateTime<chrono::Utc> = current.try_get("created_at").unwrap_or_default();

    let view = TrunkView {
        id: trunk_id,
        name, provider, host, port, transport, username,
        max_channels, active_channels, registration_required, status,
        did_count, created_at: created.to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_trunk(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trunk_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Check if DIDs are assigned to this trunk
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM did_numbers WHERE trunk_id = $1",
    )
    .bind(&trunk_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("check did_numbers failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    if count > 0 {
        return Err(ConsoleError::BadRequest(
            format!("cannot delete trunk with {count} assigned DID numbers"),
        ));
    }

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM sip_trunks WHERE id = $1",
    )
    .bind(&trunk_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete trunk failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("trunk {trunk_id}")));
    }

    Ok(Json(ApiResponse::success(format!("trunk {trunk_id} deleted"), rid())))
}

// -- DID Handlers -------------------------------------------------------------

async fn list_dids(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<DidNumberView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT d.id, d.number, d.trunk_id, t.name AS trunk_name, \
         d.assigned_to, d.assigned_type, d.description, d.created_at \
         FROM did_numbers d \
         LEFT JOIN sip_trunks t ON t.id = d.trunk_id \
         ORDER BY d.number ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("did_numbers query failed: {e}")))?;

    let dids: Vec<DidNumberView> = rows.iter().map(|row| {
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at").unwrap_or_default();
        DidNumberView {
            id: row.try_get("id").unwrap_or_default(),
            number: row.try_get("number").unwrap_or_default(),
            trunk_id: row.try_get("trunk_id").unwrap_or_default(),
            trunk_name: row.try_get("trunk_name").unwrap_or_default(),
            assigned_to: row.try_get("assigned_to").unwrap_or_default(),
            assigned_type: row.try_get("assigned_type").unwrap_or_default(),
            description: row.try_get("description").unwrap_or_default(),
            created_at: created.to_rfc3339(),
        }
    }).collect();

    Ok(Json(ApiResponse::success(dids, rid())))
}

async fn create_did(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateDidBody>,
) -> ConsoleResult<Json<ApiResponse<DidNumberView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.number.is_empty() {
        return Err(ConsoleError::BadRequest("DID number is required".into()));
    }

    // Validate assigned_type if provided
    if let Some(ref at) = body.assigned_type {
        if !["queue", "agent", "ivr", "unassigned"].contains(&at.as_str()) {
            return Err(ConsoleError::BadRequest("assigned_type must be queue, agent, ivr, or unassigned".into()));
        }
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Generate next DID-NNN ID
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM did_numbers",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count DIDs failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("DID-{:03}", count + 1);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO did_numbers (id, number, trunk_id, assigned_to, assigned_type, description) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(&body.number)
    .bind(&body.trunk_id)
    .bind(&body.assigned_to)
    .bind(&body.assigned_type)
    .bind(&body.description)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert DID failed: {e}")))?;

    // Resolve trunk name
    let trunk_name = if let Some(ref tid) = body.trunk_id {
        let name_row = rvoip_call_engine::database::sqlx::query(
            "SELECT name FROM sip_trunks WHERE id = $1",
        )
        .bind(tid)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("fetch trunk name failed: {e}")))?;
        name_row.map(|r| r.try_get::<String, _>("name").unwrap_or_default())
    } else {
        None
    };

    let view = DidNumberView {
        id,
        number: body.number,
        trunk_id: body.trunk_id,
        trunk_name,
        assigned_to: body.assigned_to,
        assigned_type: body.assigned_type,
        description: body.description,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_did(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(did_id): Path<String>,
    Json(body): Json<UpdateDidBody>,
) -> ConsoleResult<Json<ApiResponse<DidNumberView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    // Validate assigned_type if provided
    if let Some(ref at) = body.assigned_type {
        if !["queue", "agent", "ivr", "unassigned"].contains(&at.as_str()) {
            return Err(ConsoleError::BadRequest("assigned_type must be queue, agent, ivr, or unassigned".into()));
        }
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Fetch current
    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, number, trunk_id, assigned_to, assigned_type, description, created_at \
         FROM did_numbers WHERE id = $1",
    )
    .bind(&did_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch DID failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("DID {did_id}")))?;

    let trunk_id: Option<String> = body.trunk_id.or_else(|| current.try_get("trunk_id").unwrap_or_default());
    let assigned_to: Option<String> = body.assigned_to.or_else(|| current.try_get("assigned_to").unwrap_or_default());
    let assigned_type: Option<String> = body.assigned_type.or_else(|| current.try_get("assigned_type").unwrap_or_default());
    let description: Option<String> = body.description.or_else(|| current.try_get("description").unwrap_or_default());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE did_numbers SET trunk_id=$1, assigned_to=$2, assigned_type=$3, description=$4 WHERE id=$5",
    )
    .bind(&trunk_id)
    .bind(&assigned_to)
    .bind(&assigned_type)
    .bind(&description)
    .bind(&did_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update DID failed: {e}")))?;

    // Resolve trunk name
    let trunk_name = if let Some(ref tid) = trunk_id {
        let name_row = rvoip_call_engine::database::sqlx::query(
            "SELECT name FROM sip_trunks WHERE id = $1",
        )
        .bind(tid)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("fetch trunk name failed: {e}")))?;
        name_row.map(|r| r.try_get::<String, _>("name").unwrap_or_default())
    } else {
        None
    };

    let number: String = current.try_get("number").unwrap_or_default();
    let created: chrono::DateTime<chrono::Utc> = current.try_get("created_at").unwrap_or_default();

    let view = DidNumberView {
        id: did_id,
        number,
        trunk_id,
        trunk_name,
        assigned_to,
        assigned_type,
        description,
        created_at: created.to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_did(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(did_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM did_numbers WHERE id = $1",
    )
    .bind(&did_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete DID failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("DID {did_id}")));
    }

    Ok(Json(ApiResponse::success(format!("DID {did_id} deleted"), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_trunks).post(create_trunk))
        .route("/{id}", put(update_trunk).delete(delete_trunk))
        .route("/did", get(list_dids).post(create_did))
        .route("/did/{id}", put(update_did).delete(delete_did))
}
