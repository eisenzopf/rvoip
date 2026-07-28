//! `SubscriptionHandler` trait — the coordinator's escape hatch into
//! whatever multi-party-routing implementation the deployment provides.
//!
//! Architecture (v0.x MP1/MP2/MP3 sequencing):
//!
//! - **MP1** (done): orchestrator stores per-Session subscription rows.
//! - **MP2** (this file): coordinator decodes `stream.subscribe` /
//!   `stream.unsubscribe` envelopes and routes them through a
//!   `SubscriptionHandler`. The concrete implementation lives in
//!   `rvoip-core` (so it can hold `Arc<Orchestrator>`); the trait
//!   stays here to keep `rvoip-uctp` substrate-agnostic.
//! - **MP3** (future): adapter media path consults
//!   `orchestrator.subscribers_for(...)` to fan datagrams out.
//!
//! The trait deliberately takes the parsed payload structs directly so
//! implementations don't have to re-decode the JSON. Wire-format
//! changes flow through the payload types, not through this trait.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use rvoip_core::adapter::InboundRoutingHint;
use rvoip_core::identity::{AuthenticatedPrincipal, PrincipalOwnershipKey};

use crate::ids::{ConnectionId, SessionId};
use crate::payloads::stream::{StreamSubscribe, StreamUnsubscribe};

/// Outcome of a `stream.subscribe` request.
///
/// `Ok` → coordinator emits `ack` in_reply_to the request envelope.
/// `Reject{code, reason}` → coordinator emits `error` with that code
/// and reason, also in_reply_to. Codes follow the
/// CONVERSATION_PROTOCOL.md §11.2 catalog: 404 (unknown participant /
/// stream), 488 (capability mismatch), 501 (recognized but not wired
/// in this build), 503 (transient capacity / not-ready).
#[derive(Clone, PartialEq, Eq)]
pub enum SubscriptionOutcome {
    Ok,
    Reject { code: u16, reason: String },
}

impl fmt::Debug for SubscriptionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => formatter.write_str("SubscriptionOutcome::Ok"),
            Self::Reject { code, reason } => formatter
                .debug_struct("SubscriptionOutcome::Reject")
                .field("code", code)
                .field("reason_present", &!reason.is_empty())
                .field("reason_bytes", &reason.len())
                .finish(),
        }
    }
}

impl SubscriptionOutcome {
    pub fn ok() -> Self {
        Self::Ok
    }

    pub fn reject(code: u16, reason: impl Into<String>) -> Self {
        Self::Reject {
            code,
            reason: reason.into(),
        }
    }
}

/// Plug-in trait implemented by whatever owns the multi-party routing
/// table. The UCTP coordinator calls into this on inbound
/// `stream.subscribe` / `stream.unsubscribe` envelopes. The default
/// `None` handler keeps the legacy 501 reject for back-compat.
///
/// Implementations are typically not blocking, so the trait is sync
/// (no `async fn`). If a future impl needs to block, switch the trait
/// to `async-trait` — the coordinator already awaits the result.
pub trait SubscriptionHandler: Send + Sync {
    /// Handle a `stream.subscribe` envelope. The subscriber is the
    /// peer Connection that sent the envelope; the SessionId is taken
    /// from `env.sid`.
    fn subscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamSubscribe,
    ) -> SubscriptionOutcome;

    /// Handle a `stream.unsubscribe` envelope. Idempotent — removing a
    /// subscription that doesn't exist must succeed.
    fn unsubscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamUnsubscribe,
    ) -> SubscriptionOutcome;

    /// Announce that a Stream is available for subscription. The
    /// coordinator calls this once per Stream when it emits
    /// `stream.opened` (i.e. at `connection.ready` time, per
    /// CONVERSATION_PROTOCOL.md §7.4). Default impl is a no-op so
    /// [`RejectingHandler`] and similar don't have to opt in.
    ///
    /// `info` carries the publisher's `ConnectionId`, `participant`
    /// (Participant ID from `connection.offer.by_participant`), and
    /// `kind` (`"audio"` / `"video"` / `"data"`). MP2.5+ uses
    /// `participant` and `kind` to resolve `from_participant`-form
    /// and `kinds`-filtered subscriptions.
    fn register_publisher(&self, _info: PublisherInfo<'_>) {}

    /// Drop a publisher registration only if it is still owned by the named
    /// Connection. The coordinator calls this during exact Connection
    /// teardown. Implementations must not remove a same-named replacement
    /// installed by another publisher after an older Connection began
    /// closing. Default no-op.
    fn unregister_publisher(&self, _sid: &SessionId, _strm_id: &str, _publisher: &ConnectionId) {}

    /// Drop every publisher/subscriber resource owned by a Connection.
    /// Called on explicit end, transport loss, expiry, and coordinator drain.
    fn unregister_connection(&self, _sid: &SessionId, _connid: &ConnectionId) {}
}

/// Resolves a peer-supplied Session ID to the canonical Session ID used by
/// the process-wide publisher/subscriber registry.
///
/// The resolver is the authorization boundary between an untrusted wire ID
/// and a tenant-owned resource. Deployments that need two physical peers to
/// meet in the same Session (for example Bridgefu attachment tokens) provide
/// a resolver backed by their authenticated call/session store. The default
/// resolver remains peer-scoped and therefore cannot cross-connect peers.
pub trait SessionBindingResolver: Send + Sync {
    fn resolve_session(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
    ) -> Result<SessionId, ResourceBindingError>;

    /// Revalidate an already-bound Session without consuming another
    /// single-use attachment token. Stateful authorities override this to
    /// enforce revocation, tenant ownership, transport, and expiry for the
    /// full peer lifetime. Stateless resolvers remain valid by default.
    fn reauthorize_session(
        &self,
        _principal: &AuthenticatedPrincipal,
        _wire_session: &SessionId,
        _canonical_session: &SessionId,
    ) -> Result<(), ResourceBindingError> {
        Ok(())
    }

    /// Reauthorize one exact wire Connection before a subscription mutation.
    /// The default preserves session-only resolvers; deployments whose wire
    /// Session names encode a connection owner can additionally prevent a
    /// sibling Connection from consuming that authority.
    fn reauthorize_connection(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
        canonical_session: &SessionId,
        _wire_connection: &ConnectionId,
        _core_connection: &ConnectionId,
    ) -> Result<(), ResourceBindingError> {
        self.reauthorize_session(principal, wire_session, canonical_session)
    }

    /// Optionally recover one bounded, secret routing hint from an inbound
    /// Session offer. The default deliberately ignores peer capabilities.
    /// Deployments must opt in through a resolver installed only on their
    /// mutually-authenticated private ingress.
    fn resolve_inbound_routing_hint(
        &self,
        _principal: &AuthenticatedPrincipal,
        _wire_session: &SessionId,
        _intent: &str,
        _capabilities_offer: &serde_json::Value,
    ) -> Result<Option<InboundRoutingHint>, ResourceBindingError> {
        Ok(None)
    }
}

impl<F> SessionBindingResolver for F
where
    F: Fn(&AuthenticatedPrincipal, &SessionId) -> Result<SessionId, ResourceBindingError>
        + Send
        + Sync,
{
    fn resolve_session(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
    ) -> Result<SessionId, ResourceBindingError> {
        self(principal, wire_session)
    }
}

/// An explicit protocol-facing resource authorization failure.
#[derive(Clone, Eq, PartialEq)]
pub struct ResourceBindingError {
    pub code: u16,
    pub reason: String,
}

impl fmt::Debug for ResourceBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceBindingError")
            .field("code", &self.code)
            .field("reason_present", &!self.reason.is_empty())
            .field("reason_bytes", &self.reason.len())
            .finish()
    }
}

impl fmt::Display for ResourceBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "resource binding rejected (code {}, reason_present={}, reason_bytes={})",
            self.code,
            !self.reason.is_empty(),
            self.reason.len()
        )
    }
}

impl std::error::Error for ResourceBindingError {}

impl ResourceBindingError {
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }

    pub fn forbidden(reason: impl Into<String>) -> Self {
        Self::new(403, reason)
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::new(503, reason)
    }

    fn subscription_outcome(&self) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(self.code, self.reason.clone())
    }
}

/// Safe standalone resolver. Its namespace is unique to one physical peer,
/// so identical remote Session IDs on different peers never alias.
pub struct PeerScopedSessionResolver {
    namespace: String,
}

impl PeerScopedSessionResolver {
    pub fn new(namespace: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            namespace: namespace.into(),
        })
    }
}

impl SessionBindingResolver for PeerScopedSessionResolver {
    fn resolve_session(
        &self,
        _principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
    ) -> Result<SessionId, ResourceBindingError> {
        Ok(SessionId::from_string(format!(
            "{}:{}",
            self.namespace, wire_session
        )))
    }
}

/// Authenticated wire-to-core resource bindings for one physical peer.
///
/// Connection mappings are keyed by `(wire Session, wire Connection)` rather
/// than Connection alone. This avoids accidental aliasing when a peer reuses
/// a Connection ID in another logical Session. Bindings are removed exactly
/// on Connection/Session teardown and never inferred from attacker-controlled
/// strings inside the shared registry.
pub struct PeerResourceBindings {
    resolver: Arc<dyn SessionBindingResolver>,
    mutation: Mutex<()>,
    principal: RwLock<Option<AuthenticatedPrincipal>>,
    owner: RwLock<Option<PrincipalOwnershipKey>>,
    sessions: DashMap<SessionId, SessionId>,
    connections: DashMap<(SessionId, ConnectionId), ConnectionId>,
    /// Reverse ownership index. One adapter/core Connection may represent
    /// multiple sibling wire Connections inside the same wire Session, but it
    /// must never alias across wire Sessions.
    core_connections: DashMap<ConnectionId, (SessionId, HashSet<ConnectionId>)>,
}

struct RemovedConnectionBinding {
    core_session: SessionId,
    core_connection: ConnectionId,
    release_core_connection: bool,
}

impl PeerResourceBindings {
    pub fn new(resolver: Arc<dyn SessionBindingResolver>) -> Arc<Self> {
        Arc::new(Self {
            resolver,
            mutation: Mutex::new(()),
            principal: RwLock::new(None),
            owner: RwLock::new(None),
            sessions: DashMap::new(),
            connections: DashMap::new(),
            core_connections: DashMap::new(),
        })
    }

    /// Retain the authenticated principal that authorizes every mapping on
    /// this peer. Re-authentication may refresh scopes/expiry but cannot
    /// change issuer+tenant+subject ownership in place.
    pub fn authenticate(
        &self,
        principal: AuthenticatedPrincipal,
    ) -> Result<(), ResourceBindingError> {
        if principal.is_expired_at(chrono::Utc::now()) {
            return Err(ResourceBindingError::forbidden("principal-expired"));
        }
        let _mutation = self.mutation.lock();
        let incoming_owner = principal.ownership_key();
        let mut owner = self.owner.write();
        if owner
            .as_ref()
            .is_some_and(|current| current != &incoming_owner)
        {
            return Err(ResourceBindingError::forbidden(
                "principal-ownership-change",
            ));
        }

        // Authentication refresh is a transaction over the peer's complete
        // resource authority. Ownership equality alone is insufficient: a
        // same-owner credential may have lost the scope, tenant grant, or
        // other policy that authorized an already-bound Session. Validate the
        // candidate against every retained binding before publishing either
        // the new owner or principal. On any failure the previous principal
        // remains authoritative, matching the coordinator's retryable refresh
        // contract and preventing existing media routes from inheriting a
        // reduced-scope credential.
        let sessions = self
            .sessions
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();
        for (wire, canonical) in sessions {
            self.resolver
                .reauthorize_session(&principal, &wire, &canonical)?;
        }

        *owner = Some(incoming_owner);
        *self.principal.write() = Some(principal);
        Ok(())
    }

    pub fn bind_session(
        &self,
        wire_session: &SessionId,
    ) -> Result<SessionId, ResourceBindingError> {
        let _mutation = self.mutation.lock();
        self.bind_session_locked(wire_session)
    }

    fn bind_session_locked(
        &self,
        wire_session: &SessionId,
    ) -> Result<SessionId, ResourceBindingError> {
        let principal = self.active_principal_locked()?;
        if let Some(existing) = self.sessions.get(wire_session) {
            let canonical = existing.value().clone();
            self.resolver
                .reauthorize_session(&principal, wire_session, &canonical)?;
            return Ok(canonical);
        }
        let canonical = self.resolver.resolve_session(&principal, wire_session)?;
        self.sessions
            .insert(wire_session.clone(), canonical.clone());
        Ok(canonical)
    }

    /// Revalidate one existing wire-to-canonical Session binding.
    pub fn reauthorize_bound_session(
        &self,
        wire_session: &SessionId,
    ) -> Result<(), ResourceBindingError> {
        let _mutation = self.mutation.lock();
        let Some(canonical) = self
            .sessions
            .get(wire_session)
            .map(|entry| entry.value().clone())
        else {
            return Ok(());
        };
        let principal = self.active_principal_locked()?;
        self.resolver
            .reauthorize_session(&principal, wire_session, &canonical)
    }

    /// Resolve an application routing hint only after authentication and an
    /// exact Session binding have both succeeded.
    pub fn inbound_routing_hint(
        &self,
        wire_session: &SessionId,
        intent: &str,
        capabilities_offer: &serde_json::Value,
    ) -> Result<Option<InboundRoutingHint>, ResourceBindingError> {
        let _mutation = self.mutation.lock();
        let principal = self.active_principal_locked()?;
        let canonical = self.bind_session_locked(wire_session)?;
        self.resolver
            .reauthorize_session(&principal, wire_session, &canonical)?;
        self.resolver.resolve_inbound_routing_hint(
            &principal,
            wire_session,
            intent,
            capabilities_offer,
        )
    }

    /// Revalidate all retained Session grants. An empty pre-auth peer remains
    /// pending; once any Session is bound, authority loss fails closed.
    pub fn reauthorize_all_sessions(&self) -> Result<(), ResourceBindingError> {
        let _mutation = self.mutation.lock();
        let sessions = self
            .sessions
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            return Ok(());
        }
        let principal = self.active_principal_locked()?;
        for (wire, canonical) in sessions {
            self.resolver
                .reauthorize_session(&principal, &wire, &canonical)?;
        }
        Ok(())
    }

    fn active_principal_locked(&self) -> Result<AuthenticatedPrincipal, ResourceBindingError> {
        let principal = self
            .principal
            .read()
            .clone()
            .ok_or_else(|| ResourceBindingError::forbidden("peer-not-authenticated"))?;
        if principal.is_expired_at(chrono::Utc::now()) {
            return Err(ResourceBindingError::forbidden("principal-expired"));
        }
        Ok(principal)
    }

    pub fn bind_connection(
        &self,
        wire_session: &SessionId,
        wire_connection: &ConnectionId,
        core_connection: ConnectionId,
    ) -> Result<(), ResourceBindingError> {
        let _mutation = self.mutation.lock();
        let session_preexisting = self.sessions.contains_key(wire_session);
        self.bind_session_locked(wire_session)?;
        let key = (wire_session.clone(), wire_connection.clone());
        if let Some(existing) = self.connections.get(&key) {
            if existing.value() != &core_connection {
                drop(existing);
                if !session_preexisting {
                    self.sessions.remove(wire_session);
                }
                return Err(ResourceBindingError::forbidden(
                    "wire-connection-already-bound",
                ));
            }
            return Ok(());
        }
        if let Some(mut reverse) = self.core_connections.get_mut(&core_connection) {
            if &reverse.0 != wire_session {
                drop(reverse);
                if !session_preexisting {
                    self.sessions.remove(wire_session);
                }
                return Err(ResourceBindingError::forbidden(
                    "core-connection-bound-to-another-session",
                ));
            }
            reverse.1.insert(wire_connection.clone());
        } else {
            self.core_connections.insert(
                core_connection.clone(),
                (
                    wire_session.clone(),
                    HashSet::from([wire_connection.clone()]),
                ),
            );
        }
        self.connections.insert(key, core_connection);
        Ok(())
    }

    pub fn core_session(&self, wire_session: &SessionId) -> Option<SessionId> {
        self.sessions
            .get(wire_session)
            .map(|entry| entry.value().clone())
    }

    pub fn core_connection(
        &self,
        wire_session: &SessionId,
        wire_connection: &ConnectionId,
    ) -> Option<ConnectionId> {
        self.connections
            .get(&(wire_session.clone(), wire_connection.clone()))
            .map(|entry| entry.value().clone())
    }

    pub fn remove_connection(
        &self,
        wire_session: &SessionId,
        wire_connection: &ConnectionId,
    ) -> Option<ConnectionId> {
        let _mutation = self.mutation.lock();
        self.remove_connection_locked(wire_session, wire_connection)
            .map(|removed| removed.core_connection)
    }

    /// Remove one exact wire mapping and prune its Session row only after the
    /// final sibling Connection is gone. The canonical pair is retained for
    /// downstream registry cleanup after the untrusted mapping is no longer
    /// reachable.
    fn remove_connection_locked(
        &self,
        wire_session: &SessionId,
        wire_connection: &ConnectionId,
    ) -> Option<RemovedConnectionBinding> {
        let core_session = self
            .sessions
            .get(wire_session)
            .map(|entry| entry.value().clone());
        let core_connection = self
            .connections
            .remove(&(wire_session.clone(), wire_connection.clone()))
            .map(|(_, core)| core)?;
        let remove_reverse =
            if let Some(mut reverse) = self.core_connections.get_mut(&core_connection) {
                reverse.1.remove(wire_connection);
                reverse.1.is_empty()
            } else {
                // The forward row is authoritative for cleanup. A missing
                // reverse row must not suppress downstream release and leak
                // the core Connection's registry resources.
                true
            };
        if remove_reverse {
            self.core_connections.remove(&core_connection);
        }
        let has_sibling = self
            .connections
            .iter()
            .any(|entry| &entry.key().0 == wire_session);
        if !has_sibling {
            self.sessions.remove(wire_session);
        }
        core_session.map(|session| RemovedConnectionBinding {
            core_session: session,
            core_connection,
            release_core_connection: remove_reverse,
        })
    }

    pub fn remove_session(&self, wire_session: &SessionId) -> Option<SessionId> {
        let _mutation = self.mutation.lock();
        let removed_connections = self
            .connections
            .iter()
            .filter(|entry| &entry.key().0 == wire_session)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();
        for (wire, core) in removed_connections {
            self.connections.remove(&wire);
            self.core_connections.remove(&core);
        }
        self.sessions.remove(wire_session).map(|(_, core)| core)
    }

    pub fn clear(&self) {
        let _mutation = self.mutation.lock();
        self.core_connections.clear();
        self.connections.clear();
        self.sessions.clear();
    }
}

/// Translates authenticated peer-local wire IDs into canonical registry IDs
/// before delegating to a process-wide [`SubscriptionHandler`].
pub struct BoundSubscriptionHandler {
    bindings: Arc<PeerResourceBindings>,
    inner: Arc<dyn SubscriptionHandler>,
}

impl BoundSubscriptionHandler {
    pub fn new(
        bindings: Arc<PeerResourceBindings>,
        inner: Arc<dyn SubscriptionHandler>,
    ) -> Arc<Self> {
        Arc::new(Self { bindings, inner })
    }

    pub fn bindings(&self) -> Arc<PeerResourceBindings> {
        Arc::clone(&self.bindings)
    }

    fn with_resolved<T>(
        &self,
        sid: &SessionId,
        connection: &ConnectionId,
        operation: impl FnOnce(&SessionId, &ConnectionId) -> T,
    ) -> Result<T, ResourceBindingError> {
        // Hold the peer mutation barrier through the synchronous registry
        // operation. Teardown waits for an in-flight subscribe to finish,
        // removes the mapping, and then cleans the registry; a later request
        // cannot slip between cleanup and mapping removal.
        let _mutation = self.bindings.mutation.lock();
        let principal = self.bindings.active_principal_locked()?;
        let core_session = self
            .bindings
            .core_session(sid)
            .ok_or_else(|| ResourceBindingError::unavailable("session-binding-not-ready"))?;
        let core_connection = self
            .bindings
            .core_connection(sid, connection)
            .ok_or_else(|| ResourceBindingError::unavailable("connection-binding-not-ready"))?;
        self.bindings.resolver.reauthorize_connection(
            &principal,
            sid,
            &core_session,
            connection,
            &core_connection,
        )?;
        Ok(operation(&core_session, &core_connection))
    }
}

impl SubscriptionHandler for BoundSubscriptionHandler {
    fn subscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamSubscribe,
    ) -> SubscriptionOutcome {
        match self.with_resolved(sid, subscriber, |session, connection| {
            self.inner.subscribe(session, connection, request)
        }) {
            Ok(outcome) => outcome,
            Err(error) => error.subscription_outcome(),
        }
    }

    fn unsubscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamUnsubscribe,
    ) -> SubscriptionOutcome {
        match self.with_resolved(sid, subscriber, |session, connection| {
            self.inner.unsubscribe(session, connection, request)
        }) {
            Ok(outcome) => outcome,
            Err(error) => error.subscription_outcome(),
        }
    }

    fn register_publisher(&self, info: PublisherInfo<'_>) {
        let _ = self.with_resolved(info.sid, info.connection, |session, connection| {
            self.inner.register_publisher(PublisherInfo {
                sid: session,
                strm_id: info.strm_id,
                connection,
                participant: info.participant,
                kind: info.kind,
                codec: info.codec,
            });
        });
    }

    fn unregister_publisher(&self, sid: &SessionId, strm_id: &str, publisher: &ConnectionId) {
        let _mutation = self.bindings.mutation.lock();
        if let (Some(session), Some(connection)) = (
            self.bindings.core_session(sid),
            self.bindings.core_connection(sid, publisher),
        ) {
            self.inner
                .unregister_publisher(&session, strm_id, &connection);
        }
    }

    fn unregister_connection(&self, sid: &SessionId, connid: &ConnectionId) {
        let removed = {
            let _mutation = self.bindings.mutation.lock();
            self.bindings.remove_connection_locked(sid, connid)
        };
        if let Some(removed) = removed {
            if removed.release_core_connection {
                self.inner
                    .unregister_connection(&removed.core_session, &removed.core_connection);
            }
        }
    }
}

/// Peer-local namespace wrapper for a shared production handler. Wire Session
/// and Connection IDs are supplied by the remote peer, so they must not be
/// used as process-global registry keys without an authenticated peer scope.
pub struct NamespacedSubscriptionHandler {
    namespace: String,
    inner: Arc<dyn SubscriptionHandler>,
}

impl NamespacedSubscriptionHandler {
    pub fn new(namespace: impl Into<String>, inner: Arc<dyn SubscriptionHandler>) -> Arc<Self> {
        Arc::new(Self {
            namespace: namespace.into(),
            inner,
        })
    }

    fn session(&self, sid: &SessionId) -> SessionId {
        SessionId::from_string(format!("{}:{}", self.namespace, sid))
    }

    fn connection(&self, connid: &ConnectionId) -> ConnectionId {
        ConnectionId::from_string(format!("{}:{}", self.namespace, connid))
    }
}

impl SubscriptionHandler for NamespacedSubscriptionHandler {
    fn subscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamSubscribe,
    ) -> SubscriptionOutcome {
        self.inner
            .subscribe(&self.session(sid), &self.connection(subscriber), request)
    }

    fn unsubscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamUnsubscribe,
    ) -> SubscriptionOutcome {
        self.inner
            .unsubscribe(&self.session(sid), &self.connection(subscriber), request)
    }

    fn register_publisher(&self, info: PublisherInfo<'_>) {
        let sid = self.session(info.sid);
        let connection = self.connection(info.connection);
        self.inner.register_publisher(PublisherInfo {
            sid: &sid,
            strm_id: info.strm_id,
            connection: &connection,
            participant: info.participant,
            kind: info.kind,
            codec: info.codec,
        });
    }

    fn unregister_publisher(&self, sid: &SessionId, strm_id: &str, publisher: &ConnectionId) {
        self.inner
            .unregister_publisher(&self.session(sid), strm_id, &self.connection(publisher));
    }

    fn unregister_connection(&self, sid: &SessionId, connid: &ConnectionId) {
        self.inner
            .unregister_connection(&self.session(sid), &self.connection(connid));
    }
}

/// Bundle passed to [`SubscriptionHandler::register_publisher`]. Carries
/// everything the orchestrator needs to resolve `strm_id` and
/// `from_participant` subscription forms; future fields land here
/// without breaking the trait surface.
pub struct PublisherInfo<'a> {
    pub sid: &'a SessionId,
    pub strm_id: &'a str,
    pub connection: &'a ConnectionId,
    pub participant: &'a str,
    pub kind: &'a str,
    /// The codec the publisher negotiated for this Stream (the chosen
    /// codec out of [`rvoip_core::capability::negotiate_streams`]'s
    /// answer). Propagated to the `PublisherRegistry` so
    /// [`rvoip_core::Orchestrator::fanout_frame`] can hand the right
    /// `CodecInfo` to the subscriber-side adapter when allocating a
    /// fresh per-subscription MediaStream (plan B1 / MP3c).
    pub codec: Option<rvoip_core::capability::CodecInfo>,
}

/// Default handler — every request is rejected with `501 not-implemented`
/// (`multi-party-routing-not-implemented`). The receiver recognized the
/// envelope type but lacks the wiring to service it; another build of
/// the same server might. Used when no handler is configured.
///
/// Pre-v0.x servers conflated `501` and `501` as `501`; per
/// `CONVERSATION_PROTOCOL.md` §11.2 these are now distinct.
pub struct RejectingHandler;

impl SubscriptionHandler for RejectingHandler {
    fn subscribe(
        &self,
        _: &SessionId,
        _: &ConnectionId,
        _: &StreamSubscribe,
    ) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(501, "multi-party-routing-not-implemented")
    }

    fn unsubscribe(
        &self,
        _: &SessionId,
        _: &ConnectionId,
        _: &StreamUnsubscribe,
    ) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(501, "multi-party-routing-not-implemented")
    }
}

/// Convenience: wrap the default rejecting handler in an `Arc`.
pub fn rejecting_handler() -> Arc<dyn SubscriptionHandler> {
    Arc::new(RejectingHandler)
}

#[cfg(test)]
mod namespace_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RevocableResolver {
        active: Arc<AtomicBool>,
        canonical: SessionId,
    }

    struct ScopeBoundResolver {
        canonical: SessionId,
    }

    struct ExactConnectionResolver {
        canonical: SessionId,
        allowed_wire_connection: ConnectionId,
    }

    struct PrivateHintResolver;

    impl SessionBindingResolver for PrivateHintResolver {
        fn resolve_session(
            &self,
            _principal: &AuthenticatedPrincipal,
            wire_session: &SessionId,
        ) -> Result<SessionId, ResourceBindingError> {
            Ok(wire_session.clone())
        }

        fn resolve_inbound_routing_hint(
            &self,
            _principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
            intent: &str,
            capabilities_offer: &serde_json::Value,
        ) -> Result<Option<InboundRoutingHint>, ResourceBindingError> {
            if intent != "private" {
                return Err(ResourceBindingError::forbidden("private-intent-required"));
            }
            let value = capabilities_offer
                .get("private_hint")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ResourceBindingError::forbidden("private-hint-required"))?;
            InboundRoutingHint::new(value.to_owned())
                .map(Some)
                .map_err(|_| ResourceBindingError::forbidden("invalid-private-hint"))
        }
    }

    impl SessionBindingResolver for RevocableResolver {
        fn resolve_session(
            &self,
            _principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
        ) -> Result<SessionId, ResourceBindingError> {
            if self.active.load(Ordering::Acquire) {
                Ok(self.canonical.clone())
            } else {
                Err(ResourceBindingError::forbidden("session-revoked"))
            }
        }

        fn reauthorize_session(
            &self,
            _principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
            canonical_session: &SessionId,
        ) -> Result<(), ResourceBindingError> {
            if canonical_session != &self.canonical {
                return Err(ResourceBindingError::forbidden("session-mismatch"));
            }
            if self.active.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(ResourceBindingError::forbidden("session-revoked"))
            }
        }
    }

    impl SessionBindingResolver for ScopeBoundResolver {
        fn resolve_session(
            &self,
            principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
        ) -> Result<SessionId, ResourceBindingError> {
            principal
                .require_scope("private:forward")
                .map_err(|_| ResourceBindingError::forbidden("forwarding-scope-required"))?;
            Ok(self.canonical.clone())
        }

        fn reauthorize_session(
            &self,
            principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
            canonical_session: &SessionId,
        ) -> Result<(), ResourceBindingError> {
            if canonical_session != &self.canonical {
                return Err(ResourceBindingError::forbidden("session-mismatch"));
            }
            principal
                .require_scope("private:forward")
                .map_err(|_| ResourceBindingError::forbidden("forwarding-scope-required"))
        }
    }

    impl SessionBindingResolver for ExactConnectionResolver {
        fn resolve_session(
            &self,
            _principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
        ) -> Result<SessionId, ResourceBindingError> {
            Ok(self.canonical.clone())
        }

        fn reauthorize_connection(
            &self,
            _principal: &AuthenticatedPrincipal,
            _wire_session: &SessionId,
            canonical_session: &SessionId,
            wire_connection: &ConnectionId,
            _core_connection: &ConnectionId,
        ) -> Result<(), ResourceBindingError> {
            if canonical_session == &self.canonical
                && wire_connection == &self.allowed_wire_connection
            {
                Ok(())
            } else {
                Err(ResourceBindingError::forbidden(
                    "exact-connection-authority-required",
                ))
            }
        }
    }

    #[derive(Default)]
    struct RecordingHandler {
        subscriptions: parking_lot::Mutex<Vec<(SessionId, ConnectionId)>>,
        unsubscriptions: parking_lot::Mutex<Vec<(SessionId, ConnectionId)>>,
        publishers: parking_lot::Mutex<Vec<(SessionId, ConnectionId, String)>>,
        removed_publishers: parking_lot::Mutex<Vec<(SessionId, ConnectionId, String)>>,
        removed: parking_lot::Mutex<Vec<(SessionId, ConnectionId)>>,
    }

    impl SubscriptionHandler for RecordingHandler {
        fn subscribe(
            &self,
            sid: &SessionId,
            subscriber: &ConnectionId,
            _: &StreamSubscribe,
        ) -> SubscriptionOutcome {
            self.subscriptions
                .lock()
                .push((sid.clone(), subscriber.clone()));
            SubscriptionOutcome::Ok
        }

        fn unsubscribe(
            &self,
            sid: &SessionId,
            subscriber: &ConnectionId,
            _: &StreamUnsubscribe,
        ) -> SubscriptionOutcome {
            self.unsubscriptions
                .lock()
                .push((sid.clone(), subscriber.clone()));
            SubscriptionOutcome::Ok
        }

        fn register_publisher(&self, info: PublisherInfo<'_>) {
            self.publishers.lock().push((
                info.sid.clone(),
                info.connection.clone(),
                info.strm_id.to_owned(),
            ));
        }

        fn unregister_publisher(&self, sid: &SessionId, strm_id: &str, publisher: &ConnectionId) {
            self.removed_publishers.lock().push((
                sid.clone(),
                publisher.clone(),
                strm_id.to_string(),
            ));
        }

        fn unregister_connection(&self, sid: &SessionId, connid: &ConnectionId) {
            self.removed.lock().push((sid.clone(), connid.clone()));
        }
    }

    #[test]
    fn identical_wire_ids_from_two_peers_map_to_distinct_registry_ids() {
        let peer_a = NamespacedSubscriptionHandler::new("peer-a", rejecting_handler());
        let peer_b = NamespacedSubscriptionHandler::new("peer-b", rejecting_handler());
        let sid = SessionId::from_string("shared-sid");
        let connid = ConnectionId::from_string("shared-connid");

        assert_ne!(peer_a.session(&sid), peer_b.session(&sid));
        assert_ne!(peer_a.connection(&connid), peer_b.connection(&connid));
    }

    #[test]
    fn routing_hints_are_explicitly_opt_in_bounded_and_session_authorized() {
        let wire_session = SessionId::from_string("private-session");
        let default = PeerResourceBindings::new(PeerScopedSessionResolver::new("peer"));
        default
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        assert!(default
            .inbound_routing_hint(
                &wire_session,
                "private",
                &serde_json::json!({"private_hint": "credential"}),
            )
            .unwrap()
            .is_none());

        let private = PeerResourceBindings::new(Arc::new(PrivateHintResolver));
        private
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let hint = private
            .inbound_routing_hint(
                &wire_session,
                "private",
                &serde_json::json!({"private_hint": "credential"}),
            )
            .unwrap()
            .expect("private resolver opted in");
        assert_eq!(hint.expose_secret(), "credential");
        assert!(private
            .inbound_routing_hint(
                &wire_session,
                "public",
                &serde_json::json!({"private_hint": "credential"}),
            )
            .is_err());
        assert!(private
            .inbound_routing_hint(
                &wire_session,
                "private",
                &serde_json::json!({"private_hint": "x".repeat(rvoip_core::adapter::MAX_INBOUND_ROUTING_HINT_BYTES + 1)}),
            )
            .is_err());
    }

    #[test]
    fn scope_dropping_refresh_is_rejected_before_existing_media_authority_changes() {
        let wire_session = SessionId::from_string("wire-session");
        let canonical_session = SessionId::from_string("core-session");
        let bindings = PeerResourceBindings::new(Arc::new(ScopeBoundResolver {
            canonical: canonical_session.clone(),
        }));
        let mut initial = AuthenticatedPrincipal::anonymous();
        initial.scopes = vec!["private:forward".into()];
        bindings.authenticate(initial.clone()).unwrap();
        assert_eq!(
            bindings.bind_session(&wire_session).unwrap(),
            canonical_session
        );

        let mut reduced = initial;
        reduced.scopes.clear();
        assert_eq!(
            bindings.authenticate(reduced).unwrap_err(),
            ResourceBindingError::forbidden("forwarding-scope-required")
        );

        // The rejected candidate never becomes visible to the already-bound
        // datagram/media route. Its prior principal remains authoritative and
        // the exact cached binding still reauthorizes successfully.
        assert_eq!(
            bindings
                .principal
                .read()
                .as_ref()
                .expect("prior principal retained")
                .scopes,
            vec!["private:forward"]
        );
        assert_eq!(
            bindings.bind_session(&wire_session).unwrap(),
            canonical_session
        );
    }

    #[test]
    fn bound_handler_never_delegates_unbound_wire_ids() {
        let resolver = PeerScopedSessionResolver::new("peer-a");
        let bindings = PeerResourceBindings::new(resolver);
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let inner = Arc::new(RecordingHandler::default());
        let handler = BoundSubscriptionHandler::new(bindings, inner.clone());

        let outcome = handler.subscribe(
            &SessionId::from_string("wire-session"),
            &ConnectionId::from_string("wire-connection"),
            &StreamSubscribe {
                by_participant: "listener".into(),
                subscriptions: Vec::new(),
            },
        );

        assert_eq!(
            outcome,
            SubscriptionOutcome::reject(503, "session-binding-not-ready")
        );
        assert!(inner.subscriptions.lock().is_empty());
    }

    #[test]
    fn exact_connection_reauthorization_blocks_subscribe_and_unsubscribe_mutations() {
        let wire_session = SessionId::from_string("wire-session");
        let allowed_wire_connection = ConnectionId::from_string("allowed-wire-connection");
        let sibling_wire_connection = ConnectionId::from_string("sibling-wire-connection");
        let resolver = Arc::new(ExactConnectionResolver {
            canonical: SessionId::from_string("core-session"),
            allowed_wire_connection,
        });
        let bindings = PeerResourceBindings::new(resolver);
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        bindings
            .bind_connection(
                &wire_session,
                &sibling_wire_connection,
                ConnectionId::from_string("sibling-core-connection"),
            )
            .unwrap();
        let inner = Arc::new(RecordingHandler::default());
        let handler = BoundSubscriptionHandler::new(bindings, inner.clone());

        assert_eq!(
            handler.subscribe(
                &wire_session,
                &sibling_wire_connection,
                &StreamSubscribe {
                    by_participant: "listener".into(),
                    subscriptions: Vec::new(),
                },
            ),
            SubscriptionOutcome::reject(403, "exact-connection-authority-required")
        );
        assert_eq!(
            handler.unsubscribe(
                &wire_session,
                &sibling_wire_connection,
                &StreamUnsubscribe {
                    strm_ids: vec!["audio/main".into()],
                },
            ),
            SubscriptionOutcome::reject(403, "exact-connection-authority-required")
        );
        assert!(inner.subscriptions.lock().is_empty());
        assert!(inner.unsubscriptions.lock().is_empty());
    }

    #[test]
    fn bound_handler_translates_and_removes_exact_core_resources() {
        let canonical_session = SessionId::from_string("core-session");
        let resolver: Arc<dyn SessionBindingResolver> = Arc::new({
            let canonical_session = canonical_session.clone();
            move |_: &AuthenticatedPrincipal, _: &SessionId| Ok(canonical_session.clone())
        });
        let bindings = PeerResourceBindings::new(resolver);
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let wire_session = SessionId::from_string("wire-session");
        let wire_connection = ConnectionId::from_string("wire-connection");
        let core_connection = ConnectionId::from_string("core-connection");
        bindings
            .bind_connection(&wire_session, &wire_connection, core_connection.clone())
            .unwrap();

        let inner = Arc::new(RecordingHandler::default());
        let handler = BoundSubscriptionHandler::new(bindings.clone(), inner.clone());
        assert_eq!(
            handler.subscribe(
                &wire_session,
                &wire_connection,
                &StreamSubscribe {
                    by_participant: "listener".into(),
                    subscriptions: Vec::new(),
                },
            ),
            SubscriptionOutcome::Ok
        );
        handler.register_publisher(PublisherInfo {
            sid: &wire_session,
            strm_id: "audio/main",
            connection: &wire_connection,
            participant: "publisher",
            kind: "audio",
            codec: None,
        });
        handler.unregister_publisher(&wire_session, "audio/main", &wire_connection);
        handler.unregister_connection(&wire_session, &wire_connection);

        assert_eq!(
            *inner.subscriptions.lock(),
            vec![(canonical_session.clone(), core_connection.clone())]
        );
        assert_eq!(
            *inner.publishers.lock(),
            vec![(
                canonical_session.clone(),
                core_connection.clone(),
                "audio/main".into()
            )]
        );
        assert_eq!(
            *inner.removed.lock(),
            vec![(canonical_session, core_connection)]
        );
        assert_eq!(
            *inner.removed_publishers.lock(),
            vec![(
                SessionId::from_string("core-session"),
                ConnectionId::from_string("core-connection"),
                "audio/main".to_string(),
            )]
        );
        assert!(bindings
            .core_connection(&wire_session, &wire_connection)
            .is_none());
        assert!(bindings.core_session(&wire_session).is_none());
    }

    #[test]
    fn resolver_can_authorize_two_peers_into_one_canonical_session() {
        let canonical_session = SessionId::from_string("tenant-call-42");
        let resolver: Arc<dyn SessionBindingResolver> = Arc::new({
            let canonical_session = canonical_session.clone();
            move |_: &AuthenticatedPrincipal, _: &SessionId| Ok(canonical_session.clone())
        });
        let peer_a = PeerResourceBindings::new(resolver.clone());
        let peer_b = PeerResourceBindings::new(resolver);
        peer_a
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        peer_b
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();

        let wire_session_a = SessionId::from_string("attachment-token-a");
        let wire_session_b = SessionId::from_string("attachment-token-b");
        let wire_connection_a = ConnectionId::from_string("wire-a");
        let wire_connection_b = ConnectionId::from_string("wire-b");
        let core_connection_a = ConnectionId::from_string("core-a");
        let core_connection_b = ConnectionId::from_string("core-b");
        peer_a
            .bind_connection(
                &wire_session_a,
                &wire_connection_a,
                core_connection_a.clone(),
            )
            .unwrap();
        peer_b
            .bind_connection(
                &wire_session_b,
                &wire_connection_b,
                core_connection_b.clone(),
            )
            .unwrap();

        let inner = Arc::new(RecordingHandler::default());
        let handler_a = BoundSubscriptionHandler::new(peer_a, inner.clone());
        let handler_b = BoundSubscriptionHandler::new(peer_b, inner.clone());
        let request = StreamSubscribe {
            by_participant: "listener".into(),
            subscriptions: Vec::new(),
        };
        assert_eq!(
            handler_a.subscribe(&wire_session_a, &wire_connection_a, &request),
            SubscriptionOutcome::Ok
        );
        assert_eq!(
            handler_b.subscribe(&wire_session_b, &wire_connection_b, &request),
            SubscriptionOutcome::Ok
        );
        assert_eq!(
            *inner.subscriptions.lock(),
            vec![
                (canonical_session.clone(), core_connection_a),
                (canonical_session, core_connection_b),
            ]
        );
    }

    #[test]
    fn cached_bindings_and_handler_operations_reject_expired_principal() {
        let canonical_session = SessionId::from_string("core-session");
        let resolver: Arc<dyn SessionBindingResolver> = Arc::new({
            let canonical_session = canonical_session.clone();
            move |_: &AuthenticatedPrincipal, _: &SessionId| Ok(canonical_session.clone())
        });
        let bindings = PeerResourceBindings::new(resolver);
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let wire_session = SessionId::from_string("wire-session");
        let wire_connection = ConnectionId::from_string("wire-connection");
        bindings
            .bind_connection(
                &wire_session,
                &wire_connection,
                ConnectionId::from_string("core-connection"),
            )
            .unwrap();
        bindings
            .principal
            .write()
            .as_mut()
            .expect("authenticated principal")
            .expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));

        assert_eq!(
            bindings.bind_session(&wire_session).unwrap_err(),
            ResourceBindingError::forbidden("principal-expired")
        );
        let inner = Arc::new(RecordingHandler::default());
        let handler = BoundSubscriptionHandler::new(bindings, inner.clone());
        assert_eq!(
            handler.subscribe(
                &wire_session,
                &wire_connection,
                &StreamSubscribe {
                    by_participant: "listener".into(),
                    subscriptions: Vec::new(),
                },
            ),
            SubscriptionOutcome::reject(403, "principal-expired")
        );
        assert!(inner.subscriptions.lock().is_empty());
    }

    #[test]
    fn sibling_wire_connections_share_one_core_leg_and_cleanup_exactly() {
        let canonical_session = SessionId::from_string("core-session");
        let resolver: Arc<dyn SessionBindingResolver> = Arc::new({
            let canonical_session = canonical_session.clone();
            move |_: &AuthenticatedPrincipal, _: &SessionId| Ok(canonical_session.clone())
        });
        let bindings = PeerResourceBindings::new(resolver);
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let wire_session = SessionId::from_string("wire-session");
        let sibling_a = ConnectionId::from_string("wire-a");
        let sibling_b = ConnectionId::from_string("wire-b");
        let core_connection = ConnectionId::from_string("core-leg");
        bindings
            .bind_connection(&wire_session, &sibling_a, core_connection.clone())
            .unwrap();
        bindings
            .bind_connection(&wire_session, &sibling_b, core_connection.clone())
            .unwrap();

        let other_session = SessionId::from_string("other-wire-session");
        assert_eq!(
            bindings
                .bind_connection(
                    &other_session,
                    &ConnectionId::from_string("wire-c"),
                    core_connection.clone(),
                )
                .unwrap_err(),
            ResourceBindingError::forbidden("core-connection-bound-to-another-session")
        );
        assert!(
            bindings.core_session(&other_session).is_none(),
            "failed cross-Session alias must roll back its empty Session row"
        );

        let inner = Arc::new(RecordingHandler::default());
        let handler = BoundSubscriptionHandler::new(bindings.clone(), inner.clone());
        handler.unregister_connection(&wire_session, &sibling_a);
        handler.unregister_connection(&wire_session, &sibling_a);
        assert!(bindings
            .core_connection(&wire_session, &sibling_a)
            .is_none());
        assert_eq!(
            bindings.core_connection(&wire_session, &sibling_b),
            Some(core_connection.clone())
        );
        assert_eq!(
            bindings.core_session(&wire_session),
            Some(canonical_session.clone())
        );

        handler.unregister_connection(&wire_session, &sibling_b);
        assert!(bindings
            .core_connection(&wire_session, &sibling_b)
            .is_none());
        assert!(bindings.core_session(&wire_session).is_none());
        assert_eq!(
            *inner.removed.lock(),
            vec![(canonical_session, core_connection)]
        );
    }

    #[test]
    fn remove_session_releases_reverse_rows_without_touching_siblings() {
        let bindings = PeerResourceBindings::new(PeerScopedSessionResolver::new("peer"));
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let session_a = SessionId::from_string("session-a");
        let session_b = SessionId::from_string("session-b");
        let wire_a = ConnectionId::from_string("wire-a");
        let wire_b = ConnectionId::from_string("wire-b");
        let core_a = ConnectionId::from_string("core-a");
        let core_b = ConnectionId::from_string("core-b");
        bindings
            .bind_connection(&session_a, &wire_a, core_a.clone())
            .unwrap();
        bindings
            .bind_connection(&session_b, &wire_b, core_b.clone())
            .unwrap();

        assert!(bindings.remove_session(&session_a).is_some());
        assert!(bindings.core_session(&session_a).is_none());
        assert!(bindings.core_connection(&session_a, &wire_a).is_none());
        assert_eq!(bindings.core_connection(&session_b, &wire_b), Some(core_b));
        bindings
            .bind_connection(
                &session_a,
                &ConnectionId::from_string("wire-a-reused"),
                core_a,
            )
            .expect("reverse row for removed Session is released");
    }

    #[test]
    fn principal_refresh_cannot_change_resource_owner() {
        let bindings = PeerResourceBindings::new(PeerScopedSessionResolver::new("peer"));
        let mut first = AuthenticatedPrincipal::anonymous();
        first.subject = "alice".into();
        first.tenant = Some("tenant-a".into());
        bindings.authenticate(first).unwrap();

        let mut other = AuthenticatedPrincipal::anonymous();
        other.subject = "alice".into();
        other.tenant = Some("tenant-b".into());
        assert_eq!(
            bindings.authenticate(other).unwrap_err(),
            ResourceBindingError::forbidden("principal-ownership-change")
        );
    }

    #[test]
    fn cached_session_and_bound_handler_fail_closed_after_authority_revocation() {
        let active = Arc::new(AtomicBool::new(true));
        let canonical = SessionId::from_string("canonical");
        let resolver = Arc::new(RevocableResolver {
            active: Arc::clone(&active),
            canonical: canonical.clone(),
        });
        let bindings = PeerResourceBindings::new(resolver);
        bindings
            .authenticate(AuthenticatedPrincipal::anonymous())
            .unwrap();
        let wire_session = SessionId::from_string("wire");
        let wire_connection = ConnectionId::from_string("wire-connection");
        bindings
            .bind_connection(
                &wire_session,
                &wire_connection,
                ConnectionId::from_string("core-connection"),
            )
            .unwrap();
        assert_eq!(bindings.bind_session(&wire_session).unwrap(), canonical);

        active.store(false, Ordering::Release);
        assert_eq!(
            bindings.bind_session(&wire_session).unwrap_err(),
            ResourceBindingError::forbidden("session-revoked")
        );
        assert_eq!(
            bindings.reauthorize_all_sessions().unwrap_err(),
            ResourceBindingError::forbidden("session-revoked")
        );
        let inner = Arc::new(RecordingHandler::default());
        let handler = BoundSubscriptionHandler::new(bindings, inner.clone());
        assert_eq!(
            handler.subscribe(
                &wire_session,
                &wire_connection,
                &StreamSubscribe {
                    by_participant: "listener".into(),
                    subscriptions: Vec::new(),
                },
            ),
            SubscriptionOutcome::reject(403, "session-revoked")
        );
        assert!(inner.subscriptions.lock().is_empty());
    }
}
