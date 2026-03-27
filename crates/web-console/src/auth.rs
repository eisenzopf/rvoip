//! JWT authentication middleware and RBAC helpers.
//!
//! Provides an Axum middleware layer that validates `Authorization: Bearer <token>`
//! headers using `jsonwebtoken` and injects an [`AuthUser`] into request extensions.
//! Also provides role-based access control via [`require_role`].

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::ConsoleError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Role constants
// ---------------------------------------------------------------------------

/// Unrestricted system-wide access.
pub const ROLE_SUPER_ADMIN: &str = "super_admin";
/// Administrative access to the web console.
pub const ROLE_ADMIN: &str = "admin";
/// Call-center supervisor role.
pub const ROLE_SUPERVISOR: &str = "supervisor";
/// Call-center agent role.
pub const ROLE_AGENT: &str = "agent";

// ---------------------------------------------------------------------------
// AuthUser — extracted from validated JWT
// ---------------------------------------------------------------------------

/// Authenticated user identity injected into request extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    /// Unique user ID (UUID).
    pub user_id: String,
    /// Human-readable login name.
    pub username: String,
    /// Roles assigned to the user.
    pub roles: Vec<String>,
}

impl AuthUser {
    /// Return `true` if the user holds at least one of the given roles.
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        self.roles.iter().any(|r| roles.contains(&r.as_str()))
    }
}

// ---------------------------------------------------------------------------
// RBAC helper
// ---------------------------------------------------------------------------

/// Check that `user` holds at least one of `roles`, returning
/// [`ConsoleError::Unauthorized`] otherwise.
pub fn require_role(user: &AuthUser, roles: &[&str]) -> Result<(), ConsoleError> {
    if user.has_any_role(roles) {
        Ok(())
    } else {
        Err(ConsoleError::Unauthorized(format!(
            "user '{}' lacks required role (need one of: {})",
            user.username,
            roles.join(", "),
        )))
    }
}

// ---------------------------------------------------------------------------
// Paths that skip authentication
// ---------------------------------------------------------------------------

/// Returns `true` for paths that must be accessible without a valid JWT.
fn is_public_path(path: &str) -> bool {
    // Auth endpoints are public (login, refresh).
    if path.starts_with("/api/v1/auth/login") || path.starts_with("/api/v1/auth/refresh") {
        return true;
    }
    // Health-check is always public.
    if path.starts_with("/api/v1/system/health") {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Axum middleware function
// ---------------------------------------------------------------------------

/// Axum middleware that validates JWT tokens on `/api/v1/` routes.
///
/// When `AppState.decoding_key` is `None` authentication is **skipped**
/// (backward-compatible mode for deployments without users-core).
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();

    // Skip non-API or explicitly public paths.
    if !path.starts_with("/api/v1/") || is_public_path(&path) {
        return next.run(req).await;
    }

    // If no decoding key is configured, auth is disabled — pass through.
    let (decoding_key, jwt_config) = match (&state.decoding_key, &state.jwt_config) {
        (Some(dk), Some(cfg)) => (dk.clone(), cfg),
        _ => return next.run(req).await,
    };

    // Extract Bearer token from Authorization header.
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            return ConsoleError::Unauthorized("missing or malformed Authorization header".into())
                .into_response();
        }
    };

    // Build validation parameters.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[&jwt_config.issuer]);
    validation.set_audience(&jwt_config.audience);
    validation.validate_exp = true;

    // Decode and validate.
    let token_data =
        match decode::<users_core::UserClaims>(token, &decoding_key, &validation) {
            Ok(td) => td,
            Err(e) => {
                warn!(error = %e, "JWT validation failed");
                return ConsoleError::Unauthorized("invalid or expired token".into())
                    .into_response();
            }
        };

    let claims = token_data.claims;

    let auth_user = AuthUser {
        user_id: claims.sub,
        username: claims.username,
        roles: claims.roles,
    };

    // Insert into request extensions so handlers can extract it.
    req.extensions_mut().insert(auth_user);

    next.run(req).await
}

// ---------------------------------------------------------------------------
// Extractor — pull AuthUser from extensions (set by middleware)
// ---------------------------------------------------------------------------

impl<S> axum::extract::FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = ConsoleError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or_else(|| ConsoleError::Unauthorized("not authenticated".into()))
    }
}
