//! Token-bucket rate limiting middleware.
//!
//! Provides per-IP rate limiting using [`DashMap`] for concurrent tracking.
//! Two static limiters are maintained:
//!
//! - **API_LIMITER** — 200 requests per 60 seconds for general API calls
//! - **LOGIN_LIMITER** — 5 requests per 60 seconds for `/auth/login` (anti-brute-force)

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use axum::extract::ConnectInfo;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;

/// A simple token-bucket rate limiter that tracks request counts per IP address.
pub struct RateLimiter {
    requests: DashMap<IpAddr, (u64, Instant)>,
    limit: u64,
    window: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter allowing `limit` requests per `window_secs`.
    pub fn new(limit: u64, window_secs: u64) -> Self {
        Self {
            requests: DashMap::new(),
            limit,
            window: Duration::from_secs(window_secs),
        }
    }

    /// Check whether the given IP is within its rate limit.
    ///
    /// Returns `true` if the request is allowed, `false` if the limit is exceeded.
    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut entry = self.requests.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) > self.window {
            *entry = (1, now);
            true
        } else if entry.0 < self.limit {
            entry.0 += 1;
            true
        } else {
            false
        }
    }
}

/// General API rate limiter: 200 requests per 60 seconds per IP.
static API_LIMITER: LazyLock<RateLimiter> = LazyLock::new(|| RateLimiter::new(200, 60));

/// Login endpoint rate limiter: 5 requests per 60 seconds per IP (anti-brute-force).
static LOGIN_LIMITER: LazyLock<RateLimiter> = LazyLock::new(|| RateLimiter::new(5, 60));

/// Extract the client IP from the request.
///
/// Attempts to read from `ConnectInfo<SocketAddr>` (set by `into_make_service_with_connect_info`),
/// falling back to `127.0.0.1` when unavailable.
fn extract_ip(request: &Request) -> IpAddr {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}

/// Axum middleware that enforces per-IP rate limits.
///
/// Login paths (`/auth/login`) use the stricter [`LOGIN_LIMITER`]; all other
/// paths use [`API_LIMITER`]. Returns `429 Too Many Requests` when exceeded.
pub async fn rate_limit_middleware(request: Request, next: Next) -> Response {
    let ip = extract_ip(&request);
    let path = request.uri().path();

    let limiter = if path.contains("/auth/login") {
        &*LOGIN_LIMITER
    } else {
        &*API_LIMITER
    };

    if !limiter.check(ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
    }

    next.run(request).await
}
