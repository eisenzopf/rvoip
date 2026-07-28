//! Rate limiting module for API endpoints

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use serde::Deserialize;
use std::collections::{hash_map::RandomState, HashMap};
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_OVERFLOW_BUCKETS: usize = 1_024;
/// IPv6 clients are rate-limited by /64 so rotating interface identifiers
/// cannot manufacture an unbounded number of independent identities.
pub const IPV6_RATE_LIMIT_PREFIX_BITS: u8 = 64;

struct BoundedIdentityMap<T> {
    exact: HashMap<String, T>,
    overflow: HashMap<usize, T>,
    hash_builder: RandomState,
}

impl<T> Default for BoundedIdentityMap<T> {
    fn default() -> Self {
        Self {
            exact: HashMap::new(),
            overflow: HashMap::new(),
            hash_builder: RandomState::new(),
        }
    }
}

impl<T> BoundedIdentityMap<T> {
    fn get_or_insert_with(
        &mut self,
        identity: &str,
        exact_capacity: usize,
        force_overflow: bool,
        create: impl FnOnce() -> T,
    ) -> &mut T {
        if !force_overflow && self.exact.contains_key(identity) {
            return self.exact.get_mut(identity).expect("identity was present");
        }

        if !force_overflow && self.exact.len() < exact_capacity {
            return self.exact.entry(identity.to_owned()).or_insert_with(create);
        }

        let bucket_count = exact_capacity.clamp(1, MAX_OVERFLOW_BUCKETS);
        let mut hasher = self.hash_builder.build_hasher();
        identity.hash(&mut hasher);
        let bucket = (hasher.finish() as usize) % bucket_count;
        self.overflow.entry(bucket).or_insert_with(create)
    }

    fn remove_exact(&mut self, identity: &str) {
        self.exact.remove(identity);
    }

    fn retain(&mut self, mut keep: impl FnMut(&mut T) -> bool) {
        self.exact.retain(|_, value| keep(value));
        self.overflow.retain(|_, value| keep(value));
    }

    #[cfg(test)]
    fn stored_entries(&self) -> usize {
        self.exact.len() + self.overflow.len()
    }
}

#[derive(Clone)]
pub struct EnhancedRateLimiter {
    // Track by user ID when authenticated
    user_limits: Arc<RwLock<BoundedIdentityMap<UserRateLimit>>>,
    // Track by IP for unauthenticated requests
    ip_limits: Arc<RwLock<BoundedIdentityMap<Vec<Instant>>>>,
    // Failed login tracking
    failed_logins: Arc<RwLock<BoundedIdentityMap<FailedLoginInfo>>>,
    config: RateLimitConfig,
    capacities: RateLimitCapacity,
    trusted_proxies: Arc<Vec<IpNet>>,
    next_cleanup: Arc<Mutex<Instant>>,
}

/// Hard bounds for attacker-controlled rate-limit identity keyspaces.
///
/// Once an exact identity map reaches its bound, unseen identities are routed
/// through a secret-hashed bounded overflow tier. Existing exact identities
/// and active lockout state are never evicted to make room for attacker input.
#[non_exhaustive]
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

impl RateLimitCapacity {
    pub fn new(users: usize, ips: usize, failed_logins: usize) -> Self {
        Self {
            users,
            ips,
            failed_logins,
        }
    }
}

/// Declarative trusted-proxy configuration for embedding the REST router.
/// Forwarding headers are ignored unless the immediate socket peer matches a
/// configured CIDR.
#[non_exhaustive]
#[derive(Clone, Default, Deserialize)]
pub struct TrustedProxyConfig {
    #[serde(default)]
    pub cidrs: Vec<String>,
}

impl fmt::Debug for TrustedProxyConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrustedProxyConfig")
            .field("cidr_count", &self.cidrs.len())
            .finish()
    }
}

impl TrustedProxyConfig {
    pub fn new(cidrs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            cidrs: cidrs.into_iter().map(Into::into).collect(),
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
        let cleanup_interval = normalized_cleanup_interval(config.cleanup_interval);
        let next_cleanup = Instant::now()
            .checked_add(cleanup_interval)
            .unwrap_or_else(Instant::now);
        Self {
            user_limits: Arc::new(RwLock::new(BoundedIdentityMap::default())),
            ip_limits: Arc::new(RwLock::new(BoundedIdentityMap::default())),
            failed_logins: Arc::new(RwLock::new(BoundedIdentityMap::default())),
            config,
            capacities: RateLimitCapacity::default(),
            trusted_proxies: Arc::new(Vec::new()),
            next_cleanup: Arc::new(Mutex::new(next_cleanup)),
        }
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

    /// Apply trusted proxy CIDRs parsed from declarative configuration.
    pub fn with_trusted_proxy_config(mut self, config: TrustedProxyConfig) -> crate::Result<Self> {
        let proxies = config
            .cidrs
            .into_iter()
            .map(|cidr| {
                cidr.parse::<IpNet>().map_err(|error| {
                    crate::Error::Config(format!("invalid trusted proxy CIDR {cidr:?}: {error}"))
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        self.trusted_proxies = Arc::new(proxies);
        Ok(self)
    }

    /// Resolve a request identity from the real socket peer and, for an
    /// explicitly trusted proxy only, a validated forwarding header.
    pub fn client_ip(&self, peer: Option<SocketAddr>, headers: &HeaderMap) -> Option<IpAddr> {
        let peer_ip = peer?.ip();
        if !self.is_trusted_proxy(peer_ip) {
            return Some(normalize_client_ip(peer_ip));
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
                    return Some(normalize_client_ip(client));
                }
            }
        }

        if let Some(real_ip) = headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<IpAddr>().ok())
        {
            return Some(normalize_client_ip(real_ip));
        }

        Some(normalize_client_ip(peer_ip))
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
        let now = Instant::now();
        self.maybe_cleanup(now).await;
        let mut failed_logins = self.failed_logins.write().await;
        let info = failed_logins.get_or_insert_with(
            username,
            self.capacities.failed_logins,
            username.len() > MAX_IDENTITY_BYTES,
            || FailedLoginInfo {
                attempts: Vec::new(),
                locked_until: None,
            },
        );

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
        // Clear an exact identity after successful login. Overflow state is
        // deliberately retained because it may represent multiple identities.
        failed_logins.remove_exact(username);
    }

    async fn check_user_limit(&self, user_id: &str) -> Result<(), RateLimitError> {
        let now = Instant::now();
        self.maybe_cleanup(now).await;
        let mut limits = self.user_limits.write().await;
        let user_limit = limits.get_or_insert_with(
            user_id,
            self.capacities.users,
            user_id.len() > MAX_IDENTITY_BYTES,
            || UserRateLimit {
                requests: Vec::new(),
                locked_until: None,
            },
        );

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
        let identity = normalize_ip_text(ip);
        let now = Instant::now();
        self.maybe_cleanup(now).await;
        let mut limits = self.ip_limits.write().await;
        let timestamps = limits.get_or_insert_with(
            &identity,
            self.capacities.ips,
            identity.len() > MAX_IDENTITY_BYTES,
            Vec::new,
        );

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

    async fn maybe_cleanup(&self, now: Instant) {
        let interval = normalized_cleanup_interval(self.config.cleanup_interval);
        let should_cleanup = {
            let mut next_cleanup = self
                .next_cleanup
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if now < *next_cleanup {
                false
            } else {
                *next_cleanup = now.checked_add(interval).unwrap_or(now);
                true
            }
        };
        if !should_cleanup {
            return;
        }

        {
            let mut limits = self.user_limits.write().await;
            prune_user_limits(&mut limits, now);
        }
        {
            let mut limits = self.ip_limits.write().await;
            prune_ip_limits(&mut limits, now);
        }
        {
            let mut limits = self.failed_logins.write().await;
            prune_failed_logins(&mut limits, now);
        }
    }
}

fn normalized_cleanup_interval(interval: Duration) -> Duration {
    if interval.is_zero() {
        Duration::from_secs(1)
    } else {
        interval.min(Duration::from_secs(3600))
    }
}

fn prune_user_limits(limits: &mut BoundedIdentityMap<UserRateLimit>, now: Instant) {
    limits.retain(|limit| {
        limit
            .requests
            .retain(|&time| now.duration_since(time) < Duration::from_secs(3600));
        !limit.requests.is_empty() || limit.locked_until.is_some_and(|until| now < until)
    });
}

fn prune_ip_limits(limits: &mut BoundedIdentityMap<Vec<Instant>>, now: Instant) {
    limits.retain(|timestamps| {
        timestamps.retain(|&time| now.duration_since(time) < Duration::from_secs(3600));
        !timestamps.is_empty()
    });
}

fn prune_failed_logins(limits: &mut BoundedIdentityMap<FailedLoginInfo>, now: Instant) {
    limits.retain(|info| {
        info.attempts
            .retain(|&time| now.duration_since(time) < Duration::from_secs(3600));
        !info.attempts.is_empty() || info.locked_until.is_some_and(|until| now < until)
    });
}

fn normalize_client_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(address) => IpAddr::V4(address),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return IpAddr::V4(mapped);
            }
            let network = u128::from(address) & (u128::MAX << (128 - IPV6_RATE_LIMIT_PREFIX_BITS));
            IpAddr::V6(Ipv6Addr::from(network))
        }
    }
}

fn normalize_ip_text(value: &str) -> String {
    value
        .parse::<IpAddr>()
        .map(normalize_client_ip)
        .map(|address| match address {
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("{address}/{IPV6_RATE_LIMIT_PREFIX_BITS}"),
        })
        // Invalid values can only enter through the public programmatic API,
        // not the peer-aware HTTP middleware. They are forced into the bounded
        // overflow tier by retaining their original overlength marker.
        .unwrap_or_else(|_| {
            if value.len() > MAX_IDENTITY_BYTES {
                value.to_owned()
            } else {
                format!("invalid-ip:{value}")
            }
        })
}

pub enum RateLimitIdentifier {
    User(String),
    Ip(String),
}

impl std::fmt::Debug for RateLimitIdentifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, value) = match self {
            Self::User(value) => ("user", value),
            Self::Ip(value) => ("ip", value),
        };
        formatter
            .debug_struct("RateLimitIdentifier")
            .field("kind", &kind)
            .field("value_present", &!value.is_empty())
            .field("value_bytes", &value.len())
            .finish()
    }
}

#[non_exhaustive]
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
    let peer = match connect_info {
        Some(info) => info.0,
        None => {
            tracing::error!(
                "users-core router was served without ConnectInfo<SocketAddr>; refusing request"
            );
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

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
            .client_ip(Some(peer), request.headers())
            .map(|address| address.to_string())
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

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
    async fn identity_keyspaces_use_bounded_overflow_at_capacity() {
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
        limiter
            .check_rate_limit(RateLimitIdentifier::User("user-b".into()))
            .await
            .unwrap();

        limiter
            .check_rate_limit(RateLimitIdentifier::Ip("192.0.2.1".into()))
            .await
            .unwrap();
        limiter
            .check_rate_limit(RateLimitIdentifier::Ip("198.51.100.2".into()))
            .await
            .unwrap();

        limiter.record_failed_login("alice").await.unwrap();
        limiter.record_failed_login("bob").await.unwrap();
        limiter
            .record_failed_login(&"x".repeat(MAX_IDENTITY_BYTES + 1))
            .await
            .unwrap();

        assert!(limiter.user_limits.read().await.stored_entries() <= 2);
        assert!(limiter.ip_limits.read().await.stored_entries() <= 2);
        assert!(limiter.failed_logins.read().await.stored_entries() <= 2);
    }

    #[tokio::test]
    async fn saturation_does_not_evict_active_lockout_state() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            login_attempts_per_hour: 1,
            lockout_duration: Duration::from_secs(60),
            ..Default::default()
        })
        .with_capacity(RateLimitCapacity {
            users: 1,
            ips: 1,
            failed_logins: 1,
        });

        assert!(matches!(
            limiter.record_failed_login("alice").await,
            Err(RateLimitError::AccountLocked(_))
        ));
        let _ = limiter.record_failed_login("bob").await;
        assert!(matches!(
            limiter.record_failed_login("alice").await,
            Err(RateLimitError::AccountLocked(_))
        ));
    }

    #[tokio::test]
    async fn ipv6_interface_rotation_and_equivalent_forms_share_one_budget() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            requests_per_minute: 1,
            requests_per_hour: 100,
            ..Default::default()
        });

        limiter
            .check_rate_limit(RateLimitIdentifier::Ip("2001:db8:1:2::1".into()))
            .await
            .unwrap();
        assert!(matches!(
            limiter
                .check_rate_limit(RateLimitIdentifier::Ip(
                    "2001:0db8:0001:0002:ffff::abcd".into()
                ))
                .await,
            Err(RateLimitError::TooManyRequests)
        ));
        limiter
            .check_rate_limit(RateLimitIdentifier::Ip("2001:db8:1:3::1".into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ipv4_mapped_ipv6_shares_the_ipv4_budget() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            requests_per_minute: 1,
            ..Default::default()
        });
        limiter
            .check_rate_limit(RateLimitIdentifier::Ip("::ffff:192.0.2.9".into()))
            .await
            .unwrap();
        assert!(matches!(
            limiter
                .check_rate_limit(RateLimitIdentifier::Ip("192.0.2.9".into()))
                .await,
            Err(RateLimitError::TooManyRequests)
        ));
    }

    #[test]
    fn construction_and_drop_do_not_require_a_runtime_or_retain_maps() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig {
            cleanup_interval: Duration::ZERO,
            ..Default::default()
        });
        let maps = Arc::downgrade(&limiter.ip_limits);
        drop(limiter);
        assert!(maps.upgrade().is_none());
    }

    #[test]
    fn trusted_proxy_configuration_is_declarative_and_validated() {
        let limiter = EnhancedRateLimiter::new(RateLimitConfig::default())
            .with_trusted_proxy_config(TrustedProxyConfig {
                cidrs: vec!["192.0.2.0/24".into()],
            })
            .unwrap();
        assert!(limiter.is_trusted_proxy("192.0.2.10".parse().unwrap()));

        assert!(EnhancedRateLimiter::new(RateLimitConfig::default())
            .with_trusted_proxy_config(TrustedProxyConfig {
                cidrs: vec!["not-a-cidr".into()],
            })
            .is_err());
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
