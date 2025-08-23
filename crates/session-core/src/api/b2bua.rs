//! B2BUA server API for advanced call control
//! 
//! This module provides a server-focused API for B2BUA scenarios where
//! the server needs to bridge and control multiple call legs.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use crate::api::builder::SessionManagerConfig;
use crate::api::types::{SessionId, IncomingCall, CallDecision};
use crate::api::call::SimpleCall;
use crate::api::bridge::CallBridge;
use crate::api::handlers::CallHandler;
use crate::api::peer::SimplePeer;
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

/// B2BUA server that can bridge and control multiple calls
/// 
/// # Example
/// ```
/// use rvoip_session_core::api::b2bua::SimpleB2BUA;
/// 
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create B2BUA server
///     let mut b2bua = SimpleB2BUA::new("0.0.0.0:5060", "pbx").await?;
///     
///     // Accept incoming call
///     if let Some(incoming) = b2bua.next_incoming().await {
///         let inbound = incoming.accept().await?;
///         
///         // Route to destination
///         let outbound = b2bua.call("support@agents.local").await?;
///         
///         // Bridge the calls
///         let bridge = b2bua.create_bridge("call_123").await;
///         bridge.add(inbound).await;
///         bridge.add(outbound).await;
///         bridge.connect().await?;
///     }
///     
///     Ok(())
/// }
/// ```
pub struct SimpleB2BUA {
    coordinator: Arc<SessionCoordinator>,
    incoming_calls: mpsc::Receiver<IncomingCall>,
    active_bridges: Arc<RwLock<HashMap<String, CallBridge>>>,
    /// Optional UAC capability for making outbound calls
    outbound_peer: Option<SimplePeer>,
}

/// Handler that routes incoming calls to a channel
#[derive(Debug)]
struct IncomingCallRouter {
    tx: mpsc::Sender<IncomingCall>,
    coordinator: Arc<SessionCoordinator>,
}

#[async_trait::async_trait]
impl CallHandler for IncomingCallRouter {
    async fn on_incoming_call(&self, mut call: IncomingCall) -> CallDecision {
        // Add coordinator to the call so it can accept/reject
        call.coordinator = Some(self.coordinator.clone());
        
        if self.tx.send(call).await.is_ok() {
            CallDecision::Defer  // We'll handle it via the channel
        } else {
            CallDecision::Reject("Service unavailable".to_string())
        }
    }
    
    async fn on_call_ended(&self, _call: crate::api::types::CallSession, _reason: &str) {
        // TODO: Could notify via another channel if needed
    }
}

impl SimpleB2BUA {
    /// Create a B2BUA server with full capabilities
    /// 
    /// This creates a B2BUA that can both receive and make calls,
    /// enabling full bridging and call control functionality.
    /// 
    /// # Arguments
    /// * `bind_addr` - The address to bind to (e.g., "0.0.0.0:5060")
    /// * `identity` - The identity to use for outbound calls (e.g., "pbx")
    pub async fn new(bind_addr: &str, identity: &str) -> Result<Self> {
        let local_bind_addr: std::net::SocketAddr = bind_addr.parse()
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
            enable_sip_client: false,  // Server mode
            media_config: Default::default(),
        };
        
        let coordinator = SessionCoordinator::new(config, None).await?;
        let handler = IncomingCallRouter {
            tx,
            coordinator: coordinator.clone(),
        };
        
        // TODO: Need to set the handler - for now pass it in constructor
        // coordinator.set_handler(Some(Arc::new(handler))).await;
        coordinator.start().await?;
        
        // Create peer for outbound calls on next port
        let base_port = bind_addr.split(':')
            .nth(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(5060);
        
        // Try to create outbound peer on next available port
        let mut outbound_peer = None;
        for port_offset in 1..10 {
            match SimplePeer::with_port(identity, base_port + port_offset).await {
                Ok(peer) => {
                    outbound_peer = Some(peer);
                    break;
                }
                Err(_) => continue,
            }
        }
        
        let outbound_peer = outbound_peer.ok_or_else(|| {
            SessionError::ConfigError(
                "Could not create outbound peer (no available ports)".to_string()
            )
        })?;
        
        Ok(Self {
            coordinator,
            incoming_calls: rx,
            active_bridges: Arc::new(RwLock::new(HashMap::new())),
            outbound_peer: Some(outbound_peer),
        })
    }
    
    /// Get next incoming call
    /// 
    /// This will wait for an incoming call to arrive.
    pub async fn next_incoming(&mut self) -> Option<IncomingCall> {
        self.incoming_calls.recv().await
    }
    
    /// Try to get an incoming call (non-blocking)
    /// 
    /// Returns immediately with None if no calls are waiting.
    pub fn try_incoming(&mut self) -> Option<IncomingCall> {
        self.incoming_calls.try_recv().ok()
    }
    
    /// Make an outbound call
    pub async fn call(&self, target: &str) -> Result<SimpleCall> {
        self.outbound_peer
            .as_ref()
            .ok_or(SessionError::ConfigError("No outbound peer configured".to_string()))?
            .call(target)
            .await
    }
    
    /// Create a new bridge
    /// 
    /// # Arguments
    /// * `id` - Unique identifier for the bridge
    pub async fn create_bridge(&self, id: &str) -> CallBridge {
        let bridge = CallBridge::new();
        self.active_bridges.write().await.insert(id.to_string(), bridge.clone());
        bridge
    }
    
    /// Get an existing bridge
    pub async fn get_bridge(&self, id: &str) -> Option<CallBridge> {
        self.active_bridges.read().await.get(id).cloned()
    }
    
    /// Remove a bridge
    /// 
    /// This returns the bridge if it existed, allowing you to clean it up.
    pub async fn remove_bridge(&self, id: &str) -> Option<CallBridge> {
        self.active_bridges.write().await.remove(id)
    }
    
    /// List all active bridge IDs
    pub async fn list_bridges(&self) -> Vec<String> {
        self.active_bridges.read().await.keys().cloned().collect()
    }
    
    /// Get the number of active bridges
    pub async fn bridge_count(&self) -> usize {
        self.active_bridges.read().await.len()
    }
    
    /// Create a simple two-party bridge
    /// 
    /// This is a convenience method for the common case of bridging
    /// an incoming call to an outbound destination.
    pub async fn bridge_to(&self, inbound: SimpleCall, target: &str) -> Result<CallBridge> {
        // Make outbound call
        let outbound = self.call(target).await?;
        
        // Create bridge
        let bridge_id = format!("bridge_{}", inbound.id().as_str());
        let bridge = self.create_bridge(&bridge_id).await;
        
        // Add both calls
        bridge.add(inbound).await;
        bridge.add(outbound).await;
        
        // Connect them
        bridge.connect().await?;
        
        Ok(bridge)
    }
    
    /// Register the outbound peer with a SIP server
    /// 
    /// This allows the B2BUA to register for receiving calls at the server.
    pub async fn register(&mut self, server: &str) -> Result<()> {
        self.outbound_peer
            .as_mut()
            .ok_or(SessionError::ConfigError("No outbound peer configured".to_string()))?  
            .register(server).await
    }
    
    /// Shutdown the B2BUA
    /// 
    /// This will disconnect all bridges and stop both the server and
    /// outbound peer (if present).
    pub async fn shutdown(mut self) -> Result<()> {
        // Disconnect all bridges
        let bridges = self.active_bridges.write().await.drain().collect::<Vec<_>>();
        for (id, bridge) in bridges {
            tracing::info!("Disconnecting bridge {}", id);
            if let Err(e) = bridge.disconnect().await {
                tracing::warn!("Error disconnecting bridge {}: {}", id, e);
            }
        }
        
        // Shutdown outbound peer if present
        if let Some(peer) = self.outbound_peer.take() {
            peer.shutdown().await?;
        }
        
        // Stop the coordinator
        self.coordinator.stop().await
    }
}