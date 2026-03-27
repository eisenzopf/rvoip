//! Department management CRUD endpoints.
//!
//! Departments are persisted to PostgreSQL. Each department has an
//! auto-generated sequential ID (DEPT-001, DEPT-002, ...).

use axum::{Router, routing::{get, post, put, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DepartmentView {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub description: Option<String>,
    pub manager_id: Option<String>,
    pub agent_count: i64,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateDepartmentBody {
    pub name: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub manager_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDepartmentBody {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub manager_id: Option<String>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database helpers ---------------------------------------------------------

/// Ensure the departments table exists and seed defaults.
pub async fn init_departments_table(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS departments (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL UNIQUE, \
            parent_id TEXT REFERENCES departments(id), \
            description TEXT, \
            manager_id TEXT, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create departments table: {e}"))?;

    // Seed default departments (idempotent)
    let defaults = [
        ("DEPT-001", "技术支持"),
        ("DEPT-002", "销售部"),
        ("DEPT-003", "客服部"),
        ("DEPT-004", "运营部"),
    ];

    for (id, name) in &defaults {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO departments (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

fn row_to_department(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> DepartmentView {
    let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at")
        .unwrap_or_default();

    DepartmentView {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        parent_id: row.try_get("parent_id").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        manager_id: row.try_get("manager_id").unwrap_or_default(),
        agent_count: 0, // no department_id column on agents yet
        created_at: created_at.to_rfc3339(),
    }
}

/// Generate the next sequential ID (DEPT-NNN).
async fn next_dept_id(db: &rvoip_call_engine::database::DatabaseManager) -> Result<String, ConsoleError> {
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM departments",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count departments failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or(0);
    Ok(format!("DEPT-{:03}", count + 1))
}

// -- Handlers -----------------------------------------------------------------

async fn list_departments(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<DepartmentView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, parent_id, description, manager_id, created_at \
         FROM departments ORDER BY id ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("departments query failed: {e}")))?;

    let departments: Vec<DepartmentView> = rows.iter().map(row_to_department).collect();
    Ok(Json(ApiResponse::success(departments, rid())))
}

async fn create_department(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateDepartmentBody>,
) -> ConsoleResult<Json<ApiResponse<DepartmentView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    if body.name.trim().is_empty() {
        return Err(ConsoleError::BadRequest("name is required".into()));
    }

    let id = next_dept_id(db).await?;

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO departments (id, name, parent_id, description, manager_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(body.name.trim())
    .bind(&body.parent_id)
    .bind(&body.description)
    .bind(&body.manager_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert department failed: {e}")))?;

    let dept = DepartmentView {
        id,
        name: body.name.trim().to_string(),
        parent_id: body.parent_id,
        description: body.description,
        manager_id: body.manager_id,
        agent_count: 0,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(dept, rid())))
}

async fn update_department(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(dept_id): Path<String>,
    Json(body): Json<UpdateDepartmentBody>,
) -> ConsoleResult<Json<ApiResponse<DepartmentView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Fetch existing department first
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, parent_id, description, manager_id, created_at \
         FROM departments WHERE id = $1",
    )
    .bind(&dept_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch department failed: {e}")))?;

    let existing = match row {
        Some(r) => row_to_department(&r),
        None => return Err(ConsoleError::NotFound(format!("department {dept_id}"))),
    };

    let new_name = body.name.as_deref().map(|s| s.trim()).unwrap_or(&existing.name);
    let new_description = body.description.as_ref().or(existing.description.as_ref());
    let new_parent = body.parent_id.as_ref().or(existing.parent_id.as_ref());
    let new_manager = body.manager_id.as_ref().or(existing.manager_id.as_ref());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE departments SET name = $1, description = $2, parent_id = $3, manager_id = $4 \
         WHERE id = $5",
    )
    .bind(new_name)
    .bind(new_description)
    .bind(new_parent)
    .bind(new_manager)
    .bind(&dept_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update department failed: {e}")))?;

    let updated = DepartmentView {
        id: dept_id,
        name: new_name.to_string(),
        parent_id: new_parent.cloned(),
        description: new_description.cloned(),
        manager_id: new_manager.cloned(),
        agent_count: 0,
        created_at: existing.created_at,
    };

    Ok(Json(ApiResponse::success(updated, rid())))
}

async fn delete_department(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(dept_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM departments WHERE id = $1",
    )
    .bind(&dept_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete department failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("department {dept_id}")));
    }

    Ok(Json(ApiResponse::success(format!("department {dept_id} deleted"), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_departments).post(create_department))
        .route("/{dept_id}", put(update_department).delete(delete_department))
}
