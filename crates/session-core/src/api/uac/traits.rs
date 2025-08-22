//! UAC trait definitions

use async_trait::async_trait;
use crate::api::types::{SessionId, CallState};
use crate::errors::Result;

/// Event handler for UAC events
#[async_trait]
pub trait UacEventHandler: Send + Sync {
    /// Called when a call state changes
    async fn on_call_state_changed(&self, session_id: SessionId, old_state: CallState, new_state: CallState);
    
    /// Called when registration state changes
    async fn on_registration_state_changed(&self, registered: bool, reason: Option<String>);
    
    /// Called when media starts flowing
    async fn on_media_established(&self, session_id: SessionId);
    
    /// Called when DTMF is received
    async fn on_dtmf_received(&self, session_id: SessionId, digit: char);
    
    /// Called on call quality update
    async fn on_quality_update(&self, session_id: SessionId, mos_score: f32);
}

/// Default implementation that does nothing
pub struct NoOpEventHandler;

#[async_trait]
impl UacEventHandler for NoOpEventHandler {
    async fn on_call_state_changed(&self, _session_id: SessionId, _old_state: CallState, _new_state: CallState) {}
    async fn on_registration_state_changed(&self, _registered: bool, _reason: Option<String>) {}
    async fn on_media_established(&self, _session_id: SessionId) {}
    async fn on_dtmf_received(&self, _session_id: SessionId, _digit: char) {}
    async fn on_quality_update(&self, _session_id: SessionId, _mos_score: f32) {}
}