//! Cross-transport entry point.
//!
//! Per CARVE_PLAN §6 step 4 ("Define ConnectionAdapter trait + Orchestrator
//! shell. Still no impls."): the trait surface is fully defined; the
//! Orchestrator dispatches every per-connection command through the
//! [`ConnectionAdapter`] for the connection's transport. Without a registered
//! adapter (steps 7+), commands return [`RvoipError::NoAdapterForTransport`].
//!
//! Bridging is intentionally still stubbed at this step: the cross-transport
//! frame-pump (INTERFACE_DESIGN §10.2) and the SIP-fast-path bridge strategy
//! (CARVE_PLAN §3) land in subsequent steps.

use crate::adapter::{
    AdapterEvent, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest, RejectReason,
    TransferTarget,
};
use crate::bridge::BridgeManager;
use crate::capability::CapabilityDescriptor;
use crate::commands::{InboundAction, MuteDirection};
use crate::config::Config;
use crate::connection::Transport;
use crate::error::{Result, RvoipError};
use crate::events::Event;
use crate::ids::{BridgeId, ConnectionId};
use crate::message::Message;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};
use tracing::{debug, warn};

/// Per-connection registration tracked by the orchestrator so subsequent
/// commands (`end`, `hold`, `transfer`, `send_dtmf`, ...) can route to the
/// right adapter without the caller re-stating the transport.
#[derive(Clone, Debug)]
struct ConnectionEntry {
    transport: Transport,
}

pub struct Orchestrator {
    pub config: Config,
    pub bridges: BridgeManager,
    pub admission: Arc<Semaphore>,
    adapters: Arc<DashMap<Transport, Arc<dyn ConnectionAdapter>>>,
    connections: Arc<DashMap<ConnectionId, ConnectionEntry>>,
    events: broadcast::Sender<Event>,
    /// Optional cross-crate publication. When `Some`, every emitted event is
    /// also published through `infra-common::GlobalEventCoordinator` as the
    /// `RvoipCrossCrateEvent::Core(...)` variant.
    coordinator: Option<Arc<GlobalEventCoordinator>>,
}

impl Orchestrator {
    pub fn new(config: Config) -> Arc<Self> {
        let admission = Arc::new(Semaphore::new(config.max_concurrent_setups));
        let (events, _rx) = broadcast::channel(1024);
        Arc::new(Self {
            config,
            bridges: BridgeManager::new(),
            admission,
            adapters: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            events,
            coordinator: None,
        })
    }

    pub fn new_with_coordinator(
        config: Config,
        coordinator: Arc<GlobalEventCoordinator>,
    ) -> Arc<Self> {
        let admission = Arc::new(Semaphore::new(config.max_concurrent_setups));
        let (events, _rx) = broadcast::channel(1024);
        Arc::new(Self {
            config,
            bridges: BridgeManager::new(),
            admission,
            adapters: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            events,
            coordinator: Some(coordinator),
        })
    }

    /// Register a transport adapter. Spawns a background task that pulls
    /// `AdapterEvent`s from the adapter's subscribe channel and normalizes
    /// them into rvoip-core [`Event`]s on the orchestrator's broadcast bus.
    /// Returns [`RvoipError::AdapterAlreadyRegistered`] on collision.
    pub fn register(self: &Arc<Self>, adapter: Arc<dyn ConnectionAdapter>) -> Result<()> {
        let transport = adapter.transport();
        if self.adapters.contains_key(&transport) {
            return Err(RvoipError::AdapterAlreadyRegistered(transport));
        }
        let mut events = adapter.subscribe_events();
        self.adapters.insert(transport, adapter);

        // Spawn the per-adapter event-normalize loop. Each AdapterEvent is
        // translated into one or more rvoip-core Events and republished.
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                me.handle_adapter_event(transport, event);
            }
            debug!(?transport, "adapter event stream ended");
        });
        Ok(())
    }

    pub fn adapter(&self, transport: Transport) -> Result<Arc<dyn ConnectionAdapter>> {
        self.adapters
            .get(&transport)
            .map(|e| e.value().clone())
            .ok_or(RvoipError::NoAdapterForTransport(transport))
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Look up which adapter owns a given connection. Returns
    /// [`RvoipError::ConnectionNotFound`] if the connection isn't registered.
    fn adapter_for(&self, conn: &ConnectionId) -> Result<Arc<dyn ConnectionAdapter>> {
        let entry = self
            .connections
            .get(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let transport = entry.transport;
        drop(entry);
        self.adapter(transport)
    }

    fn track_connection(&self, conn: &ConnectionId, transport: Transport) {
        self.connections
            .insert(conn.clone(), ConnectionEntry { transport });
    }

    fn forget_connection(&self, conn: &ConnectionId) {
        self.connections.remove(conn);
    }

    /// Publish an event on the in-process broadcast channel and, if a
    /// `GlobalEventCoordinator` is configured, on the cross-crate bus too.
    fn emit(&self, event: Event) {
        if let Some(coordinator) = &self.coordinator {
            let cross = event.to_cross_crate();
            let coord = Arc::clone(coordinator);
            tokio::spawn(async move {
                if let Err(err) = coord.publish(Arc::new(cross)).await {
                    warn!(?err, "rvoip-core cross-crate event publish failed");
                }
            });
        }
        let _ = self.events.send(event);
    }

    fn handle_adapter_event(&self, transport: Transport, event: AdapterEvent) {
        match event {
            AdapterEvent::InboundConnection { connection } => {
                self.track_connection(&connection.id, transport);
                self.emit(Event::ConnectionInbound {
                    connection_id: connection.id.clone(),
                    at: Utc::now(),
                });
            }
            AdapterEvent::Connected { connection_id } => {
                self.emit(Event::ConnectionConnected {
                    connection_id,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Ended {
                connection_id,
                reason,
            } => {
                self.forget_connection(&connection_id);
                self.emit(Event::ConnectionEnded {
                    connection_id,
                    reason,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Failed {
                connection_id,
                detail,
            } => {
                self.forget_connection(&connection_id);
                self.emit(Event::ConnectionFailed {
                    connection_id,
                    detail,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Native { kind, detail } => {
                debug!(?transport, ?kind, ?detail, "adapter native event (unmapped)");
            }
        }
    }

    // ------------------------------------------------------------------
    // Command surface — dispatched via ConnectionAdapter.
    // ------------------------------------------------------------------

    pub async fn route_inbound_connection(
        &self,
        connection_id: ConnectionId,
        action: InboundAction,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        match action {
            InboundAction::Accept { .. } => adapter.accept(connection_id).await,
            InboundAction::Reject { reason } => adapter.reject(connection_id, reason).await,
            InboundAction::BridgeTo { .. } => Err(RvoipError::NotImplemented(
                "InboundAction::BridgeTo — bridge dispatch lands with SipBridgeStrategy (step 9+)",
            )),
        }
    }

    pub async fn originate_connection(
        &self,
        request: OriginateRequest,
    ) -> Result<ConnectionHandle> {
        // The OriginateRequest's transport is implied by which adapter we
        // call. v1: caller picks the transport by registering only one
        // adapter at a time; once multi-adapter dispatch is needed (step 9+)
        // the request grows a `transport` field.
        let transport = self
            .adapters
            .iter()
            .next()
            .map(|entry| *entry.key())
            .ok_or(RvoipError::NotImplemented(
                "no adapter registered — register one before originating",
            ))?;
        let adapter = self.adapter(transport)?;
        let handle = adapter.originate(request).await?;
        self.track_connection(&handle.connection.id, transport);
        self.emit(Event::ConnectionOutbound {
            connection_id: handle.connection.id.clone(),
            at: Utc::now(),
        });
        Ok(handle)
    }

    pub async fn end_connection(
        &self,
        connection_id: ConnectionId,
        reason: EndReason,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.end(connection_id, reason).await
    }

    pub async fn hold(&self, connection_id: ConnectionId) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.hold(connection_id).await
    }

    pub async fn resume(&self, connection_id: ConnectionId) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.resume(connection_id).await
    }

    pub async fn transfer_connection(
        &self,
        connection_id: ConnectionId,
        target: TransferTarget,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.transfer(connection_id, target).await
    }

    pub async fn send_dtmf(
        &self,
        connection_id: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.send_dtmf(connection_id, digits, duration_ms).await
    }

    pub async fn send_message(
        &self,
        connection_id: ConnectionId,
        message: Message,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.send_message(connection_id, message).await
    }

    pub async fn renegotiate_media(
        &self,
        connection_id: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<crate::capability::NegotiatedCodecs> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.renegotiate_media(connection_id, capabilities).await
    }

    // Mute/Unmute aren't on the ConnectionAdapter trait yet (per
    // INTERFACE_DESIGN §6 they're per-direction local controls). They will
    // land when the trait grows the corresponding methods.
    pub async fn mute(
        &self,
        _connection_id: ConnectionId,
        _direction: MuteDirection,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "mute — adapter trait method not defined yet",
        ))
    }

    pub async fn unmute(
        &self,
        _connection_id: ConnectionId,
        _direction: MuteDirection,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "unmute — adapter trait method not defined yet",
        ))
    }

    // Bridging stays stubbed at step 4. Step 9+ wires the SIP-fast-path
    // strategy (`rvoip-sip::server::SipBridgeStrategy`) and step 10+ wires
    // the cross-transport frame-pump.
    pub async fn bridge_connections(
        &self,
        _a: ConnectionId,
        _b: ConnectionId,
    ) -> Result<BridgeId> {
        Err(RvoipError::NotImplemented(
            "bridge_connections — fast-path strategy / frame-pump lands in step 9+",
        ))
    }

    pub async fn unbridge_connections(&self, bridge_id: BridgeId) -> Result<()> {
        match self.bridges.remove(&bridge_id) {
            Some(_handle) => {
                // Drop tears down the bridge synchronously.
                self.emit(Event::ConnectionsUnbridged {
                    bridge_id,
                    at: Utc::now(),
                });
                Ok(())
            }
            None => Err(RvoipError::BridgeNotFound(bridge_id)),
        }
    }
}

// Allow forwarding the `RejectReason` argument from older call sites that
// already had it imported. Re-exported for consumer convenience.
pub use crate::adapter::RejectReason as InboundRejectReason;
