//! CallHandler implementation for the call center
//!
//! This module provides the CallHandler trait implementation that integrates
//! with session-core to receive and process incoming calls.

use std::sync::Weak;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};
use rvoip_session_core::{CallHandler, IncomingCall, CallDecision, CallSession};

use super::core::CallCenterEngine;

/// CallHandler implementation for the call center
#[derive(Clone, Debug)]
pub struct CallCenterCallHandler {
    pub(super) engine: Weak<CallCenterEngine>,
}

#[async_trait]
impl CallHandler for CallCenterCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        debug!("CallCenterCallHandler: Received incoming call {}", call.id);
        
        // Try to get a strong reference to the engine
        if let Some(engine) = self.engine.upgrade() {
            // Process the incoming call through the call center's routing logic
            match engine.process_incoming_call(call).await {
                Ok(decision) => decision,
                Err(e) => {
                    error!("Failed to process incoming call: {}", e);
                    CallDecision::Reject("Call center processing error".to_string())
                }
            }
        } else {
            warn!("Call center engine has been dropped");
            CallDecision::Reject("Call center not available".to_string())
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("CallCenterCallHandler: Call {} ended: {}", call.id, reason);
        
        if let Some(engine) = self.engine.upgrade() {
            if let Err(e) = engine.handle_call_termination(call.id).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("CallCenterCallHandler: Call {} established", call.id);
        debug!("Local SDP available: {}, Remote SDP available: {}", 
               local_sdp.is_some(), remote_sdp.is_some());
        
        // Update call state to active/bridged
        if let Some(engine) = self.engine.upgrade() {
            engine.update_call_established(call.id).await;
        }
    }
} 