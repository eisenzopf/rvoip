//! Call quality check (QC) endpoints — templates + scoring.

use axum::{Router, routing::{get, put, post, delete}, extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct QcTemplateItemView {
    pub id: String,
    pub template_id: String,
    pub category: String,
    pub item_name: String,
    pub max_score: i32,
    pub description: Option<String>,
    pub position: i32,
}

#[derive(Debug, Serialize)]
pub struct QcTemplateView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub total_score: i32,
    pub created_at: String,
    pub items: Vec<QcTemplateItemView>,
}

#[derive(Debug, Serialize)]
pub struct QcScoreItemView {
    pub id: String,
    pub score_id: String,
    pub item_id: Option<String>,
    pub score: i32,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QcScoreView {
    pub id: String,
    pub call_id: String,
    pub agent_id: String,
    pub template_id: Option<String>,
    pub scorer_id: String,
    pub total_score: Option<i32>,
    pub max_score: Option<i32>,
    pub comments: Option<String>,
    pub scored_at: String,
    pub items: Vec<QcScoreItemView>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateBody {
    pub name: String,
    pub description: Option<String>,
    pub total_score: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateBody {
    pub name: Option<String>,
    pub description: Option<String>,
    pub total_score: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateItemBody {
    pub category: String,
    pub item_name: String,
    pub max_score: i32,
    pub description: Option<String>,
    pub position: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateItemBody {
    pub category: Option<String>,
    pub item_name: Option<String>,
    pub max_score: Option<i32>,
    pub description: Option<String>,
    pub position: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ScoreItemEntry {
    pub item_id: Option<String>,
    pub score: i32,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitScoreBody {
    pub call_id: String,
    pub agent_id: String,
    pub template_id: Option<String>,
    pub items: Vec<ScoreItemEntry>,
    pub comments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScoreQueryParams {
    pub agent_id: Option<String>,
    pub call_id: Option<String>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database init ------------------------------------------------------------

/// Ensure QC tables exist and seed default template.
pub async fn init_quality_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS qc_templates (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL, \
            description TEXT, \
            total_score INTEGER NOT NULL DEFAULT 100, \
            created_at TIMESTAMPTZ DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create qc_templates table: {e}"))?;

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS qc_template_items (\
            id TEXT PRIMARY KEY, \
            template_id TEXT NOT NULL REFERENCES qc_templates(id) ON DELETE CASCADE, \
            category TEXT NOT NULL, \
            item_name TEXT NOT NULL, \
            max_score INTEGER NOT NULL, \
            description TEXT, \
            position INTEGER DEFAULT 0\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create qc_template_items table: {e}"))?;

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS qc_scores (\
            id TEXT PRIMARY KEY, \
            call_id TEXT NOT NULL, \
            agent_id TEXT NOT NULL, \
            template_id TEXT REFERENCES qc_templates(id), \
            scorer_id TEXT NOT NULL, \
            total_score INTEGER, \
            max_score INTEGER, \
            comments TEXT, \
            scored_at TIMESTAMPTZ DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create qc_scores table: {e}"))?;

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS qc_score_items (\
            id TEXT PRIMARY KEY, \
            score_id TEXT NOT NULL REFERENCES qc_scores(id) ON DELETE CASCADE, \
            item_id TEXT REFERENCES qc_template_items(id), \
            score INTEGER NOT NULL, \
            comment TEXT\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create qc_score_items table: {e}"))?;

    // Seed default template (idempotent)
    let _ = rvoip_call_engine::database::sqlx::query(
        "INSERT INTO qc_templates (id, name, description, total_score) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
    )
    .bind("QCT-001")
    .bind("\u{6807}\u{51c6}\u{8d28}\u{68c0}\u{6a21}\u{677f}")
    .bind("\u{9ed8}\u{8ba4}\u{8d28}\u{68c0}\u{6a21}\u{677f}\u{ff0c}\u{5305}\u{542b}\u{5f00}\u{573a}\u{767d}\u{3001}\u{95ee}\u{9898}\u{89e3}\u{51b3}\u{3001}\u{7ed3}\u{675f}\u{8bed}\u{4e09}\u{4e2a}\u{5206}\u{7c7b}")
    .bind(100i32)
    .execute(db.pool())
    .await;

    // Seed default items
    let default_items: &[(&str, &str, &str, i32, i32)] = &[
        ("QCI-001", "QCT-001", "\u{5f00}\u{573a}\u{767d}", 10, 1),   // 开场白 - 问候语 10
        ("QCI-002", "QCT-001", "\u{5f00}\u{573a}\u{767d}", 10, 2),   // 开场白 - 确认身份 10
        ("QCI-003", "QCT-001", "\u{95ee}\u{9898}\u{89e3}\u{51b3}", 15, 3), // 问题解决 - 理解需求 15
        ("QCI-004", "QCT-001", "\u{95ee}\u{9898}\u{89e3}\u{51b3}", 20, 4), // 问题解决 - 提供方案 20
        ("QCI-005", "QCT-001", "\u{95ee}\u{9898}\u{89e3}\u{51b3}", 15, 5), // 问题解决 - 确认解决 15
        ("QCI-006", "QCT-001", "\u{7ed3}\u{675f}\u{8bed}", 10, 6),   // 结束语 - 其他问题 10
        ("QCI-007", "QCT-001", "\u{7ed3}\u{675f}\u{8bed}", 10, 7),   // 结束语 - 感谢 10
        ("QCI-008", "QCT-001", "\u{7ed3}\u{675f}\u{8bed}", 10, 8),   // 结束语 - 正确结束 10
    ];

    let item_names: &[&str] = &[
        "\u{95ee}\u{5019}\u{8bed}",       // 问候语
        "\u{786e}\u{8ba4}\u{8eab}\u{4efd}", // 确认身份
        "\u{7406}\u{89e3}\u{9700}\u{6c42}", // 理解需求
        "\u{63d0}\u{4f9b}\u{65b9}\u{6848}", // 提供方案
        "\u{786e}\u{8ba4}\u{89e3}\u{51b3}", // 确认解决
        "\u{5176}\u{4ed6}\u{95ee}\u{9898}", // 其他问题
        "\u{611f}\u{8c22}",               // 感谢
        "\u{6b63}\u{786e}\u{7ed3}\u{675f}", // 正确结束
    ];

    for (i, &(id, template_id, category, max_score, position)) in default_items.iter().enumerate() {
        let name = item_names.get(i).unwrap_or(&"");
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO qc_template_items (id, template_id, category, item_name, max_score, position) \
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(template_id)
        .bind(category)
        .bind(name)
        .bind(max_score)
        .bind(position)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

// -- Helpers ------------------------------------------------------------------

async fn fetch_template_items(
    db: &rvoip_call_engine::database::DatabaseManager,
    template_id: &str,
) -> Result<Vec<QcTemplateItemView>, ConsoleError> {
    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, template_id, category, item_name, max_score, description, position \
         FROM qc_template_items WHERE template_id = $1 ORDER BY position ASC",
    )
    .bind(template_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("qc_template_items query failed: {e}")))?;

    Ok(rows.iter().map(|row| QcTemplateItemView {
        id: row.try_get("id").unwrap_or_default(),
        template_id: row.try_get("template_id").unwrap_or_default(),
        category: row.try_get("category").unwrap_or_default(),
        item_name: row.try_get("item_name").unwrap_or_default(),
        max_score: row.try_get("max_score").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        position: row.try_get("position").unwrap_or_default(),
    }).collect())
}

// -- Handlers: Templates ------------------------------------------------------

async fn list_templates(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<QcTemplateView>>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, description, total_score, \
         COALESCE(TO_CHAR(created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '') AS created_at \
         FROM qc_templates ORDER BY created_at ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("qc_templates query failed: {e}")))?;

    let mut templates = Vec::new();
    for row in &rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let items = fetch_template_items(db, &id).await?;
        templates.push(QcTemplateView {
            id,
            name: row.try_get("name").unwrap_or_default(),
            description: row.try_get("description").unwrap_or_default(),
            total_score: row.try_get("total_score").unwrap_or_default(),
            created_at: row.try_get("created_at").unwrap_or_default(),
            items,
        });
    }

    Ok(Json(ApiResponse::success(templates, rid())))
}

async fn create_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateTemplateBody>,
) -> ConsoleResult<Json<ApiResponse<QcTemplateView>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.name.is_empty() {
        return Err(ConsoleError::BadRequest("template name is required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM qc_templates",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count templates failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("QCT-{:03}", count + 1);
    let total_score = body.total_score.unwrap_or(100);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO qc_templates (id, name, description, total_score) VALUES ($1, $2, $3, $4)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.description)
    .bind(total_score)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert template failed: {e}")))?;

    let view = QcTemplateView {
        id,
        name: body.name,
        description: body.description,
        total_score,
        created_at: String::new(),
        items: Vec::new(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(template_id): Path<String>,
    Json(body): Json<UpdateTemplateBody>,
) -> ConsoleResult<Json<ApiResponse<QcTemplateView>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, description, total_score FROM qc_templates WHERE id = $1",
    )
    .bind(&template_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch template failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("template {template_id}")))?;

    let name: String = body.name.unwrap_or_else(|| current.try_get("name").unwrap_or_default());
    let description: Option<String> = body.description.or_else(|| current.try_get("description").unwrap_or_default());
    let total_score: i32 = body.total_score.unwrap_or_else(|| current.try_get("total_score").unwrap_or(100));

    rvoip_call_engine::database::sqlx::query(
        "UPDATE qc_templates SET name = $1, description = $2, total_score = $3 WHERE id = $4",
    )
    .bind(&name)
    .bind(&description)
    .bind(total_score)
    .bind(&template_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update template failed: {e}")))?;

    let items = fetch_template_items(db, &template_id).await?;

    let view = QcTemplateView {
        id: template_id,
        name,
        description,
        total_score,
        created_at: String::new(),
        items,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(template_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM qc_templates WHERE id = $1",
    )
    .bind(&template_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete template failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("template {template_id}")));
    }

    Ok(Json(ApiResponse::success(format!("template {template_id} deleted"), rid())))
}

// -- Handlers: Template Items -------------------------------------------------

async fn add_template_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(template_id): Path<String>,
    Json(body): Json<CreateTemplateItemBody>,
) -> ConsoleResult<Json<ApiResponse<QcTemplateItemView>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.category.is_empty() || body.item_name.is_empty() {
        return Err(ConsoleError::BadRequest("category and item_name are required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Verify template exists
    let _ = rvoip_call_engine::database::sqlx::query(
        "SELECT id FROM qc_templates WHERE id = $1",
    )
    .bind(&template_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch template failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("template {template_id}")))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM qc_template_items WHERE template_id = $1",
    )
    .bind(&template_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count items failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("QCI-{:03}", count + 1);
    let position = body.position.unwrap_or((count + 1) as i32);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO qc_template_items (id, template_id, category, item_name, max_score, description, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&id)
    .bind(&template_id)
    .bind(&body.category)
    .bind(&body.item_name)
    .bind(body.max_score)
    .bind(&body.description)
    .bind(position)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert item failed: {e}")))?;

    let view = QcTemplateItemView {
        id,
        template_id,
        category: body.category,
        item_name: body.item_name,
        max_score: body.max_score,
        description: body.description,
        position,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_template_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((template_id, item_id)): Path<(String, String)>,
    Json(body): Json<UpdateTemplateItemBody>,
) -> ConsoleResult<Json<ApiResponse<QcTemplateItemView>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, template_id, category, item_name, max_score, description, position \
         FROM qc_template_items WHERE id = $1 AND template_id = $2",
    )
    .bind(&item_id)
    .bind(&template_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch item failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("item {item_id}")))?;

    let category: String = body.category.unwrap_or_else(|| current.try_get("category").unwrap_or_default());
    let item_name: String = body.item_name.unwrap_or_else(|| current.try_get("item_name").unwrap_or_default());
    let max_score: i32 = body.max_score.unwrap_or_else(|| current.try_get("max_score").unwrap_or_default());
    let description: Option<String> = body.description.or_else(|| current.try_get("description").unwrap_or_default());
    let position: i32 = body.position.unwrap_or_else(|| current.try_get("position").unwrap_or_default());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE qc_template_items SET category = $1, item_name = $2, max_score = $3, \
         description = $4, position = $5 WHERE id = $6 AND template_id = $7",
    )
    .bind(&category)
    .bind(&item_name)
    .bind(max_score)
    .bind(&description)
    .bind(position)
    .bind(&item_id)
    .bind(&template_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update item failed: {e}")))?;

    let view = QcTemplateItemView {
        id: item_id,
        template_id,
        category,
        item_name,
        max_score,
        description,
        position,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_template_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((template_id, item_id)): Path<(String, String)>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM qc_template_items WHERE id = $1 AND template_id = $2",
    )
    .bind(&item_id)
    .bind(&template_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete item failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("item {item_id}")));
    }

    Ok(Json(ApiResponse::success(format!("item {item_id} deleted"), rid())))
}

// -- Handlers: Scores ---------------------------------------------------------

async fn list_scores(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ScoreQueryParams>,
) -> ConsoleResult<Json<ApiResponse<Vec<QcScoreView>>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    // Build dynamic query
    let mut sql = String::from(
        "SELECT id, call_id, agent_id, template_id, scorer_id, total_score, max_score, comments, \
         COALESCE(TO_CHAR(scored_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '') AS scored_at \
         FROM qc_scores WHERE 1=1",
    );
    let mut bind_idx = 1u32;
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref agent_id) = params.agent_id {
        sql.push_str(&format!(" AND agent_id = ${bind_idx}"));
        bind_idx += 1;
        binds.push(agent_id.clone());
    }
    if let Some(ref call_id) = params.call_id {
        sql.push_str(&format!(" AND call_id = ${bind_idx}"));
        let _ = bind_idx; // suppress unused warning
        binds.push(call_id.clone());
    }
    sql.push_str(" ORDER BY scored_at DESC LIMIT 200");

    let mut query = rvoip_call_engine::database::sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    let rows = query.fetch_all(db.pool()).await
        .map_err(|e| ConsoleError::Internal(format!("qc_scores query failed: {e}")))?;

    let mut scores = Vec::new();
    for row in &rows {
        let score_id: String = row.try_get("id").unwrap_or_default();

        let item_rows = rvoip_call_engine::database::sqlx::query(
            "SELECT id, score_id, item_id, score, comment FROM qc_score_items WHERE score_id = $1",
        )
        .bind(&score_id)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("qc_score_items query failed: {e}")))?;

        let items: Vec<QcScoreItemView> = item_rows.iter().map(|r| QcScoreItemView {
            id: r.try_get("id").unwrap_or_default(),
            score_id: r.try_get("score_id").unwrap_or_default(),
            item_id: r.try_get("item_id").unwrap_or_default(),
            score: r.try_get("score").unwrap_or_default(),
            comment: r.try_get("comment").unwrap_or_default(),
        }).collect();

        scores.push(QcScoreView {
            id: score_id,
            call_id: row.try_get("call_id").unwrap_or_default(),
            agent_id: row.try_get("agent_id").unwrap_or_default(),
            template_id: row.try_get("template_id").unwrap_or_default(),
            scorer_id: row.try_get("scorer_id").unwrap_or_default(),
            total_score: row.try_get("total_score").unwrap_or_default(),
            max_score: row.try_get("max_score").unwrap_or_default(),
            comments: row.try_get("comments").unwrap_or_default(),
            scored_at: row.try_get("scored_at").unwrap_or_default(),
            items,
        });
    }

    Ok(Json(ApiResponse::success(scores, rid())))
}

async fn submit_score(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SubmitScoreBody>,
) -> ConsoleResult<Json<ApiResponse<QcScoreView>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.call_id.is_empty() || body.agent_id.is_empty() {
        return Err(ConsoleError::BadRequest("call_id and agent_id are required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let score_id = format!("QCS-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"));
    let scorer_id = auth.user_id.clone();

    let total_score: i32 = body.items.iter().map(|i| i.score).sum();
    let max_score: i32 = if let Some(ref tid) = body.template_id {
        let row = rvoip_call_engine::database::sqlx::query(
            "SELECT total_score FROM qc_templates WHERE id = $1",
        )
        .bind(tid)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("fetch template failed: {e}")))?;

        row.map(|r| r.try_get("total_score").unwrap_or(100)).unwrap_or(100)
    } else {
        100
    };

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO qc_scores (id, call_id, agent_id, template_id, scorer_id, total_score, max_score, comments) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&score_id)
    .bind(&body.call_id)
    .bind(&body.agent_id)
    .bind(&body.template_id)
    .bind(&scorer_id)
    .bind(total_score)
    .bind(max_score)
    .bind(&body.comments)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert score failed: {e}")))?;

    let mut items = Vec::new();
    for (idx, entry) in body.items.iter().enumerate() {
        let item_id = format!("{}-{:03}", score_id, idx + 1);
        rvoip_call_engine::database::sqlx::query(
            "INSERT INTO qc_score_items (id, score_id, item_id, score, comment) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&item_id)
        .bind(&score_id)
        .bind(&entry.item_id)
        .bind(entry.score)
        .bind(&entry.comment)
        .execute(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("insert score item failed: {e}")))?;

        items.push(QcScoreItemView {
            id: item_id,
            score_id: score_id.clone(),
            item_id: entry.item_id.clone(),
            score: entry.score,
            comment: entry.comment.clone(),
        });
    }

    let view = QcScoreView {
        id: score_id,
        call_id: body.call_id,
        agent_id: body.agent_id,
        template_id: body.template_id,
        scorer_id,
        total_score: Some(total_score),
        max_score: Some(max_score),
        comments: body.comments,
        scored_at: String::new(),
        items,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/templates", get(list_templates).post(create_template))
        .route("/templates/{id}", put(update_template).delete(delete_template))
        .route("/templates/{tid}/items", post(add_template_item))
        .route("/templates/{tid}/items/{iid}", put(update_template_item).delete(delete_template_item))
        .route("/scores", get(list_scores).post(submit_score))
}
