use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_core_traits::PrincipalOwnershipKey;
use tokio::sync::Mutex;

use crate::{MoqNamespace, MoqSessionId};

const MAX_TOKEN_ID_BYTES: usize = 256;
const MAX_SCOPE_BYTES: usize = 512;

/// Immutable credential and resource binding for one admitted MOQT session.
///
/// The raw SETUP credential is deliberately absent. The token identifier and
/// SHA-256 fingerprint are correlation-sensitive and are always redacted from
/// `Debug` output.
#[derive(Clone, Eq, PartialEq)]
pub struct MoqSessionLeaseBinding {
    session_id: MoqSessionId,
    owner: PrincipalOwnershipKey,
    token_id: String,
    credential_fingerprint_sha256: [u8; 32],
    namespace: MoqNamespace,
    canonical_scope: String,
    expires_at: DateTime<Utc>,
}

impl MoqSessionLeaseBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: MoqSessionId,
        owner: PrincipalOwnershipKey,
        token_id: impl Into<String>,
        credential_fingerprint_sha256: [u8; 32],
        namespace: MoqNamespace,
        canonical_scope: impl Into<String>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, MoqSessionLeaseError> {
        let token_id = token_id.into();
        let canonical_scope = canonical_scope.into();
        if owner.subject.trim().is_empty() {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "principal subject is empty",
            ));
        }
        let tenant = owner
            .tenant
            .as_deref()
            .ok_or(MoqSessionLeaseError::InvalidBinding(
                "principal tenant is missing",
            ))?;
        if tenant != namespace.tenant_id() {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "principal tenant does not own the namespace",
            ));
        }
        if owner.issuer.as_deref().is_none_or(str::is_empty) {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "principal issuer is missing",
            ));
        }
        if token_id.is_empty() || token_id.len() > MAX_TOKEN_ID_BYTES {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "token ID must contain 1 to 256 bytes",
            ));
        }
        if token_id.chars().any(char::is_control) {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "token ID contains control characters",
            ));
        }
        if credential_fingerprint_sha256 == [0; 32] {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "credential fingerprint is zero",
            ));
        }
        if canonical_scope.is_empty()
            || canonical_scope.len() > MAX_SCOPE_BYTES
            || canonical_scope.chars().any(char::is_control)
        {
            return Err(MoqSessionLeaseError::InvalidBinding(
                "canonical scope must contain 1 to 512 non-control bytes",
            ));
        }
        Ok(Self {
            session_id,
            owner,
            token_id,
            credential_fingerprint_sha256,
            namespace,
            canonical_scope,
            expires_at,
        })
    }

    pub fn session_id(&self) -> &MoqSessionId {
        &self.session_id
    }

    pub fn owner(&self) -> &PrincipalOwnershipKey {
        &self.owner
    }

    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    pub const fn credential_fingerprint_sha256(&self) -> [u8; 32] {
        self.credential_fingerprint_sha256
    }

    pub fn namespace(&self) -> &MoqNamespace {
        &self.namespace
    }

    pub fn canonical_scope(&self) -> &str {
        &self.canonical_scope
    }

    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl std::fmt::Debug for MoqSessionLeaseBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqSessionLeaseBinding")
            .field("session_id", &self.session_id)
            .field("owner", &self.owner)
            .field("token_id", &"<redacted>")
            .field("credential_fingerprint_sha256", &"<redacted>")
            .field("namespace", &self.namespace)
            .field("canonical_scope", &self.canonical_scope)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Store-issued immutable lease used for verification and finalization.
#[derive(Clone, Eq, PartialEq)]
pub struct MoqSessionLease {
    binding: MoqSessionLeaseBinding,
}

impl MoqSessionLease {
    /// Construct the store-issued lease representation for a validated
    /// binding. Durable store implementations use this after atomic acquire.
    pub fn from_binding(binding: MoqSessionLeaseBinding) -> Self {
        Self { binding }
    }

    pub fn binding(&self) -> &MoqSessionLeaseBinding {
        &self.binding
    }
}

impl std::fmt::Debug for MoqSessionLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqSessionLease")
            .field("binding", &self.binding)
            .finish()
    }
}

/// Why a session lease is being finalized.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoqSessionLeaseClose {
    PeerClosed,
    LocalClosed,
    ActivationFailed,
    AdmissionRevalidationFailed,
    ProtocolError,
    RelayShutdown,
}

/// Active-session limits enforced by a lease store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqSessionLeaseLimits {
    pub max_active_sessions: usize,
    pub max_active_sessions_per_tenant: usize,
}

impl MoqSessionLeaseLimits {
    pub fn new(
        max_active_sessions: usize,
        max_active_sessions_per_tenant: usize,
    ) -> Result<Self, MoqSessionLeaseError> {
        if max_active_sessions == 0 || max_active_sessions_per_tenant == 0 {
            return Err(MoqSessionLeaseError::InvalidConfig(
                "session lease limits must be greater than zero",
            ));
        }
        if max_active_sessions_per_tenant > max_active_sessions {
            return Err(MoqSessionLeaseError::InvalidConfig(
                "per-tenant limit exceeds the store limit",
            ));
        }
        Ok(Self {
            max_active_sessions,
            max_active_sessions_per_tenant,
        })
    }

    /// Configure tenant-only enforcement for a distributed store. The relay
    /// separately owns the process-global active-session permit pool.
    pub fn tenant_scoped(
        max_active_sessions_per_tenant: usize,
    ) -> Result<Self, MoqSessionLeaseError> {
        Self::new(usize::MAX, max_active_sessions_per_tenant)
    }
}

impl Default for MoqSessionLeaseLimits {
    fn default() -> Self {
        Self {
            max_active_sessions: 4_096,
            max_active_sessions_per_tenant: 1_000,
        }
    }
}

/// Aggregate, identifier-free lease store diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqSessionLeaseSnapshot {
    pub retained_sessions: usize,
    pub retained_tokens: usize,
    pub active_sessions: usize,
    pub tenant_buckets: usize,
    pub limits: MoqSessionLeaseLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqSessionLeaseError {
    #[error("invalid MOQT session lease configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("invalid MOQT session lease binding: {0}")]
    InvalidBinding(&'static str),
    #[error("MOQT session lease is expired")]
    Expired,
    #[error("MOQT session lease store capacity is exhausted")]
    CapacityExceeded,
    #[error("MOQT tenant active-session quota is exhausted")]
    TenantQuotaExceeded,
    #[error("MOQT credential was already bound to another session")]
    CrossSessionReplay,
    #[error("MOQT session lease principal ownership changed")]
    OwnerMismatch,
    #[error("MOQT session lease binding changed")]
    BindingMismatch,
    #[error("MOQT session lease is closed")]
    Closed,
    #[error("MOQT session lease was not found")]
    NotFound,
    #[error("MOQT session lease backend is unavailable: {0}")]
    BackendUnavailable(String),
}

/// Durable replay and tenant-quota boundary for production MOQT admission.
#[async_trait]
pub trait MoqSessionLeaseStore: Send + Sync {
    async fn acquire(
        &self,
        binding: &MoqSessionLeaseBinding,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLease, MoqSessionLeaseError>;

    async fn verify(
        &self,
        lease: &MoqSessionLease,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError>;

    async fn close(
        &self,
        lease: &MoqSessionLease,
        close: MoqSessionLeaseClose,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError>;

    async fn snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLeaseSnapshot, MoqSessionLeaseError>;
}

#[derive(Clone)]
struct MemoryLeaseEntry {
    binding: MoqSessionLeaseBinding,
    closed: bool,
}

#[derive(Default)]
struct MemoryLeaseState {
    sessions: HashMap<MoqSessionId, MemoryLeaseEntry>,
    tokens: HashMap<[u8; 32], MoqSessionId>,
    tenant_active: HashMap<String, usize>,
    active_sessions: usize,
}

/// Bounded in-memory reference implementation for all-in-one deployments and
/// semantic conformance tests.
pub struct BoundedMemoryMoqSessionLeaseStore {
    limits: MoqSessionLeaseLimits,
    state: Mutex<MemoryLeaseState>,
}

impl BoundedMemoryMoqSessionLeaseStore {
    pub fn new(limits: MoqSessionLeaseLimits) -> Result<Self, MoqSessionLeaseError> {
        let limits = MoqSessionLeaseLimits::new(
            limits.max_active_sessions,
            limits.max_active_sessions_per_tenant,
        )?;
        Ok(Self {
            limits,
            state: Mutex::new(MemoryLeaseState::default()),
        })
    }

    pub const fn limits(&self) -> MoqSessionLeaseLimits {
        self.limits
    }

    fn prune_expired(state: &mut MemoryLeaseState, now: DateTime<Utc>) {
        let expired = state
            .sessions
            .iter()
            .filter_map(|(session_id, entry)| {
                (entry.binding.expires_at() <= now).then_some(session_id.clone())
            })
            .collect::<Vec<_>>();
        for session_id in expired {
            if let Some(entry) = state.sessions.remove(&session_id) {
                state
                    .tokens
                    .remove(&entry.binding.credential_fingerprint_sha256());
                if !entry.closed {
                    state.active_sessions = state.active_sessions.saturating_sub(1);
                    decrement_tenant(state, entry.binding.namespace().tenant_id());
                }
            }
        }
    }

    fn ensure_exact(
        entry: &MemoryLeaseEntry,
        binding: &MoqSessionLeaseBinding,
    ) -> Result<(), MoqSessionLeaseError> {
        if entry.binding.owner() != binding.owner() {
            return Err(MoqSessionLeaseError::OwnerMismatch);
        }
        if entry.binding != *binding {
            return Err(MoqSessionLeaseError::BindingMismatch);
        }
        if entry.closed {
            return Err(MoqSessionLeaseError::Closed);
        }
        Ok(())
    }
}

fn decrement_tenant(state: &mut MemoryLeaseState, tenant: &str) {
    if let Some(active) = state.tenant_active.get_mut(tenant) {
        *active = active.saturating_sub(1);
        if *active == 0 {
            state.tenant_active.remove(tenant);
        }
    }
}

#[async_trait]
impl MoqSessionLeaseStore for BoundedMemoryMoqSessionLeaseStore {
    async fn acquire(
        &self,
        binding: &MoqSessionLeaseBinding,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLease, MoqSessionLeaseError> {
        if binding.expires_at() <= now {
            return Err(MoqSessionLeaseError::Expired);
        }
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        let fingerprint = binding.credential_fingerprint_sha256();
        if let Some(session_id) = state.tokens.get(&fingerprint) {
            if session_id != binding.session_id() {
                return Err(MoqSessionLeaseError::CrossSessionReplay);
            }
        }
        if let Some(entry) = state.sessions.get(binding.session_id()) {
            Self::ensure_exact(entry, binding)?;
            return Ok(MoqSessionLease::from_binding(entry.binding.clone()));
        }
        if state.active_sessions >= self.limits.max_active_sessions {
            return Err(MoqSessionLeaseError::CapacityExceeded);
        }
        let tenant = binding.namespace().tenant_id();
        if state.tenant_active.get(tenant).copied().unwrap_or(0)
            >= self.limits.max_active_sessions_per_tenant
        {
            return Err(MoqSessionLeaseError::TenantQuotaExceeded);
        }
        state
            .tokens
            .insert(fingerprint, binding.session_id().clone());
        state.sessions.insert(
            binding.session_id().clone(),
            MemoryLeaseEntry {
                binding: binding.clone(),
                closed: false,
            },
        );
        state.active_sessions += 1;
        *state.tenant_active.entry(tenant.to_string()).or_default() += 1;
        Ok(MoqSessionLease::from_binding(binding.clone()))
    }

    async fn verify(
        &self,
        lease: &MoqSessionLease,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError> {
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        if lease.binding.expires_at() <= now {
            return Err(MoqSessionLeaseError::Expired);
        }
        let entry = state
            .sessions
            .get(lease.binding.session_id())
            .ok_or(MoqSessionLeaseError::NotFound)?;
        Self::ensure_exact(entry, &lease.binding)
    }

    async fn close(
        &self,
        lease: &MoqSessionLease,
        _close: MoqSessionLeaseClose,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError> {
        if lease.binding.expires_at() <= now {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        let fingerprint = lease.binding.credential_fingerprint_sha256();
        if let Some(session_id) = state.tokens.get(&fingerprint) {
            if session_id != lease.binding.session_id() {
                return Err(MoqSessionLeaseError::CrossSessionReplay);
            }
        }
        if let Some(entry) = state.sessions.get_mut(lease.binding.session_id()) {
            if entry.binding.owner() != lease.binding.owner() {
                return Err(MoqSessionLeaseError::OwnerMismatch);
            }
            if entry.binding != lease.binding {
                return Err(MoqSessionLeaseError::BindingMismatch);
            }
            if !entry.closed {
                entry.closed = true;
                state.active_sessions = state.active_sessions.saturating_sub(1);
                decrement_tenant(&mut state, lease.binding.namespace().tenant_id());
            }
            return Ok(());
        }
        state
            .tokens
            .insert(fingerprint, lease.binding.session_id().clone());
        state.sessions.insert(
            lease.binding.session_id().clone(),
            MemoryLeaseEntry {
                binding: lease.binding.clone(),
                closed: true,
            },
        );
        Ok(())
    }

    async fn snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLeaseSnapshot, MoqSessionLeaseError> {
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        let tenants = state
            .sessions
            .values()
            .map(|entry| entry.binding.namespace().tenant_id())
            .collect::<HashSet<_>>()
            .len();
        Ok(MoqSessionLeaseSnapshot {
            retained_sessions: state.sessions.len(),
            retained_tokens: state.tokens.len(),
            active_sessions: state.active_sessions,
            tenant_buckets: tenants,
            limits: self.limits,
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn owner(tenant: &str, subject: &str) -> PrincipalOwnershipKey {
        PrincipalOwnershipKey {
            issuer: Some("https://issuer.example".into()),
            tenant: Some(tenant.into()),
            subject: subject.into(),
        }
    }

    fn binding(
        tenant: &str,
        session: &str,
        fingerprint: u8,
        expires_at: DateTime<Utc>,
    ) -> MoqSessionLeaseBinding {
        let namespace = MoqNamespace::new(tenant, "broadcast").unwrap();
        MoqSessionLeaseBinding::new(
            MoqSessionId::new(session).unwrap(),
            owner(tenant, "subject"),
            format!("token-{fingerprint}"),
            [fingerprint; 32],
            namespace,
            format!("/{tenant}/broadcast"),
            expires_at,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn idempotency_replay_quota_close_and_expiry_are_fail_closed() {
        let now = Utc::now();
        let store =
            BoundedMemoryMoqSessionLeaseStore::new(MoqSessionLeaseLimits::new(2, 1).unwrap())
                .unwrap();
        let first = binding("tenant-a", "session-a", 1, now + Duration::seconds(2));
        let lease = store.acquire(&first, now).await.unwrap();
        assert_eq!(store.acquire(&first, now).await.unwrap(), lease);
        assert_eq!(
            store
                .acquire(
                    &binding("tenant-a", "session-b", 2, now + Duration::seconds(2)),
                    now,
                )
                .await,
            Err(MoqSessionLeaseError::TenantQuotaExceeded)
        );
        assert_eq!(
            store
                .acquire(
                    &binding("tenant-b", "session-b", 1, now + Duration::seconds(2)),
                    now,
                )
                .await,
            Err(MoqSessionLeaseError::CrossSessionReplay)
        );
        store
            .close(&lease, MoqSessionLeaseClose::PeerClosed, now)
            .await
            .unwrap();
        store
            .close(&lease, MoqSessionLeaseClose::PeerClosed, now)
            .await
            .unwrap();
        assert_eq!(
            store.verify(&lease, now).await,
            Err(MoqSessionLeaseError::Closed)
        );
        assert_eq!(store.snapshot(now).await.unwrap().active_sessions, 0);
        assert_eq!(
            store
                .snapshot(now + Duration::seconds(2))
                .await
                .unwrap()
                .retained_sessions,
            0
        );
    }

    #[tokio::test]
    async fn close_racing_first_acquire_always_leaves_a_tombstone() {
        let now = Utc::now();
        let store = std::sync::Arc::new(
            BoundedMemoryMoqSessionLeaseStore::new(MoqSessionLeaseLimits::default()).unwrap(),
        );
        let binding = binding("tenant-a", "session-a", 1, now + Duration::minutes(1));
        let synthetic = MoqSessionLease::from_binding(binding.clone());
        let acquire_store = store.clone();
        let acquire_binding = binding.clone();
        let close_store = store.clone();
        let close_lease = synthetic.clone();
        let (acquire, close) = tokio::join!(
            async move { acquire_store.acquire(&acquire_binding, now).await },
            async move {
                close_store
                    .close(&close_lease, MoqSessionLeaseClose::ActivationFailed, now)
                    .await
            }
        );
        close.unwrap();
        assert!(matches!(acquire, Ok(_) | Err(MoqSessionLeaseError::Closed)));
        assert_eq!(
            store.verify(&synthetic, now).await,
            Err(MoqSessionLeaseError::Closed)
        );
    }

    #[test]
    fn binding_debug_redacts_token_material() {
        let binding = binding(
            "tenant-a",
            "session-a",
            7,
            Utc::now() + Duration::minutes(1),
        );
        let diagnostic = format!("{binding:?}");
        assert!(diagnostic.contains("<redacted>"));
        assert!(!diagnostic.contains("token-7"));
        assert!(!diagnostic.contains("070707"));
    }
}
