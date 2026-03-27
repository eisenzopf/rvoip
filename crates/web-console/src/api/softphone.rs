//! Softphone REST endpoints for SIP registration.
//!
//! Provides register/unregister/status endpoints that the browser softphone
//! uses to interact with the rvoip registrar via REST (when WebSocket SIP
//! transport is not available).

use axum::{Router, routing::post, extract::State, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub extension: String,
    pub domain: String,
    pub user_agent: String,
}

#[derive(Debug, Deserialize)]
pub struct UnregisterRequest {
    pub extension: String,
    pub domain: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub registered: bool,
    pub uri: String,
    pub expires: u32,
}

#[derive(Debug, Deserialize)]
pub struct StatusRequest {
    pub extension: String,
    pub status: String,
}

fn rid() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn register(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<RegisterRequest>,
) -> ConsoleResult<Json<ApiResponse<RegisterResponse>>> {
    let registrar = state.registrar.as_ref()
        .ok_or_else(|| ConsoleError::Internal("registrar not configured".into()))?;

    let user_id = body.extension.clone();
    let uri = format!("sip:{}@{}", body.extension, body.domain);

    let contact = rvoip_registrar_core::ContactInfo {
        uri: uri.clone(),
        instance_id: format!("web-{}", auth.user_id),
        transport: rvoip_registrar_core::Transport::WS,
        user_agent: body.user_agent.clone(),
        expires: Utc::now() + chrono::Duration::seconds(3600),
        q_value: 1.0,
        received: None,
        path: vec![],
        methods: vec![
            "INVITE".into(),
            "ACK".into(),
            "BYE".into(),
            "CANCEL".into(),
            "OPTIONS".into(),
        ],
    };

    registrar.register_user(&user_id, contact, Some(3600)).await
        .map_err(|e| ConsoleError::Internal(format!("registration failed: {e}")))?;

    tracing::info!(extension = %body.extension, user = %auth.username, "softphone registered");

    Ok(Json(ApiResponse::success(
        RegisterResponse {
            registered: true,
            uri,
            expires: 3600,
        },
        rid(),
    )))
}

async fn unregister(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UnregisterRequest>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let registrar = state.registrar.as_ref()
        .ok_or_else(|| ConsoleError::Internal("registrar not configured".into()))?;

    registrar.unregister_user(&body.extension).await
        .map_err(|e| ConsoleError::Internal(format!("unregister failed: {e}")))?;

    tracing::info!(extension = %body.extension, user = %auth.username, "softphone unregistered");

    Ok(Json(ApiResponse::success("unregistered".into(), rid())))
}

async fn update_status(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(body): Json<StatusRequest>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let registrar = state.registrar.as_ref()
        .ok_or_else(|| ConsoleError::Internal("registrar not configured".into()))?;

    let status = match body.status.as_str() {
        "available" | "Available" => rvoip_registrar_core::PresenceStatus::Available,
        "busy" | "Busy" | "in-call" => rvoip_registrar_core::PresenceStatus::InCall,
        "away" | "Away" => rvoip_registrar_core::PresenceStatus::Away,
        _ => rvoip_registrar_core::PresenceStatus::Offline,
    };

    let _ = registrar.update_presence(&body.extension, status, None).await;

    Ok(Json(ApiResponse::success("status updated".into(), rid())))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/unregister", post(unregister))
        .route("/status", post(update_status))
}
