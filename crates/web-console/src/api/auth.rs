//! Authentication API endpoints.
//!
//! All routes are nested under `/api/v1/auth`.

use axum::{
    extract::State,
    routing::post,
    routing::get,
    routing::put,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::auth::AuthUser;
use crate::error::{ConsoleError, ConsoleResult};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: LoginUserInfo,
}

#[derive(Debug, Serialize)]
pub struct LoginUserInfo {
    pub id: String,
    pub username: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user_id: String,
    pub username: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the auth sub-router.
///
/// These routes are **public** (login, refresh) or protected (logout, me,
/// password change). The auth middleware in [`crate::auth`] skips
/// `/api/v1/auth/login` and `/api/v1/auth/refresh` automatically.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/refresh", post(refresh))
        .route("/me", get(me))
        .route("/me/password", put(change_password))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /auth/login`
async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> ConsoleResult<Json<LoginResponse>> {
    let auth_service = state
        .auth_service
        .as_ref()
        .ok_or_else(|| ConsoleError::Internal("authentication not configured".into()))?;

    let result = auth_service
        .authenticate_password(&body.username, &body.password)
        .await
        .map_err(|e| match e {
            users_core::Error::InvalidCredentials => {
                ConsoleError::Unauthorized("invalid username or password".into())
            }
            other => ConsoleError::Internal(other.to_string()),
        })?;

    info!(username = %body.username, "user logged in");

    Ok(Json(LoginResponse {
        access_token: result.access_token,
        refresh_token: result.refresh_token,
        expires_in: result.expires_in.as_secs(),
        user: LoginUserInfo {
            id: result.user.id,
            username: result.user.username,
            roles: result.user.roles,
        },
    }))
}

/// `POST /auth/logout`
///
/// Stateless logout — the client is expected to discard its tokens.
/// If a pool-backed revocation store is available the server also revokes
/// refresh tokens for the user.
async fn logout(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> ConsoleResult<Json<serde_json::Value>> {
    if let Some(auth_service) = &state.auth_service {
        // Best-effort revoke; ignore errors so the client always succeeds.
        let _ = auth_service.revoke_tokens(&auth_user.user_id).await;
    }

    info!(username = %auth_user.username, "user logged out");
    Ok(Json(serde_json::json!({ "message": "logged out" })))
}

/// `POST /auth/refresh`
async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> ConsoleResult<Json<RefreshResponse>> {
    let auth_service = state
        .auth_service
        .as_ref()
        .ok_or_else(|| ConsoleError::Internal("authentication not configured".into()))?;

    let pair = auth_service
        .refresh_token(&body.refresh_token)
        .await
        .map_err(|e| match e {
            users_core::Error::InvalidCredentials => {
                ConsoleError::Unauthorized("invalid or revoked refresh token".into())
            }
            other => ConsoleError::Internal(other.to_string()),
        })?;

    Ok(Json(RefreshResponse {
        access_token: pair.access_token,
        refresh_token: pair.refresh_token,
        expires_in: pair.expires_in.as_secs(),
    }))
}

/// `GET /auth/me`
async fn me(auth_user: AuthUser) -> ConsoleResult<Json<MeResponse>> {
    Ok(Json(MeResponse {
        user_id: auth_user.user_id,
        username: auth_user.username,
        roles: auth_user.roles,
    }))
}

/// `PUT /auth/me/password`
async fn change_password(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<ChangePasswordRequest>,
) -> ConsoleResult<Json<serde_json::Value>> {
    let auth_service = state
        .auth_service
        .as_ref()
        .ok_or_else(|| ConsoleError::Internal("authentication not configured".into()))?;

    auth_service
        .change_password(&auth_user.user_id, &body.current_password, &body.new_password)
        .await
        .map_err(|e| match e {
            users_core::Error::InvalidCredentials => {
                ConsoleError::Unauthorized("current password is incorrect".into())
            }
            users_core::Error::InvalidPassword(msg) => {
                ConsoleError::BadRequest(format!("password policy violation: {}", msg))
            }
            other => ConsoleError::Internal(other.to_string()),
        })?;

    info!(username = %auth_user.username, "password changed");
    Ok(Json(serde_json::json!({ "message": "password changed" })))
}
