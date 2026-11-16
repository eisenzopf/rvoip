//! Outbound origination that depends on one exact live peer connection.
//!
//! A contact center that calls a customer back on behalf of an agent wants
//! the outbound call to exist only while the agent's own connection (for
//! example a browser leg) is still up. An [`OutboundPeerLease`] ties one
//! outbound dispatch to that exact connection lifecycle generation:
//!
//! - [`Orchestrator::reserve_outbound_peer`] reserves the dependency on a
//!   live, operationally connected connection. A connection holds at most one
//!   live lease at a time.
//! - The outbound adapter calls [`OutboundPeerLease::try_begin_dispatch`]
//!   immediately before the first packet can leave. It succeeds once, and
//!   only while the peer generation is still current; a `false` means no
//!   packet was sent.
//! - The peer's retirement (end, failure, rejection) or an explicit
//!   [`Orchestrator::revoke_outbound_peer`] revokes the lease. Before
//!   dispatch that is zero-wire; after dispatch the adapter owns
//!   cancellation of the call through [`OutboundPeerLease::revocation`].
use super::{
    ConnectionAdapter, ConnectionId, ConnectionLifecycleState, ConnectionLifecycleTicket,
    Orchestrator, Ordering, Result, RvoipError,
};
use std::sync::{atomic::AtomicBool, Arc, Weak};
use std::time::Instant;
use tokio::sync::watch;

/// One outbound dispatch's dependency on an exact live peer connection
/// generation. Clones share the same one-shot dispatch and revocation state.
/// Only the owning [`Orchestrator`] can create one.
#[derive(Clone)]
pub struct OutboundPeerLease {
    state: Arc<OutboundPeerLeaseState>,
}

pub(super) struct OutboundPeerLeaseState {
    owner: Weak<Orchestrator>,
    lifecycle: ConnectionLifecycleTicket,
    adapter: Arc<dyn ConnectionAdapter>,
    dispatched: AtomicBool,
    dispatch_not_after: Option<Instant>,
    revoked: watch::Sender<bool>,
}

impl std::fmt::Debug for OutboundPeerLeaseState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundPeerLease")
            .field("dispatched", &self.dispatched.load(Ordering::Acquire))
            .field("revoked", &*self.revoked.borrow())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for OutboundPeerLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.state, f)
    }
}

impl OutboundPeerLeaseState {
    fn revoke(&self) {
        self.revoked.send_replace(true);
    }
}

impl ConnectionLifecycleState {
    /// Called under the lifecycle lock whenever the connection retires.
    pub(super) fn revoke_outbound_peer_dependency(&self) {
        if let Some(dependency) = self
            .outbound_peer_dependency
            .as_ref()
            .and_then(Weak::upgrade)
        {
            dependency.revoke();
        }
    }
}

impl OutboundPeerLease {
    /// Claim the one dispatch this lease allows. Returns `true` at most once,
    /// and only while the peer connection's generation is still current,
    /// active, operationally connected and live at its adapter, the lease is
    /// not revoked, and its dispatch deadline (if any) has not passed.
    ///
    /// Call immediately before the first wire write of the dependent
    /// outbound call, and keep [`Self::revocation`] for that call's whole
    /// lifetime. A `false` revokes the lease; nothing may be sent.
    pub fn try_begin_dispatch(&self) -> bool {
        let Some(owner) = self.state.owner.upgrade() else {
            self.state.revoke();
            return false;
        };
        let _registry = owner
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(current) = owner
            .connection_lifecycles
            .get(&self.state.lifecycle.connection_id)
        else {
            self.state.revoke();
            return false;
        };
        let state = self
            .state
            .lifecycle
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let revoked = *self.state.revoked.borrow();
        if !Arc::ptr_eq(current.value(), &self.state.lifecycle.state)
            || state.generation != self.state.lifecycle.generation
            || !state.active
            || state.retired
            || !state.operational_connected_emitted
            || revoked
            || self
                .state
                .dispatch_not_after
                .is_some_and(|deadline| Instant::now() >= deadline)
            || !self
                .state
                .adapter
                .is_connection_live(&self.state.lifecycle.connection_id)
            || owner.ensure_operational_event_stream_healthy().is_err()
        {
            self.state.revoke();
            return false;
        }
        self.state
            .dispatched
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// A receiver that reads `true` once the lease is revoked, including a
    /// revocation that happened before this call. Revocation is permanent.
    pub fn revocation(&self) -> watch::Receiver<bool> {
        self.state.revoked.subscribe()
    }

    /// The peer connection this lease depends on.
    pub fn peer_connection_id(&self) -> &ConnectionId {
        &self.state.lifecycle.connection_id
    }
}

impl Orchestrator {
    /// Reserve one outbound dependency on a live, operationally connected
    /// connection.
    ///
    /// Core checks only transport and lifecycle identity; authorizing the
    /// caller to use that connection stays with the application. Fails with
    /// [`RvoipError::ConnectionNotFound`] for an unknown connection and with
    /// [`RvoipError::AdmissionRejected`] when the connection is not live and
    /// connected, its adapter does not report authoritative liveness, or it
    /// already backs a live lease.
    pub fn reserve_outbound_peer(
        self: &Arc<Self>,
        connection: &ConnectionId,
    ) -> Result<OutboundPeerLease> {
        self.reserve_outbound_peer_with_deadline(connection, None)
    }

    /// Like [`Self::reserve_outbound_peer`], with a fixed deadline for the
    /// first wire write: [`OutboundPeerLease::try_begin_dispatch`] fails once
    /// `dispatch_not_after` has passed. The deadline does not apply after
    /// dispatch.
    pub fn reserve_outbound_peer_before(
        self: &Arc<Self>,
        connection: &ConnectionId,
        dispatch_not_after: Instant,
    ) -> Result<OutboundPeerLease> {
        self.reserve_outbound_peer_with_deadline(connection, Some(dispatch_not_after))
    }

    fn reserve_outbound_peer_with_deadline(
        self: &Arc<Self>,
        connection: &ConnectionId,
        dispatch_not_after: Option<Instant>,
    ) -> Result<OutboundPeerLease> {
        self.ensure_operational_event_stream_healthy()?;
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entry = self
            .connections
            .get(connection)
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection.clone()))?;
        let adapter = self.adapter(entry.transport)?;
        drop(entry);
        let current = self
            .connection_lifecycles
            .get(connection)
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection.clone()))?;
        let mut lifecycle = current.value().lock().unwrap_or_else(|e| e.into_inner());
        if !adapter.lifecycle_capabilities().authoritative_liveness
            || !lifecycle.active
            || lifecycle.retired
            || !lifecycle.operational_connected_emitted
            || !adapter.is_connection_live(connection)
            || lifecycle
                .outbound_peer_dependency
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some()
        {
            return Err(RvoipError::AdmissionRejected(
                "peer connection is unavailable or already backs an outbound lease",
            ));
        }
        let (revoked, _) = watch::channel(false);
        let state = Arc::new(OutboundPeerLeaseState {
            owner: Arc::downgrade(self),
            lifecycle: ConnectionLifecycleTicket {
                connection_id: connection.clone(),
                generation: lifecycle.generation,
                state: Arc::clone(current.value()),
            },
            adapter,
            dispatched: AtomicBool::new(false),
            dispatch_not_after,
            revoked,
        });
        lifecycle.outbound_peer_dependency = Some(Arc::downgrade(&state));
        Ok(OutboundPeerLease { state })
    }

    /// Revoke the live lease on `connection`, if any, for example before
    /// handing the peer role to a replacement connection. The peer's own
    /// retirement revokes it as well.
    pub fn revoke_outbound_peer(&self, connection: &ConnectionId) {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(current) = self.connection_lifecycles.get(connection) {
            let lifecycle = current.value().lock().unwrap_or_else(|e| e.into_inner());
            lifecycle.revoke_outbound_peer_dependency();
        }
    }
}
