//! IVR (Interactive Voice Response) menu management CRUD endpoints.
//!
//! IVR menus and their key-press options are persisted to PostgreSQL.
//! Each menu has an auto-generated sequential ID (IVR-001, IVR-002, ...).

use std::collections::HashMap;

use axum::{Router, routing::{get, post, put, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct IvrMenuView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub welcome_message: Option<String>,
    pub timeout_seconds: i32,
    pub max_retries: i32,
    pub timeout_action: String,
    pub invalid_action: String,
    pub is_root: bool,
    pub business_hours_start: String,
    pub business_hours_end: String,
    pub business_days: String,
    pub after_hours_action: String,
    pub options: Vec<IvrOptionView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IvrOptionView {
    pub id: String,
    pub digit: String,
    pub label: String,
    pub action_type: String,
    pub action_target: Option<String>,
    pub announcement: Option<String>,
    pub position: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreateMenuBody {
    pub name: String,
    pub description: Option<String>,
    pub welcome_message: Option<String>,
    pub timeout_seconds: Option<i32>,
    pub max_retries: Option<i32>,
    pub timeout_action: Option<String>,
    pub invalid_action: Option<String>,
    pub is_root: Option<bool>,
    pub business_hours_start: Option<String>,
    pub business_hours_end: Option<String>,
    pub business_days: Option<String>,
    pub after_hours_action: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMenuBody {
    pub name: Option<String>,
    pub description: Option<String>,
    pub welcome_message: Option<String>,
    pub timeout_seconds: Option<i32>,
    pub max_retries: Option<i32>,
    pub timeout_action: Option<String>,
    pub invalid_action: Option<String>,
    pub is_root: Option<bool>,
    pub business_hours_start: Option<String>,
    pub business_hours_end: Option<String>,
    pub business_days: Option<String>,
    pub after_hours_action: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOptionBody {
    pub digit: String,
    pub label: String,
    pub action_type: String,
    pub action_target: Option<String>,
    pub announcement: Option<String>,
    pub position: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOptionBody {
    pub digit: Option<String>,
    pub label: Option<String>,
    pub action_type: Option<String>,
    pub action_target: Option<String>,
    pub announcement: Option<String>,
    pub position: Option<i32>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database helpers ---------------------------------------------------------

/// Ensure the IVR tables exist and seed defaults.
pub async fn init_ivr_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS ivr_menus (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL, \
            description TEXT, \
            welcome_message TEXT, \
            timeout_seconds INTEGER DEFAULT 10, \
            max_retries INTEGER DEFAULT 3, \
            timeout_action TEXT DEFAULT 'repeat', \
            invalid_action TEXT DEFAULT 'repeat', \
            is_root BOOLEAN DEFAULT FALSE, \
            business_hours_start TEXT DEFAULT '09:00', \
            business_hours_end TEXT DEFAULT '18:00', \
            business_days TEXT DEFAULT '1,2,3,4,5', \
            after_hours_action TEXT DEFAULT 'voicemail', \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create ivr_menus table: {e}"))?;

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS ivr_options (\
            id TEXT PRIMARY KEY, \
            menu_id TEXT NOT NULL REFERENCES ivr_menus(id) ON DELETE CASCADE, \
            digit TEXT NOT NULL, \
            label TEXT NOT NULL, \
            action_type TEXT NOT NULL, \
            action_target TEXT, \
            announcement TEXT, \
            position INTEGER DEFAULT 0, \
            UNIQUE(menu_id, digit)\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create ivr_options table: {e}"))?;

    // Seed default root menu (idempotent)
    let _ = rvoip_call_engine::database::sqlx::query(
        "INSERT INTO ivr_menus (id, name, description, welcome_message, is_root) \
         VALUES ($1, $2, $3, $4, TRUE) ON CONFLICT (id) DO NOTHING",
    )
    .bind("IVR-001")
    .bind("\u{4e3b}\u{83dc}\u{5355}") // 主菜单
    .bind("\u{6765}\u{7535}\u{4e3b}\u{5bfc}\u{822a}\u{83dc}\u{5355}") // 来电主导航菜单
    .bind("\u{6b22}\u{8fce}\u{81f4}\u{7535}\u{ff0c}\u{8bf7}\u{6309}1\u{83b7}\u{53d6}\u{6280}\u{672f}\u{652f}\u{6301}\u{ff0c}\u{6309}2\u{8054}\u{7cfb}\u{9500}\u{552e}\u{ff0c}\u{6309}3\u{67e5}\u{8be2}\u{8d26}\u{5355}") // 欢迎致电，请按1获取技术支持，按2联系销售，按3查询账单
    .execute(db.pool())
    .await;

    // Seed default options
    let default_options = [
        ("OPT-001", "IVR-001", "1", "\u{6280}\u{672f}\u{652f}\u{6301}", "routeToQueue", Some("support"), 0),  // 技术支持
        ("OPT-002", "IVR-001", "2", "\u{9500}\u{552e}\u{54a8}\u{8be2}", "routeToQueue", Some("sales"), 1),     // 销售咨询
        ("OPT-003", "IVR-001", "3", "\u{8d26}\u{5355}\u{67e5}\u{8be2}", "routeToQueue", Some("billing"), 2),   // 账单查询
    ];

    for (id, menu_id, digit, label, action_type, action_target, position) in &default_options {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO ivr_options (id, menu_id, digit, label, action_type, action_target, position) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(menu_id)
        .bind(digit)
        .bind(label)
        .bind(action_type)
        .bind(action_target)
        .bind(position)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

fn row_to_menu(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> IvrMenuView {
    IvrMenuView {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        welcome_message: row.try_get("welcome_message").unwrap_or_default(),
        timeout_seconds: row.try_get("timeout_seconds").unwrap_or(10),
        max_retries: row.try_get("max_retries").unwrap_or(3),
        timeout_action: row.try_get("timeout_action").unwrap_or_default(),
        invalid_action: row.try_get("invalid_action").unwrap_or_default(),
        is_root: row.try_get("is_root").unwrap_or(false),
        business_hours_start: row.try_get("business_hours_start").unwrap_or_default(),
        business_hours_end: row.try_get("business_hours_end").unwrap_or_default(),
        business_days: row.try_get("business_days").unwrap_or_default(),
        after_hours_action: row.try_get("after_hours_action").unwrap_or_default(),
        options: Vec::new(),
    }
}

fn row_to_option(row: &rvoip_call_engine::database::sqlx::postgres::PgRow) -> (String, IvrOptionView) {
    let menu_id: String = row.try_get("menu_id").unwrap_or_default();
    let opt = IvrOptionView {
        id: row.try_get("id").unwrap_or_default(),
        digit: row.try_get("digit").unwrap_or_default(),
        label: row.try_get("label").unwrap_or_default(),
        action_type: row.try_get("action_type").unwrap_or_default(),
        action_target: row.try_get("action_target").unwrap_or_default(),
        announcement: row.try_get("announcement").unwrap_or_default(),
        position: row.try_get("position").unwrap_or(0),
    };
    (menu_id, opt)
}

/// Generate the next sequential ID (IVR-NNN).
async fn next_menu_id(db: &rvoip_call_engine::database::DatabaseManager) -> Result<String, ConsoleError> {
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM ivr_menus",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count ivr_menus failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or(0);
    Ok(format!("IVR-{:03}", count + 1))
}

/// Generate the next sequential option ID (OPT-NNN).
async fn next_option_id(db: &rvoip_call_engine::database::DatabaseManager) -> Result<String, ConsoleError> {
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM ivr_options",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count ivr_options failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or(0);
    Ok(format!("OPT-{:03}", count + 1))
}

// -- Handlers -----------------------------------------------------------------

async fn list_menus(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<IvrMenuView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let menu_rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, description, welcome_message, timeout_seconds, max_retries, \
         timeout_action, invalid_action, is_root, business_hours_start, business_hours_end, \
         business_days, after_hours_action \
         FROM ivr_menus ORDER BY id ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("ivr_menus query failed: {e}")))?;

    let option_rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, menu_id, digit, label, action_type, action_target, announcement, position \
         FROM ivr_options ORDER BY position ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("ivr_options query failed: {e}")))?;

    // Group options by menu_id
    let mut options_map: HashMap<String, Vec<IvrOptionView>> = HashMap::new();
    for row in &option_rows {
        let (menu_id, opt) = row_to_option(row);
        options_map.entry(menu_id).or_default().push(opt);
    }

    let menus: Vec<IvrMenuView> = menu_rows.iter().map(|r| {
        let mut menu = row_to_menu(r);
        menu.options = options_map.remove(&menu.id).unwrap_or_default();
        menu
    }).collect();

    Ok(Json(ApiResponse::success(menus, rid())))
}

async fn create_menu(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateMenuBody>,
) -> ConsoleResult<Json<ApiResponse<IvrMenuView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    if body.name.trim().is_empty() {
        return Err(ConsoleError::BadRequest("name is required".into()));
    }

    let id = next_menu_id(db).await?;
    let timeout_seconds = body.timeout_seconds.unwrap_or(10);
    let max_retries = body.max_retries.unwrap_or(3);
    let timeout_action = body.timeout_action.as_deref().unwrap_or("repeat");
    let invalid_action = body.invalid_action.as_deref().unwrap_or("repeat");
    let is_root = body.is_root.unwrap_or(false);
    let bhs = body.business_hours_start.as_deref().unwrap_or("09:00");
    let bhe = body.business_hours_end.as_deref().unwrap_or("18:00");
    let bd = body.business_days.as_deref().unwrap_or("1,2,3,4,5");
    let aha = body.after_hours_action.as_deref().unwrap_or("voicemail");

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO ivr_menus (id, name, description, welcome_message, timeout_seconds, max_retries, \
         timeout_action, invalid_action, is_root, business_hours_start, business_hours_end, \
         business_days, after_hours_action) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(&id)
    .bind(body.name.trim())
    .bind(&body.description)
    .bind(&body.welcome_message)
    .bind(timeout_seconds)
    .bind(max_retries)
    .bind(timeout_action)
    .bind(invalid_action)
    .bind(is_root)
    .bind(bhs)
    .bind(bhe)
    .bind(bd)
    .bind(aha)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert ivr_menu failed: {e}")))?;

    let menu = IvrMenuView {
        id,
        name: body.name.trim().to_string(),
        description: body.description,
        welcome_message: body.welcome_message,
        timeout_seconds,
        max_retries,
        timeout_action: timeout_action.to_string(),
        invalid_action: invalid_action.to_string(),
        is_root,
        business_hours_start: bhs.to_string(),
        business_hours_end: bhe.to_string(),
        business_days: bd.to_string(),
        after_hours_action: aha.to_string(),
        options: Vec::new(),
    };

    Ok(Json(ApiResponse::success(menu, rid())))
}

async fn get_menu(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(menu_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<IvrMenuView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, description, welcome_message, timeout_seconds, max_retries, \
         timeout_action, invalid_action, is_root, business_hours_start, business_hours_end, \
         business_days, after_hours_action \
         FROM ivr_menus WHERE id = $1",
    )
    .bind(&menu_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch ivr_menu failed: {e}")))?;

    let mut menu = match row {
        Some(r) => row_to_menu(&r),
        None => return Err(ConsoleError::NotFound(format!("ivr menu {menu_id}"))),
    };

    let opt_rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, menu_id, digit, label, action_type, action_target, announcement, position \
         FROM ivr_options WHERE menu_id = $1 ORDER BY position ASC",
    )
    .bind(&menu_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch ivr_options failed: {e}")))?;

    menu.options = opt_rows.iter().map(|r| row_to_option(r).1).collect();

    Ok(Json(ApiResponse::success(menu, rid())))
}

async fn update_menu(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(menu_id): Path<String>,
    Json(body): Json<UpdateMenuBody>,
) -> ConsoleResult<Json<ApiResponse<IvrMenuView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, description, welcome_message, timeout_seconds, max_retries, \
         timeout_action, invalid_action, is_root, business_hours_start, business_hours_end, \
         business_days, after_hours_action \
         FROM ivr_menus WHERE id = $1",
    )
    .bind(&menu_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch ivr_menu failed: {e}")))?;

    let existing = match row {
        Some(r) => row_to_menu(&r),
        None => return Err(ConsoleError::NotFound(format!("ivr menu {menu_id}"))),
    };

    let name = body.name.as_deref().map(|s| s.trim()).unwrap_or(&existing.name);
    let description = body.description.as_ref().or(existing.description.as_ref());
    let welcome_message = body.welcome_message.as_ref().or(existing.welcome_message.as_ref());
    let timeout_seconds = body.timeout_seconds.unwrap_or(existing.timeout_seconds);
    let max_retries = body.max_retries.unwrap_or(existing.max_retries);
    let timeout_action = body.timeout_action.as_deref().unwrap_or(&existing.timeout_action);
    let invalid_action = body.invalid_action.as_deref().unwrap_or(&existing.invalid_action);
    let is_root = body.is_root.unwrap_or(existing.is_root);
    let bhs = body.business_hours_start.as_deref().unwrap_or(&existing.business_hours_start);
    let bhe = body.business_hours_end.as_deref().unwrap_or(&existing.business_hours_end);
    let bd = body.business_days.as_deref().unwrap_or(&existing.business_days);
    let aha = body.after_hours_action.as_deref().unwrap_or(&existing.after_hours_action);

    rvoip_call_engine::database::sqlx::query(
        "UPDATE ivr_menus SET name = $1, description = $2, welcome_message = $3, \
         timeout_seconds = $4, max_retries = $5, timeout_action = $6, invalid_action = $7, \
         is_root = $8, business_hours_start = $9, business_hours_end = $10, \
         business_days = $11, after_hours_action = $12 WHERE id = $13",
    )
    .bind(name)
    .bind(description)
    .bind(welcome_message)
    .bind(timeout_seconds)
    .bind(max_retries)
    .bind(timeout_action)
    .bind(invalid_action)
    .bind(is_root)
    .bind(bhs)
    .bind(bhe)
    .bind(bd)
    .bind(aha)
    .bind(&menu_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update ivr_menu failed: {e}")))?;

    // Re-fetch options for the response
    let opt_rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, menu_id, digit, label, action_type, action_target, announcement, position \
         FROM ivr_options WHERE menu_id = $1 ORDER BY position ASC",
    )
    .bind(&menu_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch ivr_options failed: {e}")))?;

    let options: Vec<IvrOptionView> = opt_rows.iter().map(|r| row_to_option(r).1).collect();

    let updated = IvrMenuView {
        id: menu_id,
        name: name.to_string(),
        description: description.cloned(),
        welcome_message: welcome_message.cloned(),
        timeout_seconds,
        max_retries,
        timeout_action: timeout_action.to_string(),
        invalid_action: invalid_action.to_string(),
        is_root,
        business_hours_start: bhs.to_string(),
        business_hours_end: bhe.to_string(),
        business_days: bd.to_string(),
        after_hours_action: aha.to_string(),
        options,
    };

    Ok(Json(ApiResponse::success(updated, rid())))
}

async fn delete_menu(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(menu_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM ivr_menus WHERE id = $1",
    )
    .bind(&menu_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete ivr_menu failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("ivr menu {menu_id}")));
    }

    Ok(Json(ApiResponse::success(format!("ivr menu {menu_id} deleted"), rid())))
}

async fn create_option(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(menu_id): Path<String>,
    Json(body): Json<CreateOptionBody>,
) -> ConsoleResult<Json<ApiResponse<IvrOptionView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    if body.digit.trim().is_empty() || body.label.trim().is_empty() {
        return Err(ConsoleError::BadRequest("digit and label are required".into()));
    }

    // Verify menu exists
    let exists = rvoip_call_engine::database::sqlx::query(
        "SELECT id FROM ivr_menus WHERE id = $1",
    )
    .bind(&menu_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("check ivr_menu failed: {e}")))?;

    if exists.is_none() {
        return Err(ConsoleError::NotFound(format!("ivr menu {menu_id}")));
    }

    let id = next_option_id(db).await?;
    let position = body.position.unwrap_or(0);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO ivr_options (id, menu_id, digit, label, action_type, action_target, announcement, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&id)
    .bind(&menu_id)
    .bind(body.digit.trim())
    .bind(body.label.trim())
    .bind(&body.action_type)
    .bind(&body.action_target)
    .bind(&body.announcement)
    .bind(position)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert ivr_option failed: {e}")))?;

    let opt = IvrOptionView {
        id,
        digit: body.digit.trim().to_string(),
        label: body.label.trim().to_string(),
        action_type: body.action_type,
        action_target: body.action_target,
        announcement: body.announcement,
        position,
    };

    Ok(Json(ApiResponse::success(opt, rid())))
}

async fn update_option(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((menu_id, option_id)): Path<(String, String)>,
    Json(body): Json<UpdateOptionBody>,
) -> ConsoleResult<Json<ApiResponse<IvrOptionView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT id, menu_id, digit, label, action_type, action_target, announcement, position \
         FROM ivr_options WHERE id = $1 AND menu_id = $2",
    )
    .bind(&option_id)
    .bind(&menu_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch ivr_option failed: {e}")))?;

    let (_, existing) = match row {
        Some(r) => row_to_option(&r),
        None => return Err(ConsoleError::NotFound(format!("ivr option {option_id}"))),
    };

    let digit = body.digit.as_deref().map(|s| s.trim()).unwrap_or(&existing.digit);
    let label = body.label.as_deref().map(|s| s.trim()).unwrap_or(&existing.label);
    let action_type = body.action_type.as_deref().unwrap_or(&existing.action_type);
    let action_target = body.action_target.as_ref().or(existing.action_target.as_ref());
    let announcement = body.announcement.as_ref().or(existing.announcement.as_ref());
    let position = body.position.unwrap_or(existing.position);

    rvoip_call_engine::database::sqlx::query(
        "UPDATE ivr_options SET digit = $1, label = $2, action_type = $3, \
         action_target = $4, announcement = $5, position = $6 WHERE id = $7 AND menu_id = $8",
    )
    .bind(digit)
    .bind(label)
    .bind(action_type)
    .bind(action_target)
    .bind(announcement)
    .bind(position)
    .bind(&option_id)
    .bind(&menu_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update ivr_option failed: {e}")))?;

    let updated = IvrOptionView {
        id: option_id,
        digit: digit.to_string(),
        label: label.to_string(),
        action_type: action_type.to_string(),
        action_target: action_target.cloned(),
        announcement: announcement.cloned(),
        position,
    };

    Ok(Json(ApiResponse::success(updated, rid())))
}

async fn delete_option(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((menu_id, option_id)): Path<(String, String)>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM ivr_options WHERE id = $1 AND menu_id = $2",
    )
    .bind(&option_id)
    .bind(&menu_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete ivr_option failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("ivr option {option_id}")));
    }

    Ok(Json(ApiResponse::success(format!("ivr option {option_id} deleted"), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_menus).post(create_menu))
        .route("/{menu_id}", get(get_menu).put(update_menu).delete(delete_menu))
        .route("/{menu_id}/options", post(create_option))
        .route("/{menu_id}/options/{option_id}", put(update_option).delete(delete_option))
}
