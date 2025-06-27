//! Call engine configuration and component definitions

use async_trait::async_trait;
use serde::{Serialize, Deserialize};

use crate::{VoipBuilderError, ComponentStatus};

/// Call engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEngineConfig {
    /// Maximum concurrent calls
    pub max_calls: u32,
    /// Call timeout duration
    pub call_timeout: std::time::Duration,
    /// Enable call recording
    pub recording_enabled: bool,
}

impl CallEngineConfig {
    /// Create a basic call engine configuration
    pub fn basic() -> Self {
        Self {
            max_calls: 1000,
            call_timeout: std::time::Duration::from_secs(300),
            recording_enabled: false,
        }
    }

    /// Create an enterprise call engine configuration
    pub fn enterprise() -> Self {
        Self {
            max_calls: 10000,
            call_timeout: std::time::Duration::from_secs(600),
            recording_enabled: true,
        }
    }
}

/// Trait for call engine components
#[async_trait]
pub trait CallEngineComponent: Send + Sync + std::fmt::Debug {
    /// Start the call engine
    async fn start(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Stop the call engine
    async fn stop(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Get component health status
    async fn health(&self) -> ComponentStatus;
    
    /// Get component configuration
 