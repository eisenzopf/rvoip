//! Standard UAS Server - Balance of simplicity and control

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::api::types::{SessionId, CallState, IncomingCall, CallDecision};
use crate::api::builder::SessionManagerConfig;
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use super::{UasConfig, UasCall, UasCallHandle, UasCallHandler, UasCallDecision, UasHandlerAdapter, CallController, NoOpController};

/// Standard UAS server with handler-based call control
pub struct UasServer {
    coordinator: Arc<SessionCoordinator>,
    config: UasConfig,
    handler: Arc<dyn UasCallHandler>,
    controller: Arc<dyn CallController>,
    active_calls: Arc<RwLock<HashMap<SessionId, UasCallHandle>>>,
    pending_calls: Arc<RwLock<Vec<IncomingCall>>>,
}

impl UasServer {
    /// Create a new UAS server with handler
    pub async fn new(config: UasConfig, handler: Arc<dyn UasCallHandler>) -> Result<Self> {
        Self::new_with_controller(config, handler, Arc::new(NoOpController)).await
    }
    
    /// Create a new UAS server with handler and controller
    pub async fn new_with_controller(
        config: UasConfig,
        handler: Arc<dyn UasCallHandler>,
        controller: Arc<dyn CallController>,
    ) -> Result<Self> {
        // Create the active calls map that will be shared
        let active_calls = Arc::new(RwLock::new(HashMap::new()));
        
        // Create adapter with tracking capability
        let adapter = Arc::new(UasHandlerAdapter::new_with_tracking(
            handler.clone(),
            active_calls.clone(),
        ));
        
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
        
        // Create coordinator with the adapter
        let coordinator = SessionCoordinator::new(
            manager_config,
            Some(adapter.clone() as Arc<dyn CallHandler>),
        ).await?;
        
        // Give adapter the coordinator reference
        adapter.set_coordinator(coordinator.clone()).await;
        
        // Start the coordinator to enable SIP transport
        coordinator.start().await?;
        
        let server = Self {
            coordinator,
            config,
            handler,
            controller,
            active_calls,  // Use the shared one from adapter
            pending_calls: Arc::new(RwLock::new(Vec::new())),
        };
        
        // Event monitoring will be implemented when we have proper event subscription
        
        Ok(server)
    }
    
    /// Process pending calls that were deferred
    pub async fn process_pending_calls(&self) -> Result<()> {
        let mut pending = self.pending_calls.write().await;
        let calls: Vec<_> = pending.drain(..).collect();
        drop(pending);
        
        for mut call in calls {
            // Pre-process with controller
            if !self.controller.pre_invite(&mut call).await {
                // Controller rejected the call
                SessionControl::reject_incoming_call(
                    &self.coordinator,
                    &call,
                    "Rejected by controller",
                ).await?;
                continue;
            }
            
            // Get handler decision
            let mut decision = self.handler.on_incoming_call(call.clone()).await;
            
            // Post-process with controller
            self.controller.post_decision(&call, &mut decision).await;
            
            // Execute decision
            match decision {
                UasCallDecision::Accept(sdp) => {
                    let sdp = if let Some(mut sdp) = sdp {
                        // Let controller manipulate SDP
                        self.controller.manipulate_sdp(&mut sdp, false).await;
                        Some(sdp)
                    } else if let Some(offer) = &call.sdp {
                        // Generate answer
                        let mut answer = MediaControl::generate_sdp_answer(
                            &self.coordinator,
                            &call.id,
                            offer,
                        ).await?;
                        self.controller.manipulate_sdp(&mut answer, false).await;
                        Some(answer)
                    } else {
                        None
                    };
                    
                    let session = SessionControl::accept_incoming_call(
                        &self.coordinator,
                        &call,
                        sdp,
                    ).await?;
                    
                    // Track the call
                    // Create a call handle for this session
                    let uas_call_handle = UasCallHandle::new(
                        session.id.clone(),
                        self.coordinator.clone(),
                        session.from.clone(),
                        session.to.clone(),
                    );
                    self.active_calls.write().await.insert(session.id.clone(), uas_call_handle);
                    
                    // Notify handler
                    self.handler.on_call_established(session).await;
                }
                UasCallDecision::Reject(reason) => {
                    SessionControl::reject_incoming_call(
                        &self.coordinator,
                        &call,
                        &reason,
                    ).await?;
                }
                UasCallDecision::Forward(target) => {
                    // Forward would be implemented here
                    // For now, reject with forwarding message
                    SessionControl::reject_incoming_call(
                        &self.coordinator,
                        &call,
                        &format!("Forwarded to {}", target),
                    ).await?;
                }
                UasCallDecision::Queue => {
                    // Re-queue for later
                    self.pending_calls.write().await.push(call);
                }
                UasCallDecision::Defer => {
                    // Re-queue for later
                    self.pending_calls.write().await.push(call);
                }
            }
        }
        
        Ok(())
    }
    
    /// Get a specific active call by session ID
    pub async fn get_call(&self, session_id: &SessionId) -> Option<UasCallHandle> {
        self.active_calls.read().await.get(session_id).cloned()
    }
    
    /// Get all active call handles
    pub async fn get_active_call_handles(&self) -> Vec<UasCallHandle> {
        self.active_calls.read().await.values().cloned().collect()
    }
    
    /// Get active calls metadata
    pub async fn get_active_calls(&self) -> Vec<UasCall> {
        let handles = self.active_calls.read().await;
        let mut calls = Vec::new();
        for handle in handles.values() {
            // Convert handle to metadata by getting session info
            if let Ok(Some(session)) = SessionControl::get_session(&self.coordinator, handle.session_id()).await {
                calls.push(UasCall::new(&session));
            }
        }
        calls
    }
    
    /// Get pending calls count
    pub async fn pending_count(&self) -> usize {
        self.pending_calls.read().await.len()
    }
    
    /// Accept a specific pending call
    pub async fn accept_call(&self, call_id: &SessionId, sdp: Option<String>) -> Result<()> {
        let mut pending = self.pending_calls.write().await;
        if let Some(pos) = pending.iter().position(|c| &c.id == call_id) {
            let call = pending.remove(pos);
            drop(pending);
            
            let session = SessionControl::accept_incoming_call(
                &self.coordinator,
                &call,
                sdp,
            ).await?;
            
            let uas_call_handle = UasCallHandle::new(
                session.id.clone(),
                self.coordinator.clone(),
                session.from.clone(),
                session.to.clone(),
            );
            self.active_calls.write().await.insert(session.id.clone(), uas_call_handle);
            self.handler.on_call_established(session).await;
            
            Ok(())
        } else {
            Err(crate::errors::SessionError::SessionNotFound(call_id.to_string()))
        }
    }
    
    /// Reject a specific pending call
    pub async fn reject_call(&self, call_id: &SessionId, reason: &str) -> Result<()> {
        let mut pending = self.pending_calls.write().await;
        if let Some(pos) = pending.iter().position(|c| &c.id == call_id) {
            let call = pending.remove(pos);
            drop(pending);
            
            SessionControl::reject_incoming_call(
                &self.coordinator,
                &call,
                reason,
            ).await
        } else {
            Err(crate::errors::SessionError::SessionNotFound(call_id.to_string()))
        }
    }
    
    
    /// Get the coordinator
    pub fn coordinator(&self) -> &Arc<SessionCoordinator> {
        &self.coordinator
    }
    
    /// Shutdown the server
    pub async fn shutdown(&self) -> Result<()> {
        // Stop accepting new calls
        self.coordinator.stop().await?;
        
        // Reject all pending calls
        let pending = self.pending_calls.write().await.drain(..).collect::<Vec<_>>();
        for call in pending {
            let _ = SessionControl::reject_incoming_call(
                &self.coordinator,
                &call,
                "Server shutting down",
            ).await;
        }
        
        // Terminate all active calls
        let calls = self.active_calls.read().await.clone();
        for (session_id, _) in calls {
            let _ = SessionControl::terminate_session(
                &self.coordinator,
                &session_id,
            ).await;
        }
        
        // Clear collections
        self.active_calls.write().await.clear();
        self.pending_calls.write().await.clear();
        
        Ok(())
    }
}