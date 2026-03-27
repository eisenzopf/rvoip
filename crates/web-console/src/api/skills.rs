//! Skill group management endpoints — definitions + agent skill assignments.

use axum::{Router, routing::{get, put, post, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SkillView {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub agent_count: i64,
}

#[derive(Debug, Serialize)]
pub struct AgentSkillView {
    pub skill_id: String,
    pub skill_name: String,
    pub proficiency: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreateSkillBody {
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillBody {
    pub name: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AgentSkillEntry {
    pub skill_id: String,
    pub proficiency: i32,
}

#[derive(Debug, Deserialize)]
pub struct SetAgentSkillsBody {
    pub skills: Vec<AgentSkillEntry>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database init ------------------------------------------------------------

/// Ensure skill tables exist and seed defaults.
pub async fn init_skills_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    // Create skill_definitions table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS skill_definitions (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL UNIQUE, \
            category TEXT, \
            description TEXT, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create skill_definitions table: {e}"))?;

    // Create agent_skills table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_skills (\
            agent_id TEXT NOT NULL, \
            skill_id TEXT NOT NULL REFERENCES skill_definitions(id), \
            proficiency INTEGER NOT NULL DEFAULT 1 CHECK (proficiency BETWEEN 1 AND 5), \
            PRIMARY KEY (agent_id, skill_id)\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create agent_skills table: {e}"))?;

    // Seed default skills (idempotent)
    let defaults = [
        ("SKL-001", "English", Some("Language"), Some("English language proficiency")),
        ("SKL-002", "Chinese", Some("Language"), Some("Chinese language proficiency")),
        ("SKL-003", "Sales", Some("Department"), Some("Sales skills")),
        ("SKL-004", "Technical Support", Some("Department"), Some("Technical support skills")),
        ("SKL-005", "VIP", Some("Service Level"), Some("VIP customer handling")),
        ("SKL-006", "Billing", Some("Department"), Some("Billing and payment support")),
    ];

    for (id, name, category, desc) in &defaults {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO skill_definitions (id, name, category, description) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(category)
        .bind(desc)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

// -- Handlers -----------------------------------------------------------------

async fn list_skills(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<SkillView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT s.id, s.name, s.category, s.description, \
         COUNT(a.agent_id) AS agent_count \
         FROM skill_definitions s \
         LEFT JOIN agent_skills a ON a.skill_id = s.id \
         GROUP BY s.id, s.name, s.category, s.description \
         ORDER BY s.name ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("skill_definitions query failed: {e}")))?;

    let skills: Vec<SkillView> = rows.iter().map(|row| SkillView {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        category: row.try_get("category").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        agent_count: row.try_get("agent_count").unwrap_or_default(),
    }).collect();

    Ok(Json(ApiResponse::success(skills, rid())))
}

async fn create_skill(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSkillBody>,
) -> ConsoleResult<Json<ApiResponse<SkillView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.name.is_empty() {
        return Err(ConsoleError::BadRequest("skill name is required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Generate next SKL-NNN ID
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM skill_definitions",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count skills failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("SKL-{:03}", count + 1);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO skill_definitions (id, name, category, description) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.category)
    .bind(&body.description)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert skill failed: {e}")))?;

    let view = SkillView {
        id,
        name: body.name,
        category: body.category,
        description: body.description,
        agent_count: 0,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_skill(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(skill_id): Path<String>,
    Json(body): Json<UpdateSkillBody>,
) -> ConsoleResult<Json<ApiResponse<SkillView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Fetch current
    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, category, description FROM skill_definitions WHERE id = $1",
    )
    .bind(&skill_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch skill failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("skill {skill_id}")))?;

    let name: String = body.name.unwrap_or_else(|| current.try_get("name").unwrap_or_default());
    let category: Option<String> = body.category.or_else(|| current.try_get("category").unwrap_or_default());
    let description: Option<String> = body.description.or_else(|| current.try_get("description").unwrap_or_default());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE skill_definitions SET name = $1, category = $2, description = $3 WHERE id = $4",
    )
    .bind(&name)
    .bind(&category)
    .bind(&description)
    .bind(&skill_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update skill failed: {e}")))?;

    // Get agent count
    let count_row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM agent_skills WHERE skill_id = $1",
    )
    .bind(&skill_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count agents failed: {e}")))?;

    let agent_count: i64 = count_row.try_get("cnt").unwrap_or_default();

    let view = SkillView { id: skill_id, name, category, description, agent_count };
    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_skill(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(skill_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Check if agents have this skill
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM agent_skills WHERE skill_id = $1",
    )
    .bind(&skill_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("check agent_skills failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    if count > 0 {
        return Err(ConsoleError::BadRequest(
            format!("cannot delete skill assigned to {count} agents"),
        ));
    }

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM skill_definitions WHERE id = $1",
    )
    .bind(&skill_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete skill failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("skill {skill_id}")));
    }

    Ok(Json(ApiResponse::success(format!("skill {skill_id} deleted"), rid())))
}

async fn get_agent_skills(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<Vec<AgentSkillView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT a.skill_id, s.name AS skill_name, a.proficiency \
         FROM agent_skills a \
         JOIN skill_definitions s ON s.id = a.skill_id \
         WHERE a.agent_id = $1 \
         ORDER BY s.name ASC",
    )
    .bind(&agent_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("agent_skills query failed: {e}")))?;

    let skills: Vec<AgentSkillView> = rows.iter().map(|row| AgentSkillView {
        skill_id: row.try_get("skill_id").unwrap_or_default(),
        skill_name: row.try_get("skill_name").unwrap_or_default(),
        proficiency: row.try_get("proficiency").unwrap_or(1),
    }).collect();

    Ok(Json(ApiResponse::success(skills, rid())))
}

async fn set_agent_skills(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(body): Json<SetAgentSkillsBody>,
) -> ConsoleResult<Json<ApiResponse<Vec<AgentSkillView>>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Delete existing skills for this agent
    rvoip_call_engine::database::sqlx::query(
        "DELETE FROM agent_skills WHERE agent_id = $1",
    )
    .bind(&agent_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete agent_skills failed: {e}")))?;

    // Insert new skills
    let mut result_skills = Vec::new();
    for entry in &body.skills {
        let proficiency = entry.proficiency.clamp(1, 5);
        rvoip_call_engine::database::sqlx::query(
            "INSERT INTO agent_skills (agent_id, skill_id, proficiency) VALUES ($1, $2, $3)",
        )
        .bind(&agent_id)
        .bind(&entry.skill_id)
        .bind(proficiency)
        .execute(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("insert agent_skill failed: {e}")))?;

        // Fetch skill name
        let name_row = rvoip_call_engine::database::sqlx::query(
            "SELECT name FROM skill_definitions WHERE id = $1",
        )
        .bind(&entry.skill_id)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("fetch skill name failed: {e}")))?;

        let skill_name: String = name_row
            .map(|r| r.try_get("name").unwrap_or_default())
            .unwrap_or_default();

        result_skills.push(AgentSkillView {
            skill_id: entry.skill_id.clone(),
            skill_name,
            proficiency,
        });
    }

    Ok(Json(ApiResponse::success(result_skills, rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_skills).post(create_skill))
        .route("/{id}", put(update_skill).delete(delete_skill))
        .route("/agents/{agent_id}", get(get_agent_skills).put(set_agent_skills))
}
