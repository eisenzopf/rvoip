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

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use redis::aio::ConnectionLike;
use redis::{AsyncCommands, Cmd, RedisFuture, Value};
use rvoip_auth_core::{
    AuthAttemptAdmission, AuthAttemptReservation, AuthAuditOutcome, AuthRateLimitKey,
    AuthRateLimitVerdict, AuthRateLimiter, CredentialAuthError, DigestNonceStatus,
    DigestReplayStore, TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const RATE_LIMIT_RESERVE_SCRIPT: &str = r#"
local function cleanup(values, expiry, now)
    local expired = redis.call(
        "ZRANGEBYSCORE", expiry, "-inf", now, "LIMIT", 0, 256
    )
    for _, member in ipairs(expired) do
        redis.call("HDEL", values, member)
        redis.call("ZREM", expiry, member)
    end
end

local function remove_if_expired(values, expiry, member, now)
    local score = redis.call("ZSCORE", expiry, member)
    if score and tonumber(score) <= now then
        redis.call("HDEL", values, member)
        redis.call("ZREM", expiry, member)
    end
end

local function select_cohort(values, expiry, proposed, overflow, limit, expires, now)
    remove_if_expired(values, expiry, proposed, now)
    remove_if_expired(values, expiry, overflow, now)
    if redis.call("HEXISTS", values, proposed) == 1 then
        return proposed
    end

    local count = redis.call("HLEN", values)
    local selected = proposed
    if count >= limit - 1 then selected = overflow end
    if redis.call("HEXISTS", values, selected) == 0 then
        if redis.call("HLEN", values) >= limit then return false end
        redis.call("HSET", values, selected, 0)
        redis.call("ZADD", expiry, expires, selected)
    end
    return selected
end

cleanup(KEYS[1], KEYS[2], tonumber(ARGV[6]))
cleanup(KEYS[3], KEYS[4], tonumber(ARGV[6]))
cleanup(KEYS[5], KEYS[6], tonumber(ARGV[6]))

if redis.call("HEXISTS", KEYS[5], ARGV[5]) == 1 then return {-1, 0} end
if redis.call("HLEN", KEYS[5]) >= tonumber(ARGV[11]) then
    local oldest = redis.call("ZRANGE", KEYS[6], 0, 0, "WITHSCORES")
    local retry = tonumber(ARGV[7]) - tonumber(ARGV[6])
    if #oldest >= 2 then retry = tonumber(oldest[2]) - tonumber(ARGV[6]) end
    return {0, math.max(retry, 1)}
end

local peer = select_cohort(
    KEYS[1], KEYS[2], ARGV[1], ARGV[3], tonumber(ARGV[9]),
    tonumber(ARGV[7]), tonumber(ARGV[6])
)
local subject = select_cohort(
    KEYS[3], KEYS[4], ARGV[2], ARGV[4], tonumber(ARGV[10]),
    tonumber(ARGV[7]), tonumber(ARGV[6])
)
if not peer or not subject then
    return {0, math.max(tonumber(ARGV[7]) - tonumber(ARGV[6]), 1)}
end

local peer_count = tonumber(redis.call("HGET", KEYS[1], peer) or "0")
local subject_count = tonumber(redis.call("HGET", KEYS[3], subject) or "0")
if peer_count >= tonumber(ARGV[8]) or subject_count >= tonumber(ARGV[8]) then
    local retry_until = tonumber(ARGV[6]) + 1
    if peer_count >= tonumber(ARGV[8]) then
        retry_until = math.max(
            retry_until,
            tonumber(redis.call("ZSCORE", KEYS[2], peer) or ARGV[7])
        )
    end
    if subject_count >= tonumber(ARGV[8]) then
        retry_until = math.max(
            retry_until,
            tonumber(redis.call("ZSCORE", KEYS[4], subject) or ARGV[7])
        )
    end
    return {0, math.max(retry_until - tonumber(ARGV[6]), 1)}
end

redis.call("HINCRBY", KEYS[1], peer, 1)
redis.call("HINCRBY", KEYS[3], subject, 1)
local peer_expiry = tonumber(redis.call("ZSCORE", KEYS[2], peer))
local subject_expiry = tonumber(redis.call("ZSCORE", KEYS[4], subject))
local reservation_expiry = math.min(peer_expiry, subject_expiry)
redis.call("HSET", KEYS[5], ARGV[5], peer .. "|" .. subject)
redis.call("ZADD", KEYS[6], reservation_expiry, ARGV[5])

for index = 1, 6 do
    local current_ttl = redis.call("TTL", KEYS[index])
    if current_ttl == -1 or current_ttl < tonumber(ARGV[12]) then
        redis.call("EXPIRE", KEYS[index], tonumber(ARGV[12]))
    end
end
return {1, 0}
"#;

const RATE_LIMIT_COMPLETE_SCRIPT: &str = r#"
local value = redis.call("HGET", KEYS[5], ARGV[1])
local expires = redis.call("ZSCORE", KEYS[6], ARGV[1])
if not value or not expires then
    redis.call("HDEL", KEYS[5], ARGV[1])
    redis.call("ZREM", KEYS[6], ARGV[1])
    return 0
end
if tonumber(expires) <= tonumber(ARGV[3]) then
    redis.call("HDEL", KEYS[5], ARGV[1])
    redis.call("ZREM", KEYS[6], ARGV[1])
    return 0
end

redis.call("HDEL", KEYS[5], ARGV[1])
redis.call("ZREM", KEYS[6], ARGV[1])
if tonumber(ARGV[2]) ~= 1 then return 1 end

local _, _, peer, subject = string.find(value, "^([^|]+)|([^|]+)$")
if not peer then return -1 end

local peer_count = redis.call("HINCRBY", KEYS[1], peer, -1)
if peer_count <= 0 then
    redis.call("HDEL", KEYS[1], peer)
    redis.call("ZREM", KEYS[2], peer)
end
local subject_count = redis.call("HINCRBY", KEYS[3], subject, -1)
if subject_count <= 0 then
    redis.call("HDEL", KEYS[3], subject)
    redis.call("ZREM", KEYS[4], subject)
end
return 1
"#;

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

/// Redis deployment mode used by [`RedisAuthProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedisAuthConnectionMode {
    /// A single Redis server addressed by [`RedisAuthConfig::redis_url`].
    SingleNode,
    /// Redis Cluster discovered from one or more seed URLs.
    Cluster,
}

/// Certificate material for verified Redis TLS and optional mutual TLS.
///
/// PEM bytes are retained in memory so reconnecting single-node managers and
/// cluster topology connections use the same reviewed trust policy. Debug
/// output exposes only presence, never certificate or private-key bytes.
#[derive(Clone, Default)]
pub struct RedisAuthTlsConfig {
    root_certificate_pem: Option<Vec<u8>>,
    client_certificate_pem: Option<Vec<u8>>,
    client_private_key_pem: Option<Vec<u8>>,
}

impl fmt::Debug for RedisAuthTlsConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisAuthTlsConfig")
            .field(
                "custom_root_certificate",
                &self.root_certificate_pem.is_some(),
            )
            .field("client_identity", &self.client_certificate_pem.is_some())
            .finish()
    }
}

impl RedisAuthTlsConfig {
    /// Use native trust roots and no client certificate.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace native roots with a PEM-encoded trust anchor bundle.
    pub fn with_root_certificate_pem(mut self, certificate: impl Into<Vec<u8>>) -> Self {
        self.root_certificate_pem = Some(certificate.into());
        self
    }

    /// Present a PEM-encoded client certificate and private key for mTLS.
    pub fn with_client_identity_pem(
        mut self,
        certificate: impl Into<Vec<u8>>,
        private_key: impl Into<Vec<u8>>,
    ) -> Self {
        self.client_certificate_pem = Some(certificate.into());
        self.client_private_key_pem = Some(private_key.into());
        self
    }

    fn into_redis(self) -> Result<redis::TlsCertificates, RedisAuthError> {
        if self
            .root_certificate_pem
            .as_ref()
            .is_some_and(Vec::is_empty)
            || self
                .client_certificate_pem
                .as_ref()
                .is_some_and(Vec::is_empty)
            || self
                .client_private_key_pem
                .as_ref()
                .is_some_and(Vec::is_empty)
        {
            return Err(invalid_client_config(
                "Redis TLS certificate and key PEM values must not be empty",
            ));
        }
        let client_tls = match (self.client_certificate_pem, self.client_private_key_pem) {
            (Some(client_cert), Some(client_key)) => Some(redis::ClientTlsConfig {
                client_cert,
                client_key,
            }),
            (None, None) => None,
            _ => {
                return Err(invalid_client_config(
                    "Redis TLS client certificate and private key must be configured together",
                ))
            }
        };
        Ok(redis::TlsCertificates {
            client_tls,
            root_cert: self.root_certificate_pem,
        })
    }
}

/// Finite Redis connection, response, retry, and whole-command bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisAuthRuntimeConfig {
    /// Maximum time for one connection attempt.
    pub connection_timeout: Duration,
    /// Maximum time awaiting one Redis response.
    pub response_timeout: Duration,
    /// Maximum wall-clock time for connection acquisition plus one command.
    pub operation_timeout: Duration,
    /// Maximum reconnect or cluster retry count.
    pub retry_attempts: u32,
    /// Minimum retry delay.
    pub min_retry_wait: Duration,
    /// Maximum retry delay.
    pub max_retry_wait: Duration,
}

impl Default for RedisAuthRuntimeConfig {
    fn default() -> Self {
        Self {
            connection_timeout: Duration::from_secs(2),
            response_timeout: Duration::from_secs(2),
            operation_timeout: Duration::from_secs(5),
            retry_attempts: 3,
            min_retry_wait: Duration::from_millis(10),
            max_retry_wait: Duration::from_millis(250),
        }
    }
}

impl RedisAuthRuntimeConfig {
    fn validate(self) -> Result<Self, RedisAuthError> {
        if self.connection_timeout.is_zero()
            || self.response_timeout.is_zero()
            || self.operation_timeout.is_zero()
            || self.retry_attempts > 16
            || self.min_retry_wait.is_zero()
            || self.max_retry_wait.is_zero()
            || self.min_retry_wait > self.max_retry_wait
            || self.operation_timeout < self.connection_timeout
            || self.operation_timeout < self.response_timeout
        {
            return Err(invalid_client_config("invalid Redis auth runtime bounds"));
        }
        duration_millis_u64(self.min_retry_wait)?;
        duration_millis_u64(self.max_retry_wait)?;
        Ok(self)
    }
}

/// Bounded cardinality for atomic authentication-attempt state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisAuthRateLimitLimits {
    /// Maximum live peer aggregate cohorts in one namespace.
    pub peer_cohorts: usize,
    /// Maximum live peer/subject cohorts in one namespace.
    pub subject_cohorts: usize,
    /// Maximum incomplete reservations in one namespace.
    pub reservations: usize,
}

/// Aggregate-safe auth-attempt state counts for diagnostics and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisAuthRateLimitCardinality {
    /// Live peer aggregate cohorts.
    pub peer_cohorts: usize,
    /// Live subject cohorts.
    pub subject_cohorts: usize,
    /// Attempts reserved but not yet completed.
    pub reservations: usize,
}

impl Default for RedisAuthRateLimitLimits {
    fn default() -> Self {
        Self {
            peer_cohorts: 4_096,
            subject_cohorts: 16_384,
            reservations: 32_768,
        }
    }
}

impl RedisAuthRateLimitLimits {
    fn validate(self) -> Result<Self, CredentialAuthError> {
        if self.peer_cohorts == 0 || self.subject_cohorts == 0 || self.reservations == 0 {
            return Err(CredentialAuthError::PolicyRejected(
                "invalid auth rate-limit cardinality bounds".to_string(),
            ));
        }
        Ok(self)
    }
}

/// Configuration for Redis-backed auth provider state.
#[derive(Clone)]
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
    /// Minimum TTL for nonce-count replay records.
    ///
    /// The provider always extends this to cover the admitted nonce's full
    /// validity and stale-retention window, so a still-valid proof cannot
    /// become replayable when this value is configured too short.
    pub nonce_count_ttl: Duration,
    /// TTL used when revoking a token without a token expiry time.
    pub token_revocation_ttl: Duration,
    /// Fixed rate-limit window.
    pub rate_limit_window: Duration,
    /// Maximum failed attempts accepted in one rate-limit window.
    pub max_failures_per_window: u32,
}

impl fmt::Debug for RedisAuthConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisAuthConfig")
            .field("redis_url", &"<redacted>")
            .field("namespace", &self.namespace)
            .field("nonce_stale_retention", &self.nonce_stale_retention)
            .field("nonce_count_ttl", &self.nonce_count_ttl)
            .field("token_revocation_ttl", &self.token_revocation_ttl)
            .field("rate_limit_window", &self.rate_limit_window)
            .field("max_failures_per_window", &self.max_failures_per_window)
            .finish()
    }
}

/// Fair cardinality limits for one Redis Digest tenant namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisDigestReplayLimits {
    /// Maximum retained active-or-stale server nonces.
    pub retained_nonces: usize,
    /// Maximum retained client replay sequences in the namespace.
    pub client_sequences: usize,
    /// Maximum retained sequences owned by one Digest username.
    pub sequences_per_username: usize,
    /// Maximum retained sequences sharing one server nonce.
    pub sequences_per_nonce: usize,
    /// Maximum retained sequences for one username and server nonce.
    pub sequences_per_username_nonce: usize,
}

impl Default for RedisDigestReplayLimits {
    fn default() -> Self {
        Self {
            retained_nonces: 4_096,
            client_sequences: 16_384,
            sequences_per_username: 4_096,
            sequences_per_nonce: 8_192,
            sequences_per_username_nonce: 4_096,
        }
    }
}

impl RedisDigestReplayLimits {
    fn validate(self) -> Result<Self, CredentialAuthError> {
        if self.retained_nonces == 0
            || self.client_sequences == 0
            || self.sequences_per_username == 0
            || self.sequences_per_nonce == 0
            || self.sequences_per_username_nonce == 0
            || self.sequences_per_username > self.client_sequences
            || self.sequences_per_nonce > self.client_sequences
            || self.sequences_per_username_nonce > self.sequences_per_username
            || self.sequences_per_username_nonce > self.sequences_per_nonce
        {
            return Err(CredentialAuthError::PolicyRejected(
                "invalid Digest replay limits".to_string(),
            ));
        }
        Ok(self)
    }
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
#[derive(Clone)]
pub struct RedisAuthProvider {
    client: RedisAuthClient,
    config: RedisAuthConfig,
    runtime: RedisAuthRuntimeConfig,
    digest_limits: RedisDigestReplayLimits,
    rate_limit_limits: RedisAuthRateLimitLimits,
}

#[derive(Clone)]
enum RedisAuthClient {
    SingleNode {
        client: redis::Client,
        connection: std::sync::Arc<tokio::sync::OnceCell<redis::aio::ConnectionManager>>,
    },
    Cluster {
        client: redis::cluster::ClusterClient,
        connection: std::sync::Arc<tokio::sync::OnceCell<redis::cluster_async::ClusterConnection>>,
    },
}

enum RedisAuthConnection {
    SingleNode {
        connection: redis::aio::ConnectionManager,
        command_timeout: Duration,
    },
    Cluster {
        connection: redis::cluster_async::ClusterConnection,
        command_timeout: Duration,
    },
}

impl ConnectionLike for RedisAuthConnection {
    fn req_packed_command<'a>(&'a mut self, command: &'a Cmd) -> RedisFuture<'a, Value> {
        match self {
            Self::SingleNode {
                connection,
                command_timeout,
            } => bounded_redis_future(*command_timeout, connection.req_packed_command(command)),
            Self::Cluster {
                connection,
                command_timeout,
            } => bounded_redis_future(*command_timeout, connection.req_packed_command(command)),
        }
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        pipeline: &'a redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisFuture<'a, Vec<Value>> {
        match self {
            Self::SingleNode {
                connection,
                command_timeout,
            } => bounded_redis_future(
                *command_timeout,
                connection.req_packed_commands(pipeline, offset, count),
            ),
            Self::Cluster {
                connection,
                command_timeout,
            } => bounded_redis_future(
                *command_timeout,
                connection.req_packed_commands(pipeline, offset, count),
            ),
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            Self::SingleNode { connection, .. } => connection.get_db(),
            Self::Cluster { connection, .. } => connection.get_db(),
        }
    }
}

impl fmt::Debug for RedisAuthProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisAuthProvider")
            .field("connection_mode", &self.connection_mode())
            .field("namespace", &self.config.namespace)
            .field("nonce_stale_retention", &self.config.nonce_stale_retention)
            .field("nonce_count_ttl", &self.config.nonce_count_ttl)
            .field("token_revocation_ttl", &self.config.token_revocation_ttl)
            .field("rate_limit_window", &self.config.rate_limit_window)
            .field(
                "max_failures_per_window",
                &self.config.max_failures_per_window,
            )
            .field("digest_limits", &self.digest_limits)
            .field("runtime", &self.runtime)
            .field("rate_limit_limits", &self.rate_limit_limits)
            .finish()
    }
}

impl RedisAuthProvider {
    /// Create a Redis provider from a Redis URL and default configuration.
    pub fn new(redis_url: impl Into<String>) -> Result<Self, RedisAuthError> {
        Self::from_config(RedisAuthConfig::new(redis_url))
    }

    /// Create a Redis provider from explicit configuration.
    pub fn from_config(config: RedisAuthConfig) -> Result<Self, RedisAuthError> {
        Self::from_config_with_runtime(config, RedisAuthRuntimeConfig::default())
    }

    /// Create a single-node provider with explicit finite runtime bounds.
    pub fn from_config_with_runtime(
        config: RedisAuthConfig,
        runtime: RedisAuthRuntimeConfig,
    ) -> Result<Self, RedisAuthError> {
        Self::from_config_with_optional_tls(config, runtime, None)
    }

    /// Create a single-node provider with explicit verified TLS material.
    pub fn from_config_with_tls(
        config: RedisAuthConfig,
        tls: RedisAuthTlsConfig,
    ) -> Result<Self, RedisAuthError> {
        Self::from_config_with_runtime_and_tls(config, RedisAuthRuntimeConfig::default(), tls)
    }

    /// Create a single-node provider with explicit runtime and TLS settings.
    pub fn from_config_with_runtime_and_tls(
        config: RedisAuthConfig,
        runtime: RedisAuthRuntimeConfig,
        tls: RedisAuthTlsConfig,
    ) -> Result<Self, RedisAuthError> {
        Self::from_config_with_optional_tls(config, runtime, Some(tls))
    }

    fn from_config_with_optional_tls(
        config: RedisAuthConfig,
        runtime: RedisAuthRuntimeConfig,
        tls: Option<RedisAuthTlsConfig>,
    ) -> Result<Self, RedisAuthError> {
        validate_auth_config(&config)?;
        let runtime = runtime.validate()?;
        let client = match tls {
            Some(tls) => {
                redis::Client::build_with_tls(config.redis_url.as_str(), tls.into_redis()?)?
            }
            None => redis::Client::open(config.redis_url.as_str())?,
        };
        Ok(Self {
            client: RedisAuthClient::SingleNode {
                client,
                connection: std::sync::Arc::new(tokio::sync::OnceCell::new()),
            },
            config,
            runtime,
            digest_limits: RedisDigestReplayLimits::default(),
            rate_limit_limits: RedisAuthRateLimitLimits::default(),
        })
    }

    /// Create a Redis Cluster provider from seed URLs and default settings.
    ///
    /// The existing [`Self::new`] constructor remains the single-node path.
    /// Seed URLs must use compatible authentication and TLS settings, as
    /// required by `redis::cluster::ClusterClient`.
    pub fn new_cluster<I, S>(seed_urls: I) -> Result<Self, RedisAuthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let seed_urls = seed_urls.into_iter().map(Into::into).collect::<Vec<_>>();
        let first_seed = seed_urls
            .first()
            .cloned()
            .ok_or_else(|| invalid_client_config("Redis Cluster requires at least one seed URL"))?;
        Self::from_cluster_config(RedisAuthConfig::new(first_seed), seed_urls)
    }

    /// Create a Redis Cluster provider from explicit auth-state settings and
    /// seed URLs.
    ///
    /// `config.redis_url` is replaced with the first seed URL so callers that
    /// inspect the compatibility field do not observe an unrelated endpoint.
    pub fn from_cluster_config<I, S>(
        config: RedisAuthConfig,
        seed_urls: I,
    ) -> Result<Self, RedisAuthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::from_cluster_config_with_runtime(config, seed_urls, RedisAuthRuntimeConfig::default())
    }

    /// Create a Redis Cluster provider with explicit finite runtime bounds.
    pub fn from_cluster_config_with_runtime<I, S>(
        config: RedisAuthConfig,
        seed_urls: I,
        runtime: RedisAuthRuntimeConfig,
    ) -> Result<Self, RedisAuthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::from_cluster_config_with_optional_tls(config, seed_urls, runtime, None)
    }

    /// Create a Redis Cluster provider with explicit verified TLS material.
    pub fn from_cluster_config_with_tls<I, S>(
        config: RedisAuthConfig,
        seed_urls: I,
        tls: RedisAuthTlsConfig,
    ) -> Result<Self, RedisAuthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::from_cluster_config_with_runtime_and_tls(
            config,
            seed_urls,
            RedisAuthRuntimeConfig::default(),
            tls,
        )
    }

    /// Create a Redis Cluster provider with explicit runtime and TLS settings.
    pub fn from_cluster_config_with_runtime_and_tls<I, S>(
        config: RedisAuthConfig,
        seed_urls: I,
        runtime: RedisAuthRuntimeConfig,
        tls: RedisAuthTlsConfig,
    ) -> Result<Self, RedisAuthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::from_cluster_config_with_optional_tls(config, seed_urls, runtime, Some(tls))
    }

    fn from_cluster_config_with_optional_tls<I, S>(
        mut config: RedisAuthConfig,
        seed_urls: I,
        runtime: RedisAuthRuntimeConfig,
        tls: Option<RedisAuthTlsConfig>,
    ) -> Result<Self, RedisAuthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        validate_auth_config(&config)?;
        let runtime = runtime.validate()?;
        let seed_urls = seed_urls.into_iter().map(Into::into).collect::<Vec<_>>();
        let first_seed = seed_urls
            .first()
            .cloned()
            .ok_or_else(|| invalid_client_config("Redis Cluster requires at least one seed URL"))?;
        let mut builder = redis::cluster::ClusterClient::builder(seed_urls)
            .connection_timeout(runtime.connection_timeout)
            .response_timeout(runtime.response_timeout)
            .retries(runtime.retry_attempts)
            .min_retry_wait(duration_millis_u64(runtime.min_retry_wait)?)
            .max_retry_wait(duration_millis_u64(runtime.max_retry_wait)?);
        if let Some(tls) = tls {
            builder = builder.certs(tls.into_redis()?);
        }
        let client = builder.build()?;
        config.redis_url = first_seed;
        Ok(Self {
            client: RedisAuthClient::Cluster {
                client,
                connection: std::sync::Arc::new(tokio::sync::OnceCell::new()),
            },
            config,
            runtime,
            digest_limits: RedisDigestReplayLimits::default(),
            rate_limit_limits: RedisAuthRateLimitLimits::default(),
        })
    }

    /// Return this provider's configuration.
    pub fn config(&self) -> &RedisAuthConfig {
        &self.config
    }

    /// Return whether this provider connects to one server or Redis Cluster.
    pub fn connection_mode(&self) -> RedisAuthConnectionMode {
        match &self.client {
            RedisAuthClient::SingleNode { .. } => RedisAuthConnectionMode::SingleNode,
            RedisAuthClient::Cluster { .. } => RedisAuthConnectionMode::Cluster,
        }
    }

    /// Return the effective finite connection and command bounds.
    pub fn runtime_config(&self) -> RedisAuthRuntimeConfig {
        self.runtime
    }

    /// Apply explicit fair limits to this provider's Digest namespace.
    pub fn with_digest_replay_limits(
        mut self,
        limits: RedisDigestReplayLimits,
    ) -> Result<Self, CredentialAuthError> {
        self.digest_limits = limits.validate()?;
        Ok(self)
    }

    /// Return the effective Digest replay limits.
    pub fn digest_replay_limits(&self) -> RedisDigestReplayLimits {
        self.digest_limits
    }

    /// Apply explicit auth-attempt state cardinality bounds.
    pub fn with_auth_rate_limit_limits(
        mut self,
        limits: RedisAuthRateLimitLimits,
    ) -> Result<Self, CredentialAuthError> {
        self.rate_limit_limits = limits.validate()?;
        Ok(self)
    }

    /// Return the effective auth-attempt cardinality bounds.
    pub fn auth_rate_limit_limits(&self) -> RedisAuthRateLimitLimits {
        self.rate_limit_limits
    }

    /// Return aggregate-safe rate-limit state cardinality.
    pub async fn auth_rate_limit_cardinality(
        &self,
    ) -> Result<RedisAuthRateLimitCardinality, RedisAuthError> {
        let mut connection = self.connection().await?;
        let peer_cohorts = connection.hlen(self.rate_peer_values_key()).await?;
        let subject_cohorts = connection.hlen(self.rate_subject_values_key()).await?;
        let reservations = connection.hlen(self.rate_reservation_values_key()).await?;
        Ok(RedisAuthRateLimitCardinality {
            peer_cohorts,
            subject_cohorts,
            reservations,
        })
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
        if self.connection_mode() == RedisAuthConnectionMode::Cluster {
            return Err(invalid_client_config(
                "namespace cleanup is only supported in single-node Redis mode",
            ));
        }
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

    async fn connection(&self) -> Result<RedisAuthConnection, redis::RedisError> {
        let initialization = async {
            match &self.client {
                RedisAuthClient::SingleNode { client, connection } => {
                    let min_retry_wait =
                        u64::try_from(self.runtime.min_retry_wait.as_millis()).unwrap_or(u64::MAX);
                    let max_retry_wait =
                        u64::try_from(self.runtime.max_retry_wait.as_millis()).unwrap_or(u64::MAX);
                    let manager = connection
                        .get_or_try_init(|| async {
                            let config = redis::aio::ConnectionManagerConfig::new()
                                .set_number_of_retries(self.runtime.retry_attempts as usize)
                                .set_factor(min_retry_wait)
                                .set_max_delay(max_retry_wait)
                                .set_connection_timeout(self.runtime.connection_timeout)
                                .set_response_timeout(self.runtime.response_timeout);
                            redis::aio::ConnectionManager::new_with_config(client.clone(), config)
                                .await
                        })
                        .await?;
                    Ok(RedisAuthConnection::SingleNode {
                        connection: manager.clone(),
                        command_timeout: self.runtime.operation_timeout,
                    })
                }
                RedisAuthClient::Cluster { client, connection } => {
                    let connection = connection
                        .get_or_try_init(|| client.get_async_connection())
                        .await?;
                    Ok(RedisAuthConnection::Cluster {
                        connection: connection.clone(),
                        command_timeout: self.runtime.operation_timeout,
                    })
                }
            }
        };
        tokio::time::timeout(self.runtime.operation_timeout, initialization)
            .await
            .map_err(|_| redis_timeout_error("Redis connection initialization timed out"))?
    }

    fn digest_pool_prefix(&self) -> String {
        // The hash tag keeps every key used by an atomic Digest Lua script in
        // one Redis Cluster slot while retaining the configured namespace for
        // administration and fixture cleanup.
        format!(
            "{}:{{{}}}:digest",
            self.config.namespace,
            digest_key(&self.config.namespace)
        )
    }

    fn nonce_expiry_key(&self) -> String {
        format!("{}:nonce-expiry", self.digest_pool_prefix())
    }

    fn nonce_retention_key(&self) -> String {
        format!("{}:nonce-retention", self.digest_pool_prefix())
    }

    fn nonce_count_values_key(&self) -> String {
        format!("{}:nc-values", self.digest_pool_prefix())
    }

    fn nonce_count_retention_key(&self) -> String {
        format!("{}:nc-retention", self.digest_pool_prefix())
    }

    fn nonce_count_username_key(&self) -> String {
        format!("{}:nc-by-username", self.digest_pool_prefix())
    }

    fn nonce_count_nonce_key(&self) -> String {
        format!("{}:nc-by-nonce", self.digest_pool_prefix())
    }

    fn nonce_count_username_nonce_key(&self) -> String {
        format!("{}:nc-by-username-nonce", self.digest_pool_prefix())
    }

    fn token_key(&self, issuer: Option<&str>, token_id: &str) -> String {
        format!(
            "{}:token:revoked:{}:{}",
            self.config.namespace,
            issuer.map(hex_key).unwrap_or_else(|| "_".to_string()),
            hex_key(token_id)
        )
    }

    fn rate_pool_prefix(&self) -> String {
        format!(
            "{}:{{{}}}:rate",
            self.config.namespace,
            digest_key(&self.config.namespace)
        )
    }

    fn rate_peer_values_key(&self) -> String {
        format!("{}:peer-values", self.rate_pool_prefix())
    }

    fn rate_peer_expiry_key(&self) -> String {
        format!("{}:peer-expiry", self.rate_pool_prefix())
    }

    fn rate_subject_values_key(&self) -> String {
        format!("{}:subject-values", self.rate_pool_prefix())
    }

    fn rate_subject_expiry_key(&self) -> String {
        format!("{}:subject-expiry", self.rate_pool_prefix())
    }

    fn rate_reservation_values_key(&self) -> String {
        format!("{}:reservation-values", self.rate_pool_prefix())
    }

    fn rate_reservation_expiry_key(&self) -> String {
        format!("{}:reservation-expiry", self.rate_pool_prefix())
    }

    fn rate_limit_cohorts(&self, key: &AuthRateLimitKey) -> (String, String) {
        // The provider namespace is the trusted tenant boundary. `realm` is
        // supplied before credential validation and can be attacker-chosen,
        // so including it here would let realm rotation bypass both limits.
        let peer = format!(
            "kind={:?}|peer={}",
            key.kind,
            key.peer.as_deref().unwrap_or("_")
        );
        let subject = format!(
            "kind={:?}|subject={}",
            key.kind,
            key.subject.as_deref().unwrap_or("_")
        );
        (digest_key(&peer), digest_key(&subject))
    }
}

#[async_trait]
impl DigestReplayStore for RedisAuthProvider {
    async fn record_nonce(
        &self,
        nonce: &str,
        expires_at: SystemTime,
    ) -> Result<(), CredentialAuthError> {
        let admitted = self.admit_nonce(nonce, expires_at).await?;
        if admitted != nonce {
            return Err(CredentialAuthError::PolicyRejected(
                "legacy Digest nonce admission reached capacity".to_string(),
            ));
        }
        Ok(())
    }

    async fn admit_nonce(
        &self,
        proposed_nonce: &str,
        expires_at: SystemTime,
    ) -> Result<String, CredentialAuthError> {
        let now = unix_seconds(SystemTime::now())?;
        let expires_unix = unix_seconds(expires_at)?;
        let retain_until = expires_unix
            .checked_add(duration_secs(self.config.nonce_stale_retention)?)
            .ok_or_else(|| {
                CredentialAuthError::PolicyRejected(
                    "Digest nonce retention is too large".to_string(),
                )
            })?;
        if retain_until <= now {
            return Err(CredentialAuthError::PolicyRejected(
                "Digest nonce is outside stale retention".to_string(),
            ));
        }
        let pool_ttl = retain_until.saturating_sub(now).max(1);
        let mut connection = self.connection().await.map_credential_error()?;
        redis::Script::new(
            r#"
            local expired = redis.call("ZRANGEBYSCORE", KEYS[2], "-inf", ARGV[4])
            for _, member in ipairs(expired) do
                redis.call("ZREM", KEYS[1], member)
            end
            redis.call("ZREMRANGEBYSCORE", KEYS[2], "-inf", ARGV[4])

            if redis.call("ZSCORE", KEYS[1], ARGV[1]) then
                redis.call("ZADD", KEYS[1], ARGV[2], ARGV[1])
                redis.call("ZADD", KEYS[2], ARGV[3], ARGV[1])
            elseif redis.call("ZCARD", KEYS[2]) >= tonumber(ARGV[5]) then
                local best = redis.call(
                    "ZREVRANGEBYSCORE",
                    KEYS[1],
                    "+inf",
                    "(" .. ARGV[4],
                    "LIMIT",
                    0,
                    1
                )
                if #best > 0 then
                    return best[1]
                end

                local oldest = redis.call("ZRANGE", KEYS[2], 0, 0)
                if #oldest > 0 then
                    redis.call("ZREM", KEYS[1], oldest[1])
                    redis.call("ZREM", KEYS[2], oldest[1])
                end
                redis.call("ZADD", KEYS[1], ARGV[2], ARGV[1])
                redis.call("ZADD", KEYS[2], ARGV[3], ARGV[1])
            else
                redis.call("ZADD", KEYS[1], ARGV[2], ARGV[1])
                redis.call("ZADD", KEYS[2], ARGV[3], ARGV[1])
            end

            local function extend_ttl(key, ttl)
                local current = redis.call("TTL", key)
                if current == -1 or current < ttl then
                    redis.call("EXPIRE", key, ttl)
                end
            end
            extend_ttl(KEYS[1], tonumber(ARGV[6]))
            extend_ttl(KEYS[2], tonumber(ARGV[6]))
            return ARGV[1]
            "#,
        )
        .key(self.nonce_expiry_key())
        .key(self.nonce_retention_key())
        .arg(proposed_nonce)
        .arg(expires_unix)
        .arg(retain_until)
        .arg(now)
        .arg(self.digest_limits.retained_nonces)
        .arg(pool_ttl)
        .invoke_async(&mut connection)
        .await
        .map_credential_error()
    }

    async fn nonce_status(
        &self,
        nonce: &str,
        now: SystemTime,
    ) -> Result<DigestNonceStatus, CredentialAuthError> {
        let mut connection = self.connection().await.map_credential_error()?;
        let status: i32 = redis::Script::new(
            r#"
            local expiry = redis.call("ZSCORE", KEYS[1], ARGV[1])
            local retain_until = redis.call("ZSCORE", KEYS[2], ARGV[1])
            if not expiry or not retain_until then
                return -1
            end
            if tonumber(retain_until) <= tonumber(ARGV[2]) then
                redis.call("ZREM", KEYS[1], ARGV[1])
                redis.call("ZREM", KEYS[2], ARGV[1])
                return -1
            end
            if tonumber(expiry) > tonumber(ARGV[2]) then
                return 1
            end
            return 0
            "#,
        )
        .key(self.nonce_expiry_key())
        .key(self.nonce_retention_key())
        .arg(nonce)
        .arg(unix_seconds(now)?)
        .invoke_async(&mut connection)
        .await
        .map_credential_error()?;
        Ok(match status {
            1 => DigestNonceStatus::Active,
            0 => DigestNonceStatus::Expired,
            _ => DigestNonceStatus::Unknown,
        })
    }

    async fn accept_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        nonce_count: u32,
    ) -> Result<bool, CredentialAuthError> {
        self.accept_client_nonce_count(
            username,
            nonce,
            "legacy-client-sequence",
            nonce_count,
            SystemTime::now(),
        )
        .await
    }

    async fn accept_client_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        cnonce: &str,
        nonce_count: u32,
        now: SystemTime,
    ) -> Result<bool, CredentialAuthError> {
        let mut connection = self.connection().await.map_credential_error()?;
        let now = unix_seconds(now)?;
        let username_key = digest_key(username);
        let nonce_key = digest_key(nonce);
        let username_nonce_key = format!("{username_key}:{nonce_key}");
        let sequence_key = format!("{username_nonce_key}:{}", digest_key(cnonce));
        let minimum_ttl = duration_secs(self.config.nonce_count_ttl)?.max(1);
        let accepted: i32 = redis::Script::new(
            r#"
            local expired = redis.call("ZRANGEBYSCORE", KEYS[4], "-inf", ARGV[6])
            for _, sequence in ipairs(expired) do
                local value = redis.call("HGET", KEYS[3], sequence)
                if value then
                    local _, _, user, nonce, user_nonce = string.find(
                        value,
                        "^%d+|([^|]+)|([^|]+)|([^|]+)$"
                    )
                    if user then
                        local user_count = redis.call("HINCRBY", KEYS[5], user, -1)
                        if user_count <= 0 then redis.call("HDEL", KEYS[5], user) end
                        local nonce_count = redis.call("HINCRBY", KEYS[6], nonce, -1)
                        if nonce_count <= 0 then redis.call("HDEL", KEYS[6], nonce) end
                        local pair_count = redis.call("HINCRBY", KEYS[7], user_nonce, -1)
                        if pair_count <= 0 then redis.call("HDEL", KEYS[7], user_nonce) end
                    end
                    redis.call("HDEL", KEYS[3], sequence)
                end
            end
            redis.call("ZREMRANGEBYSCORE", KEYS[4], "-inf", ARGV[6])

            local nonce_expiry = redis.call("ZSCORE", KEYS[1], ARGV[2])
            local retain_until = redis.call("ZSCORE", KEYS[2], ARGV[2])
            if not nonce_expiry or not retain_until
                or tonumber(nonce_expiry) <= tonumber(ARGV[6])
                or tonumber(retain_until) <= tonumber(ARGV[6]) then
                return 0
            end

            local current = redis.call("HGET", KEYS[3], ARGV[1])
            if current then
                local current_count = tonumber(string.match(current, "^(%d+)|"))
                if current_count and current_count >= tonumber(ARGV[7]) then
                    return 0
                end
                redis.call(
                    "HSET",
                    KEYS[3],
                    ARGV[1],
                    ARGV[7] .. "|" .. ARGV[3] .. "|" .. ARGV[4] .. "|" .. ARGV[5]
                )
                redis.call("ZADD", KEYS[4], retain_until, ARGV[1])
                local ttl = math.max(
                    tonumber(ARGV[12]),
                    tonumber(retain_until) - tonumber(ARGV[6]),
                    1
                )
                for index = 3, 7 do
                    local current_ttl = redis.call("TTL", KEYS[index])
                    if current_ttl == -1 or current_ttl < ttl then
                        redis.call("EXPIRE", KEYS[index], ttl)
                    end
                end
                return 1
            end

            if redis.call("HLEN", KEYS[3]) >= tonumber(ARGV[8])
                or tonumber(redis.call("HGET", KEYS[5], ARGV[3]) or "0") >= tonumber(ARGV[9])
                or tonumber(redis.call("HGET", KEYS[6], ARGV[4]) or "0") >= tonumber(ARGV[10])
                or tonumber(redis.call("HGET", KEYS[7], ARGV[5]) or "0") >= tonumber(ARGV[11]) then
                return -1
            end

            redis.call(
                "HSET",
                KEYS[3],
                ARGV[1],
                ARGV[7] .. "|" .. ARGV[3] .. "|" .. ARGV[4] .. "|" .. ARGV[5]
            )
            redis.call("ZADD", KEYS[4], retain_until, ARGV[1])
            redis.call("HINCRBY", KEYS[5], ARGV[3], 1)
            redis.call("HINCRBY", KEYS[6], ARGV[4], 1)
            redis.call("HINCRBY", KEYS[7], ARGV[5], 1)

            local ttl = math.max(tonumber(ARGV[12]), tonumber(retain_until) - tonumber(ARGV[6]), 1)
            for index = 3, 7 do
                local current_ttl = redis.call("TTL", KEYS[index])
                if current_ttl == -1 or current_ttl < ttl then
                    redis.call("EXPIRE", KEYS[index], ttl)
                end
            end
            return 1
            "#,
        )
        .key(self.nonce_expiry_key())
        .key(self.nonce_retention_key())
        .key(self.nonce_count_values_key())
        .key(self.nonce_count_retention_key())
        .key(self.nonce_count_username_key())
        .key(self.nonce_count_nonce_key())
        .key(self.nonce_count_username_nonce_key())
        .arg(sequence_key)
        .arg(nonce)
        .arg(username_key)
        .arg(nonce_key)
        .arg(username_nonce_key)
        .arg(now)
        .arg(nonce_count)
        .arg(self.digest_limits.client_sequences)
        .arg(self.digest_limits.sequences_per_username)
        .arg(self.digest_limits.sequences_per_nonce)
        .arg(self.digest_limits.sequences_per_username_nonce)
        .arg(minimum_ttl)
        .invoke_async(&mut connection)
        .await
        .map_credential_error()?;
        match accepted {
            1 => Ok(true),
            0 => Ok(false),
            _ => Err(CredentialAuthError::PolicyRejected(
                "Digest replay capacity exhausted".to_string(),
            )),
        }
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
        _key: &AuthRateLimitKey,
    ) -> Result<AuthRateLimitVerdict, CredentialAuthError> {
        Err(CredentialAuthError::PolicyRejected(
            "atomic auth-attempt reservation is required".to_string(),
        ))
    }

    async fn record_auth_result(
        &self,
        _key: &AuthRateLimitKey,
        _outcome: &AuthAuditOutcome,
    ) -> Result<(), CredentialAuthError> {
        Err(CredentialAuthError::PolicyRejected(
            "atomic auth-attempt completion is required".to_string(),
        ))
    }

    async fn reserve_auth_attempt(
        &self,
        key: &AuthRateLimitKey,
    ) -> Result<AuthAttemptAdmission, CredentialAuthError> {
        if self.config.max_failures_per_window == 0 {
            return Ok(AuthAttemptAdmission::Denied {
                retry_after: Some(self.config.rate_limit_window),
            });
        }

        let now = unix_seconds(SystemTime::now())?;
        let window = duration_secs(self.config.rate_limit_window)?.max(1);
        let expires = now.checked_add(window).ok_or_else(|| {
            CredentialAuthError::PolicyRejected("auth rate-limit window is too large".to_string())
        })?;
        let reservation_id = uuid::Uuid::new_v4().simple().to_string();
        let (peer_cohort, subject_cohort) = self.rate_limit_cohorts(key);
        let mut connection = self.connection().await.map_credential_error()?;
        let (admitted, retry_after): (i32, u64) = redis::Script::new(RATE_LIMIT_RESERVE_SCRIPT)
            .key(self.rate_peer_values_key())
            .key(self.rate_peer_expiry_key())
            .key(self.rate_subject_values_key())
            .key(self.rate_subject_expiry_key())
            .key(self.rate_reservation_values_key())
            .key(self.rate_reservation_expiry_key())
            .arg(peer_cohort)
            .arg(subject_cohort)
            .arg("overflow-peer")
            .arg("overflow-subject")
            .arg(&reservation_id)
            .arg(now)
            .arg(expires)
            .arg(self.config.max_failures_per_window)
            .arg(self.rate_limit_limits.peer_cohorts)
            .arg(self.rate_limit_limits.subject_cohorts)
            .arg(self.rate_limit_limits.reservations)
            .arg(window.saturating_add(1))
            .invoke_async(&mut connection)
            .await
            .map_credential_error()?;

        match admitted {
            1 => Ok(AuthAttemptAdmission::Reserved(AuthAttemptReservation::new(
                reservation_id,
            )?)),
            0 => Ok(AuthAttemptAdmission::Denied {
                retry_after: Some(Duration::from_secs(retry_after.max(1))),
            }),
            _ => Err(CredentialAuthError::Unavailable(
                "Redis auth-attempt reservation collision".to_string(),
            )),
        }
    }

    async fn complete_auth_attempt(
        &self,
        reservation: &AuthAttemptReservation,
        outcome: &AuthAuditOutcome,
    ) -> Result<(), CredentialAuthError> {
        let now = unix_seconds(SystemTime::now())?;
        let mut connection = self.connection().await.map_credential_error()?;
        let success = i32::from(matches!(outcome, AuthAuditOutcome::Success));
        let completion: i32 = redis::Script::new(RATE_LIMIT_COMPLETE_SCRIPT)
            .key(self.rate_peer_values_key())
            .key(self.rate_peer_expiry_key())
            .key(self.rate_subject_values_key())
            .key(self.rate_subject_expiry_key())
            .key(self.rate_reservation_values_key())
            .key(self.rate_reservation_expiry_key())
            .arg(reservation.opaque_id())
            .arg(success)
            .arg(now)
            .invoke_async(&mut connection)
            .await
            .map_credential_error()?;
        if completion < 0 {
            return Err(CredentialAuthError::Unavailable(
                "Redis auth-attempt reservation state is invalid".to_string(),
            ));
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

fn validate_auth_config(config: &RedisAuthConfig) -> Result<(), RedisAuthError> {
    if config.namespace.is_empty()
        || config.namespace.trim() != config.namespace
        || config.namespace.chars().any(char::is_control)
        || config.rate_limit_window.is_zero()
    {
        return Err(invalid_client_config("invalid Redis auth configuration"));
    }
    duration_secs_redis(config.rate_limit_window)?;
    Ok(())
}

fn duration_millis_u64(duration: Duration) -> Result<u64, RedisAuthError> {
    u64::try_from(duration.as_millis()).map_err(|_| RedisAuthError::DurationTooLarge)
}

fn bounded_redis_future<'a, T: Send + 'a>(
    timeout: Duration,
    future: RedisFuture<'a, T>,
) -> RedisFuture<'a, T> {
    Box::pin(async move {
        tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| redis_timeout_error("Redis command timed out"))?
    })
}

fn redis_timeout_error(message: &'static str) -> redis::RedisError {
    redis::RedisError::from((redis::ErrorKind::IoError, message))
}

fn invalid_client_config(message: &'static str) -> RedisAuthError {
    RedisAuthError::Redis(redis::RedisError::from((
        redis::ErrorKind::InvalidClientConfig,
        message,
    )))
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

fn digest_key(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_constructors_remain_single_node() {
        let provider = RedisAuthProvider::new("redis://127.0.0.1:6379").unwrap();
        assert_eq!(
            provider.connection_mode(),
            RedisAuthConnectionMode::SingleNode
        );
        assert_eq!(provider.config().redis_url, "redis://127.0.0.1:6379");
        assert_eq!(provider.runtime_config(), RedisAuthRuntimeConfig::default());
    }

    #[test]
    fn rediss_single_and_cluster_preserve_tls_and_auth_seed_configuration() {
        let single =
            RedisAuthProvider::new("rediss://alice:secret@cache.example.test:6380").unwrap();
        assert_eq!(
            single.connection_mode(),
            RedisAuthConnectionMode::SingleNode
        );

        let cluster = RedisAuthProvider::new_cluster([
            "rediss://alice:secret@cache-a.example.test:6380",
            "rediss://alice:secret@cache-b.example.test:6380",
            "rediss://alice:secret@cache-c.example.test:6380",
        ])
        .unwrap();
        assert_eq!(cluster.connection_mode(), RedisAuthConnectionMode::Cluster);
        assert_eq!(
            cluster.config().redis_url,
            "rediss://alice:secret@cache-a.example.test:6380"
        );

        assert!(RedisAuthProvider::new_cluster([
            "rediss://alice:secret@cache-a.example.test:6380",
            "redis://alice:secret@cache-b.example.test:6379",
        ])
        .is_err());
        assert!(RedisAuthProvider::new_cluster([
            "rediss://alice:secret@cache-a.example.test:6380",
            "rediss://alice:different@cache-b.example.test:6380",
        ])
        .is_err());
    }

    #[test]
    fn explicit_tls_material_is_redacted_and_requires_a_tls_url() {
        let tls = RedisAuthTlsConfig::new()
            .with_root_certificate_pem(b"root-certificate-canary".to_vec())
            .with_client_identity_pem(
                b"client-certificate-canary".to_vec(),
                b"private-key-canary".to_vec(),
            );
        let debug = format!("{tls:?}");
        assert!(debug.contains("custom_root_certificate: true"));
        assert!(debug.contains("client_identity: true"));
        assert!(!debug.contains("certificate-canary"));
        assert!(!debug.contains("private-key-canary"));

        assert!(RedisAuthProvider::from_config_with_tls(
            RedisAuthConfig::new("redis://127.0.0.1:6379"),
            RedisAuthTlsConfig::new(),
        )
        .is_err());
        assert!(RedisAuthProvider::from_config_with_tls(
            RedisAuthConfig::new("rediss://127.0.0.1:6379"),
            RedisAuthTlsConfig::new().with_root_certificate_pem(Vec::new()),
        )
        .is_err());
    }

    #[test]
    fn runtime_and_auth_state_bounds_reject_invalid_configuration() {
        let config =
            RedisAuthConfig::new("redis://127.0.0.1:6379").with_rate_limit_window(Duration::ZERO);
        assert!(RedisAuthProvider::from_config(config).is_err());

        let runtime = RedisAuthRuntimeConfig {
            operation_timeout: Duration::ZERO,
            ..RedisAuthRuntimeConfig::default()
        };
        assert!(RedisAuthProvider::from_config_with_runtime(
            RedisAuthConfig::new("redis://127.0.0.1:6379"),
            runtime,
        )
        .is_err());

        let provider = RedisAuthProvider::new("redis://127.0.0.1:6379").unwrap();
        assert!(provider
            .with_auth_rate_limit_limits(RedisAuthRateLimitLimits {
                reservations: 0,
                ..RedisAuthRateLimitLimits::default()
            })
            .is_err());
    }

    #[tokio::test]
    async fn unreachable_redis_is_bounded_by_the_operation_deadline() {
        let runtime = RedisAuthRuntimeConfig {
            connection_timeout: Duration::from_millis(50),
            response_timeout: Duration::from_millis(50),
            operation_timeout: Duration::from_millis(100),
            retry_attempts: 0,
            min_retry_wait: Duration::from_millis(1),
            max_retry_wait: Duration::from_millis(1),
        };
        let provider = RedisAuthProvider::from_config_with_runtime(
            RedisAuthConfig::new("redis://192.0.2.1:6379"),
            runtime,
        )
        .unwrap();
        let started = std::time::Instant::now();
        assert!(provider.revoke_token_id("unreachable", None).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn cluster_construction_is_explicit_and_preserves_auth_settings() {
        let provider = RedisAuthProvider::from_cluster_config(
            RedisAuthConfig::new("redis://ignored.invalid:6379").with_namespace("tenant-a"),
            [
                "redis://127.0.0.1:7000",
                "redis://127.0.0.1:7001",
                "redis://127.0.0.1:7002",
            ],
        )
        .unwrap();
        assert_eq!(provider.connection_mode(), RedisAuthConnectionMode::Cluster);
        assert_eq!(provider.config().redis_url, "redis://127.0.0.1:7000");
        assert_eq!(provider.config().namespace, "tenant-a");
    }

    #[test]
    fn cluster_construction_rejects_an_empty_seed_set() {
        let result = RedisAuthProvider::new_cluster(Vec::<String>::new());
        assert!(matches!(result, Err(RedisAuthError::Redis(_))));
    }

    #[test]
    fn provider_debug_redacts_redis_credentials() {
        let config = RedisAuthConfig::new("redis://alice:secret@example.invalid:6379");
        let config_debug = format!("{config:?}");
        assert!(!config_debug.contains("alice"));
        assert!(!config_debug.contains("secret"));
        assert!(!config_debug.contains("example.invalid"));

        let provider = RedisAuthProvider::from_config(config).unwrap();
        let debug = format!("{provider:?}");
        assert!(!debug.contains("alice"));
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("example.invalid"));
    }

    #[test]
    fn digest_limit_validation_rejects_zero_and_inverted_limits() {
        let provider = RedisAuthProvider::new("redis://127.0.0.1:6379").unwrap();
        assert!(provider
            .clone()
            .with_digest_replay_limits(RedisDigestReplayLimits {
                retained_nonces: 0,
                ..RedisDigestReplayLimits::default()
            })
            .is_err());
        assert!(provider
            .with_digest_replay_limits(RedisDigestReplayLimits {
                client_sequences: 4,
                sequences_per_username: 5,
                ..RedisDigestReplayLimits::default()
            })
            .is_err());
    }

    #[test]
    fn digest_script_keys_share_one_cluster_hash_tag() {
        let provider = RedisAuthProvider::from_config(
            RedisAuthConfig::new("redis://127.0.0.1:6379").with_namespace("tenant-a"),
        )
        .unwrap();
        let keys = [
            provider.nonce_expiry_key(),
            provider.nonce_retention_key(),
            provider.nonce_count_values_key(),
            provider.nonce_count_retention_key(),
            provider.nonce_count_username_key(),
            provider.nonce_count_nonce_key(),
            provider.nonce_count_username_nonce_key(),
        ];
        let expected_tag = format!("{{{}}}", digest_key("tenant-a"));
        assert!(keys.iter().all(|key| key.contains(&expected_tag)));
        assert!(keys.iter().all(|key| key.starts_with("tenant-a:")));
    }

    #[test]
    fn rate_limit_script_keys_share_one_cluster_hash_tag() {
        let provider = RedisAuthProvider::from_config(
            RedisAuthConfig::new("redis://127.0.0.1:6379").with_namespace("tenant-a"),
        )
        .unwrap();
        let keys = [
            provider.rate_peer_values_key(),
            provider.rate_peer_expiry_key(),
            provider.rate_subject_values_key(),
            provider.rate_subject_expiry_key(),
            provider.rate_reservation_values_key(),
            provider.rate_reservation_expiry_key(),
        ];
        let expected_tag = format!("{{{}}}", digest_key("tenant-a"));
        assert!(keys.iter().all(|key| key.contains(&expected_tag)));
        assert!(keys.iter().all(|key| key.starts_with("tenant-a:")));
    }
}
