//! NATS transport implementation for distributed event coordination
//!
//! This is currently a stub implementation that will be completed
//! when distributed mode is fully implemented.

use async_trait::async_trait;
use anyhow::Result;
use std::sync::Arc;

use super::{NetworkTransport, TransportReceiver, TransportReceiverImpl};
use crate::events::cross_crate::CrossCrateEvent;

/// NATS transport implementation
pub struct NatsTransport {
    /// NATS server URLs
    servers: Vec<String>,
    /// Optional cluster name
    cluster: Option<String>,
    /// Service name for this instance
    service_name: String,
}

impl NatsTransport {
    /// Create a new NATS transport
    pub fn new(
        servers: Vec<String>,
        cluster: Option<String>,
        service_name: String,
    ) -> Self {
        Self {
            servers,
            cluster,
            service_name,
        }
    }
    
    /// Connect to NATS servers
    pub async fn connect(&mut self) -> Result<()> {
        // TODO: Implement NATS connection
        tracing::warn!(
            "NATS transport connect() called but not yet implemented. \
            Servers: {:?}, Cluster: {:?}",
            self.servers,
            self.cluster
        );
        Err(anyhow::anyhow!("NATS transport not yet implemented"))
    }
}

#[async_trait]
impl NetworkTransport for NatsTransport {
    async fn send(&self, target: &str, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        tracing::warn!(
            "NATS transport send() called but not yet implemented. \
            Target: {}, Event: {}",
            target,
            event.event_type()
        );
        Err(anyhow::anyhow!(
            "NATS transport not yet implemented. \
            Cannot send event '{}' to target '{}'",
            event.event_type(),
            target
        ))
    }
    
    async fn subscribe(&self, event_types: Vec<&str>) -> Result<TransportReceiver> {
        tracing::warn!(
            "NATS transport subscribe() called but not yet implemented. \
            Event types: {:?}",
            event_types
        );
        Ok(TransportReceiver::new(Box::new(NatsReceiver::new())))
    }
    
    async fn health_check(&self) -> Result<()> {
        tracing::warn!("NATS transport health_check() called but not yet implemented");
        Err(anyhow::anyhow!("NATS transport not yet implemented"))
    }
    
    async fn shutdown(&self) -> Result<()> {
        tracing::info!("NATS transport shutdown() called (no-op for stub)");
        Ok(())
    }
}

/// NATS receiver implementation
struct NatsReceiver {
    // TODO: Add NATS subscription handle
}

impl NatsReceiver {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl TransportReceiverImpl for NatsReceiver {
    async fn recv(&mut self) -> Result<Option<Arc<dyn CrossCrateEvent>>> {
        tracing::warn!("NATS receiver recv() called but not yet implemented");
        // Return None to indicate no events available
        // In a real implementation, this would block until an event arrives
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_nats_transport_creation() {
        let transport = NatsTransport::new(
            vec!["nats://localhost:4222".to_string()],
            Some("test-cluster".to_string()),
            "test-service".to_string(),
        );
        
        assert_eq!(transport.servers.len(), 1);
        assert_eq!(transport.cluster, Some("test-cluster".to_string()));
        assert_eq!(transport.service_name, "test-service");
    }
    
    #[tokio::test]
    async fn test_nats_transport_not_implemented() {
        let mut transport = NatsTransport::new(
            vec!["nats://localhost:4222".to_string()],
            None,
            "test-service".to_string(),
        );
        
        // All methods should return not implemented errors
        assert!(transport.connect().await.is_err());
        assert!(transport.health_check().await.is_err());
        
        // Shutdown should succeed (no-op)
        assert!(transport.shutdown().await.is_ok());
    }
}
