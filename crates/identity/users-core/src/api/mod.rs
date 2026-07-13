//! REST API for users-core

pub mod rate_limit;
pub mod security_headers;

use self::rate_limit::{EnhancedRateLimiter, RateLimitConfig};
use crate::{
    api_keys::CreateApiKeyRequest, AuthenticationService, CreateUserRequest, Error as UsersError,
    UpdateUserRequest, UserFilter,
};
use axum::{
    extract::{FromRef, Json, Path, Query, State},
    http::{header, StatusCode},
    middleware::{self},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tower_http::cors::CorsLayer;

// API State
#[derive(Clone)]
pub struct ApiState {
    pub auth_service: Arc<AuthenticationService>,
    pub rate_limiter: EnhancedRateLimiter,
    pub metrics: Arc<Mutex<Metrics>>,
}

// Metrics tracking
#[derive(Debug)]
pub struct Metrics {
    pub total_users: usize,
    pub active_users: usize,
    pub total_api_keys: usize,
    pub authentication_attempts: usize,
    pub authentication_successes: usize,
    pub tokens_issued: usize,
    pub api_requests: usize,
    pub start_time: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            total_users: 0,
            active_users: 0,
            total_api_keys: 0,
            authentication_attempts: 0,
            authentication_successes: 0,
            tokens_issued: 0,
            api_requests: 0,
            start_time: Instant::now(),
        }
    }
}

// Request/Response types
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("username_present", &!self.username.is_empty())
            .field("username_len", &self.username.len())
            .field("password_present", &!self.password.is_empty())
            .field("password_len", &self.password.len())
            .finish()
    }
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

impl std::fmt::Debug for LoginResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginResponse")
            .field("access_token_present", &!self.access_token.is_empty())
            .field("access_token_len", &self.access_token.len())
            .field("refresh_token_present", &!self.refresh_token.is_empty())
            .field("refresh_token_len", &self.refresh_token.len())
            .field("token_type_len", &self.token_type.len())
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

impl std::fmt::Debug for RefreshRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefreshRequest")
            .field("refresh_token_present", &!self.refresh_token.is_empty())
            .field("refresh_token_len", &self.refresh_token.len())
            .finish()
    }
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

impl std::fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChangePasswordRequest")
            .field("old_password_present", &!self.old_password.is_empty())
            .field("old_password_len", &self.old_password.len())
            .field("new_password_present", &!self.new_password.is_empty())
            .field("new_password_len", &self.new_password.len())
            .finish()
    }
}

#[derive(Deserialize)]
pub struct UpdateRolesRequest {
    pub roles: Vec<String>,
}

impl std::fmt::Debug for UpdateRolesRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateRolesRequest")
            .field("role_count", &self.roles.len())
            .finish()
    }
}

#[derive(Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Vec<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for UserResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserResponse")
            .field("id_present", &!self.id.is_empty())
            .field("username_present", &!self.username.is_empty())
            .field("email_present", &self.email.is_some())
            .field("display_name_present", &self.display_name.is_some())
            .field("role_count", &self.roles.len())
            .field("active", &self.active)
            .field("last_login_present", &self.last_login.is_some())
            .finish()
    }
}

#[derive(Serialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub active: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for ApiKeyResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiKeyResponse")
            .field("id_present", &!self.id.is_empty())
            .field("name_len", &self.name.len())
            .field("permission_count", &self.permissions.len())
            .field("active", &self.active)
            .field("expires_at_present", &self.expires_at.is_some())
            .field("last_used_present", &self.last_used.is_some())
            .finish()
    }
}

#[derive(Serialize)]
pub struct CreateApiKeyResponse {
    pub key: String,
    pub key_info: ApiKeyResponse,
}

impl std::fmt::Debug for CreateApiKeyResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateApiKeyResponse")
            .field("key_present", &!self.key.is_empty())
            .field("key_len", &self.key.len())
            .field("key_info_present", &true)
            .finish()
    }
}

// Error response
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

impl std::fmt::Debug for ErrorResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ErrorResponse")
            .field("error", &self.error)
            .finish()
    }
}

#[derive(Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl std::fmt::Debug for ErrorDetail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ErrorDetail")
            .field("code_len", &self.code.len())
            .field("message_len", &self.message.len())
            .field("details_present", &self.details.is_some())
            .finish()
    }
}

// JWT validation for protected routes
#[derive(Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub username: String,
    pub roles: Vec<String>,
    pub permissions: Vec<String>, // For API key auth
    pub auth_type: AuthType,
}

#[derive(Clone)]
struct AccessTokenSession {
    token_id: String,
    expires_at: DateTime<Utc>,
}

struct RequestAuthentication {
    context: AuthContext,
    access_token: Option<AccessTokenSession>,
}

struct LogoutAuthentication(RequestAuthentication);

#[derive(Debug, Clone, PartialEq)]
pub enum AuthType {
    Jwt,
    ApiKey,
}

impl AuthContext {
    /// Check if the user has a specific permission (for API key auth)
    pub fn has_permission(&self, permission: &str) -> bool {
        if self.auth_type == AuthType::Jwt {
            // JWT tokens have full permissions
            true
        } else {
            // API keys need explicit permissions
            self.permissions.contains(&permission.to_string())
                || self.permissions.contains(&"*".to_string()) // Wildcard permission
        }
    }

    /// Check whether this credential has administrative authority.
    ///
    /// API keys are attenuated credentials: an admin owner's role is
    /// necessary but not sufficient; the key must also carry `admin` or `*`.
    pub fn is_admin(&self) -> bool {
        let owner_is_admin = self.roles.iter().any(|role| role == "admin");
        owner_is_admin
            && (self.auth_type == AuthType::Jwt
                || self
                    .permissions
                    .iter()
                    .any(|permission| permission == "admin" || permission == "*"))
    }

    fn require_permission(&self, permission: &str) -> Result<(), AppError> {
        self.has_permission(permission)
            .then_some(())
            .ok_or(AppError::Forbidden)
    }
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthContext")
            .field("user_present", &!self.user_id.is_empty())
            .field("username_present", &!self.username.is_empty())
            .field("role_count", &self.roles.len())
            .field("permission_count", &self.permissions.len())
            .field("auth_type", &self.auth_type)
            .finish()
    }
}

/// Create the REST API router
pub fn create_router(auth_service: Arc<AuthenticationService>) -> Router {
    let state = ApiState {
        auth_service,
        rate_limiter: EnhancedRateLimiter::new(RateLimitConfig::default()),
        metrics: Arc::new(Mutex::new(Metrics {
            start_time: Instant::now(),
            ..Default::default()
        })),
    };

    create_router_with_state(state)
}

/// Create the REST API router with a custom ApiState (useful for testing)
pub fn create_router_with_state(state: ApiState) -> Router {
    Router::new()
        // Authentication endpoints
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/refresh", post(refresh))
        .route("/auth/jwks.json", get(jwks))
        // User management endpoints (protected)
        .route("/users", post(create_user))
        .route("/users", get(list_users))
        .route("/users/:id", get(get_user))
        .route("/users/:id", put(update_user))
        .route("/users/:id", delete(delete_user))
        .route("/users/:id/password", post(change_password))
        .route("/users/:id/roles", post(update_roles))
        // API key management (protected)
        .route("/users/:id/api-keys", post(create_api_key))
        .route("/users/:id/api-keys", get(list_api_keys))
        .route("/api-keys/:id", delete(revoke_api_key))
        // Health and metrics
        .route("/health", get(health_check))
        .route("/metrics", get(metrics))
        // Apply middleware (order matters - security headers should be outermost)
        .layer(middleware::from_fn(
            security_headers::security_headers_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::rate_limit_middleware,
        ))
        .with_state(state)
}

/// TLS configuration
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub enabled: bool,
}

/// Create and start the API server with optional TLS
pub async fn create_server_with_tls(
    app: Router,
    addr: SocketAddr,
    tls_config: Option<TlsConfig>,
) -> anyhow::Result<()> {
    match tls_config {
        Some(tls) if tls.enabled => {
            // Use HTTPS with axum-server
            use axum_server::tls_rustls::RustlsConfig;

            let config = RustlsConfig::from_pem_file(&tls.cert_path, &tls.key_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to load TLS config: {}", e))?;

            tracing::info!("🔒 Starting HTTPS server on https://{}", addr);
            tracing::info!("   Certificate: {}", tls.cert_path.display());
            tracing::info!("   Private key: {}", tls.key_path.display());

            axum_server::bind_rustls(addr, config)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;
        }
        _ => {
            // Fallback to HTTP with big warning
            tracing::warn!("⚠️  WARNING: Starting server without TLS encryption!");
            tracing::warn!("⚠️  This is INSECURE and should NOT be used in production!");
            tracing::warn!("⚠️  All traffic including passwords will be sent in PLAIN TEXT!");
            tracing::warn!("");
            tracing::warn!("To enable HTTPS, provide TLS configuration with:");
            tracing::warn!("  - Certificate file (cert_path)");
            tracing::warn!("  - Private key file (key_path)");
            tracing::warn!("  - Set enabled = true");

            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!("Starting HTTP server on http://{}", addr);

            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;
        }
    }
    Ok(())
}

// Authentication handlers

#[axum::debug_handler]
async fn login(
    State(state): State<ApiState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // Track authentication attempt
    {
        let mut metrics = state.metrics.lock().unwrap();
        metrics.authentication_attempts += 1;
    }

    // Check if account is locked due to failed attempts
    let auth_result = state
        .auth_service
        .authenticate_password(&req.username, &req.password)
        .await;

    // Handle rate limiting for login attempts
    use self::rate_limit::{handle_login_rate_limit, RateLimitError};
    let rate_limit_result = handle_login_rate_limit(
        &state.rate_limiter,
        &req.username,
        auth_result.as_ref().map(|_| ()).map_err(|_| ()),
    )
    .await;

    // Check rate limit first
    if let Err(e) = rate_limit_result {
        return match e {
            RateLimitError::AccountLocked(duration) => {
                Err(AppError::AccountLocked(duration.as_secs()))
            }
            _ => Err(AppError::InvalidCredentials),
        };
    }

    // Now handle the authentication result
    let auth_result = auth_result?;

    // Track successful authentication
    {
        let mut metrics = state.metrics.lock().unwrap();
        metrics.authentication_successes += 1;
        metrics.tokens_issued += 2; // Access + refresh token
    }

    Ok(Json(LoginResponse {
        access_token: auth_result.access_token,
        refresh_token: auth_result.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: auth_result.expires_in.as_secs(),
    }))
}

async fn logout(
    State(state): State<ApiState>,
    LogoutAuthentication(auth): LogoutAuthentication,
) -> Result<StatusCode, AppError> {
    state
        .auth_service
        .revoke_tokens(&auth.context.user_id)
        .await?;
    if let Some(access_token) = auth.access_token {
        state
            .auth_service
            .revoke_access_token_jti(
                &access_token.token_id,
                Some(&auth.context.user_id),
                access_token.expires_at,
            )
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh(
    State(state): State<ApiState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let token_pair = state.auth_service.refresh_token(&req.refresh_token).await?;

    Ok(Json(LoginResponse {
        access_token: token_pair.access_token,
        refresh_token: token_pair.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: token_pair.expires_in.as_secs(),
    }))
}

async fn jwks(State(state): State<ApiState>) -> Result<Json<serde_json::Value>, AppError> {
    let jwk = state.auth_service.jwt_issuer().public_key_jwk()?;
    Ok(Json(serde_json::json!({
        "keys": [jwk]
    })))
}

// User management handlers

async fn create_user(
    State(state): State<ApiState>,
    auth: AuthContext,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), AppError> {
    require_admin_permission(&auth, "admin")?;
    let user = state.auth_service.create_user(req).await?;

    Ok((
        StatusCode::CREATED,
        Json(UserResponse {
            id: user.id,
            username: user.username,
            email: user.email,
            display_name: user.display_name,
            roles: user.roles,
            active: user.active,
            created_at: user.created_at,
            updated_at: user.updated_at,
            last_login: user.last_login,
        }),
    ))
}

async fn list_users(
    State(state): State<ApiState>,
    auth: AuthContext,
    Query(filter): Query<UserFilter>,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    require_admin_permission(&auth, "read")?;
    let users = state.auth_service.user_store().list_users(filter).await?;

    let responses: Vec<UserResponse> = users
        .into_iter()
        .map(|u| UserResponse {
            id: u.id,
            username: u.username,
            email: u.email,
            display_name: u.display_name,
            roles: u.roles,
            active: u.active,
            created_at: u.created_at,
            updated_at: u.updated_at,
            last_login: u.last_login,
        })
        .collect();

    Ok(Json(responses))
}

async fn get_user(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    auth.require_permission("read")?;
    // Users can get their own info, admins can get anyone's
    if auth.user_id != id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }

    let user = state
        .auth_service
        .user_store()
        .get_user(&id)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
        roles: user.roles,
        active: user.active,
        created_at: user.created_at,
        updated_at: user.updated_at,
        last_login: user.last_login,
    }))
}

async fn update_user(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    authorize_user_update(&auth, &id, &req)?;

    let user = state
        .auth_service
        .user_store()
        .update_user(&id, req)
        .await?;

    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
        roles: user.roles,
        active: user.active,
        created_at: user.created_at,
        updated_at: user.updated_at,
        last_login: user.last_login,
    }))
}

fn authorize_user_update(
    auth: &AuthContext,
    user_id: &str,
    request: &UpdateUserRequest,
) -> Result<(), AppError> {
    auth.require_permission("write")?;
    if auth.user_id != user_id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    if !auth.is_admin() && (request.roles.is_some() || request.active.is_some()) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

fn require_admin_permission(auth: &AuthContext, permission: &str) -> Result<(), AppError> {
    if !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    auth.require_permission(permission)
}

async fn delete_user(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    require_admin_permission(&auth, "delete")?;
    state.auth_service.user_store().delete_user(&id).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn change_password(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    auth.require_permission("write")?;
    // Users can only change their own password
    if auth.user_id != id {
        return Err(AppError::Forbidden);
    }

    state
        .auth_service
        .change_password(&id, &req.old_password, &req.new_password)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn update_roles(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateRolesRequest>,
) -> Result<StatusCode, AppError> {
    require_admin_permission(&auth, "admin")?;

    // Update the user's roles
    let update_req = UpdateUserRequest {
        email: None,
        display_name: None,
        roles: Some(req.roles),
        active: None,
    };

    state
        .auth_service
        .user_store()
        .update_user(&id, update_req)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// API key handlers

async fn create_api_key(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(user_id): Path<String>,
    Json(mut req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), AppError> {
    authorize_api_key_creation(&auth, &user_id, &mut req)?;

    let (key_info, raw_key) = state
        .auth_service
        .api_key_store()
        .create_api_key(req)
        .await?;

    // Track API key creation
    {
        let mut metrics = state.metrics.lock().unwrap();
        metrics.total_api_keys += 1;
    }

    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            key: raw_key,
            key_info: ApiKeyResponse {
                id: key_info.id,
                name: key_info.name,
                permissions: key_info.permissions,
                active: key_info.active,
                expires_at: key_info.expires_at,
                created_at: key_info.created_at,
                last_used: key_info.last_used,
            },
        }),
    ))
}

fn authorize_api_key_creation(
    auth: &AuthContext,
    path_user_id: &str,
    request: &mut CreateApiKeyRequest,
) -> Result<(), AppError> {
    auth.require_permission("write")?;
    if auth.user_id != path_user_id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    let requests_privileged_grant = request
        .permissions
        .iter()
        .any(|permission| permission == "*" || permission == "admin");
    let has_non_key_admin_authority = auth.auth_type == AuthType::Jwt && auth.is_admin();
    if requests_privileged_grant && !has_non_key_admin_authority {
        return Err(AppError::Forbidden);
    }
    if auth.auth_type == AuthType::ApiKey
        && request
            .permissions
            .iter()
            .any(|permission| !auth.has_permission(permission))
    {
        return Err(AppError::Forbidden);
    }
    // The authorized path identity is authoritative. Never let a body field
    // retarget key ownership after the access check above.
    request.user_id = path_user_id.to_owned();
    Ok(())
}

async fn list_api_keys(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<ApiKeyResponse>>, AppError> {
    auth.require_permission("read")?;
    // Users can list their own API keys, admins can list anyone's
    if auth.user_id != user_id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }

    let keys = state
        .auth_service
        .api_key_store()
        .list_api_keys(&user_id)
        .await?;

    let responses: Vec<ApiKeyResponse> = keys
        .into_iter()
        .map(|k| ApiKeyResponse {
            id: k.id,
            name: k.name,
            permissions: k.permissions,
            active: k.active,
            expires_at: k.expires_at,
            created_at: k.created_at,
            last_used: k.last_used,
        })
        .collect();

    Ok(Json(responses))
}

async fn revoke_api_key(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    auth.require_permission("delete")?;
    // Get the API key to check ownership
    let keys = state
        .auth_service
        .api_key_store()
        .list_api_keys(&auth.user_id)
        .await?;

    // Check if user owns this key or is admin
    let owns_key = keys.iter().any(|k| k.id == id);
    if !owns_key && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }

    state
        .auth_service
        .api_key_store()
        .revoke_api_key(&id)
        .await?;

    // Track API key revocation
    {
        let mut metrics = state.metrics.lock().unwrap();
        if metrics.total_api_keys > 0 {
            metrics.total_api_keys -= 1;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

// Health and metrics

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "users-core",
        "timestamp": Utc::now(),
    }))
}

async fn metrics(State(state): State<ApiState>) -> Result<Json<serde_json::Value>, AppError> {
    // Get metrics from the state and clone the values we need
    let (auth_attempts, auth_successes, tokens_issued, api_requests, total_api_keys, start_time) = {
        let metrics = state.metrics.lock().unwrap();
        (
            metrics.authentication_attempts,
            metrics.authentication_successes,
            metrics.tokens_issued,
            metrics.api_requests,
            metrics.total_api_keys,
            metrics.start_time,
        )
    };

    let uptime = start_time.elapsed().as_secs();

    // Query database for user and API key counts
    let user_count = match state
        .auth_service
        .user_store()
        .list_users(UserFilter::default())
        .await
    {
        Ok(users) => users.len(),
        Err(_) => 0,
    };

    let active_user_count = match state
        .auth_service
        .user_store()
        .list_users(UserFilter {
            active: Some(true),
            ..Default::default()
        })
        .await
    {
        Ok(users) => users.len(),
        Err(_) => 0,
    };

    // Calculate success rate
    let success_rate = if auth_attempts > 0 {
        (auth_successes as f64 / auth_attempts as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(serde_json::json!({
        "users": {
            "total": user_count,
            "active": active_user_count,
        },
        "api_keys": {
            "total": total_api_keys,
            "active": total_api_keys, // API metrics count creations/revocations; list APIs expose suspension state.
        },
        "authentication": {
            "attempts": auth_attempts,
            "successes": auth_successes,
            "success_rate": success_rate,
            "tokens_issued": tokens_issued,
        },
        "api_requests": api_requests,
        "uptime_seconds": uptime,
    })))
}

// Error handling

pub enum AppError {
    Internal(anyhow::Error),
    InvalidCredentials,
    NotFound,
    Forbidden,
    BadRequest(String),
    AccountLocked(u64), // seconds until unlock
}

impl std::fmt::Debug for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let class = match self {
            Self::Internal(_) => "internal",
            Self::InvalidCredentials => "invalid-credentials",
            Self::NotFound => "not-found",
            Self::Forbidden => "forbidden",
            Self::BadRequest(_) => "bad-request",
            Self::AccountLocked(_) => "account-locked",
        };
        formatter
            .debug_struct("AppError")
            .field("class", &class)
            .finish()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message, retry_after) = match self {
            AppError::Internal(e) => {
                let _ = e;
                tracing::error!(error_class = "internal", "users API request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", "An internal error occurred".to_string(), None)
            },
            AppError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "Invalid username or password".to_string(), None)
            },
            AppError::NotFound => {
                (StatusCode::NOT_FOUND, "NOT_FOUND", "Resource not found".to_string(), None)
            },
            AppError::Forbidden => {
                (StatusCode::FORBIDDEN, "FORBIDDEN", "Access denied".to_string(), None)
            },
            AppError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg, None)
            },
            AppError::AccountLocked(seconds) => {
                (StatusCode::TOO_MANY_REQUESTS, "ACCOUNT_LOCKED",
                 format!("Account temporarily locked due to too many failed login attempts. Try again in {} seconds.", seconds),
                 Some(seconds))
            },
        };

        let body = Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message,
                details: None,
            },
        });

        let mut response = (status, body).into_response();

        // Add Retry-After header if applicable
        if let Some(seconds) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, seconds.to_string().parse().unwrap());
        }

        response
    }
}

impl From<UsersError> for AppError {
    fn from(err: UsersError) -> Self {
        match err {
            UsersError::InvalidCredentials => AppError::InvalidCredentials,
            UsersError::UserNotFound(_) => AppError::NotFound,
            UsersError::UserAlreadyExists(_) => {
                AppError::BadRequest("User already exists".to_string())
            }
            UsersError::InvalidPassword(msg) => AppError::BadRequest(msg),
            _ => AppError::Internal(err.into()),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

// Authentication middleware

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for AuthContext
where
    S: Send + Sync,
    ApiState: axum::extract::FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(extract_request_authentication(parts, state).await?.context)
    }
}

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for LogoutAuthentication
where
    S: Send + Sync,
    ApiState: axum::extract::FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        extract_request_authentication(parts, state).await.map(Self)
    }
}

async fn extract_request_authentication<S>(
    parts: &mut axum::http::request::Parts,
    state: &S,
) -> Result<RequestAuthentication, AppError>
where
    S: Send + Sync,
    ApiState: axum::extract::FromRef<S>,
{
    // Check for Bearer token first
    if let Some(auth_header) = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            let api_state = ApiState::from_ref(state);

            let claims = api_state
                .auth_service
                .jwt_issuer()
                .validate_access_token(token)
                .map_err(|_| AppError::Forbidden)?;
            if api_state
                .auth_service
                .is_access_token_revoked(&claims.jti)
                .await
                .map_err(|_| AppError::Forbidden)?
            {
                return Err(AppError::Forbidden);
            }
            let user = api_state
                .auth_service
                .user_store()
                .get_user(&claims.sub)
                .await
                .map_err(|_| AppError::Forbidden)?
                .filter(|user| user.active)
                .ok_or(AppError::Forbidden)?;
            let expires_at =
                DateTime::<Utc>::from_timestamp(claims.exp as i64, 0).ok_or(AppError::Forbidden)?;

            return Ok(RequestAuthentication {
                context: AuthContext {
                    user_id: user.id,
                    username: user.username,
                    roles: user.roles,
                    permissions: vec![], // User JWTs carry full user authority.
                    auth_type: AuthType::Jwt,
                },
                access_token: Some(AccessTokenSession {
                    token_id: claims.jti,
                    expires_at,
                }),
            });
        }
    }

    // Check for API key
    if let Some(api_key) = parts
        .headers
        .get("X-API-Key")
        .and_then(|value| value.to_str().ok())
    {
        let api_state = ApiState::from_ref(state);

        let api_key_info = api_state
            .auth_service
            .api_key_store()
            .validate_api_key(api_key)
            .await
            .map_err(|_| AppError::Forbidden)?
            .ok_or(AppError::Forbidden)?;

        // Get the user to construct AuthContext
        let user = api_state
            .auth_service
            .user_store()
            .get_user(&api_key_info.user_id)
            .await
            .map_err(|_| AppError::Forbidden)?
            .filter(|user| user.active)
            .ok_or(AppError::Forbidden)?;

        return Ok(RequestAuthentication {
            context: AuthContext {
                user_id: user.id,
                username: user.username,
                roles: user.roles,
                permissions: api_key_info.permissions,
                auth_type: AuthType::ApiKey,
            },
            access_token: None,
        });
    }

    // No valid authentication found
    Err(AppError::Forbidden)
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    fn auth(auth_type: AuthType, roles: &[&str], permissions: &[&str]) -> AuthContext {
        AuthContext {
            user_id: "user-a".into(),
            username: "alice".into(),
            roles: roles.iter().map(|value| (*value).to_owned()).collect(),
            permissions: permissions
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            auth_type,
        }
    }

    fn update() -> UpdateUserRequest {
        UpdateUserRequest {
            email: Some("alice@example.test".into()),
            display_name: None,
            roles: None,
            active: None,
        }
    }

    fn api_key_request(owner: &str) -> CreateApiKeyRequest {
        CreateApiKeyRequest {
            user_id: owner.into(),
            name: "integration".into(),
            permissions: vec!["read".into()],
            expires_at: None,
        }
    }

    #[test]
    fn self_service_cannot_change_roles_or_active_state() {
        let context = auth(AuthType::Jwt, &["user"], &[]);

        let mut roles = update();
        roles.roles = Some(vec!["admin".into()]);
        assert!(matches!(
            authorize_user_update(&context, "user-a", &roles),
            Err(AppError::Forbidden)
        ));

        let mut active = update();
        active.active = Some(false);
        assert!(matches!(
            authorize_user_update(&context, "user-a", &active),
            Err(AppError::Forbidden)
        ));
        assert!(authorize_user_update(&context, "user-a", &update()).is_ok());
    }

    #[test]
    fn api_key_does_not_inherit_admin_owners_cross_user_authority() {
        let write_only_admin_key = auth(AuthType::ApiKey, &["admin"], &["write"]);
        assert!(matches!(
            authorize_user_update(&write_only_admin_key, "user-b", &update()),
            Err(AppError::Forbidden)
        ));

        let mut privileged_update = update();
        privileged_update.roles = Some(vec!["admin".into()]);
        assert!(matches!(
            authorize_user_update(&write_only_admin_key, "user-a", &privileged_update),
            Err(AppError::Forbidden)
        ));
        privileged_update.roles = None;
        privileged_update.active = Some(false);
        assert!(matches!(
            authorize_user_update(&write_only_admin_key, "user-a", &privileged_update),
            Err(AppError::Forbidden)
        ));
    }

    #[test]
    fn explicitly_admin_scoped_api_key_can_use_admin_owner_authority() {
        let admin_key = auth(AuthType::ApiKey, &["admin"], &["write", "admin"]);
        assert!(authorize_user_update(&admin_key, "user-b", &update()).is_ok());

        let mut privileged_update = update();
        privileged_update.roles = Some(vec!["admin".into()]);
        assert!(authorize_user_update(&admin_key, "user-b", &privileged_update).is_ok());

        let wildcard_key = auth(AuthType::ApiKey, &["admin"], &["*"]);
        privileged_update.roles = None;
        privileged_update.active = Some(false);
        assert!(authorize_user_update(&wildcard_key, "user-b", &privileged_update).is_ok());
    }

    #[test]
    fn api_key_permissions_are_enforced_even_for_admin_users() {
        let under_scoped = auth(AuthType::ApiKey, &["admin"], &["read"]);
        let mut request = api_key_request("user-a");
        assert!(matches!(
            authorize_api_key_creation(&under_scoped, "user-a", &mut request),
            Err(AppError::Forbidden)
        ));

        let scoped = auth(AuthType::ApiKey, &["admin"], &["write"]);
        request.permissions = vec!["write".into()];
        assert!(authorize_api_key_creation(&scoped, "user-a", &mut request).is_ok());

        let mut escalation = api_key_request("user-a");
        escalation.permissions = vec!["write".into(), "delete".into()];
        assert!(matches!(
            authorize_api_key_creation(&scoped, "user-a", &mut escalation),
            Err(AppError::Forbidden)
        ));

        let wildcard_key = auth(AuthType::ApiKey, &["admin"], &["*"]);
        let mut privileged = api_key_request("user-a");
        privileged.permissions = vec!["*".into()];
        assert!(matches!(
            authorize_api_key_creation(&wildcard_key, "user-a", &mut privileged),
            Err(AppError::Forbidden)
        ));

        let non_admin_jwt = auth(AuthType::Jwt, &["user"], &[]);
        privileged.permissions = vec!["admin".into()];
        assert!(matches!(
            authorize_api_key_creation(&non_admin_jwt, "user-a", &mut privileged),
            Err(AppError::Forbidden)
        ));

        let admin_jwt = auth(AuthType::Jwt, &["admin"], &[]);
        assert!(authorize_api_key_creation(&admin_jwt, "user-a", &mut privileged).is_ok());
    }

    #[test]
    fn user_listing_requires_admin_role_and_read_permission() {
        let ordinary_user = auth(AuthType::Jwt, &["user"], &[]);
        assert!(matches!(
            require_admin_permission(&ordinary_user, "read"),
            Err(AppError::Forbidden)
        ));

        let under_scoped_admin = auth(AuthType::ApiKey, &["admin"], &["write"]);
        assert!(matches!(
            require_admin_permission(&under_scoped_admin, "read"),
            Err(AppError::Forbidden)
        ));

        let scoped_admin = auth(AuthType::ApiKey, &["admin"], &["read", "admin"]);
        assert!(require_admin_permission(&scoped_admin, "read").is_ok());
    }

    #[test]
    fn api_key_path_owner_is_authoritative_over_body_owner() {
        let context = auth(AuthType::Jwt, &["user"], &[]);
        let mut request = api_key_request("attacker-selected-owner");
        authorize_api_key_creation(&context, "user-a", &mut request).unwrap();
        assert_eq!(request.user_id, "user-a");
    }

    #[test]
    fn jwt_extractor_enforces_revocation_active_user_and_current_roles() {
        let source = include_str!("mod.rs");
        assert!(source.contains(".is_access_token_revoked(&claims.jti)"));
        assert!(source.contains(".filter(|user| user.active)"));
        assert!(source.contains("roles: user.roles"));
    }
}
