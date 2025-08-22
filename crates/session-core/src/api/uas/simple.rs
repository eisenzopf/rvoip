//! Simple UAS Server - Maximum simplicity for basic servers

use std::sync::Arc;
use tokio::sync::RwLock;
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::api::handlers::CallHandler;
use crate::api::types::{IncomingCall, CallDecision};
use crate::api::builder::SessionManagerConfig;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use super::{UasConfig, AlwaysAcceptHandler};

/// Simplest possible UAS server that auto-accepts all calls
pub struct SimpleUasServer {
    coordinator: Arc<SessionCoordinator>,
    config: UasConfig,
}

impl SimpleUasServer {
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