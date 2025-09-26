//! Simple API for common intermediary use cases

use crate::common::{types::*, errors::Result};
use std::sync::Arc;

/// Configuration for an intermediary
#[derive(Debug, Clone)]
pub struct IntermediaryConfig {
    /// Operating mode
    pub mode: IntermediaryMode,
    /// Local IP address
    pub local_ip: std::net::IpAddr,
    /// SIP port
    pub sip_port: u16,
    /// Media port range start
    pub media_port_start: u16,
    /// Media port range end
    pub media_port_end: u16,
}

impl Default for IntermediaryConfig {
    fn default() -> Self {
        Self {
            mode: IntermediaryMode::B2BUA,
            local_ip: "127.0.0.1".parse().unwrap(),
            sip_port: 5060,
            media_port_start: 10000,
            media_port_end: 20000,
        }
    }
}

/// Simple intermediary builder
pub struct IntermediaryBuilder {
    config: IntermediaryConfig,
    routing_engine: Option<Arc<dyn crate::routing::RoutingEngine>>,
    policy_engine: Option<Arc<dyn crate::policy::PolicyEngine>>,
}

impl IntermediaryBuilder {
    pub fn new() -> Self {
        Self {
            config: IntermediaryConfig::default(),
            routing_engine: None,
            policy_engine: None,
        }
    }

    pub fn mode(mut self, mode: IntermediaryMode) -> Self {
        self.config.mode = mode;
        self
    }

    pub fn routing_engine(mut self, engine: Arc<dyn crate::routing::RoutingEngine>) -> Self {
        self.routing_engine = Some(engine);
        self
    }

    pub fn policy_engine(mut self, engine: Arc<dyn crate::policy::PolicyEngine>) -> Self {
        self.policy_engine = Some(engine);
        self
    }

    pub async fn build(self) -> Result<Intermediary> {
        let routing_engine = self.routing_engine
            .unwrap_or_else(|| Arc::new(crate::routing::BasicRoutingEngine::new()));
        let policy_engine = self.policy_engine
            .unwrap_or_else(|| Arc::new(crate::policy::BasicPolicyEngine::new()));

        Ok(Intermediary {
            config: self.config,
            routing_engine,
            policy_engine,
        })
    }
}

/// High-level intermediary interface
pub struct Intermediary {
    config: IntermediaryConfig,
    routing_engine: Arc<dyn crate::routing::RoutingEngine>,
    policy_engine: Arc<dyn crate::policy::PolicyEngine>,
}

impl Intermediary {
    /// Create a new intermediary with default settings
    pub async fn new(mode: IntermediaryMode) -> Result<Self> {
        IntermediaryBuilder::new()
            .mode(mode)
            .build()
            .await
    }

    /// Get the current operating mode
    pub fn mode(&self) -> &IntermediaryMode {
        &self.config.mode
    }

    /// Process an incoming SIP request
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