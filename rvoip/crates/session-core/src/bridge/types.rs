//! Bridge Types
//!
//! Types for session bridging.

use crate::api::types::SessionId;

/// Bridge identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BridgeId(pub String);

impl BridgeId {
    pub fn new(id: &str) -> Self {
        Self(id.to_string())
    }
}

/// Bridge configuration
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub max_sessions: usize,
    pub auto_start: bool,
    pub auto_stop_on_empty: bool,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            max_sessions: 10,
            auto_start: true,
            auto_stop_on_empty: true,
        }
    }
} 