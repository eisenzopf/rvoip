//! UAS trait definitions

use async_trait::async_trait;
use crate::api::types::{SessionId, CallState, IncomingCall, CallSession};
use crate::errors::Result;
use super::UasCallDecision;

/// Handler for incoming calls in UAS
#[async_trait]
pub trait UasCallHandler: Send + Sync {
    /// Handle an incoming call
    async fn on_incoming_call(&self, call: IncomingCall) -> UasCallDecision;
    
    /// Called when a call is established
    async fn on_call_established(&self, session: CallSession);
    
    /// Called when a call ends
    async fn on_call_ended(&self, session: CallSession, reason: String);
    
    /// Called when DTMF is received
    async fn on_dtmf_received(&self, session_id: SessionId, digit: char);
    
    /// Called on media quality update
    async fn on_quality_update(&self, session_id: SessionId, mos_score: f32);
}

/// Controller for advanced call control
#[async_trait]
pub trait CallController: Send + Sync {
    /// Pre-process INVITE before handler sees it
    async fn pre_invite(&self, call: &mut IncomingCall) -> bool;
    
    /// Post-process after handler decision
    async fn post_decision(&self, call: &IncomingCall, decision: &mut UasCallDecision);
    
    /// Handle in-dialog requests (INFO, UPDATE, etc.)
    async fn on_in_dialog_request(&self, session_id: SessionId, method: String, body: Option<String>);
    
    /// Hook for custom SDP manipulation
    async fn manipulate_sdp(&self, sdp: &mut String, is_offer: bool);
    
    /// Hook for custom header injection
    async fn inject_headers(&self, headers: &mut Vec<(String, String)>);
}

/// Default implementation that accepts all calls
#[derive(Debug)]
pub struct AlwaysAcceptHandler;

#[async_trait]
impl crate::api::handlers::CallHandler for AlwaysAcceptHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> crate::api::types::CallDecision {
        // Generate SDP answer if we have an offer
        if call.sdp.is_some() {
            // In real implementation, generate proper SDP
            let answer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\n";
            crate::api::types::CallDecision::Accept(Some(answer.to_string()))
        } else {
            crate::api::types::CallDecision::Accept(None)
        }
    }
    
    async fn on_call_ended(&self, _call: crate::api::types::CallSession, _reason: &str) {}
}

#[async_trait]
impl UasCallHandler for AlwaysAcceptHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> UasCallDecision {
        // Generate SDP answer if we have an offer
        if call.sdp.is_some() {
            // In real implementation, generate proper SDP
            let answer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\n";
            UasCallDecision::Accept(Some(answer.to_string()))
        } else {
            UasCallDecision::Accept(None)
        }
    }
    
    async fn on_call_established(&self, _session: CallSession) {}
    async fn on_call_ended(&self, _session: CallSession, _reason: String) {}
    async fn on_dtmf_received(&self, _session_id: SessionId, _digit: char) {}
    async fn on_quality_update(&self, _session_id: SessionId, _mos_score: f32) {}
}

/// Default no-op controller
#[derive(Debug)]
pub struct NoOpController;

#[async_trait]
impl CallController for NoOpController {
    async fn pre_invite(&self, _call: &mut IncomingCall) -> bool { true }
    async fn post_decision(&self, _call: &IncomingCall, _decision: &mut UasCallDecision) {}
    async fn on_in_dialog_request(&self, _session_id: SessionId, _method: String, _body: Option<String>) {}
    async fn manipulate_sdp(&self, _sdp: &mut String, _is_offer: bool) {}
    async fn inject_headers(&self, _headers: &mut Vec<(String, String)>) {}
}