//! Proxy-specific functionality

use crate::common::{types::*, errors::Result};
use std::sync::Arc;

/// Proxy mode coordinator
pub struct ProxyCoordinator {
    mode: ProxyMode,
    routing_engine: Arc<dyn crate::routing::RoutingEngine>,
    policy_engine: Arc<dyn crate::policy::PolicyEngine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMode {
    Stateless,
    TransactionStateful,
    CallStateful,
}

impl ProxyCoordinator {
    pub fn new(
        mode: ProxyMode,
        routing_engine: Arc<dyn crate::routing::RoutingEngine>,
        policy_engine: Arc<dyn crate::policy::PolicyEngine>,
    ) -> Self {
        Self {
            mode,
            routing_engine,
            policy_engine,
        }
    }

    pub async fn process_request(
        &self,
        from: &str,
        to: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<RoutingDecision> {
        // Apply policies
        let policies = self.policy_engine.evaluate(from, to, method, headers).await?;

        // Check for rejections
        for policy in &policies {
            if let PolicyAction::Reject { code, reason } = policy {
                return Err(crate::common::errors::IntermediaryError::PolicyViolation(
                    format!("{}: {}", code, reason)
                ));
            }
        }

        // Get routing decision
        let mut decision = self.routing_engine.route(from, to, method, headers).await?;
        decision.policies = policies;

        Ok(decision)
    }
}