//! B2BUA server API for advanced call control
//!
//! This module provides a server-focused API for B2BUA scenarios where
//! the server needs to bridge and control multiple call legs.
//!
//! A B2BUA (Back-to-Back User Agent) terminates SIP dialogs on both sides,
//! generating fresh SIP headers for the outbound leg while relaying media
//! through its own RTP endpoints.
//!
//! # Architecture
//!
//! ```text
//! Caller ──INVITE──> SimpleB2BUA ──INVITE──> Callee
//!        <──200 OK──            <──200 OK──
//!        ──RTP────> relay ──────> ──RTP────>
//!        <──RTP──── relay <────── <──RTP────
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::api::bridge::CallBridge;
use crate::api::builder::{SessionManagerConfig, SipTransportType};
use crate::api::call::SimpleCall;
use crate::api::handlers::CallHandler;
use crate::api::peer::SimplePeer;
use crate::api::types::{CallDecision, CallSession, IncomingCall, SessionId};
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

// ---------------------------------------------------------------------------
// B2BUA Session tracking
// ---------------------------------------------------------------------------

/// State of a B2BUA bridged session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum B2buaLegState {
    /// Outbound leg is being set up.
    SettingUp,
    /// Both legs are active and bridged.
    Active,
    /// Tear-down in progress.
    Terminating,
    /// Fully torn down.
    Terminated,
}

/// Tracks the relationship between an inbound and outbound call.
struct B2buaSessionPair {
    /// State of this paired session.
    state: B2buaLegState,
    /// Bridge connecting the two legs.
    bridge_id: Option<String>,
    /// Inbound call session id.
    inbound_id: SessionId,
    /// Outbound call session id.
    outbound_id: SessionId,
}

// ---------------------------------------------------------------------------
// SimpleB2BUA
// ---------------------------------------------------------------------------

/// B2BUA server that can bridge and control multiple calls
///
/// `SimpleB2BUA` acts as both a UAS (for incoming calls) and a UAC (for
/// outbound calls), bridging them together.  It generates fresh SIP headers
/// for the outbound leg, keeping the two dialogs independent.
///
/// # Example
/// ```no_run
/// use rvoip_session_core::api::b2bua::SimpleB2BUA;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let mut b2bua = SimpleB2BUA::new("0.0.0.0:5060", "pbx").await?;
///
///     // Accept incoming call (leg A)
///     if let Some(incoming) = b2bua.next_incoming().await {
///         let inbound = incoming.accept().await?;
///
///         // Create outbound call (leg B) and bridge
///         let bridge = b2bua.bridge_to(inbound, "sip:support@agents.local").await?;
///     }
///
///     Ok(())
/// }
/// ```
pub struct SimpleB2BUA {
    coordinator: Arc<SessionCoordinator>,
    incoming_calls: mpsc::Receiver<IncomingCall>,
    active_bridges: Arc<RwLock<HashMap<String, CallBridge>>>,
    /// Tracked session pairs (key = bridge id).
    session_pairs: Arc<RwLock<HashMap<String, B2buaSessionPair>>>,
    /// Optional UAC capability for making outbound calls.
    outbound_peer: Option<SimplePeer>,
}

/// Handler that routes incoming calls to a channel.
#[derive(Debug)]
struct IncomingCallRouter {
    tx: mpsc::Sender<IncomingCall>,
    coordinator: Arc<SessionCoordinator>,
}

#[async_trait::async_trait]
impl CallHandler for IncomingCallRouter {
    async fn on_incoming_call(&self, mut call: IncomingCall) -> CallDecision {
        call.coordinator = Some(self.coordinator.clone());

        if self.tx.send(call).await.is_ok() {
            CallDecision::Defer
        } else {
            CallDecision::Reject("Service unavailable".to_string())
        }
    }

    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {
        // Event routing for call termination is handled via session_pairs.
    }
}

impl SimpleB2BUA {
    /// Create a B2BUA server with full capabilities.
    ///
    /// This creates a B2BUA that can both receive and make calls, enabling
    /// full bridging and call control functionality.
    ///
    /// # Arguments
    /// * `bind_addr` - The address to bind to (e.g., "0.0.0.0:5060")
    /// * `identity`  - The identity to use for outbound calls (e.g., "pbx")
    pub async fn new(bind_addr: &str, identity: &str) -> Result<Self> {
        let local_bind_addr: std::net::SocketAddr = bind_addr
            .parse()
            .map_err(|_| SessionError::ConfigError("Invalid bind address".to_string()))?;

        let (tx, rx) = mpsc::channel(100);

        let config = SessionManagerConfig {
            sip_port: local_bind_addr.port(),
            local_address: format!("sip:b2bua@{}", bind_addr),
            local_bind_addr,
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: false,
            media_config: Default::default(),
            sip_transport: SipTransportType::Udp,
        };

        let coordinator = SessionCoordinator::new(config, None).await?;
        let _handler = IncomingCallRouter {
            tx,
            coordinator: coordinator.clone(),
        };

        // TODO: Set the handler on the coordinator once the API is available.
        // coordinator.set_handler(Some(Arc::new(handler))).await;
        coordinator.start().await?;

        // Create peer for outbound calls on next available port.
        let base_port = bind_addr
            .split(':')
            .nth(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(5060u16);

        let mut outbound_peer = None;
        for port_offset in 1..10u16 {
            match SimplePeer::new(identity)
                .port(base_port.wrapping_add(port_offset))
                .await
            {
                Ok(peer) => {
                    outbound_peer = Some(peer);
                    break;
                }
                Err(_) => continue,
            }
        }

        let outbound_peer = outbound_peer.ok_or_else(|| {
            SessionError::ConfigError(
                "Could not create outbound peer (no available ports)".to_string(),
            )
        })?;

        Ok(Self {
            coordinator,
            incoming_calls: rx,
            active_bridges: Arc::new(RwLock::new(HashMap::new())),
            session_pairs: Arc::new(RwLock::new(HashMap::new())),
            outbound_peer: Some(outbound_peer),
        })
    }

    // -----------------------------------------------------------------------
    // Incoming call handling
    // -----------------------------------------------------------------------

    /// Get the next incoming call (blocking).
    pub async fn next_incoming(&mut self) -> Option<IncomingCall> {
        self.incoming_calls.recv().await
    }

    /// Try to get an incoming call (non-blocking).
    pub fn try_incoming(&mut self) -> Option<IncomingCall> {
        self.incoming_calls.try_recv().ok()
    }

    // -----------------------------------------------------------------------
    // Outbound call creation
    // -----------------------------------------------------------------------

    /// Make an outbound call via the B2BUA's UAC peer.
    pub async fn call(&self, target: &str) -> Result<SimpleCall> {
        self.outbound_peer
            .as_ref()
            .ok_or(SessionError::ConfigError(
                "No outbound peer configured".to_string(),
            ))?
            .call(target)
            .await
    }

    // -----------------------------------------------------------------------
    // Bridge management
    // -----------------------------------------------------------------------

    /// Create a new bridge.
    pub async fn create_bridge(&self, id: &str) -> CallBridge {
        let bridge = CallBridge::new();
        self.active_bridges
            .write()
            .await
            .insert(id.to_string(), bridge.clone());
        bridge
    }

    /// Get an existing bridge.
    pub async fn get_bridge(&self, id: &str) -> Option<CallBridge> {
        self.active_bridges.read().await.get(id).cloned()
    }

    /// Remove a bridge.
    pub async fn remove_bridge(&self, id: &str) -> Option<CallBridge> {
        self.active_bridges.write().await.remove(id)
    }

    /// List all active bridge IDs.
    pub async fn list_bridges(&self) -> Vec<String> {
        self.active_bridges.read().await.keys().cloned().collect()
    }

    /// Get the number of active bridges.
    pub async fn bridge_count(&self) -> usize {
        self.active_bridges.read().await.len()
    }

    /// Create a simple two-party bridge between an inbound and outbound call.
    ///
    /// This is the primary B2BUA operation: accept an incoming call on leg A,
    /// originate an outbound call on leg B, and bridge media between them.
    pub async fn bridge_to(&self, inbound: SimpleCall, target: &str) -> Result<CallBridge> {
        // Make outbound call (fresh Call-ID, Via, From tag are generated
        // automatically by the outbound SimplePeer).
        let outbound = self.call(target).await?;

        let bridge_id = format!("b2bua_{}", inbound.id().as_str());
        let bridge = self.create_bridge(&bridge_id).await;

        // Track the session pair.
        {
            let pair = B2buaSessionPair {
                state: B2buaLegState::SettingUp,
                bridge_id: Some(bridge_id.clone()),
                inbound_id: inbound.id().clone(),
                outbound_id: outbound.id().clone(),
            };
            self.session_pairs
                .write()
                .await
                .insert(bridge_id.clone(), pair);
        }

        // Add both calls to the bridge and connect.
        bridge.add(inbound).await;
        bridge.add(outbound).await;
        bridge.connect().await?;

        // Mark active.
        if let Some(pair) = self.session_pairs.write().await.get_mut(&bridge_id) {
            pair.state = B2buaLegState::Active;
        }

        tracing::info!(bridge = %bridge_id, target = %target, "B2BUA bridge established");

        Ok(bridge)
    }

    /// Terminate a bridged session by bridge id.
    ///
    /// Disconnects the bridge and marks the session pair as terminated.
    pub async fn terminate_bridge(&self, bridge_id: &str) -> Result<()> {
        // Mark terminating.
        if let Some(pair) = self.session_pairs.write().await.get_mut(bridge_id) {
            pair.state = B2buaLegState::Terminating;
        }

        // Disconnect bridge.
        if let Some(bridge) = self.remove_bridge(bridge_id).await {
            bridge.disconnect().await?;
        }

        // Mark terminated.
        if let Some(pair) = self.session_pairs.write().await.get_mut(bridge_id) {
            pair.state = B2buaLegState::Terminated;
        }

        tracing::info!(bridge = %bridge_id, "B2BUA bridge terminated");
        Ok(())
    }

    /// Get the state of a bridged session.
    pub async fn bridge_state(&self, bridge_id: &str) -> Option<B2buaLegState> {
        self.session_pairs
            .read()
            .await
            .get(bridge_id)
            .map(|p| p.state.clone())
    }

    /// List all tracked session pair IDs (bridge IDs).
    pub async fn list_session_pairs(&self) -> Vec<String> {
        self.session_pairs.read().await.keys().cloned().collect()
    }

    /// Get the number of active (bridged) session pairs.
    pub async fn active_pair_count(&self) -> usize {
        self.session_pairs
            .read()
            .await
            .values()
            .filter(|p| p.state == B2buaLegState::Active)
            .count()
    }

    // -----------------------------------------------------------------------
    // Registration
    // -----------------------------------------------------------------------

    /// Register the outbound peer with a SIP server.
    pub async fn register(&mut self, server: &str) -> Result<()> {
        self.outbound_peer
            .as_mut()
            .ok_or(SessionError::ConfigError(
                "No outbound peer configured".to_string(),
            ))?
            .register(server)
            .await
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Shutdown the B2BUA, disconnecting all bridges and stopping both the
    /// server and outbound peer.
    pub async fn shutdown(mut self) -> Result<()> {
        // Disconnect all bridges.
        let bridges: Vec<(String, CallBridge)> =
            self.active_bridges.write().await.drain().collect();
        for (id, bridge) in bridges {
            tracing::info!("Disconnecting bridge {}", id);
            if let Err(e) = bridge.disconnect().await {
                tracing::warn!("Error disconnecting bridge {}: {}", id, e);
            }
        }

        // Mark all pairs terminated.
        for pair in self.session_pairs.write().await.values_mut() {
            pair.state = B2buaLegState::Terminated;
        }

        // Shutdown outbound peer if present.
        if let Some(peer) = self.outbound_peer.take() {
            peer.shutdown().await?;
        }

        // Stop the coordinator.
        self.coordinator.stop().await
    }
}
