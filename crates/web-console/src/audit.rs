//! Audit log middleware — records POST/PUT/DELETE requests to PostgreSQL.
//!
//! Runs AFTER the handler so we can capture the response status code.
//! Inserts are non-blocking (spawned as background tasks).

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::auth::AuthUser;
use crate::server::AppState;

/// Ensure the `audit_log` table exists.
pub async fn init_audit_table(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS audit_log (\
            id BIGSERIAL PRIMARY KEY, \
            user_id TEXT, \
            username TEXT, \
            action TEXT NOT NULL, \
            resource_type TEXT, \
            resource_id TEXT, \
            status_code INTEGER, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create audit_log table: {e}"))?;

    Ok(())
}

/// Axum middleware that logs write operations to the `audit_log` table.
///
/// Only POST, PUT, and DELETE requests are logged. GET and OPTIONS are
/// silently passed through. The database insert is spawned as a background
/// task so it never blocks the HTTP response.
pub async fn audit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    // Extract auth user before passing request to next handler
    let auth_user = request.extensions().get::<AuthUser>().cloned();

    let response = next.run(request).await;

    // Only audit write operations
    if method == axum::http::Method::GET || method == axum::http::Method::OPTIONS {
        return response;
    }

    let status_code = response.status().as_u16() as i32;

    if let (Some(user), Some(db)) = (auth_user, state.engine.database_manager()) {
        let pool = db.pool().clone();
        let resource_type = extract_resource_type(&path);
        let resource_id = extract_resource_id(&path);
        let action = format!("{} {}", method, path);

        tokio::spawn(async move {
            if let Err(e) = rvoip_call_engine::database::sqlx::query(
                "INSERT INTO audit_log (user_id, username, action, resource_type, resource_id, status_code, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, NOW())",
            )
            .bind(&user.user_id)
            .bind(&user.username)
            .bind(&action)
            .bind(resource_type.as_deref())
            .bind(resource_id.as_deref())
            .bind(status_code)
            .execute(&pool)
            .await
            {
                tracing::warn!("Failed to write audit log: {}", e);
            }
        });
    }

    response
}

/// Extract the resource type from a URL path like `/api/v1/agents/123`.
/// Returns `Some("agents")` for the above example.
fn extract_resource_type(path: &str) -> Option<String> {
    // Path format: /api/v1/<resource_type>/...
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // segments: ["api", "v1", "agents", "123", ...]
    segments.get(2).map(|s| s.to_string())
}

/// Extract the resource ID from a URL path like `/api/v1/agents/123`.
/// Returns `Some("123")` for the above example.
fn extract_resource_id(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // segments: ["api", "v1", "agents", "123", ...]
    segments.get(3).map(|s| s.to_string())
}
