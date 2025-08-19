//! Federated event bus for plane-aware routing
//!
//! This module provides the RvoipFederatedBus that can route events between
//! Transport, Media, and Signaling planes whether they're local or distributed.

use std::sync::Arc;
use async_trait::async_trait;
use infra_common::events::{
    api::{EventSystem, EventPublisher, EventSubscriber},
    builder::{EventSystemBuilder, ImplementationType},
};
use crate::manager::events::SessionEvent;
use crate::errors::{Result, SessionError};

/// Event routing affinity for federated deployment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventAffinity {
    /// Event stays within the same plane (intra-plane)
    IntraPlane,
    /// Event crosses plane boundaries (inter-plane)
    InterPlane { target_plane: PlaneType },
    /// Event needs to reach all planes
    GlobalBroadcast,
}

/// The three federated planes in RVOIP architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaneType {
    /// Transport plane: sip-transport + rtp-core transport
    Transport,
    /// Media plane: media-core + rtp-core media processing  
    Media,
    /// Signaling plane: session-core + dialog-core + transaction-core
    Signaling,
}

/// Plane deployment mode
#[derive(Debug, Clone)]
pub enum PlaneDeployment {
    /// Plane runs locally in the same process
    Local,
    /// Plane runs on a remote server
    Remote { endpoint: String },
    /// Hybrid deployment with multiple instances
    Hybrid { instances: Vec<String> },
}

/// Configuration for the federated bus
#[derive(Debug, Clone)]
pub struct FederatedBusConfig {
    /// Local plane type for this instance
    pub local_plane: PlaneType,
    
    /// Deployment configuration for each plane
    pub plane_deployments: std::collections::HashMap<PlaneType, PlaneDeployment>,
    
    /// Event system capacity
    pub capacity: usize,
    
    /// Enable high-performance mode
    pub use_static_fast_path: bool,
}

impl Default for FederatedBusConfig {
    fn default() -> Self {
        let mut deployments = std::collections::HashMap::new();
        deployments.insert(PlaneType::Transport, PlaneDeployment::Local);
        deployments.insert(PlaneType::Media, PlaneDeployment::Local);
        deployments.insert(PlaneType::Signaling, PlaneDeployment::Local);
        
        Self {
            local_plane: PlaneType::Signaling,
            plane_deployments: deployments,
            capacity: 10_000,
            use_static_fast_path: true,
        }
    }
}

/// High-performance federated event bus
pub struct RvoipFederatedBus {
    /// Configuration
    config: FederatedBusConfig,
    
    /// Local event system (infra-common)
    local_system: Arc<infra_common::events::system::EventSystem>,
    
    /// Local publisher
    local_publisher: Box<dyn EventPublisher<SessionEvent>>,
    
    /// Network transport for remote events (future Phase 3)
    /// TODO: Implement QUIC transport in Phase 3
    network_transport: Option<Arc<dyn NetworkTransport>>,
}

/// Network transport trait (stub for Phase 3)
#[async_trait]
pub trait NetworkTransport: Send + Sync {
    async fn send_to_plane(&self, plane: PlaneType, event: SessionEvent) -> Result<()>;
    async fn broadcast(&self, event: SessionEvent) -> Result<()>;
}

impl RvoipFederatedBus {
    /// Create a new federated bus with default configuration (monolithic)
    pub fn new() -> Self {
        Self::with_config(FederatedBusConfig::default())
    }
    
    /// Create with custom configuration
    pub fn with_config(config: FederatedBusConfig) -> Self {
        // Register SessionEvent as a StaticEvent type
        infra_common::events::registry::register_static_event::<SessionEvent>();
        
        let implementation = if config.use_static_fast_path {
            ImplementationType::StaticFastPath
        } else {
            ImplementationType::ZeroCopy
        };
        
        let local_system = Arc::new(
            EventSystemBuilder::new()
                .implementation(implementation)
                .channel_capacity(config.capacity)
                .build()
        );
        
        let local_publisher = local_system.create_publisher::<SessionEvent>();
        
        Self {
            config,
            local_system,
            local_publisher,
            network_transport: None, // TODO: Phase 3
        }
    }
    
    /// Start the federated bus
    pub async fn start(&self) -> Result<()> {
        self.local_system.start().await
            .map_err(|e| SessionError::internal(&format!("Failed to start local system: {}", e)))
    }
    
    /// Shutdown the federated bus
    pub async fn shutdown(&self) -> Result<()> {
        self.local_system.shutdown().await
            .map_err(|e| SessionError::internal(&format!("Failed to shutdown local system: {}", e)))
    }
    
    /// Publish an event with intelligent routing
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        let affinity = self.determine_event_affinity(&event);
        
        match affinity {
            EventAffinity::IntraPlane => {
                // Event stays local - use high-performance local system
                self.local_publisher.publish(event).await
                    .map_err(|e| SessionError::internal(&format!("Local publish failed: {}", e)))
            },
            EventAffinity::InterPlane { target_plane } => {
                // Check if target plane is local or remote
                match self.config.plane_deployments.get(&target_plane) {
                    Some(PlaneDeployment::Local) => {
                        // Target is local, use local system
                        self.local_publisher.publish(event).await
                            .map_err(|e| SessionError::internal(&format!("Local inter-plane publish failed: {}", e)))
                    },
                    Some(PlaneDeployment::Remote { .. }) => {
                        // Target is remote, use network transport
                        if let Some(transport) = &self.network_transport {
                            transport.send_to_plane(target_plane, event).await
                        } else {
                            // Fallback to local for now (Phase 3 will implement network)
                            self.local_publisher.publish(event).await
                                .map_err(|e| SessionError::internal(&format!("Remote publish fallback failed: {}", e)))
                        }
                    },
                    _ => {
                        // Unknown deployment, default to local
                        self.local_publisher.publish(event).await
                            .map_err(|e| SessionError::internal(&format!("Unknown deployment publish failed: {}", e)))
                    }
                }
            },
            EventAffinity::GlobalBroadcast => {
                // Broadcast to all planes
                if let Some(transport) = &self.network_transport {
                    transport.broadcast(event.clone()).await?;
                }
                // Also publish locally
                self.local_publisher.publish(event).await
                    .map_err(|e| SessionError::internal(&format!("Global broadcast failed: {}", e)))
            }
        }
    }
    
    /// Subscribe to events on the local plane
    pub async fn subscribe(&self) -> Result<Box<dyn EventSubscriber<SessionEvent>>> {
        self.local_system.subscribe::<SessionEvent>().await
            .map_err(|e| SessionError::internal(&format!("Subscribe failed: {}", e)))
    }
    
    /// Subscribe with filtering
    pub async fn subscribe_filtered<F>(&self, filter: F) -> Result<Box<dyn EventSubscriber<SessionEvent>>>
    where
        F: Fn(&SessionEvent) -> bool + Send + Sync + 'static,
    {
        self.local_system.subscribe_filtered(filter).await
            .map_err(|e| SessionError::internal(&format!("Filtered subscribe failed: {}", e)))
    }
    
    /// Determine event affinity for intelligent routing
    fn determine_event_affinity(&self, event: &SessionEvent) -> EventAffinity {
        match event {
            // High-frequency media events stay within media plane
            SessionEvent::RtpPacketProcessed { .. } |
            SessionEvent::AudioFrameReceived { .. } |
            SessionEvent::AudioFrameRequested { .. } |
            SessionEvent::MediaQuality { .. } => EventAffinity::IntraPlane,
            
            // Media negotiation crosses signaling <-> media boundary
            SessionEvent::MediaNegotiated { .. } |
            SessionEvent::SdpNegotiationRequested { .. } |
            SessionEvent::SdpEvent { .. } => EventAffinity::InterPlane { target_plane: PlaneType::Media },
            
            // Transfer and call control stay in signaling plane
            SessionEvent::IncomingTransferRequest { .. } |
            SessionEvent::TransferProgress { .. } |
            SessionEvent::SessionCreated { .. } |
            SessionEvent::StateChanged { .. } => EventAffinity::IntraPlane,
            
            // Critical events broadcast to all planes
            SessionEvent::SessionTerminated { .. } |
            SessionEvent::Error { .. } => EventAffinity::GlobalBroadcast,
            
            // Default to intra-plane for safety
            _ => EventAffinity::IntraPlane,
        }
    }
}

impl Default for RvoipFederatedBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::*;
    
    #[tokio::test]
    async fn test_federated_bus_local_mode() {
        let bus = RvoipFederatedBus::new();
        bus.start().await.unwrap();
        
        // Test publishing events
        let event = SessionEvent::SessionCreated {
            session_id: SessionId::new(),
            from: "test@example.com".to_string(),
            to: "user@example.com".to_string(),
            call_state: CallState::Initiating,
        };
        
        bus.publish_event(event).await.unwrap();
        
        bus.shutdown().await.unwrap();
    }
    
    #[test]
    fn test_event_affinity_classification() {
        let bus = RvoipFederatedBus::new();
        
        // Test RTP events stay intra-plane
        let rtp_event = SessionEvent::RtpPacketProcessed {
            session_id: SessionId::new(),
            processing_type: crate::media::types::RtpProcessingType::ZeroCopy,
            performance_metrics: crate::media::types::RtpProcessingMetrics {
                zero_copy_packets_processed: 1,
                traditional_packets_processed: 0,
                fallback_events: 0,
                average_processing_time_zero_copy: 1.0,
                average_processing_time_traditional: 0.0,
                allocation_reduction_percentage: 90.0,
            },
        };
        
        assert_eq!(bus.determine_event_affinity(&rtp_event), EventAffinity::IntraPlane);
        
        // Test session termination broadcasts globally
        let term_event = SessionEvent::SessionTerminated {
            session_id: SessionId::new(),
            reason: "Test termination".to_string(),
        };
        
        assert_eq!(bus.determine_event_affinity(&term_event), EventAffinity::GlobalBroadcast);
    }
}