//! Rate limiting module for API endpoints

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use std::collections::HashMap;
use axum::{
    middleware::Next,
    response::{Response, IntoResponse},
    http::{Request, StatusCode, header},
    extract::{State, ConnectInfo},
};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct EnhancedRateLimiter {
    // Track by user ID when authenticated
    user_limits: Arc<RwLock<HashMap<String, UserRateLimit>>>,
    // Track by IP for unauthenticated requests
    ip_limits: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    // Failed login tracking
    failed_logins: Arc<RwLock<HashMap<String, FailedLoginInfo>>>,
    config: RateLimitConfig,
}

#[derive(Clone)]
pub struct RateLimitConfig {
    pub requests_per_minute: usize,
    pub requests_per_hour: usize,
    pub login_attempts_per_hour: usize,
    pub lockout_duration: Duration,
    pub cleanup_interval: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 100,
            requests_per_hour: 1000,
            login_attempts_per_hour: 5,
            lockout_duration: Duration::from_secs(900), // 15 minutes
            cleanup_interval: Duration::from_secs(300), // 5 minutes
        }
    }
}

#[derive(Clone)]
struct UserRateLimit {
    requests: Vec<Instant>,
    locked_until: Option<Instant>,
}

#[derive(Clone)]
struct FailedLoginInfo {
    attempts: Vec<Instant>,
    locked_until: Option<Instant>,
}

impl EnhancedRateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        let limiter = Self {
            user_limits: Arc::new(RwLock::new(HashMap::new())),
            ip_limits: Arc::new(RwLock::new(HashMap::new())),
            failed_logins: Arc::new(RwLock::new(HashMap::new())),
            config,
        };
        
        // Start cleanup task
        let limiter_clone = limiter.clone();
        tokio::spawn(async move {
            limiter_clone.cleanup_loop().await;
        });
        
        limiter
    }
    
    pub async fn check_rate_limit(&self, identifier: RateLimitIdentifier) -> Result<(), RateLimitError> {
        match identifier {
            RateLimitIdentifier::User(user_id) => self.check_user_limit(&user_id).await,
            RateLimitIdentifier::Ip(ip) => self.check_ip_limit(&ip).await,
        }
    }
    
    pub async fn record_failed_login(&self, username: &str) -> Result<(), RateLimitError> {
        let mut failed_logins = self.failed_logins.write().await;
        let now = Instant::now();
        
        let info = failed_logins.entry(username.to_string()).or_insert_with(|| FailedLoginInfo {
            attempts: Vec::new(),
            locked_until: None,
        });
        
        // Check if already locked
        if let Some(locked_until) = info.locked_until {
            if now < locked_until {
                return Err(RateLimitError::AccountLocked(locked_until - now));
            } else {
                // Lock expired, reset
                info.locked_until = None;
                info.attempts.clear();
            }
        }
        
        // Clean old attempts
        info.attempts.retain(|&t| now.duration_since(t) < Duration::from_secs(3600));
        
        // Add new attempt
        info.attempts.push(now);
        
        // Check if we should lock the account
        if info.attempts.len() >= self.config.login_attempts_per_hour {
            info.locked_until = Some(now + self.config.lockout_duration);
            return Err(RateLimitError::AccountLocked(self.config.lockout_duration));
        }
        
        Ok(())
    }
    
    pub async fn record_successful_login(&self, username: &str) {
        let mut failed_logins = self.failed_logins.write().await;
        // Clear failed attempts on successful login
        failed_logins.remove(username);
    }
    
    async fn check_user_limit(&self, user_id: &str) -> Result<(), RateLimitError> {
        let mut limits = self.user_limits.write().await;
        let now = Instant::now();
        
        let user_limit = limits.entry(user_id.to_string()).or_insert_with(|| UserRateLimit {
            requests: Vec::new(),
            locked_until: None,
        });
        
        // Check if account is locked
        if let Some(locked_until) = user_limit.locked_until {
            if now < locked_until {
                return Err(RateLimitError::AccountLocked(locked_until - now));
            } else {
                user_limit.locked_until = None;
            }
        }
        
        // Clean old requests
        user_limit.requests.retain(|&t| now.duration_since(t) < Duration::from_secs(60));
        
        if user_limit.requests.len() >= self.config.requests_per_minute {
            return Err(RateLimitError::TooManyRequests);
        }
        
        user_limit.requests.push(now);
        Ok(())
    }
    
    async fn check_ip_limit(&self, ip: &str) -> Result<(), RateLimitError> {
        let mut limits = self.ip_limits.write().await;
        let now = Instant::now();
        
        let timestamps = limits.entry(ip.to_string()).or_insert_with(Vec::new);
        
        // Clean old requests
        timestamps.retain(|&t| now.duration_since(t) < Duration::from_secs(60));
        
        if timestamps.len() >= self.config.requests_per_minute {
            return Err(RateLimitError::TooManyRequests);
        }
        
        timestamps.push(now);
        Ok(())
    }
    
    async fn cleanup_loop(&self) {
        let mut interval = tokio::time::interval(self.config.cleanup_interval);
        
        loop {
            interval.tick().await;
            
            // Cleanup old entries
            let now = Instant::now();
            
            // Cleanup user limits
            {
                let mut user_limits = self.user_limits.write().await;
                user_limits.retain(|_, limit| {
                    !limit.requests.is_empty() || 
                    limit.locked_until.map(|l| now < l).unwrap_or(false)
                });
            }
            
            // Cleanup IP limits
            {
                let mut ip_limits = self.ip_limits.write().await;
                ip_limits.retain(|_, timestamps| {
                    timestamps.retain(|&t| now.duration_since(t) < Duration::from_secs(3600));
                    !timestamps.is_empty()
                });
            }
            
            // Cleanup failed logins
            {
                let mut failed_logins = self.failed_logins.write().await;
                failed_logins.retain(|_, info| {
                    info.attempts.retain(|&t| now.duration_since(t) < Duration::from_secs(3600));
                    !info.attempts.is_empty() || info.locked_until.map(|l| now < l).unwrap_or(false)
                });
            }
        }
    }
}

#[derive(Debug)]
pub enum RateLimitIdentifier {
    User(String),
    Ip(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("Too many requests")]
    TooManyRequests,
    
    #[error("Account temporarily locked")]
    AccountLocked(Duration),
}

// Rate limiting middleware
pub async fn rate_limit_middleware(
    State(state): State<crate::api::ApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    use crate::{UserClaims, api::AuthType};
    use jsonwebtoken::{decode, Algorithm, Validation, DecodingKey};
    
    // Try to extract user ID from JWT token in Authorization header
    let mut user_id = None;
    
    if let Some(auth_header) = request.headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            // Try to decode the JWT to get the user ID
            if let Ok(public_key) = state.auth_service.jwt_issuer().public_key_pem() {
                if let Ok(decoding_key) = DecodingKey::from_rsa_pem(public_key.as_bytes()) {
                    let mut validation = Validation::new(Algorithm::RS256);
                    validation.set_issuer(&["https://users.rvoip.local"]);
                    validation.set_audience(&["rvoip-api", "rvoip-sip"]);
                    
                    if let Ok(token_data) = decode::<UserClaims>(token, &decoding_key, &validation) {
                        user_id = Some(token_data.claims.sub);
                    }
                }
            }
        }
    }
    
    // Determine identifier
    let identifier = if let Some(uid) = user_id {
        RateLimitIdentifier::User(uid)
    } else {
        // Get real IP from connection info
        let ip = connect_info
            .map(|ci| ci.0.ip().to_string())
            .or_else(|| {
                // Fallback to X-Forwarded-For if behind proxy (but be careful!)
                request.headers()
                    .get("x-real-ip")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown".to_string());
        
        RateLimitIdentifier::Ip(ip)
    };
    
    // Check rate limit
    match state.rate_limiter.check_rate_limit(identifier).await {
        Ok(()) => Ok(next.run(request).await),
        Err(RateLimitError::TooManyRequests) => {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            response.headers_mut().insert(
                header::RETRY_AFTER,
                "60".parse().unwrap()
            );
            Ok(response)
        }
        Err(RateLimitError::AccountLocked(duration)) => {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            response.headers_mut().insert(
                header::RETRY_AFTER,
                duration.as_secs().to_string().parse().unwrap()
            );
            Ok(response)
        }
    }
}

// Special handling for login endpoint
pub async fn handle_login_rate_limit(
    rate_limiter: &EnhancedRateLimiter,
    username: &str,
    login_result: Result<(), ()>,
) -> Result<(), RateLimitError> {
    match login_result {
        Ok(()) => {
            // Clear failed attempts on successful login
            rate_limiter.record_successful_login(username).await;
            Ok(())
        }
        Err(()) => {
            // Record failed attempt
            rate_limiter.record_failed_login(username).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_rate_limiting() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            requests_per_minute: 5,
            ..Default::default()
        });
        
        // Test user rate limiting
        for i in 0..5 {
            assert!(limiter.check_rate_limit(RateLimitIdentifier::User("user1".to_string())).await.is_ok());
        }
        
        // 6th request should fail
        assert!(limiter.check_rate_limit(RateLimitIdentifier::User("user1".to_string())).await.is_err());
        
        // Different user should work
        assert!(limiter.check_rate_limit(RateLimitIdentifier::User("user2".to_string())).await.is_ok());
    }
    
    #[tokio::test]
    async fn test_failed_login_lockout() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            login_attempts_per_hour: 3,
            lockout_duration: Duration::from_secs(1),
            ..Default::default()
        });
        
        // Record 3 failed attempts
        for _ in 0..3 {
            let result = limiter.record_failed_login("testuser").await;
            if result.is_err() {
                break;
            }
        }
        
        // 4th attempt should be locked
        assert!(matches!(
            limiter.record_failed_login("testuser").await,
            Err(RateLimitError::AccountLocked(_))
        ));
        
        // Wait for lockout to expire
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Should work again
        assert!(limiter.record_failed_login("testuser").await.is_ok());
    }
}
