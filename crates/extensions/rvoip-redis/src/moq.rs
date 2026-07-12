//! Redis-backed durable MOQT admission leases.
//!
//! Redis atomically enforces the configured tenant-scoped active-session
//! quota. A relay separately owns its process-global active-session permits;
//! this store deliberately does not approximate a cross-tenant global limit.
//! All keys touched by one admission operation share a tenant hash tag, so the
//! Lua operations are also valid when a cluster-aware Redis client is added.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_moq::{
    MoqSessionLease, MoqSessionLeaseBinding, MoqSessionLeaseClose, MoqSessionLeaseError,
    MoqSessionLeaseLimits, MoqSessionLeaseSnapshot, MoqSessionLeaseStore,
};

const DEFAULT_NAMESPACE: &str = "rvoip:moq";
const DEFAULT_TENANT_LIMIT: usize = 1_000;
const MAX_NAMESPACE_BYTES: usize = 128;
const BACKEND_UNAVAILABLE: &str = "redis MOQT session lease operation failed";

const RESULT_OK: i64 = 0;
const RESULT_IDEMPOTENT: i64 = 1;
const RESULT_CROSS_SESSION_REPLAY: i64 = 2;
const RESULT_OWNER_MISMATCH: i64 = 3;
const RESULT_BINDING_MISMATCH: i64 = 4;
const RESULT_CLOSED: i64 = 5;
const RESULT_TENANT_QUOTA: i64 = 6;
const RESULT_INCONSISTENT: i64 = 7;
const RESULT_EXPIRED: i64 = 8;
const RESULT_NOT_FOUND: i64 = 9;

const ACQUIRE_SCRIPT: &str = r#"
local now_ms = tonumber(ARGV[1])
local expires_ms = tonumber(ARGV[2])
local tenant_limit = tonumber(ARGV[3])

redis.call("ZREMRANGEBYSCORE", KEYS[3], "-inf", now_ms)

local token_session = redis.call("HGET", KEYS[2], "session")
if token_session and token_session ~= ARGV[4] then
    return 2
end

local session_state = redis.call("HGET", KEYS[1], "state")
if session_state or token_session then
    if not session_state or not token_session then
        return 7
    end
    if redis.call("HGET", KEYS[2], "state") ~= session_state then
        return 7
    end
    if redis.call("HGET", KEYS[1], "issuer") ~= ARGV[6]
        or redis.call("HGET", KEYS[1], "tenant") ~= ARGV[7]
        or redis.call("HGET", KEYS[1], "subject") ~= ARGV[8]
        or redis.call("HGET", KEYS[2], "issuer") ~= ARGV[6]
        or redis.call("HGET", KEYS[2], "tenant") ~= ARGV[7]
        or redis.call("HGET", KEYS[2], "subject") ~= ARGV[8] then
        return 3
    end
    if redis.call("HGET", KEYS[1], "session") ~= ARGV[4]
        or redis.call("HGET", KEYS[1], "fingerprint") ~= ARGV[5]
        or redis.call("HGET", KEYS[1], "token_id") ~= ARGV[9]
        or redis.call("HGET", KEYS[1], "namespace") ~= ARGV[10]
        or redis.call("HGET", KEYS[1], "scope") ~= ARGV[11]
        or redis.call("HGET", KEYS[1], "expires_ms") ~= ARGV[2]
        or redis.call("HGET", KEYS[2], "fingerprint") ~= ARGV[5]
        or redis.call("HGET", KEYS[2], "token_id") ~= ARGV[9]
        or redis.call("HGET", KEYS[2], "namespace") ~= ARGV[10]
        or redis.call("HGET", KEYS[2], "scope") ~= ARGV[11]
        or redis.call("HGET", KEYS[2], "expires_ms") ~= ARGV[2] then
        return 4
    end
    if session_state == "closed" then
        return 5
    end
    if expires_ms <= now_ms then
        return 8
    end
    if not redis.call("ZSCORE", KEYS[3], ARGV[4]) then
        return 7
    end
    return 1
end

if expires_ms <= now_ms then
    return 8
end
if redis.call("ZCARD", KEYS[3]) >= tenant_limit then
    return 6
end

redis.call("HSET", KEYS[1],
    "state", "active",
    "session", ARGV[4],
    "fingerprint", ARGV[5],
    "issuer", ARGV[6],
    "tenant", ARGV[7],
    "subject", ARGV[8],
    "token_id", ARGV[9],
    "namespace", ARGV[10],
    "scope", ARGV[11],
    "expires_ms", ARGV[2])
redis.call("HSET", KEYS[2],
    "state", "active",
    "session", ARGV[4],
    "fingerprint", ARGV[5],
    "issuer", ARGV[6],
    "tenant", ARGV[7],
    "subject", ARGV[8],
    "token_id", ARGV[9],
    "namespace", ARGV[10],
    "scope", ARGV[11],
    "expires_ms", ARGV[2])
redis.call("PEXPIREAT", KEYS[1], expires_ms)
redis.call("PEXPIREAT", KEYS[2], expires_ms)
redis.call("ZADD", KEYS[3], expires_ms, ARGV[4])
local maximum = redis.call("ZREVRANGE", KEYS[3], 0, 0, "WITHSCORES")
if maximum[2] then
    redis.call("PEXPIREAT", KEYS[3], maximum[2])
end
return 0
"#;

const VERIFY_SCRIPT: &str = r#"
local now_ms = tonumber(ARGV[1])
local expires_ms = tonumber(ARGV[2])
redis.call("ZREMRANGEBYSCORE", KEYS[3], "-inf", now_ms)

if expires_ms <= now_ms then
    return 8
end
local session_state = redis.call("HGET", KEYS[1], "state")
local token_session = redis.call("HGET", KEYS[2], "session")
if not session_state and not token_session then
    return 9
end
if not session_state or not token_session then
    return 7
end
if token_session ~= ARGV[3] then
    return 2
end
if redis.call("HGET", KEYS[2], "state") ~= session_state then
    return 7
end
if redis.call("HGET", KEYS[1], "issuer") ~= ARGV[5]
    or redis.call("HGET", KEYS[1], "tenant") ~= ARGV[6]
    or redis.call("HGET", KEYS[1], "subject") ~= ARGV[7]
    or redis.call("HGET", KEYS[2], "issuer") ~= ARGV[5]
    or redis.call("HGET", KEYS[2], "tenant") ~= ARGV[6]
    or redis.call("HGET", KEYS[2], "subject") ~= ARGV[7] then
    return 3
end
if redis.call("HGET", KEYS[1], "session") ~= ARGV[3]
    or redis.call("HGET", KEYS[1], "fingerprint") ~= ARGV[4]
    or redis.call("HGET", KEYS[1], "token_id") ~= ARGV[8]
    or redis.call("HGET", KEYS[1], "namespace") ~= ARGV[9]
    or redis.call("HGET", KEYS[1], "scope") ~= ARGV[10]
    or redis.call("HGET", KEYS[1], "expires_ms") ~= ARGV[2]
    or redis.call("HGET", KEYS[2], "fingerprint") ~= ARGV[4]
    or redis.call("HGET", KEYS[2], "token_id") ~= ARGV[8]
    or redis.call("HGET", KEYS[2], "namespace") ~= ARGV[9]
    or redis.call("HGET", KEYS[2], "scope") ~= ARGV[10]
    or redis.call("HGET", KEYS[2], "expires_ms") ~= ARGV[2] then
    return 4
end
if session_state == "closed" then
    return 5
end
if not redis.call("ZSCORE", KEYS[3], ARGV[3]) then
    return 7
end
return 0
"#;

const CLOSE_SCRIPT: &str = r#"
local now_ms = tonumber(ARGV[1])
local expires_ms = tonumber(ARGV[2])
redis.call("ZREMRANGEBYSCORE", KEYS[3], "-inf", now_ms)

local session_state = redis.call("HGET", KEYS[1], "state")
local token_session = redis.call("HGET", KEYS[2], "session")
if token_session and token_session ~= ARGV[3] then
    return 2
end
if session_state or token_session then
    if not session_state or not token_session then
        return 7
    end
    if redis.call("HGET", KEYS[2], "state") ~= session_state then
        return 7
    end
    if redis.call("HGET", KEYS[1], "issuer") ~= ARGV[5]
        or redis.call("HGET", KEYS[1], "tenant") ~= ARGV[6]
        or redis.call("HGET", KEYS[1], "subject") ~= ARGV[7]
        or redis.call("HGET", KEYS[2], "issuer") ~= ARGV[5]
        or redis.call("HGET", KEYS[2], "tenant") ~= ARGV[6]
        or redis.call("HGET", KEYS[2], "subject") ~= ARGV[7] then
        return 3
    end
    if redis.call("HGET", KEYS[1], "session") ~= ARGV[3]
        or redis.call("HGET", KEYS[1], "fingerprint") ~= ARGV[4]
        or redis.call("HGET", KEYS[1], "token_id") ~= ARGV[8]
        or redis.call("HGET", KEYS[1], "namespace") ~= ARGV[9]
        or redis.call("HGET", KEYS[1], "scope") ~= ARGV[10]
        or redis.call("HGET", KEYS[1], "expires_ms") ~= ARGV[2]
        or redis.call("HGET", KEYS[2], "fingerprint") ~= ARGV[4]
        or redis.call("HGET", KEYS[2], "token_id") ~= ARGV[8]
        or redis.call("HGET", KEYS[2], "namespace") ~= ARGV[9]
        or redis.call("HGET", KEYS[2], "scope") ~= ARGV[10]
        or redis.call("HGET", KEYS[2], "expires_ms") ~= ARGV[2] then
        return 4
    end
else
    redis.call("HSET", KEYS[1],
        "state", "closed",
        "session", ARGV[3],
        "fingerprint", ARGV[4],
        "issuer", ARGV[5],
        "tenant", ARGV[6],
        "subject", ARGV[7],
        "token_id", ARGV[8],
        "namespace", ARGV[9],
        "scope", ARGV[10],
        "expires_ms", ARGV[2],
        "close_reason", ARGV[11])
    redis.call("HSET", KEYS[2],
        "state", "closed",
        "session", ARGV[3],
        "fingerprint", ARGV[4],
        "issuer", ARGV[5],
        "tenant", ARGV[6],
        "subject", ARGV[7],
        "token_id", ARGV[8],
        "namespace", ARGV[9],
        "scope", ARGV[10],
        "expires_ms", ARGV[2],
        "close_reason", ARGV[11])
end

redis.call("HSET", KEYS[1], "state", "closed", "close_reason", ARGV[11])
redis.call("HSET", KEYS[2], "state", "closed", "close_reason", ARGV[11])
redis.call("PEXPIREAT", KEYS[1], expires_ms)
redis.call("PEXPIREAT", KEYS[2], expires_ms)
redis.call("ZREM", KEYS[3], ARGV[3])
local maximum = redis.call("ZREVRANGE", KEYS[3], 0, 0, "WITHSCORES")
if maximum[2] then
    redis.call("PEXPIREAT", KEYS[3], maximum[2])
else
    redis.call("DEL", KEYS[3])
end
return 0
"#;

const SNAPSHOT_ACTIVE_SCRIPT: &str = r#"
redis.call("ZREMRANGEBYSCORE", KEYS[1], "-inf", ARGV[1])
local count = redis.call("ZCARD", KEYS[1])
if count == 0 then
    redis.call("DEL", KEYS[1])
end
return count
"#;

/// Configuration for Redis-backed MOQT admission leases.
#[derive(Clone, Eq, PartialEq)]
pub struct RedisMoqSessionLeaseConfig {
    /// Redis connection URL. It is always redacted from `Debug` output.
    pub redis_url: String,
    /// Logical key namespace shared by all relay instances in one deployment.
    pub namespace: String,
    /// Atomic active-session quota applied independently to each tenant.
    pub max_active_sessions_per_tenant: usize,
}

impl RedisMoqSessionLeaseConfig {
    pub fn new(redis_url: impl Into<String>) -> Self {
        Self {
            redis_url: redis_url.into(),
            namespace: DEFAULT_NAMESPACE.to_owned(),
            max_active_sessions_per_tenant: DEFAULT_TENANT_LIMIT,
        }
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    pub const fn with_max_active_sessions_per_tenant(mut self, maximum: usize) -> Self {
        self.max_active_sessions_per_tenant = maximum;
        self
    }
}

impl std::fmt::Debug for RedisMoqSessionLeaseConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedisMoqSessionLeaseConfig")
            .field("redis_url", &"<redacted>")
            .field("namespace", &self.namespace)
            .field(
                "max_active_sessions_per_tenant",
                &self.max_active_sessions_per_tenant,
            )
            .finish()
    }
}

/// Redis-backed durable lease store for production MOQT admission.
#[derive(Clone)]
pub struct RedisMoqSessionLeaseStore {
    client: redis::Client,
    config: RedisMoqSessionLeaseConfig,
    namespace_hex: String,
    limits: MoqSessionLeaseLimits,
}

impl std::fmt::Debug for RedisMoqSessionLeaseStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedisMoqSessionLeaseStore")
            .field("config", &self.config)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl RedisMoqSessionLeaseStore {
    pub fn new(redis_url: impl Into<String>) -> Result<Self, MoqSessionLeaseError> {
        Self::from_config(RedisMoqSessionLeaseConfig::new(redis_url))
    }

    pub fn from_config(config: RedisMoqSessionLeaseConfig) -> Result<Self, MoqSessionLeaseError> {
        validate_config(&config)?;
        let client = redis::Client::open(config.redis_url.as_str()).map_err(backend_error)?;
        let limits = MoqSessionLeaseLimits::tenant_scoped(config.max_active_sessions_per_tenant)?;
        let namespace_hex = hex_bytes(config.namespace.as_bytes());
        Ok(Self {
            client,
            config,
            namespace_hex,
            limits,
        })
    }

    pub const fn config(&self) -> &RedisMoqSessionLeaseConfig {
        &self.config
    }

    pub const fn limits(&self) -> MoqSessionLeaseLimits {
        self.limits
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, MoqSessionLeaseError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(backend_error)
    }

    fn keys(&self, binding: &MoqSessionLeaseBinding) -> LeaseKeys {
        let tenant_hex = hex_bytes(binding.namespace().tenant_id().as_bytes());
        let prefix = format!("{{rvoip-moq:{tenant_hex}}}:{}:lease", self.namespace_hex);
        LeaseKeys {
            session: format!(
                "{prefix}:session:{}",
                hex_bytes(binding.session_id().as_str().as_bytes())
            ),
            token: format!(
                "{prefix}:token:{}",
                hex_bytes(&binding.credential_fingerprint_sha256())
            ),
            active: format!("{prefix}:active"),
        }
    }

    fn scan_pattern(&self, suffix: &str) -> String {
        format!("{{rvoip-moq:*}}:{}:lease:{suffix}", self.namespace_hex)
    }

    async fn scan_keys(&self, pattern: &str) -> Result<HashSet<String>, MoqSessionLeaseError> {
        let mut connection = self.connection().await?;
        let mut cursor = 0_u64;
        let mut keys = HashSet::new();
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(256_u16)
                .query_async(&mut connection)
                .await
                .map_err(backend_error)?;
            keys.extend(batch);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        Ok(keys)
    }

    #[cfg(test)]
    fn key_names_for_test(&self, binding: &MoqSessionLeaseBinding) -> LeaseKeys {
        self.keys(binding)
    }
}

#[async_trait]
impl MoqSessionLeaseStore for RedisMoqSessionLeaseStore {
    async fn acquire(
        &self,
        binding: &MoqSessionLeaseBinding,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLease, MoqSessionLeaseError> {
        if binding.expires_at() <= now {
            return Err(MoqSessionLeaseError::Expired);
        }
        let keys = self.keys(binding);
        let mut connection = self.connection().await?;
        let result: i64 = redis::Script::new(ACQUIRE_SCRIPT)
            .key(&keys.session)
            .key(&keys.token)
            .key(&keys.active)
            .arg(now.timestamp_millis())
            .arg(binding.expires_at().timestamp_millis())
            .arg(self.config.max_active_sessions_per_tenant)
            .arg(binding.session_id().as_str())
            .arg(hex_bytes(&binding.credential_fingerprint_sha256()))
            .arg(required_issuer(binding)?)
            .arg(required_tenant(binding)?)
            .arg(&binding.owner().subject)
            .arg(binding.token_id())
            .arg(binding.namespace().as_str())
            .arg(binding.canonical_scope())
            .invoke_async(&mut connection)
            .await
            .map_err(backend_error)?;
        match_result(result)?;
        Ok(MoqSessionLease::from_binding(binding.clone()))
    }

    async fn verify(
        &self,
        lease: &MoqSessionLease,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError> {
        let binding = lease.binding();
        if binding.expires_at() <= now {
            return Err(MoqSessionLeaseError::Expired);
        }
        let keys = self.keys(binding);
        let mut connection = self.connection().await?;
        let result: i64 = redis::Script::new(VERIFY_SCRIPT)
            .key(&keys.session)
            .key(&keys.token)
            .key(&keys.active)
            .arg(now.timestamp_millis())
            .arg(binding.expires_at().timestamp_millis())
            .arg(binding.session_id().as_str())
            .arg(hex_bytes(&binding.credential_fingerprint_sha256()))
            .arg(required_issuer(binding)?)
            .arg(required_tenant(binding)?)
            .arg(&binding.owner().subject)
            .arg(binding.token_id())
            .arg(binding.namespace().as_str())
            .arg(binding.canonical_scope())
            .invoke_async(&mut connection)
            .await
            .map_err(backend_error)?;
        match_result(result)
    }

    async fn close(
        &self,
        lease: &MoqSessionLease,
        close: MoqSessionLeaseClose,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError> {
        let binding = lease.binding();
        if binding.expires_at() <= now {
            return Ok(());
        }
        let keys = self.keys(binding);
        let mut connection = self.connection().await?;
        let result: i64 = redis::Script::new(CLOSE_SCRIPT)
            .key(&keys.session)
            .key(&keys.token)
            .key(&keys.active)
            .arg(now.timestamp_millis())
            .arg(binding.expires_at().timestamp_millis())
            .arg(binding.session_id().as_str())
            .arg(hex_bytes(&binding.credential_fingerprint_sha256()))
            .arg(required_issuer(binding)?)
            .arg(required_tenant(binding)?)
            .arg(&binding.owner().subject)
            .arg(binding.token_id())
            .arg(binding.namespace().as_str())
            .arg(binding.canonical_scope())
            .arg(close_reason(close))
            .invoke_async(&mut connection)
            .await
            .map_err(backend_error)?;
        match_result(result)
    }

    async fn snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLeaseSnapshot, MoqSessionLeaseError> {
        // SCAN is intentionally diagnostic and non-transactional. Admission,
        // verification, close, replay, and quota decisions remain atomic Lua.
        let session_keys = self.scan_keys(&self.scan_pattern("session:*")).await?;
        let token_keys = self.scan_keys(&self.scan_pattern("token:*")).await?;
        let active_keys = self.scan_keys(&self.scan_pattern("active")).await?;
        let tenant_buckets = session_keys
            .iter()
            .filter_map(|key| key.split_once('}').map(|(tag, _)| tag.to_owned()))
            .collect::<HashSet<_>>()
            .len();

        let mut active_sessions = 0_usize;
        let mut connection = self.connection().await?;
        for key in active_keys {
            let count: usize = redis::Script::new(SNAPSHOT_ACTIVE_SCRIPT)
                .key(key)
                .arg(now.timestamp_millis())
                .invoke_async(&mut connection)
                .await
                .map_err(backend_error)?;
            active_sessions = active_sessions.saturating_add(count);
        }

        Ok(MoqSessionLeaseSnapshot {
            retained_sessions: session_keys.len(),
            retained_tokens: token_keys.len(),
            active_sessions,
            tenant_buckets,
            limits: self.limits,
        })
    }
}

#[derive(Debug)]
struct LeaseKeys {
    session: String,
    token: String,
    active: String,
}

fn validate_config(config: &RedisMoqSessionLeaseConfig) -> Result<(), MoqSessionLeaseError> {
    if config.namespace.is_empty()
        || config.namespace.len() > MAX_NAMESPACE_BYTES
        || config.namespace.chars().any(char::is_control)
    {
        return Err(MoqSessionLeaseError::InvalidConfig(
            "Redis namespace must contain 1 to 128 non-control bytes",
        ));
    }
    if config.max_active_sessions_per_tenant == 0 {
        return Err(MoqSessionLeaseError::InvalidConfig(
            "Redis tenant session limit must be greater than zero",
        ));
    }
    Ok(())
}

fn required_issuer(binding: &MoqSessionLeaseBinding) -> Result<&str, MoqSessionLeaseError> {
    binding
        .owner()
        .issuer
        .as_deref()
        .ok_or(MoqSessionLeaseError::InvalidBinding(
            "principal issuer is missing",
        ))
}

fn required_tenant(binding: &MoqSessionLeaseBinding) -> Result<&str, MoqSessionLeaseError> {
    binding
        .owner()
        .tenant
        .as_deref()
        .ok_or(MoqSessionLeaseError::InvalidBinding(
            "principal tenant is missing",
        ))
}

fn close_reason(close: MoqSessionLeaseClose) -> &'static str {
    match close {
        MoqSessionLeaseClose::PeerClosed => "peer_closed",
        MoqSessionLeaseClose::LocalClosed => "local_closed",
        MoqSessionLeaseClose::ActivationFailed => "activation_failed",
        MoqSessionLeaseClose::AdmissionRevalidationFailed => "revalidation_failed",
        MoqSessionLeaseClose::ProtocolError => "protocol_error",
        MoqSessionLeaseClose::RelayShutdown => "relay_shutdown",
    }
}

fn match_result(result: i64) -> Result<(), MoqSessionLeaseError> {
    match result {
        RESULT_OK | RESULT_IDEMPOTENT => Ok(()),
        RESULT_CROSS_SESSION_REPLAY => Err(MoqSessionLeaseError::CrossSessionReplay),
        RESULT_OWNER_MISMATCH => Err(MoqSessionLeaseError::OwnerMismatch),
        RESULT_BINDING_MISMATCH => Err(MoqSessionLeaseError::BindingMismatch),
        RESULT_CLOSED => Err(MoqSessionLeaseError::Closed),
        RESULT_TENANT_QUOTA => Err(MoqSessionLeaseError::TenantQuotaExceeded),
        RESULT_EXPIRED => Err(MoqSessionLeaseError::Expired),
        RESULT_NOT_FOUND => Err(MoqSessionLeaseError::NotFound),
        RESULT_INCONSISTENT => Err(MoqSessionLeaseError::BackendUnavailable(
            BACKEND_UNAVAILABLE.to_owned(),
        )),
        _ => Err(MoqSessionLeaseError::BackendUnavailable(
            BACKEND_UNAVAILABLE.to_owned(),
        )),
    }
}

fn backend_error(_: redis::RedisError) -> MoqSessionLeaseError {
    // Do not leak endpoints, credentials, Redis keys, or script arguments into
    // errors that may reach logs or diagnostics.
    MoqSessionLeaseError::BackendUnavailable(BACKEND_UNAVAILABLE.to_owned())
}

fn hex_bytes(input: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use rvoip_core_traits::PrincipalOwnershipKey;
    use rvoip_moq::{MoqNamespace, MoqSessionId};

    use super::*;

    fn binding() -> MoqSessionLeaseBinding {
        MoqSessionLeaseBinding::new(
            MoqSessionId::new("session-secret-looking").expect("valid session ID"),
            PrincipalOwnershipKey {
                issuer: Some("https://issuer.example".to_owned()),
                tenant: Some("tenant-unsafe".to_owned()),
                subject: "subject".to_owned(),
            },
            "token-id-secret-looking",
            [0x5a; 32],
            MoqNamespace::new("tenant-unsafe", "broadcast").expect("valid namespace"),
            "broadcast:subscribe:broadcast",
            Utc::now() + Duration::minutes(5),
        )
        .expect("valid binding")
    }

    #[test]
    fn debug_and_keys_do_not_expose_credentials_or_unsafe_hash_tags() {
        let url = "redis://username:raw-password@example.invalid:6379";
        let store = RedisMoqSessionLeaseStore::from_config(
            RedisMoqSessionLeaseConfig::new(url).with_namespace("deployment{unsafe}"),
        )
        .expect("valid store");
        let debug = format!("{store:?}");
        assert!(!debug.contains("raw-password"));
        assert!(!debug.contains("username"));

        let binding = binding();
        let keys = store.key_names_for_test(&binding);
        for key in [&keys.session, &keys.token, &keys.active] {
            assert!(!key.contains(binding.token_id()));
            assert!(!key.contains(binding.namespace().tenant_id()));
            assert_eq!(key.find('{'), Some(0));
            assert_eq!(key.matches('{').count(), 1);
            assert_eq!(key.matches('}').count(), 1);
            assert_eq!(
                key.split_once('}').map(|(tag, _)| tag),
                Some("{rvoip-moq:74656e616e742d756e73616665")
            );
        }
    }

    #[test]
    fn rejects_invalid_configuration_without_exposing_url() {
        let error = RedisMoqSessionLeaseStore::from_config(
            RedisMoqSessionLeaseConfig::new("redis://secret@example.invalid")
                .with_max_active_sessions_per_tenant(0),
        )
        .expect_err("zero tenant limit must fail");
        assert_eq!(
            error,
            MoqSessionLeaseError::InvalidConfig(
                "Redis tenant session limit must be greater than zero"
            )
        );
        assert!(!error.to_string().contains("secret"));
    }
}
