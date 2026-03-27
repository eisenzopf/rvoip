//! Schedule / shift management CRUD endpoints.
//!
//! Two tables: `shifts` (shift definitions) and `schedule_entries` (per-agent
//! daily assignments). Auto-generated sequential IDs (SHF-001, ENT-001, ...).

use axum::{Router, routing::{get, post, put, delete}, extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ShiftView {
    pub id: String,
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub break_minutes: i32,
    pub color: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateShiftBody {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub break_minutes: Option<i32>,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateShiftBody {
    pub name: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub break_minutes: Option<i32>,
    pub color: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScheduleEntryView {
    pub id: String,
    pub agent_id: String,
    pub shift_id: Option<String>,
    pub date: String,
    pub status: String,
    pub check_in_at: Option<String>,
    pub check_out_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEntryBody {
    pub agent_id: String,
    pub shift_id: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEntryBody {
    pub status: Option<String>,
    pub check_in_at: Option<String>,
    pub check_out_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EntryQuery {
    pub date: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AttendanceSummary {
    pub scheduled: i64,
    pub checked_in: i64,
    pub checked_out: i64,
    pub absent: i64,
    pub leave: i64,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database helpers ---------------------------------------------------------

/// Ensure shifts + schedule_entries tables exist and seed defaults.
pub async fn init_schedules_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS shifts (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL, \
            start_time TEXT NOT NULL, \
            end_time TEXT NOT NULL, \
            break_minutes INTEGER DEFAULT 60, \
            color TEXT DEFAULT '#4CAF50', \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create shifts table: {e}"))?;

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS schedule_entries (\
            id TEXT PRIMARY KEY, \
            agent_id TEXT NOT NULL, \
            shift_id TEXT REFERENCES shifts(id), \
            date DATE NOT NULL, \
            status TEXT DEFAULT 'scheduled' CHECK (status IN ('scheduled','checked_in','checked_out','absent','leave')), \
            check_in_at TIMESTAMPTZ, \
            check_out_at TIMESTAMPTZ, \
            notes TEXT, \
            UNIQUE(agent_id, date)\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create schedule_entries table: {e}"))?;

    // Seed default shifts (idempotent)
    let defaults: &[(&str, &str, &str, &str, i32, &str)] = &[
        ("SHF-001", "早班", "09:00", "17:00", 60, "#4CAF50"),
        ("SHF-002", "中班", "13:00", "21:00", 60, "#2196F3"),
        ("SHF-003", "晚班", "21:00", "05:00", 60, "#FF9800"),
    ];

    for (id, name, start, end, brk, color) in defaults {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO shifts (id, name, start_time, end_time, break_minutes, color) \
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(start)
        .bind(end)
        .bind(brk)
        .bind(color)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

fn row_to_shift(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> ShiftView {
    let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at")
        .unwrap_or_default();
    ShiftView {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        start_time: row.try_get("start_time").unwrap_or_default(),
        end_time: row.try_get("end_time").unwrap_or_default(),
        break_minutes: row.try_get("break_minutes").unwrap_or(60),
        color: row.try_get("color").unwrap_or_default(),
        created_at: created_at.to_rfc3339(),
    }
}

fn row_to_entry(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> ScheduleEntryView {
    let check_in: Option<chrono::DateTime<chrono::Utc>> = row.try_get("check_in_at").unwrap_or(None);
    let check_out: Option<chrono::DateTime<chrono::Utc>> = row.try_get("check_out_at").unwrap_or(None);
    let date_val: chrono::NaiveDate = row.try_get("date").unwrap_or_default();

    ScheduleEntryView {
        id: row.try_get("id").unwrap_or_default(),
        agent_id: row.try_get("agent_id").unwrap_or_default(),
        shift_id: row.try_get("shift_id").unwrap_or(None),
        date: date_val.to_string(),
        status: row.try_get("status").unwrap_or_default(),
        check_in_at: check_in.map(|t| t.to_rfc3339()),
        check_out_at: check_out.map(|t| t.to_rfc3339()),
        notes: row.try_get("notes").unwrap_or(None),
    }
}

async fn next_shift_id(db: &rvoip_call_engine::database::DatabaseManager) -> Result<String, ConsoleError> {
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM shifts",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count shifts failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or(0);
    Ok(format!("SHF-{:03}", count + 1))
}

async fn next_entry_id(db: &rvoip_call_engine::database::DatabaseManager) -> Result<String, ConsoleError> {
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM schedule_entries",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count schedule_entries failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or(0);
    Ok(format!("ENT-{:03}", count + 1))
}

// -- Shift Handlers -----------------------------------------------------------

async fn list_shifts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<ShiftView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, start_time, end_time, break_minutes, color, created_at \
         FROM shifts ORDER BY id ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("shifts query failed: {e}")))?;

    let shifts: Vec<ShiftView> = rows.iter().map(row_to_shift).collect();
    Ok(Json(ApiResponse::success(shifts, rid())))
}

async fn create_shift(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateShiftBody>,
) -> ConsoleResult<Json<ApiResponse<ShiftView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    if body.name.trim().is_empty() {
        return Err(ConsoleError::BadRequest("name is required".into()));
    }

    let id = next_shift_id(db).await?;
    let brk = body.break_minutes.unwrap_or(60);
    let color = body.color.as_deref().unwrap_or("#4CAF50");

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO shifts (id, name, start_time, end_time, break_minutes, color) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(body.name.trim())
    .bind(&body.start_time)
    .bind(&body.end_time)
    .bind(brk)
    .bind(color)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert shift failed: {e}")))?;

    let shift = ShiftView {
        id,
        name: body.name.trim().to_string(),
        start_time: body.start_time,
        end_time: body.end_time,
        break_minutes: brk,
        color: color.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(shift, rid())))
}

async fn update_shift(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(shift_id): Path<String>,
    Json(body): Json<UpdateShiftBody>,
) -> ConsoleResult<Json<ApiResponse<ShiftView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, start_time, end_time, break_minutes, color, created_at \
         FROM shifts WHERE id = $1",
    )
    .bind(&shift_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch shift failed: {e}")))?;

    let existing = match row {
        Some(r) => row_to_shift(&r),
        None => return Err(ConsoleError::NotFound(format!("shift {shift_id}"))),
    };

    let new_name = body.name.as_deref().map(|s| s.trim()).unwrap_or(&existing.name);
    let new_start = body.start_time.as_deref().unwrap_or(&existing.start_time);
    let new_end = body.end_time.as_deref().unwrap_or(&existing.end_time);
    let new_brk = body.break_minutes.unwrap_or(existing.break_minutes);
    let new_color = body.color.as_deref().unwrap_or(&existing.color);

    rvoip_call_engine::database::sqlx::query(
        "UPDATE shifts SET name = $1, start_time = $2, end_time = $3, break_minutes = $4, color = $5 \
         WHERE id = $6",
    )
    .bind(new_name)
    .bind(new_start)
    .bind(new_end)
    .bind(new_brk)
    .bind(new_color)
    .bind(&shift_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update shift failed: {e}")))?;

    let updated = ShiftView {
        id: shift_id,
        name: new_name.to_string(),
        start_time: new_start.to_string(),
        end_time: new_end.to_string(),
        break_minutes: new_brk,
        color: new_color.to_string(),
        created_at: existing.created_at,
    };

    Ok(Json(ApiResponse::success(updated, rid())))
}

async fn delete_shift(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(shift_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM shifts WHERE id = $1",
    )
    .bind(&shift_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete shift failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("shift {shift_id}")));
    }

    Ok(Json(ApiResponse::success(format!("shift {shift_id} deleted"), rid())))
}

// -- Entry Handlers -----------------------------------------------------------

async fn list_entries(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<EntryQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<ScheduleEntryView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    // Build query dynamically based on filters
    let (sql, binds) = match (&q.date, &q.agent_id) {
        (Some(date), Some(agent)) => (
            "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
             FROM schedule_entries WHERE date = $1::date AND agent_id = $2 ORDER BY date ASC",
            vec![date.clone(), agent.clone()],
        ),
        (Some(date), None) => (
            "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
             FROM schedule_entries WHERE date = $1::date ORDER BY date ASC",
            vec![date.clone()],
        ),
        (None, Some(agent)) => (
            "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
             FROM schedule_entries WHERE agent_id = $1 ORDER BY date ASC",
            vec![agent.clone()],
        ),
        (None, None) => (
            "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
             FROM schedule_entries ORDER BY date ASC LIMIT 200",
            vec![],
        ),
    };

    let mut query = rvoip_call_engine::database::sqlx::query(sql);
    for b in &binds {
        query = query.bind(b);
    }

    let rows = query
        .fetch_all(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("schedule_entries query failed: {e}")))?;

    let entries: Vec<ScheduleEntryView> = rows.iter().map(row_to_entry).collect();
    Ok(Json(ApiResponse::success(entries, rid())))
}

async fn create_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateEntryBody>,
) -> ConsoleResult<Json<ApiResponse<ScheduleEntryView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    if body.agent_id.trim().is_empty() || body.shift_id.trim().is_empty() || body.date.trim().is_empty() {
        return Err(ConsoleError::BadRequest("agent_id, shift_id, and date are required".into()));
    }

    let id = next_entry_id(db).await?;

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO schedule_entries (id, agent_id, shift_id, date) \
         VALUES ($1, $2, $3, $4::date)",
    )
    .bind(&id)
    .bind(body.agent_id.trim())
    .bind(body.shift_id.trim())
    .bind(body.date.trim())
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert schedule_entry failed: {e}")))?;

    let entry = ScheduleEntryView {
        id,
        agent_id: body.agent_id.trim().to_string(),
        shift_id: Some(body.shift_id.trim().to_string()),
        date: body.date.trim().to_string(),
        status: "scheduled".to_string(),
        check_in_at: None,
        check_out_at: None,
        notes: None,
    };

    Ok(Json(ApiResponse::success(entry, rid())))
}

async fn update_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(entry_id): Path<String>,
    Json(body): Json<UpdateEntryBody>,
) -> ConsoleResult<Json<ApiResponse<ScheduleEntryView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
         FROM schedule_entries WHERE id = $1",
    )
    .bind(&entry_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch entry failed: {e}")))?;

    let existing = match row {
        Some(r) => row_to_entry(&r),
        None => return Err(ConsoleError::NotFound(format!("entry {entry_id}"))),
    };

    let new_status = body.status.as_deref().unwrap_or(&existing.status);
    let new_notes = body.notes.as_ref().or(existing.notes.as_ref());

    // For check_in/out we only update if explicitly provided
    let new_check_in = body.check_in_at.as_ref().or(existing.check_in_at.as_ref());
    let new_check_out = body.check_out_at.as_ref().or(existing.check_out_at.as_ref());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE schedule_entries SET status = $1, check_in_at = $2::timestamptz, \
         check_out_at = $3::timestamptz, notes = $4 WHERE id = $5",
    )
    .bind(new_status)
    .bind(new_check_in)
    .bind(new_check_out)
    .bind(new_notes)
    .bind(&entry_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update entry failed: {e}")))?;

    let updated = ScheduleEntryView {
        id: entry_id,
        agent_id: existing.agent_id,
        shift_id: existing.shift_id,
        date: existing.date,
        status: new_status.to_string(),
        check_in_at: new_check_in.cloned(),
        check_out_at: new_check_out.cloned(),
        notes: new_notes.cloned(),
    };

    Ok(Json(ApiResponse::success(updated, rid())))
}

async fn delete_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(entry_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM schedule_entries WHERE id = $1",
    )
    .bind(&entry_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete entry failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("entry {entry_id}")));
    }

    Ok(Json(ApiResponse::success(format!("entry {entry_id} deleted"), rid())))
}

async fn checkin_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(entry_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<ScheduleEntryView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let now = chrono::Utc::now().to_rfc3339();

    rvoip_call_engine::database::sqlx::query(
        "UPDATE schedule_entries SET check_in_at = $1::timestamptz, status = 'checked_in' \
         WHERE id = $2",
    )
    .bind(&now)
    .bind(&entry_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("checkin failed: {e}")))?;

    // Re-fetch the updated entry
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
         FROM schedule_entries WHERE id = $1",
    )
    .bind(&entry_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch entry failed: {e}")))?;

    match row {
        Some(r) => Ok(Json(ApiResponse::success(row_to_entry(&r), rid()))),
        None => Err(ConsoleError::NotFound(format!("entry {entry_id}"))),
    }
}

async fn checkout_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(entry_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<ScheduleEntryView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let now = chrono::Utc::now().to_rfc3339();

    rvoip_call_engine::database::sqlx::query(
        "UPDATE schedule_entries SET check_out_at = $1::timestamptz, status = 'checked_out' \
         WHERE id = $2",
    )
    .bind(&now)
    .bind(&entry_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("checkout failed: {e}")))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, agent_id, shift_id, date, status, check_in_at, check_out_at, notes \
         FROM schedule_entries WHERE id = $1",
    )
    .bind(&entry_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch entry failed: {e}")))?;

    match row {
        Some(r) => Ok(Json(ApiResponse::success(row_to_entry(&r), rid()))),
        None => Err(ConsoleError::NotFound(format!("entry {entry_id}"))),
    }
}

async fn today_attendance(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<AttendanceSummary>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(AttendanceSummary {
            scheduled: 0, checked_in: 0, checked_out: 0, absent: 0, leave: 0,
        }, rid()))),
    };

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT \
            COUNT(*) FILTER (WHERE status = 'scheduled') AS scheduled, \
            COUNT(*) FILTER (WHERE status = 'checked_in') AS checked_in, \
            COUNT(*) FILTER (WHERE status = 'checked_out') AS checked_out, \
            COUNT(*) FILTER (WHERE status = 'absent') AS absent, \
            COUNT(*) FILTER (WHERE status = 'leave') AS on_leave \
         FROM schedule_entries WHERE date = CURRENT_DATE",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("attendance query failed: {e}")))?;

    let summary = AttendanceSummary {
        scheduled: row.try_get("scheduled").unwrap_or(0),
        checked_in: row.try_get("checked_in").unwrap_or(0),
        checked_out: row.try_get("checked_out").unwrap_or(0),
        absent: row.try_get("absent").unwrap_or(0),
        leave: row.try_get("on_leave").unwrap_or(0),
    };

    Ok(Json(ApiResponse::success(summary, rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/shifts", get(list_shifts).post(create_shift))
        .route("/shifts/{shift_id}", put(update_shift).delete(delete_shift))
        .route("/entries", get(list_entries).post(create_entry))
        .route("/entries/{entry_id}", put(update_entry).delete(delete_entry))
        .route("/entries/{entry_id}/checkin", post(checkin_entry))
        .route("/entries/{entry_id}/checkout", post(checkout_entry))
        .route("/today", get(today_attendance))
}
