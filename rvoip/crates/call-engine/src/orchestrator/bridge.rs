//! Bridge management policies
//!
//! This module provides bridge management policies and configuration for
//! call center bridge operations. The actual bridge operations are handled
//! by session-core APIs through the CallCenterEngine.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::{info, debug};

use crate::error::{CallCenterError, Result};
use rvoip_session_core::api::{SessionId, BridgeId};

/// Bridge type enumeration for call center operations
#[derive(Debug, Clone)]
pub enum BridgeType {
    /// Agent-customer 1:1 call
    AgentCustomer {
        agent_session: SessionId,
        customer_session: SessionId,
    },
    /// Conference call with multiple participants
    Conference {
        participants: Vec<SessionId>,
    },
    /// Supervised call with agent, customer, and supervisor
    Supervised {
        agent_session: SessionId,
        customer_session: SessionId,
        supervisor_session: SessionId,
    },
}

/// Bridge configuration for call center operations
#[derive(Debug, Clone)]
pub struct CallCenterBridgeConfig {
    /// Maximum number of participants
    pub max_participants: usize,
    /// Enable recording for this bridge
    pub enable_recording: bool,
    /// Bridge name/description
    pub name: String,
    /// Department or queue this bridge belongs to
    pub department: Option<String>,
}

/// Bridge statistics for monitoring
#[derive(Debug, Clone)]
pub struct BridgeStats {
    pub active_bridges: usize,
    pub total_sessions: usize,
}

/// Bridge management policies for call center operations
/// 
/// Note: Actual bridge operations are performed by session-core APIs
/// through the CallCenterEngine. This module provides business logic
/// and policies for bridge management.
pub struct BridgeManager {
    /// Bridge policies and configurations
    bridge_configs: HashMap<String, CallCenterBridgeConfig>,
}

impl BridgeManager {
    /// Create a new bridge manager for call center policies
    pub fn new() -> Self {
        Self {
            bridge_configs: HashMap::new(),
        }
    }
    
    /// Create bridge configuration for agent-customer calls
    pub fn create_agent_customer_config(
        &mut self,
        agent_session: SessionId,
        customer_session: SessionId,
        enable_recording: bool,
    ) -> CallCenterBridgeConfig {
        info!("🌉 Creating agent-customer bridge config: {} ↔ {}", agent_session, customer_session);
        
        CallCenterBridgeConfig {
            max_participants: 2,
            enable_recording,
            name: format!("Agent-Customer: {} ↔ {}", agent_session, customer_session),
            department: None,
        }
    }
    
    /// Create bridge configuration for conference calls
    pub fn create_conference_config(
        &mut self,
        participants: Vec<SessionId>,
        enable_recording: bool,
    ) -> CallCenterBridgeConfig {
        info!("🎙️ Creating conference bridge config with {} participants", participants.len());
        
        CallCenterBridgeConfig {
            max_participants: participants.len().max(10), // Allow growth
            enable_recording,
            name: format!("Conference with {} participants", participants.len()),
            department: None,
        }
    }
    
    /// Store bridge configuration for tracking
    pub fn store_bridge_config(&mut self, bridge_id: String, config: CallCenterBridgeConfig) {
        debug!("📋 Storing bridge config for: {}", bridge_id);
        self.bridge_configs.insert(bridge_id, config);
    }
    
    /// Get bridge configuration
    pub fn get_bridge_config(&self, bridge_id: &str) -> Option<&CallCenterBridgeConfig> {
        self.bridge_configs.get(bridge_id)
    }
    
    /// Remove bridge configuration (when bridge is destroyed)
    pub fn remove_bridge_config(&mut self, bridge_id: &str) -> Option<CallCenterBridgeConfig> {
        self.bridge_configs.remove(bridge_id)
    }
    
    /// Get bridge statistics for monitoring
    pub fn get_statistics(&self) -> BridgeStats {
        BridgeStats {
            active_bridges: self.bridge_configs.len(),
            total_sessions: self.bridge_configs.values()
                .map(|config| config.max_participants)
                .sum(),
        }
    }
}

impl Default for BridgeManager {
    fn default() -> Self {
        Self::new()
    }
} 