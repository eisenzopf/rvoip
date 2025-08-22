//! UAS handler implementations and utilities

use async_trait::async_trait;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use crate::api::types::{IncomingCall, CallDecision, SessionId};
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use super::{UasCallHandler, UasCallDecision, UasCallHandle};

/// Adapter to use UasCallHandler with the existing CallHandler trait
pub struct UasHandlerAdapter {
    handler: Arc<dyn UasCallHandler>,
    active_calls: Arc<RwLock<HashMap<SessionId, UasCallHandle>>>,
    coordinator: Arc<RwLock<Option<Arc<SessionCoordinator>>>>,
}

impl std::fmt::Debug for UasHandlerAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UasHandlerAdapter").finish()
    }
}

impl UasHandlerAdapter {
    pub fn new(handler: Arc<dyn UasCallHandler>) -> Self {
        Self { 
            handler,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            coordinator: Arc::new(RwLock::new(None)),
        }
    }
    
    pub fn new_with_tracking(
        handler: Arc<dyn UasCallHandler>,
        active_calls: Arc<RwLock<HashMap<SessionId, UasCallHandle>>>,
    ) -> Self {
        Self {
            handler,
            active_calls,
            coordinator: Arc::new(RwLock::new(None)),
        }
    }
    
    pub async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        *self.coordinator.write().await = Some(coordinator);
    }
}

#[async_trait]
impl CallHandler for UasHandlerAdapter {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let decision = self.handler.on_incoming_call(call.clone()).await;
        
        match decision {
            UasCallDecision::Accept(sdp) => CallDecision::Accept(sdp),
            UasCallDecision::Reject(reason) => CallDecision::Reject(reason),
            UasCallDecision::Forward(target) => CallDecision::Forward(target),
            UasCallDecision::Queue | UasCallDecision::Defer => CallDecision::Defer,
        }
    }
    
    async fn on_call_established(&self, session: crate::api::types::CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        // Create and track the call handle
        if let Some(coordinator) = self.coordinator.read().await.as_ref() {
            let handle = UasCallHandle::new(
                session.id.clone(),
                coordinator.clone(),
                session.from.clone(),
                session.to.clone(),
            );
            self.active_calls.write().await.insert(session.id.clone(), handle);
        }
        
        // Notify the user's handler
        self.handler.on_call_established(session).await;
    }
    
    async fn on_call_ended(&self, call: crate::api::types::CallSession, reason: &str) {
        self.handler.on_call_ended(call, reason.to_string()).await;
    }
}