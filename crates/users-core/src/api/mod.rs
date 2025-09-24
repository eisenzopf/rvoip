//! REST API for users-core

use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{State, Path, Query, Json, FromRef},
    http::{StatusCode, HeaderMap, header, Request},
    response::{IntoResponse, Response},
    middleware::{self, Next},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;
use crate::{
    AuthenticationService, CreateUserRequest, UpdateUserRequest, UserFilter,
    api_keys::CreateApiKeyRequest, UserClaims, Error as UsersError,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, Algorithm, Validation, DecodingKey};

// API State
#[derive(Clone)]
pub struct ApiState {
    pub auth_service: Arc<AuthenticationService>,
    pub rate_limiter: RateLimiter,
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
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolesRequest {
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub key: String,
    pub key_info: ApiKeyResponse,
}

// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

// JWT validation for protected routes
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub username: String,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,  // For API key auth
    pub auth_type: AuthType,
}

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
            self.permissions.contains(&permission.to_string()) || 
            self.permissions.contains(&"*".to_string())  // Wildcard permission
        }
    }
    
    /// Check if the user has admin role
    pub fn is_admin(&self) -> bool {
        self.roles.contains(&"admin".to_string())
    }
}

/// Create the REST API router
pub fn create_router(auth_service: Arc<AuthenticationService>) -> Router {
    let state = ApiState { 
        auth_service,
        rate_limiter: RateLimiter::new(100, Duration::from_secs(60)), // 100 requests per minute
        metrics: Arc::new(Mutex::new(Metrics {
            start_time: Instant::now(),
            ..Default::default()
        })),
    };
    
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
        
        // Apply middleware
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware))
        .with_state(state)
}

// Authentication handlers

async fn login(
    State(state): State<ApiState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // Track authentication attempt
    {
        let mut metrics = state.metrics.lock().unwrap();
        metrics.authentication_attempts += 1;
    }
    
    let auth_result = state.auth_service
        .authenticate_password(&req.username, &req.password)
        .await?;
    
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
    auth: AuthContext,
) -> Result<StatusCode, AppError> {
    state.auth_service.revoke_tokens(&auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh(
    State(state): State<ApiState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let token_pair = state.auth_service
        .refresh_token(&req.refresh_token)
        .await?;
    
    Ok(Json(LoginResponse {
        access_token: token_pair.access_token,
        refresh_token: token_pair.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: token_pair.expires_in.as_secs(),
    }))
}

async fn jwks(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let jwk = state.auth_service.jwt_issuer().public_key_jwk();
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
    // Only admins can create users
    if !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    let user = state.auth_service.create_user(req).await?;
    
    Ok((StatusCode::CREATED, Json(UserResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        display_name: user.display_name,
        roles: user.roles,
        active: user.active,
        created_at: user.created_at,
        updated_at: user.updated_at,
        last_login: user.last_login,
    })))
}

async fn list_users(
    State(state): State<ApiState>,
    _auth: AuthContext,
    Query(filter): Query<UserFilter>,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    let users = state.auth_service
        .user_store()
        .list_users(filter)
        .await?;
    
    let responses: Vec<UserResponse> = users.into_iter()
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
    // Users can get their own info, admins can get anyone's
    if auth.user_id != id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    
    let user = state.auth_service
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
    // Users can update their own info (limited), admins can update anyone
    if auth.user_id != id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    
    let user = state.auth_service
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

async fn delete_user(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Only admins can delete users
    if !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    state.auth_service
        .user_store()
        .delete_user(&id)
        .await?;
    
    Ok(StatusCode::NO_CONTENT)
}

async fn change_password(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(id): Path<String>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    // Users can only change their own password
    if auth.user_id != id {
        return Err(AppError::Forbidden);
    }
    
    state.auth_service
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
    // Only admins can update roles
    if !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    
    // Update the user's roles
    let update_req = UpdateUserRequest {
        email: None,
        display_name: None,
        roles: Some(req.roles),
        active: None,
    };
    
    state.auth_service
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
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), AppError> {
    // Users can create their own API keys, admins can create for anyone
    if auth.user_id != user_id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    
    let (key_info, raw_key) = state.auth_service
        .api_key_store()
        .create_api_key(req)
        .await?;
    
    // Track API key creation
    {
        let mut metrics = state.metrics.lock().unwrap();
        metrics.total_api_keys += 1;
    }
    
    Ok((StatusCode::CREATED, Json(CreateApiKeyResponse {
        key: raw_key,
        key_info: ApiKeyResponse {
            id: key_info.id,
            name: key_info.name,
            permissions: key_info.permissions,
            expires_at: key_info.expires_at,
            created_at: key_info.created_at,
            last_used: key_info.last_used,
        },
    })))
}

async fn list_api_keys(
    State(state): State<ApiState>,
    auth: AuthContext,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<ApiKeyResponse>>, AppError> {
    // Users can list their own API keys, admins can list anyone's
    if auth.user_id != user_id && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    
    let keys = state.auth_service
        .api_key_store()
        .list_api_keys(&user_id)
        .await?;
    
    let responses: Vec<ApiKeyResponse> = keys.into_iter()
        .map(|k| ApiKeyResponse {
            id: k.id,
            name: k.name,
            permissions: k.permissions,
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
    // Get the API key to check ownership
    let keys = state.auth_service
        .api_key_store()
        .list_api_keys(&auth.user_id)
        .await?;
    
    // Check if user owns this key or is admin
    let owns_key = keys.iter().any(|k| k.id == id);
    if !owns_key && !auth.is_admin() {
        return Err(AppError::Forbidden);
    }
    
    state.auth_service
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
    let user_count = match state.auth_service
        .user_store()
        .list_users(UserFilter::default())
        .await 
    {
        Ok(users) => users.len(),
        Err(_) => 0,
    };
    
    let active_user_count = match state.auth_service
        .user_store()
        .list_users(UserFilter { active: Some(true), ..Default::default() })
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
            "active": total_api_keys, // Simplified: all keys are considered active
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

#[derive(Debug)]
pub enum AppError {
    Internal(anyhow::Error),
    InvalidCredentials,
    NotFound,
    Forbidden,
    BadRequest(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::Internal(e) => {
                tracing::error!("Internal error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", "An internal error occurred".to_string())
            },
            AppError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "Invalid username or password".to_string())
            },
            AppError::NotFound => {
                (StatusCode::NOT_FOUND, "NOT_FOUND", "Resource not found".to_string())
            },
            AppError::Forbidden => {
                (StatusCode::FORBIDDEN, "FORBIDDEN", "Access denied".to_string())
            },
            AppError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg)
            },
        };
        
        let body = Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message,
                details: None,
            },
        });
        
        (status, body).into_response()
    }
}

impl From<UsersError> for AppError {
    fn from(err: UsersError) -> Self {
        match err {
            UsersError::InvalidCredentials => AppError::InvalidCredentials,
            UsersError::UserNotFound(_) => AppError::NotFound,
            UsersError::UserAlreadyExists(_) => AppError::BadRequest("User already exists".to_string()),
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
        // Check for Bearer token first
        if let Some(auth_header) = parts.headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            if let Some(token) = auth_header.strip_prefix("Bearer ") {
                let api_state = ApiState::from_ref(state);
                
                // Get public key and validate
                let public_key = api_state.auth_service
                    .jwt_issuer()
                    .public_key_pem()
                    .map_err(|e| AppError::Internal(e.into()))?;
                
                let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes())
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("Invalid public key: {}", e)))?;
                
                let mut validation = Validation::new(Algorithm::RS256);
                validation.set_issuer(&["https://users.rvoip.local"]);
                validation.set_audience(&["rvoip-api", "rvoip-sip"]);
                
                let token_data = decode::<crate::UserClaims>(token, &decoding_key, &validation)
                    .map_err(|_| AppError::Forbidden)?;
                
                return Ok(AuthContext {
                    user_id: token_data.claims.sub,
                    username: token_data.claims.username,
                    roles: token_data.claims.roles,
                    permissions: vec![],  // JWT tokens don't have granular permissions
                    auth_type: AuthType::Jwt,
                });
            }
        }
        
        // Check for API key
        if let Some(api_key) = parts.headers
            .get("X-API-Key")
            .and_then(|value| value.to_str().ok())
        {
            let api_state = ApiState::from_ref(state);
            
            let api_key_info = api_state.auth_service
                .api_key_store()
                .validate_api_key(api_key)
                .await
                .map_err(|_| AppError::Forbidden)?
                .ok_or(AppError::Forbidden)?;
            
            // Get the user to construct AuthContext
            let user = api_state.auth_service
                .user_store()
                .get_user(&api_key_info.user_id)
                .await
                .map_err(|_| AppError::Forbidden)?
                .ok_or(AppError::Forbidden)?;
            
            return Ok(AuthContext {
                user_id: user.id,
                username: user.username,
                roles: user.roles,
                permissions: api_key_info.permissions,
                auth_type: AuthType::ApiKey,
            });
        }
        
        // No valid authentication found
        Err(AppError::Forbidden)
    }
}

// Rate limiting (simple in-memory implementation)

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window,
        }
    }
    
    pub fn check_rate_limit(&self, key: &str) -> bool {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        
        let timestamps = requests.entry(key.to_string()).or_insert_with(Vec::new);
        
        // Remove old timestamps
        timestamps.retain(|&ts| now.duration_since(ts) < self.window);
        
        if timestamps.len() < self.max_requests {
            timestamps.push(now);
            true
        } else {
            false
        }
    }
}

// Rate limiting middleware
async fn rate_limit_middleware(
    State(state): State<ApiState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Track API request
    {
        let mut metrics = state.metrics.lock().unwrap();
        metrics.api_requests += 1;
    }
    
    // Get client identifier (IP address or user ID from token)
    let client_id = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    
    if !state.rate_limiter.check_rate_limit(&client_id) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    
    Ok(next.run(request).await)
}
