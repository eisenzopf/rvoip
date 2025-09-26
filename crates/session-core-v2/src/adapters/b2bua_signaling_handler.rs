//! B2BUA Signaling Handler
//!
//! Implements the SignalingHandler trait to intercept incoming calls
//! and make B2BUA routing decisions using the RoutingAdapter.

use async_trait::async_trait;
use std::sync::Arc;
use dashmap::DashMap;
use crate::adapters::{
    SignalingHandler, SignalingDecision,
    RoutingAdapter, RoutingDecision, MediaMode,
};
use crate::types::{SessionId, DialogId};
use tracing::{debug, info, warn};

/// B2BUA-specific signaling handler that intercepts calls for routing
pub struct B2buaSignalingHandler {
    /// Routing adapter for making routing decisions
    routing_adapter: Arc<RoutingAdapter>,

    /// Track which sessions are B2BUA-handled
    b2bua_sessions: Arc<DashMap<SessionId, B2buaSessionInfo>>,

    /// Whether to accept non-B2BUA calls
    allow_direct_calls: bool,
}

/// Information about a B2BUA-handled session
#[derive(Debug, Clone)]
pub struct B2buaSessionInfo {
    /// Original target before B2BUA routing
    pub original_to: String,

    /// Routed target after B2BUA decision
    pub routed_to: String,

    /// Media handling mode
    pub media_mode: MediaMode,

    /// Call ID for correlation
    pub call_id: String,
}

impl B2buaSignalingHandler {
    /// Create a new B2BUA signaling handler
    pub fn new(routing_adapter: Arc<RoutingAdapter>) -> Self {
        Self {
            routing_adapter,
            b2bua_sessions: Arc::new(DashMap::new()),
            allow_direct_calls: true,
        }
    }

    /// Create handler that rejects non-B2BUA calls
    pub fn strict(routing_adapter: Arc<RoutingAdapter>) -> Self {
        Self {
            routing_adapter,
            b2bua_sessions: Arc::new(DashMap::new()),
            allow_direct_calls: false,
        }
    }

    /// Check if a session is B2BUA-handled
    pub fn is_b2bua_session(&self, session_id: &SessionId) -> bool {
        self.b2bua_sessions.contains_key(session_id)
    }

    /// Get B2BUA routing info for a session
    pub fn get_b2bua_info(&self, session_id: &SessionId) -> Option<B2buaSessionInfo> {
        self.b2bua_sessions.get(session_id).map(|e| e.clone())
    }
}

#[async_trait]
impl SignalingHandler for B2buaSignalingHandler {
    /// Handle incoming INVITE - main B2BUA interception point
    async fn handle_incoming_invite(
        &self,
        from: &str,
        to: &str,
        call_id: &str,
        _dialog_id: &DialogId,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing INVITE from {} to {}", from, to);

        // Get routing decision from adapter
        match self.routing_adapter.route_invite(from, to, call_id).await {
            Ok(RoutingDecision::B2bua { target, media_mode }) => {
                info!("B2BUA routing call {} to {}", call_id, target);

                // Store B2BUA session info
                // Note: We don't have SessionId yet at this point, it will be created later
                // We'll need to correlate via dialog_id or call_id

                // Return custom decision with B2BUA routing info
                SignalingDecision::Custom {
                    action: "b2bua_route".to_string(),
                    data: Some(serde_json::json!({
                        "target": target,
                        "media_mode": media_mode,
                        "original_to": to,
                        "call_id": call_id,
                    }).to_string()),
                }
            }

            Ok(RoutingDecision::Direct { endpoint }) => {
                if self.allow_direct_calls {
                    debug!("Direct routing to {}", endpoint);
                    SignalingDecision::Accept
                } else {
                    warn!("Direct calls not allowed, rejecting");
                    SignalingDecision::Reject {
                        reason: "Direct calls not permitted".to_string(),
                    }
                }
            }

            Ok(RoutingDecision::Reject { reason, status_code }) => {
                warn!("Rejecting call {}: {} ({})", call_id, reason, status_code);
                SignalingDecision::Reject { reason }
            }

            Ok(RoutingDecision::Queue { priority, queue_name }) => {
                debug!("Queueing call {} to {} with priority {}", call_id, queue_name, priority);
                SignalingDecision::Custom {
                    action: "queue".to_string(),
                    data: Some(serde_json::json!({
                        "queue": queue_name,
                        "priority": priority,
                        "call_id": call_id,
                    }).to_string()),
                }
            }

            Ok(RoutingDecision::LoadBalance { .. }) => {
                // This should have been resolved by the routing adapter
                warn!("Unexpected LoadBalance decision from routing adapter");
                SignalingDecision::Reject {
                    reason: "Internal routing error".to_string(),
                }
            }

            Err(e) => {
                warn!("Routing error for call {}: {}", call_id, e);
                SignalingDecision::Reject {
                    reason: format!("Routing failed: {}", e),
                }
            }
        }
    }

    /// Handle response - track B2BUA call progress
    async fn handle_response(
        &self,
        status_code: u16,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing {} response", status_code);

        // For B2BUA sessions, we might want to track progress
        if let Some(sid) = _session_id {
            if let Some(info) = self.b2bua_sessions.get(sid) {
                match status_code {
                    100..=199 => {
                        debug!("B2BUA call {} proceeding ({})", info.call_id, status_code);
                    }
                    200..=299 => {
                        info!("B2BUA call {} established ({})", info.call_id, status_code);
                    }
                    300..=399 => {
                        info!("B2BUA call {} redirected ({})", info.call_id, status_code);
                    }
                    400..=499 => {
                        warn!("B2BUA call {} client error ({})", info.call_id, status_code);
                        // Mark backend as potentially problematic
                        if status_code != 401 && status_code != 407 {
                            // Don't mark as failed for auth challenges
                            self.routing_adapter.mark_backend_failed(&info.routed_to).await;
                        }
                    }
                    500..=599 => {
                        warn!("B2BUA call {} server error ({})", info.call_id, status_code);
                        // Mark backend as failed
                        self.routing_adapter.mark_backend_failed(&info.routed_to).await;
                    }
                    600..=699 => {
                        info!("B2BUA call {} global failure ({})", info.call_id, status_code);
                    }
                    _ => {}
                }
            }
        }

        // Accept all responses for now
        SignalingDecision::Accept
    }

    /// Handle BYE request
    async fn handle_bye(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing BYE");

        // Clean up B2BUA session info
        if let Some(sid) = _session_id {
            if let Some((_, info)) = self.b2bua_sessions.remove(sid) {
                info!("B2BUA call {} terminated", info.call_id);
                // Mark backend as successful (call completed normally)
                self.routing_adapter.mark_backend_success(&info.routed_to).await;
            }
        }

        SignalingDecision::Accept
    }

    /// Handle CANCEL request
    async fn handle_cancel(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing CANCEL");

        // Clean up B2BUA session info
        if let Some(sid) = _session_id {
            if let Some((_, info)) = self.b2bua_sessions.remove(sid) {
                info!("B2BUA call {} cancelled", info.call_id);
            }
        }

        SignalingDecision::Accept
    }

    /// Handle UPDATE request
    async fn handle_update(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing UPDATE");

        // For B2BUA sessions, we typically accept updates
        SignalingDecision::Accept
    }

    /// Handle re-INVITE request
    async fn handle_reinvite(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing re-INVITE");

        // For B2BUA sessions, we typically accept re-INVITEs
        SignalingDecision::Accept
    }

    /// Handle REFER request (transfer)
    async fn handle_refer(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
        refer_to: &str,
    ) -> SignalingDecision {
        debug!("B2BUA handler processing REFER to {}", refer_to);

        // For B2BUA sessions, we might want to route the transfer through B2BUA as well
        if let Some(sid) = _session_id {
            if self.is_b2bua_session(sid) {
                // Route the transfer target through the routing adapter
                match self.routing_adapter.route_invite("transfer", refer_to, "transfer").await {
                    Ok(RoutingDecision::B2bua { target, .. }) => {
                        info!("B2BUA routing transfer to {}", target);
                        SignalingDecision::Custom {
                            action: "b2bua_transfer".to_string(),
                            data: Some(target),
                        }
                    }
                    _ => SignalingDecision::Accept,
                }
            } else {
                SignalingDecision::Accept
            }
        } else {
            SignalingDecision::Accept
        }
    }
}

/// Extension methods for storing B2BUA session info after session creation
impl B2buaSignalingHandler {
    /// Store B2BUA session info after session is created
    pub fn register_b2bua_session(
        &self,
        session_id: SessionId,
        call_id: String,
        original_to: String,
        routed_to: String,
        media_mode: MediaMode,
    ) {
        let info = B2buaSessionInfo {
            original_to,
            routed_to,
            media_mode,
            call_id: call_id.clone(),
        };

        self.b2bua_sessions.insert(session_id, info);
        debug!("Registered B2BUA session with Call-ID {}", call_id);
    }

    /// Remove B2BUA session info
    pub fn unregister_b2bua_session(&self, session_id: &SessionId) {
        self.b2bua_sessions.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_b2bua_routing_decision() {
        let routing_adapter = Arc::new(RoutingAdapter::new());

        // Add a B2BUA routing rule
        routing_adapter.add_rule(crate::adapters::RoutingRule {
            id: "test-b2bua".to_string(),
            pattern: "sip:service@*".to_string(),
            match_type: crate::adapters::MatchType::To,
            decision: RoutingDecision::B2bua {
                target: "sip:backend@server.local".to_string(),
                media_mode: MediaMode::Relay,
            },
            priority: 10,
            enabled: true,
        }).await.unwrap();

        let handler = B2buaSignalingHandler::new(routing_adapter);

        // Test B2BUA routing
        let decision = handler.handle_incoming_invite(
            "sip:alice@client.com",
            "sip:service@gateway.com",
            "call-123",
            &DialogId::new(),
        ).await;

        match decision {
            SignalingDecision::Custom { action, data } => {
                assert_eq!(action, "b2bua_route");
                assert!(data.is_some());
                let data_str = data.unwrap();
                assert!(data_str.contains("backend@server.local"));
            }
            _ => panic!("Expected Custom decision for B2BUA routing"),
        }
    }

    #[tokio::test]
    async fn test_direct_call_handling() {
        let routing_adapter = Arc::new(RoutingAdapter::new());

        // Set default to direct routing
        routing_adapter.set_default_decision(RoutingDecision::Direct {
            endpoint: "sip:direct@server.com".to_string(),
        }).await;

        let handler = B2buaSignalingHandler::new(routing_adapter);

        // Test direct call acceptance
        let decision = handler.handle_incoming_invite(
            "sip:alice@client.com",
            "sip:bob@server.com",
            "call-456",
            &DialogId::new(),
        ).await;

        assert!(matches!(decision, SignalingDecision::Accept));
    }

    #[tokio::test]
    async fn test_strict_mode_rejects_direct() {
        let routing_adapter = Arc::new(RoutingAdapter::new());

        // Set default to direct routing
        routing_adapter.set_default_decision(RoutingDecision::Direct {
            endpoint: "sip:direct@server.com".to_string(),
        }).await;

        // Create handler in strict mode
        let handler = B2buaSignalingHandler::strict(routing_adapter);

        // Test that direct calls are rejected
        let decision = handler.handle_incoming_invite(
            "sip:alice@client.com",
            "sip:bob@server.com",
            "call-789",
            &DialogId::new(),
        ).await;

        match decision {
            SignalingDecision::Reject { reason } => {
                assert!(reason.contains("not permitted"));
            }
            _ => panic!("Expected Reject decision in strict mode"),
        }
    }
}