//! Simple UAS Server - Maximum simplicity for basic servers

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::api::handlers::CallHandler;
use crate::api::types::{IncomingCall, CallDecision, SessionId};
use crate::api::builder::SessionManagerConfig;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use super::{UasConfig, AlwaysAcceptHandler, UasCallHandler, UasCallHandle};

/// Simplest possible UAS server that auto-accepts all calls
pub struct SimpleUasServer {
    coordinator: Arc<SessionCoordinator>,
    config: UasConfig,
    calls: Arc<RwLock<HashMap<SessionId, UasCallHandle>>>,
}

impl SimpleUasServer {
    /// Create a new UAS server with custom handler
    /// 
    /// # Example
    /// ```no_run
    /// use rvoip_session_core::api::uas::{SimpleUasServer, UasCallHandler};
    /// use rvoip_session_core::api::types::{IncomingCall, CallDecision};
    /// use async_trait::async_trait;
    /// 
    /// #[derive(Clone)]
    /// struct MyHandler;
    /// 
    /// #[async_trait]
    /// impl UasCallHandler for MyHandler {
    ///     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
    ///         println!("Incoming call from {}", call.from);
    ///         CallDecision::Accept(None)
    ///     }
    /// }
    /// 
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let handler = MyHandler;
    ///     let server = SimpleUasServer::new(
    ///         "127.0.0.1:5061",
    ///         "sip:uas@127.0.0.1:5061",
    ///         handler,
    ///     ).await?;
    ///     
    ///     tokio::signal::ctrl_c().await?;
    ///     server.shutdown().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn new<H>(bind_addr: &str, identity: &str, handler: H) -> Result<Self> 
    where
        H: UasCallHandler + Send + Sync + 'static
    {
        let config = UasConfig {
            local_addr: bind_addr.to_string(),
            identity: identity.to_string(),
            auto_answer: true,
            ..Default::default()
        };
        
        // Parse local address to get bind address
        let local_bind_addr: std::net::SocketAddr = bind_addr.parse()
            .unwrap_or_else(|_| "0.0.0.0:5061".parse().unwrap());
        
        // Create SessionManagerConfig
        let manager_config = SessionManagerConfig {
            sip_port: local_bind_addr.port(),
            local_address: identity.to_string(),
            local_bind_addr,
            media_port_start: 42000,
            media_port_end: 43000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: false,
            media_config: Default::default(),
        };
        
        // Wrap the UasCallHandler to implement CallHandler
        struct HandlerWrapper<H: UasCallHandler> {
            inner: H,
        }
        
        impl<H: UasCallHandler> std::fmt::Debug for HandlerWrapper<H> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("HandlerWrapper").finish()
            }
        }
        
        #[async_trait::async_trait]
        impl<H: UasCallHandler + Send + Sync> CallHandler for HandlerWrapper<H> {
            async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
                // Convert to UAS decision type
                match self.inner.on_incoming_call(call).await {
                    super::UasCallDecision::Accept(sdp) => CallDecision::Accept(sdp),
                    super::UasCallDecision::Reject(reason) => CallDecision::Reject(reason),
                    super::UasCallDecision::Forward(target) => CallDecision::Forward(target),
                    super::UasCallDecision::Queue => CallDecision::Accept(None), // Queue not supported yet
                    super::UasCallDecision::Defer => CallDecision::Accept(None), // Defer not supported yet
                }
            }
            
            async fn on_call_established(&self, call: crate::api::types::CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
                self.inner.on_call_established(call).await;
            }
            
            async fn on_call_ended(&self, call: crate::api::types::CallSession, reason: &str) {
                self.inner.on_call_ended(call, reason.to_string()).await;
            }
        }
        
        let wrapper = HandlerWrapper {
            inner: handler,
        };
        
        // Create coordinator with handler
        let coordinator = SessionCoordinator::new(
            manager_config,
            Some(Arc::new(wrapper)),
        ).await?;
        
        // Start listening
        coordinator.start().await?;
        
        Ok(Self {
            coordinator,
            config,
            calls: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Get a call handle by session ID
    pub fn get_call(&self, session_id: &SessionId) -> Option<UasCallHandle> {
        // Create a new handle with coordinator reference
        // In production, we'd track actual call details
        Some(UasCallHandle::new(
            session_id.clone(),
            self.coordinator.clone(),
            "sip:remote@example.com".to_string(),  // Would get from actual session
            self.config.identity.clone(),
        ))
    }
    
    /// Create a server that always accepts incoming calls
    /// 
    /// # Example
    /// ```no_run
    /// use rvoip_session_core::api::uas::SimpleUasServer;
    /// 
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     // Create a server that accepts all calls
    ///     let server = SimpleUasServer::always_accept("0.0.0.0:5060").await?;
    ///     
    ///     // Server is now listening for calls
    ///     // Calls will be automatically accepted
    ///     
    ///     // Keep running...
    ///     tokio::signal::ctrl_c().await?;
    ///     
    ///     server.shutdown().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn always_accept(bind_addr: &str) -> Result<Self> {
        let config = UasConfig {
            local_addr: bind_addr.to_string(),
            auto_answer: true,
            ..Default::default()
        };
        
        // Parse local address to get bind address
        let local_bind_addr: std::net::SocketAddr = config.local_addr.parse()
            .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap());
        
        // Create SessionManagerConfig
        let manager_config = SessionManagerConfig {
            sip_port: local_bind_addr.port(),
            local_address: config.identity.clone(),
            local_bind_addr,
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: false,
            media_config: Default::default(),
        };
        
        // Create coordinator with auto-accept handler
        let coordinator = SessionCoordinator::new(
            manager_config,
            Some(Arc::new(AlwaysAcceptHandler)),
        ).await?;
        
        // Start listening
        coordinator.start().await?;
        
        Ok(Self {
            coordinator,
            config,
            calls: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create a server that rejects all calls (useful for maintenance mode)
    pub async fn always_reject(bind_addr: &str, reason: String) -> Result<Self> {
        let config = UasConfig {
            local_addr: bind_addr.to_string(),
            auto_answer: false,
            ..Default::default()
        };
        
        #[derive(Debug)]
        struct RejectHandler {
            reason: String,
        }
        
        #[async_trait::async_trait]
        impl CallHandler for RejectHandler {
            async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
                CallDecision::Reject(self.reason.clone())
            }
            
            async fn on_call_ended(&self, _call: crate::api::types::CallSession, _reason: &str) {}
        }
        
        // Parse local address to get bind address
        let local_bind_addr: std::net::SocketAddr = config.local_addr.parse()
            .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap());
        
        // Create SessionManagerConfig
        let manager_config = SessionManagerConfig {
            sip_port: local_bind_addr.port(),
            local_address: config.identity.clone(),
            local_bind_addr,
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: false,
            media_config: Default::default(),
        };
        
        let coordinator = SessionCoordinator::new(
            manager_config,
            Some(Arc::new(RejectHandler { reason })),
        ).await?;
        
        coordinator.start().await?;
        
        Ok(Self {
            coordinator,
            config,
            calls: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create a server that forwards all calls to another destination
    pub async fn always_forward(bind_addr: &str, forward_to: String) -> Result<Self> {
        let config = UasConfig {
            local_addr: bind_addr.to_string(),
            ..Default::default()
        };
        
        #[derive(Debug)]
        struct ForwardHandler {
            target: String,
        }
        
        #[async_trait::async_trait]
        impl CallHandler for ForwardHandler {
            async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
                CallDecision::Forward(self.target.clone())
            }
            
            async fn on_call_ended(&self, _call: crate::api::types::CallSession, _reason: &str) {}
        }
        
        // Parse local address to get bind address
        let local_bind_addr: std::net::SocketAddr = config.local_addr.parse()
            .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap());
        
        // Create SessionManagerConfig
        let manager_config = SessionManagerConfig {
            sip_port: local_bind_addr.port(),
            local_address: config.identity.clone(),
            local_bind_addr,
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: false,
            media_config: Default::default(),
        };
        
        let coordinator = SessionCoordinator::new(
            manager_config,
            Some(Arc::new(ForwardHandler { target: forward_to })),
        ).await?;
        
        coordinator.start().await?;
        
        Ok(Self {
            coordinator,
            config,
            calls: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Get active call count
    pub async fn active_calls(&self) -> Result<usize> {
        let sessions = SessionControl::list_active_sessions(&self.coordinator).await?;
        Ok(sessions.len())
    }
    
    /// Get the coordinator for advanced operations
    pub fn coordinator(&self) -> &Arc<SessionCoordinator> {
        &self.coordinator
    }
    
    /// Shutdown the server
    pub async fn shutdown(&self) -> Result<()> {
        // Stop accepting new calls
        self.coordinator.stop().await?;
        
        // Terminate all active sessions
        let sessions = SessionControl::list_active_sessions(&self.coordinator).await?;
        for session_id in sessions {
            let _ = SessionControl::terminate_session(&self.coordinator, &session_id).await;
        }
        
        Ok(())
    }
}