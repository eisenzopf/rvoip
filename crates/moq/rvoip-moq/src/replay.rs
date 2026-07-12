use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_core_traits::PrincipalOwnershipKey;
use tokio::sync::Mutex;

/// Maximum encoded length of a transport session identifier.
pub const MAX_MOQ_SESSION_ID_BYTES: usize = 128;

/// Validated, transport-assigned MOQT session identifier.
///
/// This is a routing identifier, not an authentication credential. A session
/// ID is deliberately limited to a conservative ASCII alphabet so it has one
/// exact representation in logs, Redis keys, and transport adapters.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MoqSessionId(String);

impl MoqSessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, MoqReplayError> {
        let value = value.into();
        if value.is_empty() {
            return Err(MoqReplayError::InvalidSessionId("session ID is empty"));
        }
        if value.len() > MAX_MOQ_SESSION_ID_BYTES {
            return Err(MoqReplayError::InvalidSessionId(
                "session ID exceeds the maximum encoded length",
            ));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
        {
            return Err(MoqReplayError::InvalidSessionId(
                "session ID contains a non-canonical character",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MoqSessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Replay key that binds one already-validated token to one MOQT session.
///
/// The constructor accepts only a SHA-256 fingerprint. Raw bearer tokens,
/// certificate bytes, and other credentials therefore never enter this model.
/// Its `Debug` implementation also redacts the fingerprint to keep stable token
/// pseudonyms out of logs.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct MoqTokenBinding {
    session_id: MoqSessionId,
    token_fingerprint_sha256: [u8; 32],
}

impl MoqTokenBinding {
    pub fn from_sha256(
        session_id: MoqSessionId,
        token_fingerprint_sha256: [u8; 32],
    ) -> Result<Self, MoqReplayError> {
        if token_fingerprint_sha256 == [0; 32] {
            return Err(MoqReplayError::InvalidTokenFingerprint);
        }
        Ok(Self {
            session_id,
            token_fingerprint_sha256,
        })
    }

    pub fn session_id(&self) -> &MoqSessionId {
        &self.session_id
    }

    /// Returns the caller-supplied SHA-256 token fingerprint for durable replay
    /// stores. It is not the token itself and must still not be logged.
    pub fn token_fingerprint_sha256(&self) -> [u8; 32] {
        self.token_fingerprint_sha256
    }
}

impl std::fmt::Debug for MoqTokenBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqTokenBinding")
            .field("session_id", &self.session_id)
            .field("token_fingerprint_sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqReplayError {
    #[error("invalid MOQT session ID: {0}")]
    InvalidSessionId(&'static str),
    #[error("MOQT token fingerprint must be a nonzero SHA-256 digest")]
    InvalidTokenFingerprint,
    #[error("MOQT replay claim expiry is not in the future")]
    InvalidExpiry,
    #[error("MOQT token was already bound to another session")]
    CrossSessionReplay,
    #[error("MOQT session was already bound to another principal owner")]
    SessionOwnerChanged,
    #[error("MOQT token was already consumed by a closed session")]
    TokenConsumed,
    #[error("MOQT replay claim was not found")]
    ClaimNotFound,
    #[error("MOQT replay store reached its configured capacity")]
    CapacityExceeded,
    #[error("MOQT replay store capacity must be greater than zero")]
    InvalidCapacity,
}

/// Token replay protection used by MOQT origins, relays, and subscribers.
///
/// A repeated claim in the same session is idempotent. The same token
/// fingerprint in a different session is a replay and must fail. Implementors
/// must fail closed when they cannot durably establish either condition.
#[async_trait]
pub trait MoqTokenReplayStore: Send + Sync {
    async fn claim(
        &self,
        binding: &MoqTokenBinding,
        owner: &PrincipalOwnershipKey,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), MoqReplayError>;

    async fn verify(
        &self,
        binding: &MoqTokenBinding,
        owner: &PrincipalOwnershipKey,
        now: DateTime<Utc>,
    ) -> Result<(), MoqReplayError>;

    /// Atomically mark a transport session and its token binding closed.
    ///
    /// Claims remain as tombstones until their original expiry so disconnecting
    /// can never make a single-use token reusable.
    async fn close(
        &self,
        binding: &MoqTokenBinding,
        owner: &PrincipalOwnershipKey,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), MoqReplayError>;

    async fn retained_claims(&self, now: DateTime<Utc>) -> usize;
}

#[derive(Clone)]
struct ReplayEntry {
    session_id: MoqSessionId,
    owner: PrincipalOwnershipKey,
    expires_at: DateTime<Utc>,
    closed: bool,
}

#[derive(Clone)]
struct SessionEntry {
    owner: PrincipalOwnershipKey,
    expires_at: DateTime<Utc>,
    closed: bool,
}

struct ReplayState {
    entries: HashMap<[u8; 32], ReplayEntry>,
    sessions: HashMap<MoqSessionId, SessionEntry>,
}

/// Bounded standalone replay store for tests and all-in-one deployments.
///
/// Live entries are never evicted to make room. When the limit is reached the
/// store fails closed with [`MoqReplayError::CapacityExceeded`]. Expired claims
/// are removed before every claim, verification, and count operation.
pub struct BoundedMemoryMoqReplayStore {
    capacity: usize,
    state: Mutex<ReplayState>,
}

impl BoundedMemoryMoqReplayStore {
    pub fn new(capacity: usize) -> Result<Self, MoqReplayError> {
        if capacity == 0 {
            return Err(MoqReplayError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            state: Mutex::new(ReplayState {
                entries: HashMap::with_capacity(capacity.min(1024)),
                sessions: HashMap::with_capacity(capacity.min(1024)),
            }),
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn prune_expired_entries(entries: &mut HashMap<[u8; 32], ReplayEntry>, now: DateTime<Utc>) {
        entries.retain(|_, entry| entry.expires_at > now);
    }

    fn prune_expired_sessions(
        sessions: &mut HashMap<MoqSessionId, SessionEntry>,
        now: DateTime<Utc>,
    ) {
        sessions.retain(|_, entry| entry.expires_at > now);
    }

    fn prune_expired(state: &mut ReplayState, now: DateTime<Utc>) {
        Self::prune_expired_entries(&mut state.entries, now);
        Self::prune_expired_sessions(&mut state.sessions, now);
    }
}

#[async_trait]
impl MoqTokenReplayStore for BoundedMemoryMoqReplayStore {
    async fn claim(
        &self,
        binding: &MoqTokenBinding,
        owner: &PrincipalOwnershipKey,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), MoqReplayError> {
        if expires_at <= now {
            return Err(MoqReplayError::InvalidExpiry);
        }

        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        let fingerprint = binding.token_fingerprint_sha256;

        if let Some(session) = state.sessions.get(binding.session_id()) {
            if session.owner != *owner {
                return Err(MoqReplayError::SessionOwnerChanged);
            }
            if session.closed {
                return Err(MoqReplayError::TokenConsumed);
            }
        }

        if let Some(existing) = state.entries.get_mut(&fingerprint) {
            if existing.session_id != binding.session_id {
                return Err(MoqReplayError::CrossSessionReplay);
            }
            if existing.closed {
                return Err(MoqReplayError::TokenConsumed);
            }
            if existing.owner != *owner {
                return Err(MoqReplayError::SessionOwnerChanged);
            }
            // An idempotent retry may shorten, but can never extend, the
            // original token claim lifetime.
            existing.expires_at = existing.expires_at.min(expires_at);
            return Ok(());
        }

        if state.entries.len() >= self.capacity
            || (!state.sessions.contains_key(binding.session_id())
                && state.sessions.len() >= self.capacity)
        {
            return Err(MoqReplayError::CapacityExceeded);
        }

        state.entries.insert(
            fingerprint,
            ReplayEntry {
                session_id: binding.session_id.clone(),
                owner: owner.clone(),
                expires_at,
                closed: false,
            },
        );
        state
            .sessions
            .entry(binding.session_id.clone())
            .and_modify(|session| session.expires_at = session.expires_at.max(expires_at))
            .or_insert_with(|| SessionEntry {
                owner: owner.clone(),
                expires_at,
                closed: false,
            });
        Ok(())
    }

    async fn verify(
        &self,
        binding: &MoqTokenBinding,
        owner: &PrincipalOwnershipKey,
        now: DateTime<Utc>,
    ) -> Result<(), MoqReplayError> {
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        match state.sessions.get(binding.session_id()) {
            Some(session) if session.owner != *owner => {
                return Err(MoqReplayError::SessionOwnerChanged);
            }
            Some(session) if session.closed => return Err(MoqReplayError::TokenConsumed),
            Some(_) => {}
            None => return Err(MoqReplayError::ClaimNotFound),
        }
        match state.entries.get(&binding.token_fingerprint_sha256) {
            Some(entry)
                if entry.session_id == binding.session_id
                    && entry.owner == *owner
                    && !entry.closed =>
            {
                Ok(())
            }
            Some(entry) if entry.owner != *owner => Err(MoqReplayError::SessionOwnerChanged),
            Some(entry) if entry.session_id == binding.session_id => {
                Err(MoqReplayError::TokenConsumed)
            }
            Some(_) => Err(MoqReplayError::CrossSessionReplay),
            None => Err(MoqReplayError::ClaimNotFound),
        }
    }

    async fn close(
        &self,
        binding: &MoqTokenBinding,
        owner: &PrincipalOwnershipKey,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), MoqReplayError> {
        if expires_at <= now {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        if let Some(session) = state.sessions.get(binding.session_id()) {
            if session.owner != *owner {
                return Err(MoqReplayError::SessionOwnerChanged);
            }
        }
        if let Some(entry) = state.entries.get(&binding.token_fingerprint_sha256) {
            if entry.session_id != binding.session_id {
                return Err(MoqReplayError::CrossSessionReplay);
            }
            if entry.owner != *owner {
                return Err(MoqReplayError::SessionOwnerChanged);
            }
        }
        if !state
            .entries
            .contains_key(&binding.token_fingerprint_sha256)
            && state.entries.len() >= self.capacity
        {
            return Err(MoqReplayError::CapacityExceeded);
        }
        if !state.sessions.contains_key(binding.session_id())
            && state.sessions.len() >= self.capacity
        {
            return Err(MoqReplayError::CapacityExceeded);
        }

        let token_retained_until = state
            .entries
            .get(&binding.token_fingerprint_sha256)
            .map_or(expires_at, |entry| entry.expires_at.max(expires_at));
        let mut retained_until = state
            .sessions
            .get(binding.session_id())
            .map_or(token_retained_until, |session| {
                session.expires_at.max(token_retained_until)
            });
        for entry in state
            .entries
            .values()
            .filter(|entry| &entry.session_id == binding.session_id())
        {
            retained_until = retained_until.max(entry.expires_at);
        }
        for entry in state
            .entries
            .values_mut()
            .filter(|entry| &entry.session_id == binding.session_id())
        {
            entry.closed = true;
        }

        state.entries.insert(
            binding.token_fingerprint_sha256,
            ReplayEntry {
                session_id: binding.session_id.clone(),
                owner: owner.clone(),
                expires_at: token_retained_until,
                closed: true,
            },
        );
        state.sessions.insert(
            binding.session_id.clone(),
            SessionEntry {
                owner: owner.clone(),
                expires_at: retained_until,
                closed: true,
            },
        );
        Ok(())
    }

    async fn retained_claims(&self, now: DateTime<Utc>) -> usize {
        let mut state = self.state.lock().await;
        Self::prune_expired(&mut state, now);
        state.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn binding(session: &str, byte: u8) -> MoqTokenBinding {
        MoqTokenBinding::from_sha256(MoqSessionId::new(session).unwrap(), [byte; 32]).unwrap()
    }

    fn owner(subject: &str) -> PrincipalOwnershipKey {
        PrincipalOwnershipKey {
            issuer: Some("issuer".into()),
            tenant: Some("tenant".into()),
            subject: subject.into(),
        }
    }

    #[tokio::test]
    async fn same_session_is_idempotent_but_cross_session_is_replay() {
        let now = Utc::now();
        let store = BoundedMemoryMoqReplayStore::new(2).unwrap();
        let first = binding("session-a", 1);
        let replay = binding("session-b", 1);
        let owner = owner("subject-a");

        store
            .claim(&first, &owner, now + Duration::minutes(2), now)
            .await
            .unwrap();
        store
            .claim(&first, &owner, now + Duration::minutes(3), now)
            .await
            .unwrap();
        assert_eq!(
            store
                .claim(&replay, &owner, now + Duration::minutes(2), now)
                .await,
            Err(MoqReplayError::CrossSessionReplay)
        );
    }

    #[tokio::test]
    async fn session_owner_is_immutable_until_the_claim_expires() {
        let now = Utc::now();
        let store = BoundedMemoryMoqReplayStore::new(2).unwrap();
        let first = binding("session-a", 1);
        let second = binding("session-a", 2);
        let owner_a = owner("subject-a");
        let owner_b = owner("subject-b");
        store
            .claim(&first, &owner_a, now + Duration::seconds(1), now)
            .await
            .unwrap();
        assert_eq!(
            store
                .claim(&second, &owner_b, now + Duration::minutes(1), now)
                .await,
            Err(MoqReplayError::SessionOwnerChanged)
        );

        // The transport must still assign non-reused IDs, but an expired
        // tombstone no longer consumes bounded standalone-store capacity.
        store
            .claim(
                &second,
                &owner_b,
                now + Duration::minutes(1),
                now + Duration::seconds(1),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn capacity_fails_closed_and_expired_claims_are_reclaimed() {
        let now = Utc::now();
        let store = BoundedMemoryMoqReplayStore::new(1).unwrap();
        let first = binding("session-a", 1);
        let second = binding("session-b", 2);
        let owner = owner("subject-a");
        store
            .claim(&first, &owner, now + Duration::seconds(1), now)
            .await
            .unwrap();
        assert_eq!(
            store
                .claim(&second, &owner, now + Duration::minutes(1), now)
                .await,
            Err(MoqReplayError::CapacityExceeded)
        );
        store
            .claim(
                &second,
                &owner,
                now + Duration::minutes(1),
                now + Duration::seconds(1),
            )
            .await
            .unwrap();
        assert_eq!(store.retained_claims(now + Duration::seconds(1)).await, 1);
    }

    #[tokio::test]
    async fn close_tombstones_the_token_until_expiry() {
        let now = Utc::now();
        let store = BoundedMemoryMoqReplayStore::new(2).unwrap();
        let first = binding("session-a", 1);
        let second = binding("session-b", 2);
        let owner = owner("subject-a");
        for value in [&first, &second] {
            store
                .claim(value, &owner, now + Duration::minutes(1), now)
                .await
                .unwrap();
        }
        store
            .close(&first, &owner, now + Duration::minutes(1), now)
            .await
            .unwrap();
        assert_eq!(
            store.verify(&first, &owner, now).await,
            Err(MoqReplayError::TokenConsumed)
        );
        store.verify(&second, &owner, now).await.unwrap();
        assert_eq!(store.retained_claims(now).await, 2);

        let cross_session = binding("session-c", 1);
        assert_eq!(
            store
                .claim(&cross_session, &owner, now + Duration::minutes(1), now)
                .await,
            Err(MoqReplayError::CrossSessionReplay)
        );
        assert_eq!(
            store
                .claim(&first, &owner, now + Duration::minutes(1), now)
                .await,
            Err(MoqReplayError::TokenConsumed)
        );
    }

    #[tokio::test]
    async fn close_tombstones_every_session_token_until_the_longest_claim_expires() {
        let now = Utc::now();
        let store = BoundedMemoryMoqReplayStore::new(3).unwrap();
        let first = binding("session-a", 1);
        let second = binding("session-a", 2);
        let third = binding("session-a", 3);
        let owner = owner("subject-a");
        store
            .claim(&first, &owner, now + Duration::minutes(2), now)
            .await
            .unwrap();
        store
            .claim(&second, &owner, now + Duration::minutes(3), now)
            .await
            .unwrap();

        // A close carrying a shorter expiry cannot shorten either existing
        // token tombstone or the session-owner tombstone.
        store
            .close(&first, &owner, now + Duration::seconds(30), now)
            .await
            .unwrap();
        assert_eq!(
            store.verify(&second, &owner, now).await,
            Err(MoqReplayError::TokenConsumed)
        );
        assert_eq!(
            store
                .claim(
                    &third,
                    &owner,
                    now + Duration::minutes(4),
                    now + Duration::minutes(2),
                )
                .await,
            Err(MoqReplayError::TokenConsumed)
        );
        let second_replay = binding("session-b", 2);
        assert_eq!(
            store
                .claim(
                    &second_replay,
                    &owner,
                    now + Duration::minutes(4),
                    now + Duration::minutes(2),
                )
                .await,
            Err(MoqReplayError::CrossSessionReplay)
        );

        // All token and session tombstones are reclaimed at their exact
        // maximum expiry; only then can the bounded store accept a new claim.
        store
            .claim(
                &third,
                &owner,
                now + Duration::minutes(4),
                now + Duration::minutes(3),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn close_racing_the_first_claim_always_leaves_a_tombstone() {
        let now = Utc::now();
        let expires = now + Duration::minutes(1);
        let store = std::sync::Arc::new(BoundedMemoryMoqReplayStore::new(2).unwrap());
        let first = binding("session-a", 1);
        let owner = owner("subject-a");
        let claim_store = store.clone();
        let claim_binding = first.clone();
        let claim_owner = owner.clone();
        let close_store = store.clone();
        let close_binding = first.clone();
        let close_owner = owner.clone();

        let (claim, close) = tokio::join!(
            async move {
                claim_store
                    .claim(&claim_binding, &claim_owner, expires, now)
                    .await
            },
            async move {
                close_store
                    .close(&close_binding, &close_owner, expires, now)
                    .await
            }
        );
        assert!(matches!(claim, Ok(()) | Err(MoqReplayError::TokenConsumed)));
        close.unwrap();
        assert_eq!(
            store.verify(&first, &owner, now).await,
            Err(MoqReplayError::TokenConsumed)
        );
        let replay = binding("session-b", 1);
        assert_eq!(
            store.claim(&replay, &owner, expires, now).await,
            Err(MoqReplayError::CrossSessionReplay)
        );
    }

    #[test]
    fn identifiers_are_canonical_and_token_debug_is_redacted() {
        assert!(MoqSessionId::new("session/a").is_err());
        assert!(MoqSessionId::new("").is_err());
        assert_eq!(
            MoqTokenBinding::from_sha256(MoqSessionId::new("session").unwrap(), [0; 32]),
            Err(MoqReplayError::InvalidTokenFingerprint)
        );
        let binding = binding("session", 9);
        let diagnostic = format!("{binding:?}");
        assert!(diagnostic.contains("<redacted>"));
        assert!(!diagnostic.contains("090909"));
    }
}
