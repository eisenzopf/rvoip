//! Peer-scoped media routing for UCTP datagram substrates.
//!
//! The eight-byte UCTP media header carries only a peer-local `u16` stream
//! identifier. It does not carry a Session or Connection identifier, so every
//! media stream sharing one QUIC or WebTransport peer must allocate from one
//! namespace. [`PeerMediaRouter`] owns that namespace and provides exact
//! indexes back to the authenticated core Session, Connection, and Stream.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::num::NonZeroU16;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::identity::PrincipalOwnershipKey;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A core Session/Connection pair sharing one authenticated UCTP peer.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerMediaConnectionKey {
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
}

impl fmt::Debug for PeerMediaConnectionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PeerMediaConnectionKey")
    }
}

impl PeerMediaConnectionKey {
    pub fn new(session_id: SessionId, connection_id: ConnectionId) -> Self {
        Self {
            session_id,
            connection_id,
        }
    }
}

/// Exact core route represented by a peer-local UCTP media identifier.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerMediaRouteKey {
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
    pub stream_id: StreamId,
}

impl fmt::Debug for PeerMediaRouteKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PeerMediaRouteKey")
    }
}

impl PeerMediaRouteKey {
    pub fn new(session_id: SessionId, connection_id: ConnectionId, stream_id: StreamId) -> Self {
        Self {
            session_id,
            connection_id,
            stream_id,
        }
    }

    pub fn connection_key(&self) -> PeerMediaConnectionKey {
        PeerMediaConnectionKey::new(self.session_id.clone(), self.connection_id.clone())
    }
}

/// Publisher route used for optional Orchestrator media fanout.
///
/// This is deliberately separate from [`PeerMediaRouteKey`]: subscriber-side
/// streams have a local route but no publisher fanout key, while a future
/// forwarding route may intentionally deliver locally under a different
/// Stream ID than the publisher registry uses.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerMediaFanoutKey {
    pub session_id: SessionId,
    pub publisher_connection_id: ConnectionId,
    pub stream_id: StreamId,
}

impl fmt::Debug for PeerMediaFanoutKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PeerMediaFanoutKey")
    }
}

impl PeerMediaFanoutKey {
    pub fn new(
        session_id: SessionId,
        publisher_connection_id: ConnectionId,
        stream_id: StreamId,
    ) -> Self {
        Self {
            session_id,
            publisher_connection_id,
            stream_id,
        }
    }
}

/// Inputs committed after a peer-local identifier has been reserved.
pub struct PeerMediaRegistration {
    pub owner: PrincipalOwnershipKey,
    pub route: PeerMediaRouteKey,
    pub fanout: Option<PeerMediaFanoutKey>,
    pub stream: Arc<dyn MediaStream>,
    pub ingress: mpsc::Sender<MediaFrame>,
}

impl PeerMediaRegistration {
    pub fn new(
        owner: PrincipalOwnershipKey,
        route: PeerMediaRouteKey,
        stream: Arc<dyn MediaStream>,
        ingress: mpsc::Sender<MediaFrame>,
    ) -> Self {
        Self {
            owner,
            route,
            fanout: None,
            stream,
            ingress,
        }
    }

    pub fn with_fanout(mut self, fanout: PeerMediaFanoutKey) -> Self {
        self.fanout = Some(fanout);
        self
    }
}

impl fmt::Debug for PeerMediaRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerMediaRegistration")
            .field("fanout_present", &self.fanout.is_some())
            .field("stream_kind", &self.stream.kind())
            .finish_non_exhaustive()
    }
}

/// A committed peer-local media route.
pub struct PeerMediaBinding {
    local_id: NonZeroU16,
    owner: PrincipalOwnershipKey,
    route: PeerMediaRouteKey,
    fanout: Option<PeerMediaFanoutKey>,
    stream: Arc<dyn MediaStream>,
    ingress: mpsc::Sender<MediaFrame>,
    cancel: CancellationToken,
}

impl PeerMediaBinding {
    pub fn local_id(&self) -> NonZeroU16 {
        self.local_id
    }

    pub fn owner(&self) -> &PrincipalOwnershipKey {
        &self.owner
    }

    pub fn route(&self) -> &PeerMediaRouteKey {
        &self.route
    }

    pub fn fanout(&self) -> Option<&PeerMediaFanoutKey> {
        self.fanout.as_ref()
    }

    pub fn stream(&self) -> &Arc<dyn MediaStream> {
        &self.stream
    }

    pub fn ingress(&self) -> mpsc::Sender<MediaFrame> {
        self.ingress.clone()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }

    fn snapshot(&self) -> PeerMediaBindingSnapshot {
        PeerMediaBindingSnapshot {
            local_id: self.local_id,
            owner: self.owner.clone(),
            route: self.route.clone(),
            fanout: self.fanout.clone(),
            kind: self.stream.kind(),
            codec: self.stream.codec(),
            direction: self.stream.direction(),
            ingress_capacity: self.ingress.capacity(),
            ingress_max_capacity: self.ingress.max_capacity(),
            ingress_closed: self.ingress.is_closed(),
            cancelled: self.cancel.is_cancelled(),
        }
    }
}

impl fmt::Debug for PeerMediaBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerMediaBinding")
            .field("local_id", &self.local_id)
            .field("fanout_present", &self.fanout.is_some())
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Immutable diagnostics for one committed route.
#[derive(Clone)]
pub struct PeerMediaBindingSnapshot {
    pub local_id: NonZeroU16,
    pub owner: PrincipalOwnershipKey,
    pub route: PeerMediaRouteKey,
    pub fanout: Option<PeerMediaFanoutKey>,
    pub kind: StreamKind,
    pub codec: CodecInfo,
    pub direction: Direction,
    pub ingress_capacity: usize,
    pub ingress_max_capacity: usize,
    pub ingress_closed: bool,
    pub cancelled: bool,
}

impl fmt::Debug for PeerMediaBindingSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerMediaBindingSnapshot")
            .field("local_id", &self.local_id)
            .field("fanout_present", &self.fanout.is_some())
            .field("kind", &self.kind)
            .field("codec_present", &!self.codec.name.is_empty())
            .field("direction", &self.direction)
            .field("ingress_capacity", &self.ingress_capacity)
            .field("ingress_max_capacity", &self.ingress_max_capacity)
            .field("ingress_closed", &self.ingress_closed)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

/// Aggregate-safe peer media diagnostics.
#[derive(Clone)]
pub struct PeerMediaRouterSnapshot {
    pub shutdown: bool,
    /// Count of identifiers issued during this peer lifetime, including
    /// abandoned reservations. Issued identifiers are never reused.
    pub issued_local_ids: u32,
    pub reserved_local_ids: Vec<NonZeroU16>,
    pub bindings: Vec<PeerMediaBindingSnapshot>,
    pub session_count: usize,
    pub connection_count: usize,
}

impl fmt::Debug for PeerMediaRouterSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerMediaRouterSnapshot")
            .field("shutdown", &self.shutdown)
            .field("issued_local_ids", &self.issued_local_ids)
            .field("reserved_local_id_count", &self.reserved_local_ids.len())
            .field("binding_count", &self.bindings.len())
            .field("session_count", &self.session_count)
            .field("connection_count", &self.connection_count)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum PeerMediaRouterError {
    ShuttingDown,
    LocalIdExhausted,
    ReservationLost,
    RouterDropped,
    DuplicateRoute {
        route: PeerMediaRouteKey,
        existing_local_id: NonZeroU16,
    },
    StreamIdMismatch {
        expected: StreamId,
        actual: StreamId,
    },
    OwnerMismatch,
}

impl fmt::Debug for PeerMediaRouterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ShuttingDown => "PeerMediaRouterError::ShuttingDown",
            Self::LocalIdExhausted => "PeerMediaRouterError::LocalIdExhausted",
            Self::ReservationLost => "PeerMediaRouterError::ReservationLost",
            Self::RouterDropped => "PeerMediaRouterError::RouterDropped",
            Self::DuplicateRoute { .. } => "PeerMediaRouterError::DuplicateRoute",
            Self::StreamIdMismatch { .. } => "PeerMediaRouterError::StreamIdMismatch",
            Self::OwnerMismatch => "PeerMediaRouterError::OwnerMismatch",
        })
    }
}

impl fmt::Display for PeerMediaRouterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ShuttingDown => "peer media router is shutting down",
            Self::LocalIdExhausted => "peer media stream namespace is exhausted",
            Self::ReservationLost => "peer media reservation no longer exists",
            Self::RouterDropped => "peer media router is unavailable",
            Self::DuplicateRoute { .. } => "peer media route is already bound",
            Self::StreamIdMismatch { .. } => "peer media stream identity mismatch",
            Self::OwnerMismatch => "peer media route owner mismatch",
        })
    }
}

impl std::error::Error for PeerMediaRouterError {}

#[derive(Default)]
struct RouterState {
    shutdown: bool,
    /// Stored as `u32` so 65,536 is an unambiguous exhausted sentinel.
    next_local_id: u32,
    reservations: HashMap<NonZeroU16, CancellationToken>,
    by_local_id: HashMap<NonZeroU16, Arc<PeerMediaBinding>>,
    by_route: HashMap<PeerMediaRouteKey, NonZeroU16>,
    by_session: HashMap<SessionId, HashSet<NonZeroU16>>,
    by_connection: HashMap<PeerMediaConnectionKey, HashSet<NonZeroU16>>,
}

impl RouterState {
    fn active() -> Self {
        Self {
            next_local_id: 1,
            ..Self::default()
        }
    }
}

/// One media namespace for one authenticated QUIC/WebTransport peer.
///
/// The router is synchronous because registration and datagram lookup touch
/// only in-memory indexes. No lock is held across channel sends or async work.
pub struct PeerMediaRouter {
    state: Mutex<RouterState>,
    cancel: CancellationToken,
}

impl PeerMediaRouter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RouterState::active()),
            cancel: CancellationToken::new(),
        })
    }

    /// Reserve the next peer-local identifier.
    ///
    /// Identifiers are monotonically allocated and never reused during the
    /// peer lifetime, even if the reservation is dropped or registration
    /// fails. This prevents delayed datagrams from reaching a new stream.
    pub fn reserve(self: &Arc<Self>) -> Result<PeerMediaReservation, PeerMediaRouterError> {
        let mut state = self.state.lock();
        if state.shutdown {
            return Err(PeerMediaRouterError::ShuttingDown);
        }
        if state.next_local_id > u32::from(u16::MAX) {
            return Err(PeerMediaRouterError::LocalIdExhausted);
        }

        let local_id = NonZeroU16::new(state.next_local_id as u16)
            .expect("peer media allocator starts at one");
        state.next_local_id += 1;
        let cancel = self.cancel.child_token();
        state.reservations.insert(local_id, cancel.clone());

        Ok(PeerMediaReservation {
            router: Arc::downgrade(self),
            local_id,
            cancel,
            active: true,
        })
    }

    pub fn lookup(&self, local_id: NonZeroU16) -> Option<Arc<PeerMediaBinding>> {
        self.state.lock().by_local_id.get(&local_id).cloned()
    }

    pub fn lookup_owned(
        &self,
        local_id: NonZeroU16,
        owner: &PrincipalOwnershipKey,
    ) -> Result<Option<Arc<PeerMediaBinding>>, PeerMediaRouterError> {
        let binding = self.lookup(local_id);
        if binding
            .as_ref()
            .is_some_and(|binding| binding.owner() != owner)
        {
            return Err(PeerMediaRouterError::OwnerMismatch);
        }
        Ok(binding)
    }

    pub fn lookup_route(&self, route: &PeerMediaRouteKey) -> Option<Arc<PeerMediaBinding>> {
        let state = self.state.lock();
        state
            .by_route
            .get(route)
            .and_then(|local_id| state.by_local_id.get(local_id))
            .cloned()
    }

    pub fn bindings_for_session(&self, session_id: &SessionId) -> Vec<Arc<PeerMediaBinding>> {
        let state = self.state.lock();
        collect_bindings(
            &state,
            state.by_session.get(session_id).into_iter().flatten(),
        )
    }

    pub fn bindings_for_connection(
        &self,
        connection: &PeerMediaConnectionKey,
    ) -> Vec<Arc<PeerMediaBinding>> {
        let state = self.state.lock();
        collect_bindings(
            &state,
            state.by_connection.get(connection).into_iter().flatten(),
        )
    }

    pub fn remove_local_id(&self, local_id: NonZeroU16) -> Option<Arc<PeerMediaBinding>> {
        let binding = remove_one(&mut self.state.lock(), local_id);
        if let Some(binding) = binding.as_ref() {
            binding.cancel();
        }
        binding
    }

    pub fn remove_local_id_owned(
        &self,
        local_id: NonZeroU16,
        owner: &PrincipalOwnershipKey,
    ) -> Result<Option<Arc<PeerMediaBinding>>, PeerMediaRouterError> {
        let mut state = self.state.lock();
        if state
            .by_local_id
            .get(&local_id)
            .is_some_and(|binding| binding.owner() != owner)
        {
            return Err(PeerMediaRouterError::OwnerMismatch);
        }
        let binding = remove_one(&mut state, local_id);
        drop(state);
        if let Some(binding) = binding.as_ref() {
            binding.cancel();
        }
        Ok(binding)
    }

    pub fn remove_route(&self, route: &PeerMediaRouteKey) -> Option<Arc<PeerMediaBinding>> {
        let local_id = self.state.lock().by_route.get(route).copied();
        local_id.and_then(|local_id| self.remove_local_id(local_id))
    }

    pub fn remove_connection(
        &self,
        connection: &PeerMediaConnectionKey,
    ) -> Vec<Arc<PeerMediaBinding>> {
        let mut state = self.state.lock();
        let local_ids: Vec<_> = state
            .by_connection
            .get(connection)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let removed = remove_many(&mut state, local_ids);
        drop(state);
        cancel_all(&removed);
        removed
    }

    pub fn remove_session(&self, session_id: &SessionId) -> Vec<Arc<PeerMediaBinding>> {
        let mut state = self.state.lock();
        let local_ids: Vec<_> = state
            .by_session
            .get(session_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let removed = remove_many(&mut state, local_ids);
        drop(state);
        cancel_all(&removed);
        removed
    }

    pub fn snapshot(&self) -> PeerMediaRouterSnapshot {
        let state = self.state.lock();
        let mut reserved_local_ids: Vec<_> = state.reservations.keys().copied().collect();
        reserved_local_ids.sort_unstable();
        let mut bindings: Vec<_> = state
            .by_local_id
            .values()
            .map(|binding| binding.snapshot())
            .collect();
        bindings.sort_unstable_by_key(|binding| binding.local_id);
        PeerMediaRouterSnapshot {
            shutdown: state.shutdown,
            issued_local_ids: state.next_local_id.saturating_sub(1),
            reserved_local_ids,
            bindings,
            session_count: state.by_session.len(),
            connection_count: state.by_connection.len(),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.state.lock().shutdown
    }

    /// Stop new allocations, cancel every reservation and binding, clear all
    /// indexes, and return the removed bindings so adapters may await their
    /// transport-specific `MediaStream::close` implementations.
    pub fn shutdown(&self) -> Vec<Arc<PeerMediaBinding>> {
        self.cancel.cancel();
        let mut state = self.state.lock();
        if state.shutdown {
            return Vec::new();
        }
        state.shutdown = true;
        for (_, reservation) in state.reservations.drain() {
            reservation.cancel();
        }
        let removed: Vec<_> = state.by_local_id.drain().map(|(_, value)| value).collect();
        state.by_route.clear();
        state.by_session.clear();
        state.by_connection.clear();
        drop(state);
        cancel_all(&removed);
        removed
    }

    fn commit(
        &self,
        local_id: NonZeroU16,
        registration: PeerMediaRegistration,
    ) -> Result<Arc<PeerMediaBinding>, PeerMediaRouterError> {
        let actual_stream_id = registration.stream.id();
        if actual_stream_id != registration.route.stream_id {
            return Err(PeerMediaRouterError::StreamIdMismatch {
                expected: registration.route.stream_id,
                actual: actual_stream_id,
            });
        }

        let mut state = self.state.lock();
        if state.shutdown {
            return Err(PeerMediaRouterError::ShuttingDown);
        }
        let Some(cancel) = state.reservations.get(&local_id).cloned() else {
            return Err(PeerMediaRouterError::ReservationLost);
        };
        if let Some(existing_local_id) = state.by_route.get(&registration.route).copied() {
            return Err(PeerMediaRouterError::DuplicateRoute {
                route: registration.route,
                existing_local_id,
            });
        }

        let connection = registration.route.connection_key();
        let session_id = registration.route.session_id.clone();
        let route = registration.route.clone();
        let binding = Arc::new(PeerMediaBinding {
            local_id,
            owner: registration.owner,
            route: registration.route,
            fanout: registration.fanout,
            stream: registration.stream,
            ingress: registration.ingress,
            cancel,
        });

        state.reservations.remove(&local_id);
        state.by_route.insert(route, local_id);
        state
            .by_session
            .entry(session_id)
            .or_default()
            .insert(local_id);
        state
            .by_connection
            .entry(connection)
            .or_default()
            .insert(local_id);
        state.by_local_id.insert(local_id, binding.clone());
        Ok(binding)
    }

    fn release_reservation(&self, local_id: NonZeroU16) {
        if let Some(cancel) = self.state.lock().reservations.remove(&local_id) {
            cancel.cancel();
        }
    }
}

impl Default for PeerMediaRouter {
    fn default() -> Self {
        Self {
            state: Mutex::new(RouterState::active()),
            cancel: CancellationToken::new(),
        }
    }
}

impl Drop for PeerMediaRouter {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// An allocation that may be used to construct the transport MediaStream
/// before atomically publishing it in all router indexes.
pub struct PeerMediaReservation {
    router: Weak<PeerMediaRouter>,
    local_id: NonZeroU16,
    cancel: CancellationToken,
    active: bool,
}

impl PeerMediaReservation {
    pub fn local_id(&self) -> NonZeroU16 {
        self.local_id
    }

    /// Pass this token into the transport MediaStream constructor. Dropping an
    /// uncommitted reservation, removing its committed binding, or shutting
    /// down the peer then stops the stream's tasks through the same token.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn commit(
        mut self,
        registration: PeerMediaRegistration,
    ) -> Result<Arc<PeerMediaBinding>, PeerMediaRouterError> {
        let router = self
            .router
            .upgrade()
            .ok_or(PeerMediaRouterError::RouterDropped)?;
        let binding = router.commit(self.local_id, registration)?;
        self.active = false;
        Ok(binding)
    }
}

impl fmt::Debug for PeerMediaReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerMediaReservation")
            .field("local_id", &self.local_id)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl Drop for PeerMediaReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.cancel.cancel();
        if let Some(router) = self.router.upgrade() {
            router.release_reservation(self.local_id);
        }
    }
}

fn collect_bindings<'a>(
    state: &RouterState,
    local_ids: impl Iterator<Item = &'a NonZeroU16>,
) -> Vec<Arc<PeerMediaBinding>> {
    let mut bindings: Vec<_> = local_ids
        .filter_map(|local_id| state.by_local_id.get(local_id).cloned())
        .collect();
    bindings.sort_unstable_by_key(|binding| binding.local_id());
    bindings
}

fn remove_many(
    state: &mut RouterState,
    local_ids: impl IntoIterator<Item = NonZeroU16>,
) -> Vec<Arc<PeerMediaBinding>> {
    local_ids
        .into_iter()
        .filter_map(|local_id| remove_one(state, local_id))
        .collect()
}

fn remove_one(state: &mut RouterState, local_id: NonZeroU16) -> Option<Arc<PeerMediaBinding>> {
    let binding = state.by_local_id.remove(&local_id)?;
    state.by_route.remove(binding.route());

    let session_id = &binding.route().session_id;
    let remove_session_index = state
        .by_session
        .get_mut(session_id)
        .is_some_and(|local_ids| {
            local_ids.remove(&local_id);
            local_ids.is_empty()
        });
    if remove_session_index {
        state.by_session.remove(session_id);
    }

    let connection = binding.route().connection_key();
    let remove_connection_index =
        state
            .by_connection
            .get_mut(&connection)
            .is_some_and(|local_ids| {
                local_ids.remove(&local_id);
                local_ids.is_empty()
            });
    if remove_connection_index {
        state.by_connection.remove(&connection);
    }

    Some(binding)
}

fn cancel_all(bindings: &[Arc<PeerMediaBinding>]) {
    for binding in bindings {
        binding.cancel();
    }
}
