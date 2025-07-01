//! API server configuration and component definitions

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::net::SocketAddr;

use crate::{VoipBuilderError, ComponentStatus};

/// API server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiServerConfig {
    /// HTTP server bind address
    pub bind_address: SocketAddr,
    /// Enable REST API
    pub rest_enabled: bool,
    /// Enable WebSocket API
    pub websocket_enabled: bool,
    /// Enable GraphQL API
    pub graphql_enabled: bool,
}

impl ApiServerConfig {
    /// Create a REST-only API server configuration
    pub fn rest_only() -> Self {
        Self {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            rest_enabled: true,
            websocket_enabled: false,
            graphql_enabled: false,
        }
    }

    /// Create a REST and WebSocket API server configuration
    pub fn rest_and_websocket() -> Self {
        Self {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            rest_enabled: true,
            websocket_enabled: true,
            graphql_enabled: false,
        }
    }
}

/// Trait for API server components
#[async_trait]
pub trait ApiServerComponent: Send + Sync + std::fmt::Debug {
    /// Start the API server
    async fn start(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Stop the API server
    async fn stop(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Get component health status
    async fn health(&self) -> ComponentStatus;
    
    /// Get component configuration
    fn config(&self) -> &ApiServerConfig;
} 