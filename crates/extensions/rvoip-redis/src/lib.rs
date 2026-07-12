//! Redis-backed auth provider implementations for RVoIP.
//!
//! This crate is an optional extension. Core protocol crates consume
//! `rvoip-auth-core` provider traits; applications that need shared auth state
//! in a clustered deployment can use `RedisAuthProvider` as a concrete
//! implementation for SIP Digest replay, token revocation, and auth rate
//! limiting.

#[cfg(feature = "moq")]
mod moq;

#[cfg(feature = "moq")]
pub use moq::{RedisMoqSessionLeaseConfig, RedisMoqSessionLeaseStore};

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use redis::AsyncCommands;
use rvoip_auth_core::{
    AuthAuditOutcome, AuthRateLimitKey, AuthRateLimitVerdict, AuthRateLimiter, CredentialAuthError,
    DigestNonceStatus, DigestReplayStore, TokenRevocationChecker, TokenRevocationContext,
    TokenRevocationStatus,
};
use thiserror::Error;

/// Errors returned while constructing or administering Redis auth providers.
#[derive(Debug, Error)]
pub enum RedisAuthError {
    /// Redis client or command failure.
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// A configured duration was too large for Redis second-granularity TTLs.
    #[error("duration is too large for redis ttl seconds")]
    DurationTooLarge,
}

/// Configuration for Redis-backed auth provider state.
#[derive(Debug, Clone)]
pub struct RedisAuthConfig {
    /// Redis connection URL, such as `redis://127.0.0.1:6379`.
    pub redis_url: String,
    /// Namespace prefix for all keys written by this provider.
    pub namespace: String,
    /// Extra retention for issued Digest nonce records after nonce expiry.
    ///
    /// Retaining expired nonce records lets SIP Digest UAS code distinguish a
    /// known stale nonce from an unknown nonce.
    pub nonce_stale_retention: Duration,
    /// TTL for nonce-count replay records.
    pub nonce_count_ttl: Duration,
    /// TTL used when revoking a token without a token expiry time.
    pub token_revocation_ttl: Duration,
    /// Fixed rate-limit window.
    pub rate_limit_window: Duration,
    /// Maximum failed attempts accepted in one rate-limit window.
    pub max_failures_per_window: u32,
}

impl RedisAuthConfig {
    /// Build a Redis auth provider config with production-oriented defaults.
    pub fn new(redis_url: impl Into<String>) -> Self {
        Self {
            redis_url: redis_url.into(),
            namespace: "rvoip:auth".to_string(),
            nonce_stale_retention: Duration::from_secs(300),
            nonce_count_ttl: Duration::from_secs(600),
            token_revocation_ttl: Duration::from_secs(24 * 60 * 60),
            rate_limit_window: Duration::from_secs(60),
            max_failures_per_window: 10,
        }
    }

    /// Set the Redis key namespace.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Set how long expired Digest nonce records are retained.
    pub fn with_nonce_stale_retention(mut self, retention: Duration) -> Self {
        self.nonce_stale_retention = retention;
        self
    }

    /// Set the TTL for nonce-count replay records.
    pub fn with_nonce_count_ttl(mut self, ttl: Duration) -> Self {
        self.nonce_count_ttl = ttl;
        self
    }

    /// Set the default TTL for token revocation markers.
    pub fn with_token_revocation_ttl(mut self, ttl: Duration) -> Self {
        self.token_revocation_ttl = ttl;
        self
    }

    /// Set the fixed rate-limit window.
    pub fn with_rate_limit_window(mut self, window: Duration) -> Self {
        self.rate_limit_window = window;
        self
    }

    /// Set the maximum failed attempts permitted in one rate-limit window.
    pub fn with_max_failures_per_window(mut self, max_failures: u32) -> Self {
        self.max_failures_per_window = max_failures;
        self
    }
}

/// Redis-backed auth provider for shared enterprise auth state.
#[derive(Debug, Clone)]
pub struct RedisAuthProvider {
    client: redis::Client,
    config: RedisAuthConfig,
}

impl RedisAuthProvider {
    /// Create a Redis provider from a Redis URL and default configuration.
    pub fn new(redis_url: impl Into<String>) -> Result<Self, RedisAuthError> {
        Self::from_config(RedisAuthConfig::new(redis_url))
    }

    /// Create a Redis provider from explicit configuration.
    pub fn from_config(config: RedisAuthConfig) -> Result<Self, RedisAuthError> {
        let client = redis::Client::open(config.redis_url.as_str())?;
        Ok(Self { client, config })
    }

    /// Return this provider's configuration.
    pub fn config(&self) -> &RedisAuthConfig {
        &self.config
    }

    /// Revoke a token id globally until its expiry time or the configured
    /// default revocation TTL.
    pub async fn revoke_token_id(
        &self,
        token_id: &str,
        expires_at: Option<SystemTime>,
    ) -> Result<(), RedisAuthError> {
        let key = self.token_key(None, token_id);
        self.set_revocation_key(&key, expires_at).await
    }

    /// Revoke a token using the same context shape supplied to
    /// `TokenRevocationChecker`.
    pub async fn revoke_token(
        &self,
        context: &TokenRevocationContext,
    ) -> Result<(), RedisAuthError> {
        let key = self.token_key(context.issuer.as_deref(), &context.token_id);
        self.set_revocation_key(&key, context.expires_at).await
    }

    /// Remove keys in this provider namespace.
    ///
    /// This helper is intended for local fixtures and integration tests.
    pub async fn clear_namespace_for_tests(&self) -> Result<(), RedisAuthError> {
        let mut connection = self.connection().await?;
        let pattern = format!("{}:*", self.config.namespace);
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(pattern)
            .query_async(&mut connection)
            .await?;
        if !keys.is_empty() {
            let _: () = redis::cmd("DEL")
                .arg(keys)
                .query_async(&mut connection)
                .await?;
        }
        Ok(())
    }

    async fn set_revocation_key(
        &self,
        key: &str,
        expires_at: Option<SystemTime>,
    ) -> Result<(), RedisAuthError> {
        let ttl = ttl_from_expiry_or_default(expires_at, self.config.token_revocation_ttl)?;
        let mut connection = self.connection().await?;
        let _: () = connection.set_ex(key, "revoked", ttl).await?;
        Ok(())
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        self.client.get_multiplexed_async_connection().await
    }

    fn nonce_key(&self, nonce: &str) -> String {
        format!("{}:digest:nonce:{}", self.config.namespace, hex_key(nonce))
    }

    fn nonce_count_key(&self, username: &str, nonce: &str) -> String {
        format!(
            "{}:digest:nc:{}:{}",
            self.config.namespace,
            hex_key(username),
            hex_key(nonce)
        )
    }

    fn token_key(&self, issuer: Option<&str>, token_id: &str) -> String {
        format!(
            "{}:token:revoked:{}:{}",
            self.config.namespace,
            issuer.map(hex_key).unwrap_or_else(|| "_".to_string()),
            hex_key(token_id)
        )
    }

    fn rate_limit_key(&self, key: &AuthRateLimitKey) -> String {
        let canonical = format!(
            "kind={:?}|subject={}|realm={}|peer={}",
            key.kind,
            key.subject.as_deref().unwrap_or("_"),
            key.realm.as_deref().unwrap_or("_"),
            key.peer.as_deref().unwrap_or("_")
        );
        format!("{}:rate:{}", self.config.namespace, hex_key(&canonical))
    }
}

#[async_trait]
impl DigestReplayStore for RedisAuthProvider {
    async fn record_nonce(
        &self,
        nonce: &str,
        expires_at: SystemTime,
    ) -> Result<(), CredentialAuthError> {
        let expires_unix = unix_seconds(expires_at)?;
        let ttl_until_expiry = expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_else(|_| Duration::from_secs(0));
        let ttl = ttl_until_expiry
            .saturating_add(self.config.nonce_stale_retention)
            .max(Duration::from_secs(1));
        let mut connection = self.connection().await.map_credential_error()?;
        let _: () = connection
            .set_ex(self.nonce_key(nonce), expires_unix, duration_secs(ttl)?)
            .await
            .map_credential_error()?;
        Ok(())
    }

    async fn nonce_status(
        &self,
        nonce: &str,
        now: SystemTime,
    ) -> Result<DigestNonceStatus, CredentialAuthError> {
        let mut connection = self.connection().await.map_credential_error()?;
        let expires_unix: Option<u64> = connection
            .get(self.nonce_key(nonce))
            .await
            .map_credential_error()?;
        let Some(expires_unix) = expires_unix else {
            return Ok(DigestNonceStatus::Unknown);
        };
        if unix_seconds(now)? < expires_unix {
            Ok(DigestNonceStatus::Active)
        } else {
            Ok(DigestNonceStatus::Expired)
        }
    }

    async fn accept_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        nonce_count: u32,
    ) -> Result<bool, CredentialAuthError> {
        let mut connection = self.connection().await.map_credential_error()?;
        let ttl = duration_secs(self.config.nonce_count_ttl)?;
        let accepted: i32 = redis::Script::new(
            r#"
            local current = redis.call("GET", KEYS[1])
            if current and tonumber(current) >= tonumber(ARGV[1]) then
                return 0
            end
            redis.call("SET", KEYS[1], ARGV[1], "EX", ARGV[2])
            return 1
            "#,
        )
        .key(self.nonce_count_key(username, nonce))
        .arg(nonce_count)
        .arg(ttl)
        .invoke_async(&mut connection)
        .await
        .map_credential_error()?;
        Ok(accepted == 1)
    }
}

#[async_trait]
impl TokenRevocationChecker for RedisAuthProvider {
    async fn check_token(
        &self,
        context: &TokenRevocationContext,
    ) -> Result<TokenRevocationStatus, CredentialAuthError> {
        let mut connection = self.connection().await.map_credential_error()?;
        let global_key = self.token_key(None, &context.token_id);
        let issuer_key = context
            .issuer
            .as_deref()
            .map(|issuer| self.token_key(Some(issuer), &context.token_id));
        let global_revoked: bool = connection.exists(global_key).await.map_credential_error()?;
        if global_revoked {
            return Ok(TokenRevocationStatus::Revoked);
        }
        if let Some(issuer_key) = issuer_key {
            let issuer_revoked: bool =
                connection.exists(issuer_key).await.map_credential_error()?;
            if issuer_revoked {
                return Ok(TokenRevocationStatus::Revoked);
            }
        }
        Ok(TokenRevocationStatus::Active)
    }
}

#[async_trait]
impl AuthRateLimiter for RedisAuthProvider {
    async fn check_auth_attempt(
        &self,
        key: &AuthRateLimitKey,
    ) -> Result<AuthRateLimitVerdict, CredentialAuthError> {
        if self.config.max_failures_per_window == 0 {
            return Ok(AuthRateLimitVerdict::Denied {
                retry_after: Some(self.config.rate_limit_window),
            });
        }

        let redis_key = self.rate_limit_key(key);
        let mut connection = self.connection().await.map_credential_error()?;
        let count: Option<u32> = connection.get(&redis_key).await.map_credential_error()?;
        if count.unwrap_or(0) < self.config.max_failures_per_window {
            return Ok(AuthRateLimitVerdict::Allowed);
        }
        let ttl_seconds: i64 = redis::cmd("TTL")
            .arg(&redis_key)
            .query_async(&mut connection)
            .await
            .map_credential_error()?;
        let retry_after = if ttl_seconds > 0 {
            Some(Duration::from_secs(ttl_seconds as u64))
        } else {
            Some(self.config.rate_limit_window)
        };
        Ok(AuthRateLimitVerdict::Denied { retry_after })
    }

    async fn record_auth_result(
        &self,
        key: &AuthRateLimitKey,
        outcome: &AuthAuditOutcome,
    ) -> Result<(), CredentialAuthError> {
        let redis_key = self.rate_limit_key(key);
        let mut connection = self.connection().await.map_credential_error()?;
        match outcome {
            AuthAuditOutcome::Success => {
                let _: () = connection.del(redis_key).await.map_credential_error()?;
            }
            AuthAuditOutcome::Failure(_) => {
                let ttl = duration_secs(self.config.rate_limit_window)?;
                let _: i32 = redis::Script::new(
                    r#"
                    local current = redis.call("INCR", KEYS[1])
                    if current == 1 then
                        redis.call("EXPIRE", KEYS[1], ARGV[1])
                    end
                    return current
                    "#,
                )
                .key(redis_key)
                .arg(ttl)
                .invoke_async(&mut connection)
                .await
                .map_credential_error()?;
            }
        }
        Ok(())
    }
}

trait CredentialRedisResult<T> {
    fn map_credential_error(self) -> Result<T, CredentialAuthError>;
}

impl<T> CredentialRedisResult<T> for Result<T, redis::RedisError> {
    fn map_credential_error(self) -> Result<T, CredentialAuthError> {
        self.map_err(|err| CredentialAuthError::Unavailable(err.to_string()))
    }
}

fn unix_seconds(time: SystemTime) -> Result<u64, CredentialAuthError> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|err| CredentialAuthError::PolicyRejected(err.to_string()))
}

fn ttl_from_expiry_or_default(
    expires_at: Option<SystemTime>,
    default_ttl: Duration,
) -> Result<u64, RedisAuthError> {
    let ttl = expires_at
        .and_then(|expiry| expiry.duration_since(SystemTime::now()).ok())
        .unwrap_or(default_ttl)
        .max(Duration::from_secs(1));
    duration_secs_redis(ttl)
}

fn duration_secs(duration: Duration) -> Result<u64, CredentialAuthError> {
    u64::try_from(duration.as_secs() as u128)
        .map_err(|_| CredentialAuthError::PolicyRejected("duration too large".to_string()))
}

fn duration_secs_redis(duration: Duration) -> Result<u64, RedisAuthError> {
    u64::try_from(duration.as_secs() as u128).map_err(|_| RedisAuthError::DurationTooLarge)
}

fn hex_key(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input.as_bytes() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
