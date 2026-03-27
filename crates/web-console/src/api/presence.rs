//! Presence management endpoints.

use axum::{Router, routing::{get, put}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_registrar_core::{PresenceStatus, BasicStatus};

use crate::auth::AuthUser;
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

fn rid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PresenceView {
    pub user_id: String,
    pub status: String,
    pub note: Option<String>,
    pub last_updated: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePresenceBody {
    pub status: String,
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn basic_status_str(s: &BasicStatus) -> &'static str {
    match s {
        BasicStatus::Open => "online",
        BasicStatus::Closed => "offline",
    }
}

fn parse_presence_status(s: &str) -> PresenceStatus {
    match s {
        "Available" => PresenceStatus::Available,
        "Busy" => PresenceStatus::Busy,
        "Away" => PresenceStatus::Away,
        "DoNotDisturb" => PresenceStatus::DoNotDisturb,
        "Offline" => PresenceStatus::Offline,
        _ => PresenceStatus::Available,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_presence(
    State(state): State<AppState>,
) -> ConsoleResult<Json<ApiResponse<Vec<PresenceView>>>> {
    let registrar = match &state.registrar {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let users = registrar.list_registered_users().await;
    let mut result = Vec::with_capacity(users.len());

    for user_id in &users {
        if let Ok(ps) = registrar.get_presence(user_id).await {
            let extended = ps
                .extended_status
                .as_ref()
                .map(|es| format!("{:?}", es))
                .unwrap_or_else(|| basic_status_str(&ps.basic_status).to_string());

            result.push(PresenceView {
                user_id: ps.user_id.clone(),
                status: extended,
                note: ps.note.clone(),
                last_updated: ps.last_updated.to_rfc3339(),
            });
        }
    }

    Ok(Json(ApiResponse::success(result, rid())))
}

async fn get_user_presence(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<PresenceView>>> {
    let registrar = state
        .registrar
        .as_ref()
        .ok_or_else(|| ConsoleError::Internal("registrar not available".into()))?;

    let ps = registrar
        .get_presence(&user_id)
        .await
        .map_err(|e| ConsoleError::NotFound(format!("presence for {user_id}: {e}")))?;

    let extended = ps
        .extended_status
        .as_ref()
        .map(|es| format!("{:?}", es))
        .unwrap_or_else(|| basic_status_str(&ps.basic_status).to_string());

    Ok(Json(ApiResponse::success(
        PresenceView {
            user_id: ps.user_id.clone(),
            status: extended,
            note: ps.note.clone(),
            last_updated: ps.last_updated.to_rfc3339(),
        },
        rid(),
    )))
}

async fn update_my_presence(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdatePresenceBody>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let registrar = state
        .registrar
        .as_ref()
        .ok_or_else(|| ConsoleError::Internal("registrar not available".into()))?;

    let status = parse_presence_status(&body.status);

    registrar
        .update_presence(&auth.user_id, status, body.note)
        .await
        .map_err(|e| ConsoleError::Internal(format!("failed to update presence: {e}")))?;

    Ok(Json(ApiResponse::success(
        "presence updated".to_string(),
        rid(),
    )))
}

async fn buddy_list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<Vec<PresenceView>>>> {
    let registrar = state
        .registrar
        .as_ref()
        .ok_or_else(|| ConsoleError::Internal("registrar not available".into()))?;

    let buddies = registrar
        .get_buddy_list(&auth.user_id)
        .await
        .map_err(|e| ConsoleError::Internal(format!("failed to get buddy list: {e}")))?;

    let views: Vec<PresenceView> = buddies
        .iter()
        .map(|b| PresenceView {
            user_id: b.user_id.clone(),
            status: format!("{:?}", b.status),
            note: b.note.clone(),
            last_updated: String::new(),
        })
        .collect();

    Ok(Json(ApiResponse::success(views, rid())))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_presence))
        .route("/me", put(update_my_presence))
        .route("/buddies", get(buddy_list))
        .route("/{user_id}", get(get_user_presence))
}
