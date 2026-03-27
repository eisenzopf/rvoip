//! Agent management endpoints — list + CRUD.

use axum::{Router, routing::{get, post, put, delete}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::{AdminApi, SupervisorApi};
use rvoip_call_engine::agent::{Agent, AgentId, AgentStatus};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::AuthUser;
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct AgentView {
    pub id: String,
    pub sip_uri: String,
    pub contact_uri: String,
    pub display_name: String,
    pub status: String,
    pub skills: Vec<String>,
    pub current_calls: usize,
    pub max_calls: usize,
    pub performance_score: f64,
    pub department: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentsResponse {
    pub agents: Vec<AgentView>,
    pub total: usize,
    pub online: usize,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub id: Option<String>,
    pub display_name: String,
    pub extension: Option<String>,
    pub sip_uri: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default = "default_max_calls")]
    pub max_concurrent_calls: u32,
    pub department: Option<String>,
}

fn default_max_calls() -> u32 { 3 }

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub sip_uri: Option<String>,
    pub display_name: Option<String>,
    pub skills: Option<Vec<String>>,
    pub max_concurrent_calls: Option<u32>,
    pub status: Option<String>,
    pub department: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatusBody {
    pub status: String,
}

/// Generate the next sequential agent ID (AGT-001, AGT-002, ...).
async fn next_agent_id(state: &AppState) -> String {
    if let Some(db) = state.engine.database_manager() {
        let row = rvoip_call_engine::database::sqlx::query(
            "SELECT agent_id FROM agents ORDER BY agent_id DESC LIMIT 1"
        )
        .fetch_optional(db.pool())
        .await
        .ok()
        .flatten();

        if let Some(row) = row {
            let last_id: String = row.try_get("agent_id").unwrap_or_default();
            if let Some(num_str) = last_id.strip_prefix("AGT-") {
                if let Ok(num) = num_str.parse::<u32>() {
                    return format!("AGT-{:03}", num + 1);
                }
            }
        }
    }
    format!("AGT-{:03}", 1)
}

/// Find next available extension starting from 1001.
async fn next_extension(state: &AppState) -> String {
    if let Some(db) = state.engine.database_manager() {
        let rows = rvoip_call_engine::database::sqlx::query(
            "SELECT contact_uri FROM agents WHERE contact_uri IS NOT NULL ORDER BY agent_id DESC"
        )
        .fetch_all(db.pool())
        .await
        .unwrap_or_default();

        let mut max_ext: u32 = 1000;
        for row in &rows {
            let uri: String = row.try_get("contact_uri").unwrap_or_default();
            if let Some(ext_str) = uri.strip_prefix("sip:") {
                if let Some(ext) = ext_str.split('@').next() {
                    if let Ok(num) = ext.parse::<u32>() {
                        if num > max_ext {
                            max_ext = num;
                        }
                    }
                }
            }
        }
        format!("{}", max_ext + 1)
    } else {
        "1001".to_string()
    }
}

async fn list_agents(
    State(state): State<AppState>,
) -> ConsoleResult<Json<ApiResponse<AgentsResponse>>> {
    let supervisor = SupervisorApi::new(state.engine.clone());
    let agents = supervisor.list_agents().await;

    let agent_list: Vec<AgentView> = agents
        .iter()
        .map(|a| AgentView {
            id: a.agent_id.0.clone(),
            sip_uri: a.sip_uri.clone(),
            contact_uri: a.contact_uri.clone(),
            display_name: a.agent_id.0.clone(),
            status: format!("{:?}", a.status),
            skills: a.skills.clone(),
            current_calls: a.current_calls,
            max_calls: a.max_calls,
            performance_score: a.performance_score,
            department: None,
            extension: None,
        })
        .collect();

    let online = agent_list.iter().filter(|a| a.status != "Offline").count();
    let total = agent_list.len();

    Ok(Json(ApiResponse::success(
        AgentsResponse { agents: agent_list, total, online },
        uuid::Uuid::new_v4().to_string(),
    )))
}

async fn create_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<CreateAgentRequest>,
) -> ConsoleResult<Json<ApiResponse<AgentView>>> {
    if req.display_name.is_empty() {
        return Err(ConsoleError::BadRequest("display_name is required".into()));
    }

    let agent_id = match req.id {
        Some(ref id) if !id.is_empty() => id.clone(),
        _ => next_agent_id(&state).await,
    };

    let extension = match req.extension {
        Some(ref ext) if !ext.is_empty() => ext.clone(),
        _ => next_extension(&state).await,
    };

    let domain = &state.engine.config().general.domain;
    let sip_uri = match req.sip_uri {
        Some(ref uri) if !uri.is_empty() => uri.clone(),
        _ => format!("sip:{extension}@{domain}"),
    };

    let agent = Agent {
        id: agent_id.clone(),
        sip_uri: sip_uri.clone(),
        display_name: req.display_name.clone(),
        skills: req.skills.clone(),
        max_concurrent_calls: req.max_concurrent_calls,
        status: AgentStatus::Available,
        department: req.department.clone(),
        extension: Some(extension.clone()),
    };

    let admin = AdminApi::new(state.engine.clone());
    admin.add_agent(agent).await?;

    let view = AgentView {
        id: agent_id,
        sip_uri,
        contact_uri: String::new(),
        display_name: req.display_name,
        status: "Available".into(),
        skills: req.skills,
        current_calls: 0,
        max_calls: req.max_concurrent_calls as usize,
        performance_score: 1.0,
        department: req.department,
        extension: Some(extension),
    };

    Ok(Json(ApiResponse::success(view, uuid::Uuid::new_v4().to_string())))
}

async fn update_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let admin = AdminApi::new(state.engine.clone());

    // Get current agent data via list
    let agents = admin.list_agents().await?;
    let existing = agents.iter().find(|a| a.id == agent_id)
        .ok_or_else(|| ConsoleError::NotFound(format!("agent {agent_id}")))?;

    let status = match req.status.as_deref() {
        Some("Available") => AgentStatus::Available,
        Some("Busy") => AgentStatus::Busy(vec![]),
        Some("Offline") => AgentStatus::Offline,
        Some("PostCallWrapUp") => AgentStatus::PostCallWrapUp,
        _ => existing.status.clone(),
    };

    let updated = Agent {
        id: agent_id.clone(),
        sip_uri: req.sip_uri.unwrap_or_else(|| existing.sip_uri.clone()),
        display_name: req.display_name.unwrap_or_else(|| existing.display_name.clone()),
        skills: req.skills.clone().unwrap_or_else(|| existing.skills.clone()),
        max_concurrent_calls: req.max_concurrent_calls.unwrap_or(existing.max_concurrent_calls),
        status,
        department: req.department.or_else(|| existing.department.clone()),
        extension: req.extension.or_else(|| existing.extension.clone()),
    };

    admin.update_agent(updated).await?;

    Ok(Json(ApiResponse::success(
        format!("agent {agent_id} updated"),
        uuid::Uuid::new_v4().to_string(),
    )))
}

async fn update_agent_status(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(body): Json<UpdateStatusBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let admin = AdminApi::new(state.engine.clone());

    let agents = admin.list_agents().await?;
    let existing = agents.iter().find(|a| a.id == agent_id)
        .ok_or_else(|| ConsoleError::NotFound(format!("agent {agent_id}")))?;

    let status = match body.status.as_str() {
        "Available" => AgentStatus::Available,
        "Busy" => AgentStatus::Busy(vec![]),
        "Offline" => AgentStatus::Offline,
        "PostCallWrapUp" => AgentStatus::PostCallWrapUp,
        other => return Err(ConsoleError::BadRequest(format!("unknown status: {other}"))),
    };

    let updated = Agent {
        id: agent_id.clone(),
        sip_uri: existing.sip_uri.clone(),
        display_name: existing.display_name.clone(),
        skills: existing.skills.clone(),
        max_concurrent_calls: existing.max_concurrent_calls,
        status,
        department: existing.department.clone(),
        extension: existing.extension.clone(),
    };

    admin.update_agent(updated).await?;

    Ok(Json(ApiResponse::success(
        format!("agent {agent_id} status changed to {}", body.status),
        uuid::Uuid::new_v4().to_string(),
    )))
}

async fn delete_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(agent_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let admin = AdminApi::new(state.engine.clone());
    // remove_agent may fail if no active session exists, but DB cleanup still works
    let _ = admin.remove_agent(&AgentId(agent_id.clone())).await;

    // Ensure agent is marked offline in DB regardless
    if let Some(db) = state.engine.database_manager() {
        db.mark_agent_offline(&agent_id)
            .await
            .map_err(|e| ConsoleError::Internal(e.to_string()))?;
    }

    Ok(Json(ApiResponse::success(
        format!("agent {agent_id} removed"),
        uuid::Uuid::new_v4().to_string(),
    )))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_agents).post(create_agent))
        .route("/{agent_id}", put(update_agent).delete(delete_agent))
        .route("/{agent_id}/status", put(update_agent_status))
}
