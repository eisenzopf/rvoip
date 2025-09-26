//! B2BUA-specific functionality

use crate::common::{types::*, errors::Result};
use rvoip_session_core_v2::api::simple::{SimplePeer, CallId};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

/// B2BUA session coordinator
pub struct B2BUACoordinator {
    routing_engine: Arc<dyn crate::routing::RoutingEngine>,
    policy_engine: Arc<dyn crate::policy::PolicyEngine>,
    session_pairs: Arc<RwLock<HashMap<IntermediarySessionId, SessionPair>>>,
}

/// A pair of call legs in a B2BUA scenario
pub struct SessionPair {
    pub id: IntermediarySessionId,
    pub inbound_leg: CallLeg,
    pub outbound_leg: CallLeg,
    pub bridged: bool,
}

/// Represents one leg of a B2BUA call
pub struct CallLeg {
    pub session_id: CallId,
    pub peer: Arc<SimplePeer>,
    pub from: String,
    pub to: String,
}

impl B2BUACoordinator {
    pub fn new(
        routing_engine: Arc<dyn crate::routing::RoutingEngine>,
        policy_engine: Arc<dyn crate::policy::PolicyEngine>,
    ) -> Self {
        Self {
            routing_engine,
            policy_engine,
            session_pairs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn handle_incoming_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<IntermediarySessionId> {
        // Apply policies
        let policies = self.policy_engine.evaluate(from, to, "INVITE", &[]).await?;

        // Check for rejections
        for policy in &policies {
            if let PolicyAction::Reject { code, reason } = policy {
                return Err(crate::common::errors::IntermediaryError::PolicyViolation(
                    format!("{}: {}", code, reason)
                ));
            }
        }

        // Get routing decision
        let decision = self.routing_engine.route(from, to, "INVITE", &[]).await?;

        if decision.targets.is_empty() {
            return Err(crate::common::errors::IntermediaryError::RoutingError(
                "No routing targets available".to_string()
            ));
        }

        // Create session pair ID
        let pair_id = IntermediarySessionId::new();

        // TODO: Create inbound and outbound legs using session-core-v2
        // This would involve:
        // 1. Creating inbound leg (UAS) using SimplePeer
        // 2. Accepting the incoming call
        // 3. Creating outbound leg (UAC) using SimplePeer
        // 4. Making outbound call to routing target
        // 5. Bridging the media when both legs are established

        Ok(pair_id)
    }

    pub async fn terminate_session(&self, session_id: &IntermediarySessionId) -> Result<()> {
        let mut pairs = self.session_pairs.write().await;
        if let Some(pair) = pairs.remove(session_id) {
            // Hangup both legs
            // pair.inbound_leg.peer.hangup(&pair.inbound_leg.session_id).await?;
            // pair.outbound_leg.peer.hangup(&pair.outbound_leg.session_id).await?;
            Ok(())
        } else {
            Err(crate::common::errors::IntermediaryError::SessionNotFound(
                session_id.0.clone()
            ))
        }
    }
}