//! Simple B2BUA API for session-core-v2
//!
//! Provides a dead-simple API similar to SimplePeer for B2BUA operations.
//! Wraps UnifiedCoordinator while adding B2BUA-specific functionality.

use std::sync::Arc;
use crate::{
    api::UnifiedCoordinator,
    adapters::{
        RoutingAdapter, RoutingDecision, MediaMode,
        b2bua_signaling_handler::B2buaSignalingHandler,
    },
    api::bridge_coordinator::BridgeCoordinator,
    state_table::types::SessionId,
    errors::{Result, SessionError},
};
use tracing::info;

/// Configuration for B2BUA
#[derive(Debug, Clone)]
pub struct B2buaConfig {
    /// SIP registrar address
    pub registrar: String,

    /// Local SIP address
    pub local_addr: String,

    /// Username for authentication
    pub username: String,

    /// Password for authentication
    pub password: String,

    /// Whether to allow direct (non-B2BUA) calls
    pub allow_direct_calls: bool,

    /// Default media mode for B2BUA calls
    pub default_media_mode: MediaMode,

    /// Whether to record B2BUA calls by default
    pub record_by_default: bool,

    /// Recording storage path
    pub recording_path: Option<String>,
}

impl Default for B2buaConfig {
    fn default() -> Self {
        Self {
            registrar: "sip:registrar@localhost:5060".to_string(),
            local_addr: "127.0.0.1:5070".to_string(),
            username: "b2bua".to_string(),
            password: "password".to_string(),
            allow_direct_calls: true,
            default_media_mode: MediaMode::Relay,
            record_by_default: false,
            recording_path: Some("/tmp/b2bua_recordings".to_string()),
        }
    }
}

/// Simple B2BUA API - "Dead simple" like SimplePeer
pub struct SimpleB2bua {
    /// Wrapped UnifiedCoordinator for core functionality
    coordinator: Arc<UnifiedCoordinator>,

    /// Bridge coordinator for managing B2BUA pairs
    bridge_coordinator: Arc<BridgeCoordinator>,

    /// Routing adapter for B2BUA decisions
    routing_adapter: Arc<RoutingAdapter>,

    /// B2BUA signaling handler
    signaling_handler: Arc<B2buaSignalingHandler>,

    /// Configuration
    config: B2buaConfig,
}

impl SimpleB2bua {
    /// Create a new B2BUA instance
    pub async fn new(config: B2buaConfig) -> Result<Self> {
        // Create UnifiedCoordinator with B2BUA config
        // Parse local address to get IP and port
        let local_addr: std::net::SocketAddr = config.local_addr.parse()
            .map_err(|_| SessionError::ConfigurationError(format!("Invalid local address: {}", config.local_addr)))?;

        let unified_config = crate::api::unified::Config {
            local_ip: local_addr.ip(),
            sip_port: local_addr.port(),
            bind_addr: local_addr,
            local_uri: format!("sip:{}@{}", config.username, config.local_addr),
            ..Default::default()
        };
        let coordinator = UnifiedCoordinator::new(unified_config).await?;

        // Get the routing adapter
        let routing_adapter = Arc::new(RoutingAdapter::new());

        // Create B2BUA signaling handler
        let signaling_handler = if config.allow_direct_calls {
            Arc::new(B2buaSignalingHandler::new(routing_adapter.clone()))
        } else {
            Arc::new(B2buaSignalingHandler::strict(routing_adapter.clone()))
        };

        // TODO: Set the signaling handler in the dialog adapter when it supports it
        // For now, we'll need to work with the default behavior

        // Create bridge coordinator using the adapters directly
        // Note: We need to get these from the coordinator's internal structure
        // This is a temporary solution until we have proper accessor methods
        let bridge_coordinator = Arc::new(BridgeCoordinator::new());

        Ok(Self {
            coordinator,
            bridge_coordinator,
            routing_adapter,
            signaling_handler,
            config,
        })
    }

    // ===== Routing Configuration =====

    /// Add a B2BUA routing rule
    pub async fn add_routing_rule(
        &self,
        pattern: String,
        target: String,
        media_mode: MediaMode,
    ) -> Result<()> {
        use crate::adapters::{RoutingRule, MatchType};

        let rule = RoutingRule {
            id: format!("rule_{}", uuid::Uuid::new_v4()),
            pattern,
            match_type: MatchType::To,
            decision: RoutingDecision::B2bua { target, media_mode },
            priority: 10,
            enabled: true,
        };

        self.routing_adapter.add_rule(rule).await?;
        info!("Added B2BUA routing rule");
        Ok(())
    }

    /// Set load balancing targets
    pub async fn set_load_balance_targets(
        &self,
        pattern: String,
        targets: Vec<String>,
    ) -> Result<()> {
        use crate::adapters::{RoutingRule, MatchType, LoadBalanceAlgorithm};

        let rule = RoutingRule {
            id: format!("lb_{}", uuid::Uuid::new_v4()),
            pattern,
            match_type: MatchType::To,
            decision: RoutingDecision::LoadBalance {
                targets,
                algorithm: LoadBalanceAlgorithm::RoundRobin,
            },
            priority: 5,
            enabled: true,
        };

        self.routing_adapter.add_rule(rule).await?;
        info!("Added load balancing rule");
        Ok(())
    }

    /// Enable/disable direct calls
    pub async fn set_allow_direct_calls(&mut self, allow: bool) {
        self.config.allow_direct_calls = allow;

        // Update signaling handler
        self.signaling_handler = if allow {
            Arc::new(B2buaSignalingHandler::new(self.routing_adapter.clone()))
        } else {
            Arc::new(B2buaSignalingHandler::strict(self.routing_adapter.clone()))
        };

        // TODO: Update the signaling handler in the dialog adapter when it supports it
    }

    // ===== B2BUA Operations =====

    /// Register B2BUA with SIP registrar
    pub async fn register(&self) -> Result<()> {
        // TODO: Implement registration when UnifiedCoordinator supports it
        info!("B2BUA registration requested for {}", self.config.registrar);
        Ok(())
    }

    /// Unregister from SIP registrar
    pub async fn unregister(&self) -> Result<()> {
        // TODO: Implement unregistration when UnifiedCoordinator supports it
        info!("B2BUA unregistration requested");
        Ok(())
    }

    /// Get active B2BUA bridge pairs
    pub async fn get_active_bridges(&self) -> Vec<(SessionId, SessionId)> {
        // Get all bridges from the coordinator
        // For now, we'll return an empty vec since we need access to the internal bridges map
        // TODO: Add a method to BridgeCoordinator to get bridge pairs
        Vec::new()
    }

    /// Get bridge info for a session
    pub async fn get_bridge_info(&self, _session_id: &SessionId) -> Option<(SessionId, MediaMode)> {
        // TODO: Get the actual bridge partner from BridgeCoordinator
        // For now, return None
        None
    }

    /// Manually bridge two sessions
    pub async fn bridge_sessions(
        &self,
        inbound_id: &SessionId,
        outbound_id: &SessionId,
        media_mode: MediaMode,
    ) -> Result<()> {
        // Register the bridge
        let bridge_id = format!("bridge_{}", uuid::Uuid::new_v4());
        let call_id = format!("call_{}", uuid::Uuid::new_v4());
        let _bridge_id = self.bridge_coordinator.register_bridge(
            inbound_id.clone(),
            outbound_id.clone(),
            media_mode,
            call_id,
            bridge_id.clone(),
            "B2BUA Manual Bridge".to_string(),
            "user@b2bua".to_string(),
        ).await?;
        Ok(())
    }

    /// Destroy a bridge
    pub async fn unbridge_session(&self, session_id: &SessionId) -> Result<()> {
        // Get the bridge ID and terminate it
        if let Some(bridge_id) = self.bridge_coordinator.get_bridge_id(session_id) {
            self.bridge_coordinator.terminate_bridge(&bridge_id).await
        } else {
            Err(SessionError::SessionNotFound(format!("No bridge found for session {}", session_id.0)))
        }
    }

    // ===== Call Control =====

    /// Make an outbound B2BUA call
    pub async fn make_b2bua_call(
        &self,
        from: String,
        to: String,
        media_mode: Option<MediaMode>,
    ) -> Result<(SessionId, SessionId)> {
        let mode = media_mode.unwrap_or(self.config.default_media_mode);

        // Create inbound leg (from caller)
        let inbound_session = self.coordinator.make_call(&from, &to).await?;

        // Determine routing target
        let target = match self.routing_adapter.route_invite(&from, &to, "manual").await? {
            RoutingDecision::B2bua { target, .. } => target,
            _ => to.clone(), // Default to original target
        };

        // Create outbound leg (to target)
        let outbound_session = self.coordinator.make_call(&to, &target).await?;

        // Bridge the sessions
        let bridge_id = format!("b2bua_{}", uuid::Uuid::new_v4());
        let call_id = format!("call_{}", uuid::Uuid::new_v4());
        self.bridge_coordinator.register_bridge(
            inbound_session.clone(),
            outbound_session.clone(),
            mode,
            call_id.clone(),
            bridge_id.clone(),
            format!("B2BUA Call: {} -> {}", from, to),
            from.clone(),
        ).await?;

        // Register with signaling handler
        self.signaling_handler.register_b2bua_session(
            inbound_session.clone(),
            format!("b2bua_{}", uuid::Uuid::new_v4()),
            to,
            target,
            mode,
        );

        Ok((inbound_session, outbound_session))
    }

    /// Transfer a B2BUA call
    pub async fn transfer_call(
        &self,
        session_id: &SessionId,
        transfer_to: String,
    ) -> Result<SessionId> {
        // Get the bridged partner
        if let Some((partner_id, media_mode)) = self.get_bridge_info(session_id).await {
            // Unbridge current sessions
            self.unbridge_session(session_id).await?;

            // Terminate the old outbound leg
            self.coordinator.hangup(&partner_id).await?;

            // Create new outbound leg
            let new_session = self.coordinator.make_call(
                &format!("transfer_{}", session_id.0),
                &transfer_to,
            ).await?;

            // Bridge with new session
            self.bridge_sessions(session_id, &new_session, media_mode).await?;

            Ok(new_session)
        } else {
            Err(SessionError::SessionNotFound(format!("No bridge found for session {}", session_id.0)))
        }
    }

    // ===== Recording Control =====

    /// Start recording a B2BUA bridge
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<String> {
        if let Some((partner_id, _)) = self.get_bridge_info(session_id).await {
            // Configure recording
            let _config = crate::adapters::media_adapter::RecordingConfig {
                file_path: format!(
                    "{}/b2bua_{}_{}.wav",
                    self.config.recording_path.as_ref().unwrap_or(&"/tmp".to_string()),
                    session_id.0,
                    chrono::Utc::now().timestamp()
                ),
                format: crate::adapters::media_adapter::AudioFormat::Wav,
                sample_rate: 8000,
                channels: 1,
                include_mixed: true,
                separate_tracks: true,
            };

            // TODO: Start recording on the bridge when we have access to media adapter
            // For now, just return a mock recording ID
            Ok(format!("recording_{}_{}", session_id.0, chrono::Utc::now().timestamp()))
        } else {
            Err(SessionError::SessionNotFound(format!("No bridge found for session {}", session_id.0)))
        }
    }

    /// Stop recording
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.coordinator.stop_recording(session_id).await
    }

    // ===== Health & Monitoring =====

    /// Get backend health status
    pub async fn get_backend_health(&self, _backend: &str) -> crate::adapters::BackendHealth {
        // TODO: Implement when RoutingAdapter has get_backend_health
        crate::adapters::BackendHealth {
            uri: _backend.to_string(),
            state: crate::adapters::BackendState::Healthy,
            failure_count: 0,
            success_count: 0,
            failure_threshold: 3,
            last_check: std::time::Instant::now(),
        }
    }

    /// Mark backend as failed
    pub async fn mark_backend_failed(&self, backend: &str) {
        self.routing_adapter.mark_backend_failed(backend).await;
    }

    /// Mark backend as recovered
    pub async fn mark_backend_recovered(&self, backend: &str) {
        self.routing_adapter.mark_backend_success(backend).await;
    }

    // ===== Event Subscription =====

    /// Subscribe to leg events for a bridge
    pub async fn subscribe_to_leg_events(&self, _session_id: &SessionId) -> tokio::sync::mpsc::Receiver<crate::api::bridge_coordinator::LegEvent> {
        // TODO: Implement event subscription when BridgeCoordinator supports it
        let (_tx, rx) = tokio::sync::mpsc::channel(100);
        rx
    }

    // ===== Delegate Methods to UnifiedCoordinator =====

    /// Answer an incoming call
    pub async fn answer(&self, session_id: &SessionId) -> Result<()> {
        self.coordinator.accept_call(session_id).await
    }

    /// Reject an incoming call
    pub async fn reject(&self, session_id: &SessionId) -> Result<()> {
        self.coordinator.reject_call(session_id, "Rejected by B2BUA").await
    }

    /// Hang up a call
    pub async fn hang_up(&self, session_id: &SessionId) -> Result<()> {
        self.coordinator.hangup(session_id).await
    }

    /// Put call on hold
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        self.coordinator.hold(session_id).await
    }

    /// Resume held call
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        self.coordinator.resume(session_id).await
    }

    /// Send DTMF digit
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        self.coordinator.send_dtmf(session_id, digit).await
    }

    /// Get the underlying coordinator (for advanced usage)
    pub fn get_coordinator(&self) -> Arc<UnifiedCoordinator> {
        self.coordinator.clone()
    }
}

/// Builder pattern for SimpleB2bua
pub struct B2buaBuilder {
    config: B2buaConfig,
}

impl B2buaBuilder {
    /// Create a new builder with default config
    pub fn new() -> Self {
        Self {
            config: B2buaConfig::default(),
        }
    }

    /// Set registrar address
    pub fn registrar(mut self, registrar: String) -> Self {
        self.config.registrar = registrar;
        self
    }

    /// Set local address
    pub fn local_addr(mut self, addr: String) -> Self {
        self.config.local_addr = addr;
        self
    }

    /// Set credentials
    pub fn credentials(mut self, username: String, password: String) -> Self {
        self.config.username = username;
        self.config.password = password;
        self
    }

    /// Set whether to allow direct calls
    pub fn allow_direct_calls(mut self, allow: bool) -> Self {
        self.config.allow_direct_calls = allow;
        self
    }

    /// Set default media mode
    pub fn default_media_mode(mut self, mode: MediaMode) -> Self {
        self.config.default_media_mode = mode;
        self
    }

    /// Set recording configuration
    pub fn recording(mut self, enabled: bool, path: Option<String>) -> Self {
        self.config.record_by_default = enabled;
        self.config.recording_path = path;
        self
    }

    /// Build the SimpleB2bua instance
    pub async fn build(self) -> Result<SimpleB2bua> {
        SimpleB2bua::new(self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_b2bua_creation() {
        let b2bua = B2buaBuilder::new()
            .registrar("sip:test@localhost:5060".to_string())
            .credentials("user".to_string(), "pass".to_string())
            .allow_direct_calls(false)
            .build()
            .await;

        assert!(b2bua.is_ok());
    }

    #[tokio::test]
    async fn test_routing_rule_addition() {
        let b2bua = SimpleB2bua::new(B2buaConfig::default()).await.unwrap();

        let result = b2bua.add_routing_rule(
            "sip:*@gateway.com".to_string(),
            "sip:backend@server.local".to_string(),
            MediaMode::Relay,
        ).await;

        assert!(result.is_ok());
    }
}