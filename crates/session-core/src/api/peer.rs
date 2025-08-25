//! Unified peer API for P2P and client scenarios
//! 
//! This module provides a simplified interface for SIP peers that can both
//! make and receive calls, abstracting away the UAC/UAS distinction.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use crate::api::builder::SessionManagerConfig;
use crate::api::control::SessionControl;
use crate::api::types::{SessionId, IncomingCall, CallDecision};
use crate::api::call::SimpleCall;
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

/// A SIP peer that can make and receive calls
/// 
/// # Example
/// ```
/// # use rvoip_session_core::api::SimplePeer;
/// # use rvoip_session_core::errors::Result;
/// # 
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
/// # // Use a random port to avoid conflicts when tests run in parallel
/// # let port = 6000 + (std::process::id() % 1000) as u16;
/// // Create a peer with builder pattern
/// let mut alice = SimplePeer::new("alice")
///     .local_addr("127.0.0.1")  // Optional, defaults to 0.0.0.0
///     .port(port)               // Optional, defaults to 5060
///     .await?;
/// 
/// // Register with SIP server (for receiving calls)
/// alice.register("sip:registrar@example.com").await?;
/// 
/// // Make call - auto-detects protocol, defaults to port 5060
/// // let call = alice.call("bob@127.0.0.1").await?;
/// 
/// // Check for incoming calls
/// if let Some(incoming) = alice.try_incoming() {
///     // let call = incoming.accept().await?;
///     // Handle the call...
/// }
/// 
/// # alice.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct SimplePeer {
    pub(crate) identity: String,
    pub(crate) coordinator: Arc<SessionCoordinator>,
    incoming_calls: mpsc::Receiver<IncomingCall>,
    registrar: Option<String>,
    pub(crate) local_addr: String,
    pub(crate) port: u16,
}

/// Builder for creating a SimplePeer with custom configuration
pub struct PeerBuilder {
    identity: String,
    local_addr: Option<String>,
    port: Option<u16>,
}

impl PeerBuilder {
    /// Set the local address to bind to (default: "0.0.0.0")
    pub fn local_addr(mut self, addr: &str) -> Self {
        self.local_addr = Some(addr.to_string());
        self
    }
    
    /// Set the port to bind to (default: 5060)
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }
}

// Implement IntoFuture for PeerBuilder to allow await directly
impl std::future::IntoFuture for PeerBuilder {
    type Output = Result<SimplePeer>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let local_addr = self.local_addr.unwrap_or_else(|| "0.0.0.0".to_string());
            let port = self.port.unwrap_or(5060);
            
            SimplePeer::create(&self.identity, &local_addr, port).await
        })
    }
}

/// Builder for making calls with custom options
pub struct CallBuilder<'a> {
    peer: &'a SimplePeer,
    target: String,
    port: Option<u16>,
    call_id: Option<String>,
}

impl<'a> CallBuilder<'a> {
    /// Create a new call builder
    fn new(peer: &'a SimplePeer, target: &str) -> Self {
        Self {
            peer,
            target: target.to_string(),
            port: None,
            call_id: None,
        }
    }
    
    /// Set the port for the target (default: 5060)
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }
    
    /// Set a custom call ID
    pub fn call_id(mut self, id: &str) -> Self {
        self.call_id = Some(id.to_string());
        self
    }
}

// Implement IntoFuture for CallBuilder to allow await directly
impl<'a> std::future::IntoFuture for CallBuilder<'a> {
    type Output = Result<SimpleCall>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.peer.call_with_options(
                &self.target,
                self.port,
                self.call_id.as_deref()
            ).await
        })
    }
}

/// Internal handler that routes incoming calls to a channel
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

impl SimplePeer {
    /// Get the presence coordinator for testing
    #[doc(hidden)]
    pub fn presence_coordinator(&self) -> Arc<RwLock<crate::coordinator::presence::PresenceCoordinator>> {
        self.coordinator.presence_coordinator.clone()
    }
    
    /// Create a new peer with builder pattern
    /// 
    /// # Example
    /// ```
    /// # use rvoip_session_core::api::SimplePeer;
    /// # use rvoip_session_core::errors::Result;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() -> Result<()> {
    /// # // Use a random port to avoid conflicts when tests run in parallel
    /// # let port = 7000 + (std::process::id() % 1000) as u16;
    /// let peer = SimplePeer::new("alice")
    ///     .local_addr("127.0.0.1")
    ///     .port(port)
    ///     .await?;
    /// # peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(identity: &str) -> PeerBuilder {
        PeerBuilder {
            identity: identity.to_string(),
            local_addr: None,
            port: None,
        }
    }
    
    /// Internal method to create a peer with specific configuration
    async fn create(identity: &str, local_addr: &str, port: u16) -> Result<Self> {
        let bind_addr = format!("{}:{}", local_addr, port);
        let local_bind_addr = bind_addr.parse()
            .map_err(|_| SessionError::ConfigError("Invalid address".to_string()))?;
        
        // Create channel for incoming calls
        let (tx, rx) = mpsc::channel(100);
        
        // Create config
        let config = SessionManagerConfig {
            sip_port: port,
            local_address: format!("sip:{}@{}:{}", identity, local_addr, port),
            local_bind_addr,
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: true,  // Can act as client
            media_config: Default::default(),
        };
        
        // Create coordinator with router handler
        let coordinator = SessionCoordinator::new(config, None).await?;
        let handler = IncomingCallRouter { 
            tx,
            coordinator: coordinator.clone(),
        };
        
        // TODO: Need to set the handler - for now pass it in constructor
        // coordinator.set_handler(Some(Arc::new(handler))).await;
        coordinator.start().await?;
        
        Ok(Self {
            identity: identity.to_string(),
            coordinator,
            incoming_calls: rx,
            registrar: None,
            local_addr: local_addr.to_string(),
            port,
        })
    }
    
    /// Register with a SIP server
    /// 
    /// After registration, you can make calls using just the username
    /// (e.g., "bob" instead of "bob@server.com").
    pub async fn register(&mut self, server: &str) -> Result<()> {
        self.registrar = Some(server.to_string());
        // TODO: Implement actual SIP REGISTER
        tracing::info!("Registered {} to {}", self.identity, server);
        Ok(())
    }
    
    /// Unregister from the SIP server
    pub async fn unregister(&mut self) -> Result<()> {
        if let Some(server) = self.registrar.take() {
            // TODO: Send unregister
            tracing::info!("Unregistered {} from {}", self.identity, server);
        }
        Ok(())
    }
    
    /// Make an outgoing call with builder pattern for advanced options
    /// 
    /// # Arguments
    /// * `target` - The call target. Can be:
    ///   - Full SIP URI: "sip:bob@example.com:5060"
    ///   - User@host: "bob@example.com" (uses port 5060)
    ///   - Just username (if registered): "bob"
    ///   - Phone number: "tel:+14155551234" or "+14155551234"
    /// 
    /// # Example
    /// ```
    /// # use rvoip_session_core::api::SimplePeer;
    /// # use rvoip_session_core::errors::Result;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() -> Result<()> {
    /// # // Use a random port to avoid conflicts when tests run in parallel
    /// # let port = 8000 + (std::process::id() % 1000) as u16;
    /// # let mut peer = SimplePeer::new("alice").port(port).await?;
    /// // Simple call with default port (would connect in real scenario)
    /// // let call = peer.call("bob@example.com").await?;
    /// 
    /// // Call with custom port (would connect in real scenario)
    /// // let call2 = peer.call("charlie@example.com")
    /// //     .port(5070)
    /// //     .call_id("my-call-123")
    /// //     .await?;
    /// # peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn call(&self, target: &str) -> CallBuilder {
        CallBuilder::new(self, target)
    }
    
    /// Internal method to make a call with options
    async fn call_with_options(
        &self,
        target: &str,
        port: Option<u16>,
        call_id: Option<&str>,
    ) -> Result<SimpleCall> {
        let port = port.unwrap_or(5060);
        
        // Auto-detect protocol and format URI
        let target_uri = if target.starts_with("sip:") {
            target.to_string()
        } else if target.starts_with("tel:") {
            target.to_string()
        } else if target.starts_with("+") || target.chars().all(|c| c.is_numeric() || c == '-') {
            // Phone number
            format!("tel:{}", target)
        } else if target.contains("@") {
            // User@host format
            if target.contains(":") && target.split('@').nth(1).unwrap().contains(":") {
                // Already has port
                format!("sip:{}", target)
            } else {
                // Add port
                format!("sip:{}:{}", target, port)
            }
        } else if let Some(registrar) = &self.registrar {
            // Just username, use registrar
            format!("sip:{}@{}", target, registrar)
        } else {
            // Default to SIP with port
            format!("sip:{}:{}", target, port)
        };
        
        let from_uri = format!("sip:{}@{}:{}", self.identity, self.local_addr, self.port);
        
        // Use SessionControl to create the call
        use crate::api::control::SessionControl;
        let prepared = SessionControl::prepare_outgoing_call(
            &self.coordinator,
            &from_uri,
            &target_uri,
        ).await?;
        
        // TODO: Set call_id if provided
        // if let Some(id) = call_id {
        //     prepared.set_call_id(id);
        // }
        
        let session = SessionControl::initiate_prepared_call(
            &self.coordinator,
            &prepared,
        ).await?;
        
        // Create SimpleCall
        SimpleCall::from_session(session, self.coordinator.clone()).await
    }
    
    /// Get the next incoming call (blocking)
    /// 
    /// This will wait until an incoming call arrives.
    pub async fn next_incoming(&mut self) -> Option<IncomingCall> {
        self.incoming_calls.recv().await
    }
    
    /// Try to get an incoming call (non-blocking)
    /// 
    /// Returns immediately with None if no calls are waiting.
    pub fn try_incoming(&mut self) -> Option<IncomingCall> {
        self.incoming_calls.try_recv().ok()
    }
    
    /// Get the peer's identity
    pub fn identity(&self) -> &str {
        &self.identity
    }
    
    /// Get the peer's local address
    pub fn local_addr(&self) -> &str {
        &self.local_addr
    }
    
    /// Get the peer's SIP port
    pub fn port(&self) -> u16 {
        self.port
    }
    
    /// Get the registrar if registered
    pub fn registrar(&self) -> Option<&str> {
        self.registrar.as_deref()
    }
    
    /// Shutdown the peer
    /// 
    /// This will unregister (if registered) and stop the coordinator.
    pub async fn shutdown(mut self) -> Result<()> {
        self.unregister().await?;
        self.coordinator.stop().await?;
        Ok(())
    }
}