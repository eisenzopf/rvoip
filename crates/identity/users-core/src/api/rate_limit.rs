//! Rate limiting module for API endpoints

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const MAX_IDENTITY_BYTES: usize = 256;

#[derive(Clone)]
pub struct EnhancedRateLimiter {
    // Track by user ID when authenticated
    user_limits: Arc<RwLock<HashMap<String, UserRateLimit>>>,
    // Track by IP for unauthenticated requests
    ip_limits: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    // Failed login tracking
    failed_logins: Arc<RwLock<HashMap<String, FailedLoginInfo>>>,
    config: RateLimitConfig,
    capacities: RateLimitCapacity,
    trusted_proxies: Arc<Vec<IpNet>>,
}

/// Hard bounds for attacker-controlled rate-limit identity keyspaces.
///
/// Reaching a bound fails new identities closed with `429`; an active entry is
/// never evicted to make room for an attacker-selected key.
#[derive(Clone, Copy, Debug)]
pub struct RateLimitCapacity {
    pub users: usize,
    pub ips: usize,
    pub failed_logins: usize,
}

impl Default for RateLimitCapacity {
    fn default() -> Self {
        Self {
            users: 16_384,
            ips: 16_384,
            failed_logins: 16_384,
        }
    }
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
            capacities: RateLimitCapacity::default(),
            trusted_proxies: Arc::new(Vec::new()),
        };

        // Start cleanup task
        let limiter_clone = limiter.clone();
        tokio::spawn(async move {
            limiter_clone.cleanup_loop().await;
        });

        limiter
    }

    /// Override identity-map bounds. Zero is normalized to one so a
    /// configuration error cannot accidentally disable fail-closed tracking.
    pub fn with_capacity(mut self, capacity: RateLimitCapacity) -> Self {
        self.capacities = RateLimitCapacity {
            users: capacity.users.max(1),
            ips: capacity.ips.max(1),
            failed_logins: capacity.failed_logins.max(1),
        };
        self
    }

    /// Trust forwarded client-address headers only when the immediate network
    /// peer is within one of these explicitly configured proxy CIDRs.
    pub fn with_trusted_proxies(mut self, proxies: impl IntoIterator<Item = IpNet>) -> Self {
        self.trusted_proxies = Arc::new(proxies.into_iter().collect());
        self
    }

    /// Resolve a request identity from the real socket peer and, for an
    /// explicitly trusted proxy only, a validated forwarding header.
    pub fn client_ip(&self, peer: Option<SocketAddr>, headers: &HeaderMap) -> Option<IpAddr> {
        let peer_ip = peer?.ip();
        if !self.is_trusted_proxy(peer_ip) {
            return Some(peer_ip);
        }

        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        {
            let parsed = forwarded
                .split(',')
                .map(str::trim)
                .map(str::parse::<IpAddr>)
                .collect::<Result<Vec<_>, _>>();
            if let Ok(addresses) = parsed {
                if let Some(client) = addresses
                    .into_iter()
                    .rev()
                    .find(|address| !self.is_trusted_proxy(*address))
                {
                    return Some(client);
                }
            }
        }

        if let Some(real_ip) = headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<IpAddr>().ok())
        {
            return Some(real_ip);
        }

        Some(peer_ip)
    }

    fn is_trusted_proxy(&self, address: IpAddr) -> bool {
        self.trusted_proxies
            .iter()
            .any(|network| network.contains(&address))
    }

    pub async fn check_rate_limit(
        &self,
        identifier: RateLimitIdentifier,
    ) -> Result<(), RateLimitError> {
        match identifier {
            RateLimitIdentifier::User(user_id) => self.check_user_limit(&user_id).await,
            RateLimitIdentifier::Ip(ip) => self.check_ip_limit(&ip).await,
        }
    }

    pub async fn record_failed_login(&self, username: &str) -> Result<(), RateLimitError> {
        if username.len() > MAX_IDENTITY_BYTES {
            return Err(RateLimitError::CapacityExhausted);
        }
        let mut failed_logins = self.failed_logins.write().await;
        let now = Instant::now();

        if !failed_logins.contains_key(username) {
            prune_failed_logins(&mut failed_logins, now);
            if failed_logins.len() >= self.capacities.failed_logins {
                return Err(RateLimitError::CapacityExhausted);
            }
        }

        let info = failed_logins
            .entry(username.to_string())
            .or_insert_with(|| FailedLoginInfo {
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
        info.attempts
            .retain(|&t| now.duration_since(t) < Duration::from_secs(3600));

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
        if user_id.len() > MAX_IDENTITY_BYTES {
            return Err(RateLimitError::CapacityExhausted);
        }
        let mut limits = self.user_limits.write().await;
        let now = Instant::now();

        if !limits.contains_key(user_id) {
            prune_user_limits(&mut limits, now);
            if limits.len() >= self.capacities.users {
                return Err(RateLimitError::CapacityExhausted);
            }
        }

        let user_limit = limits
            .entry(user_id.to_string())
            .or_insert_with(|| UserRateLimit {
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
        user_limit
            .requests
            .retain(|&t| now.duration_since(t) < Duration::from_secs(3600));

        let minute_count = user_limit
            .requests
            .iter()
            .filter(|&&t| now.duration_since(t) < Duration::from_secs(60))
            .count();
        if minute_count >= self.config.requests_per_minute
            || user_limit.requests.len() >= self.config.requests_per_hour
        {
            return Err(RateLimitError::TooManyRequests);
        }

        user_limit.requests.push(now);
        Ok(())
    }

    async fn check_ip_limit(&self, ip: &str) -> Result<(), RateLimitError> {
        if ip.len() > MAX_IDENTITY_BYTES {
            return Err(RateLimitError::CapacityExhausted);
        }
        let mut limits = self.ip_limits.write().await;
        let now = Instant::now();

        if !limits.contains_key(ip) {
            prune_ip_limits(&mut limits, now);
            if limits.len() >= self.capacities.ips {
                return Err(RateLimitError::CapacityExhausted);
            }
        }

        let timestamps = limits.entry(ip.to_string()).or_insert_with(Vec::new);

        // Clean old requests
        timestamps.retain(|&t| now.duration_since(t) < Duration::from_secs(3600));

        let minute_count = timestamps
            .iter()
            .filter(|&&t| now.duration_since(t) < Duration::from_secs(60))
            .count();
        if minute_count >= self.config.requests_per_minute
            || timestamps.len() >= self.config.requests_per_hour
        {
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
                prune_user_limits(&mut user_limits, now);
            }

            // Cleanup IP limits
            {
                let mut ip_limits = self.ip_limits.write().await;
                prune_ip_limits(&mut ip_limits, now);
            }

            // Cleanup failed logins
            {
                let mut failed_logins = self.failed_logins.write().await;
                prune_failed_logins(&mut failed_logins, now);
            }
        }
    }
}

fn prune_user_limits(limits: &mut HashMap<String, UserRateLimit>, now: Instant) {
    limits.retain(|_, limit| {
        limit
            .requests
            .retain(|&time| now.duration_since(time) < Duration::from_secs(3600));
        !limit.requests.is_empty() || limit.locked_until.is_some_and(|until| now < until)
    });
}

fn prune_ip_limits(limits: &mut HashMap<String, Vec<Instant>>, now: Instant) {
    limits.retain(|_, timestamps| {
        timestamps.retain(|&time| now.duration_since(time) < Duration::from_secs(3600));
        !timestamps.is_empty()
    });
}

fn prune_failed_logins(limits: &mut HashMap<String, FailedLoginInfo>, now: Instant) {
    limits.retain(|_, info| {
        info.attempts
            .retain(|&time| now.duration_since(time) < Duration::from_secs(3600));
        !info.attempts.is_empty() || info.locked_until.is_some_and(|until| now < until)
    });
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

    #[error("Rate-limit identity capacity exhausted")]
    CapacityExhausted,
}

// Rate limiting middleware
pub async fn rate_limit_middleware(
    State(state): State<crate::api::ApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Try to extract user ID from JWT token in Authorization header
    let mut user_id = None;

    if let Some(auth_header) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            // Validate the JWT with the issuer's configured issuer and audience.
            let issuer = state.auth_service.jwt_issuer();
            if let Ok(claims) = issuer.validate_access_token(token) {
                user_id = Some(claims.sub);
            }
        }
    }

    // Determine identifier
    let identifier = if let Some(uid) = user_id {
        RateLimitIdentifier::User(uid)
    } else {
        let ip = state
            .rate_limiter
            .client_ip(connect_info.map(|info| info.0), request.headers())
            .map(|address| address.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        RateLimitIdentifier::Ip(ip)
    };

    // Check rate limit
    match state.rate_limiter.check_rate_limit(identifier).await {
        Ok(()) => Ok(next.run(request).await),
        Err(RateLimitError::TooManyRequests) => {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
            Ok(response)
        }
        Err(RateLimitError::AccountLocked(duration)) => {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            response.headers_mut().insert(
                header::RETRY_AFTER,
                duration.as_secs().to_string().parse().unwrap(),
            );
            Ok(response)
        }
        Err(RateLimitError::CapacityExhausted) => {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
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
    async fn forwarded_headers_are_ignored_for_untrusted_peers() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig::default());
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.9".parse().unwrap());
        headers.insert(
            "x-forwarded-for",
            "203.0.113.10, 198.51.100.7".parse().unwrap(),
        );

        let peer = "192.0.2.8:443".parse().unwrap();
        assert_eq!(limiter.client_ip(Some(peer), &headers), Some(peer.ip()));
        assert_eq!(limiter.client_ip(None, &headers), None);
    }

    #[tokio::test]
    async fn trusted_proxy_uses_rightmost_untrusted_forwarded_address() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig::default())
            .with_trusted_proxies(["192.0.2.0/24".parse().unwrap()]);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.10, 198.51.100.7, 192.0.2.7".parse().unwrap(),
        );

        assert_eq!(
            limiter.client_ip(Some("192.0.2.8:443".parse().unwrap()), &headers),
            Some("198.51.100.7".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn identity_keyspaces_fail_closed_at_capacity() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            requests_per_minute: 100,
            requests_per_hour: 100,
            login_attempts_per_hour: 100,
            ..Default::default()
        })
        .with_capacity(RateLimitCapacity {
            users: 1,
            ips: 1,
            failed_logins: 1,
        });

        limiter
            .check_rate_limit(RateLimitIdentifier::User("user-a".into()))
            .await
            .unwrap();
        assert!(matches!(
            limiter
                .check_rate_limit(RateLimitIdentifier::User("user-b".into()))
                .await,
            Err(RateLimitError::CapacityExhausted)
        ));

        limiter
            .check_rate_limit(RateLimitIdentifier::Ip("192.0.2.1".into()))
            .await
            .unwrap();
        assert!(matches!(
            limiter
                .check_rate_limit(RateLimitIdentifier::Ip("192.0.2.2".into()))
                .await,
            Err(RateLimitError::CapacityExhausted)
        ));

        limiter.record_failed_login("alice").await.unwrap();
        assert!(matches!(
            limiter.record_failed_login("bob").await,
            Err(RateLimitError::CapacityExhausted)
        ));
        assert!(matches!(
            limiter
                .record_failed_login(&"x".repeat(MAX_IDENTITY_BYTES + 1))
                .await,
            Err(RateLimitError::CapacityExhausted)
        ));
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            requests_per_minute: 5,
            ..Default::default()
        });

        // Test user rate limiting
        for _i in 0..5 {
            assert!(limiter
                .check_rate_limit(RateLimitIdentifier::User("user1".to_string()))
                .await
                .is_ok());
        }

        // 6th request should fail
        assert!(limiter
            .check_rate_limit(RateLimitIdentifier::User("user1".to_string()))
            .await
            .is_err());

        // Different user should work
        assert!(limiter
            .check_rate_limit(RateLimitIdentifier::User("user2".to_string()))
            .await
            .is_ok());
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
