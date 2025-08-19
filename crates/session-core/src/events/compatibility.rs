//! Compatibility layer for gradual migration from old to new event system
//!
//! This module provides adapters and bridges to allow coexistence of the old
//! tokio::broadcast system with the new infra-common based system during transition.

use std::sync::Arc;
use tokio::sync::broadcast;
use async_trait::async_trait;
use crate::manager::events::{SessionEvent, SessionEventProcessor, SessionEventSubscriber};
use crate::errors::{Result, SessionError};
use super::infra_system::InfraSessionEventSystem;
use super::federated_bus::RvoipFederatedBus;

/// Configuration for the event system adapter
#[derive(Debug, Clone)]
pub struct EventAdapterConfig {
    /// Use the new infra-common system for publishing
    pub use_new_publisher: bool,
    
    /// Use the new infra-common system for subscribing  
    pub use_new_subscriber: bool,
    
    /// Bridge events between old and new systems
    pub enable_bridge: bool,
    
    /// Capacity for the bridge channel
    pub bridge_capacity: usize,
}

impl Default for EventAdapterConfig {
    fn default() -> Self {
        Self {
            use_new_publisher: true,    // Default to new system for publishing
            use_new_subscriber: false,  // Keep old system for subscribing during transition
            enable_bridge: true,        // Bridge events between systems
            bridge_capacity: 1000,
        }
    }
}

/// Adapter that can use either old or new event system based on configuration
pub struct EventAdapter {
    config: EventAdapterConfig,
    
    // Old system (tokio::broadcast)
    old_processor: Option<Arc<SessionEventProcessor>>,
    
    // New system (infra-common)
    new_system: Option<Arc<InfraSessionEventSystem>>,
    federated_bus: Option<Arc<RvoipFederatedBus>>,
    
    // Bridge between systems
    bridge_handle: Option<tokio::task::JoinHandle<()>>,
}

impl EventAdapter {
    /// Create a new adapter with default configuration
    pub fn new() -> Self {
        Self::with_config(EventAdapterConfig::default())
    }
    
    /// Create adapter with custom configuration
    pub fn with_config(config: EventAdapterConfig) -> Self {
        Self {
            config,
            old_processor: None,
            new_system: None,
            federated_bus: None,
            bridge_handle: None,
        }
    }
    
    /// Initialize the adapter with both old and new systems
    pub async fn initialize(&mut self) -> Result<()> {
        // Initialize old system if needed
        if !self.config.use_new_publisher || !self.config.use_new_subscriber || self.config.enable_bridge {
            let old_processor = Arc::new(SessionEventProcessor::new());
            old_processor.start().await?;
            self.old_processor = Some(old_processor);
        }
        
        // Initialize new system if needed
        if self.config.use_new_publisher || self.config.use_new_subscriber {
            let new_system = Arc::new(InfraSessionEventSystem::new());
            new_system.start().await?;
            self.new_system = Some(new_system);
            
            // Also initialize federated bus
            let federated_bus = Arc::new(RvoipFederatedBus::new());
            federated_bus.start().await?;
            self.federated_bus = Some(federated_bus);
        }
        
        // Set up bridge if enabled
        if self.config.enable_bridge {
            self.setup_bridge().await?;
        }
        
        Ok(())
    }
    
    /// Publish an event using the configured publisher
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        if self.config.use_new_publisher {
            // Use new high-performance system
            if let Some(federated_bus) = &self.federated_bus {
                federated_bus.publish_event(event).await?;
            } else if let Some(new_system) = &self.new_system {
                new_system.publish_event(event).await?;
            } else {
                return Err(SessionError::internal("New system not initialized"));
            }
        } else {
            // Use old system
            if let Some(old_processor) = &self.old_processor {
                old_processor.publish_event(event).await?;
            } else {
                return Err(SessionError::internal("Old system not initialized"));
            }
        }
        
        Ok(())
    }
    
    /// Subscribe to events using the configured subscriber
    pub async fn subscribe(&self) -> Result<Box<dyn EventSubscriberTrait>> {
        if self.config.use_new_subscriber {
            // Use new system
            if let Some(federated_bus) = &self.federated_bus {
                let subscriber = federated_bus.subscribe().await?;
                Ok(Box::new(NewEventSubscriber { inner: subscriber }))
            } else if let Some(new_system) = &self.new_system {
                let subscriber = new_system.subscribe().await?;
                Ok(Box::new(NewEventSubscriber { inner: subscriber }))
            } else {
                Err(SessionError::internal("New system not initialized"))
            }
        } else {
            // Use old system
            if let Some(old_processor) = &self.old_processor {
                let subscriber = old_processor.subscribe().await?;
                Ok(Box::new(OldEventSubscriber { inner: subscriber }))
            } else {
                Err(SessionError::internal("Old system not initialized"))
            }
        }
    }
    
    /// Setup bridge between old and new systems
    async fn setup_bridge(&mut self) -> Result<()> {
        if let (Some(old_processor), Some(federated_bus)) = (&self.old_processor, &self.federated_bus) {
            let mut old_subscriber = old_processor.subscribe().await?;
            let federated_bus_clone = federated_bus.clone();
            
            // Spawn bridge task
            let handle = tokio::spawn(async move {
                loop {
                    match old_subscriber.receive().await {
                        Ok(event) => {
                            // Forward event from old to new system
                            if let Err(e) = federated_bus_clone.publish_event(event).await {
                                tracing::warn!("Failed to bridge event to new system: {}", e);
                            }
                        },
                        Err(e) => {
                            tracing::error!("Bridge subscriber error: {}", e);
                            break;
                        }
                    }
                }
            });
            
            self.bridge_handle = Some(handle);
        }
        
        Ok(())
    }
    
    /// Shutdown the adapter
    pub async fn shutdown(&mut self) -> Result<()> {
        // Stop bridge
        if let Some(handle) = self.bridge_handle.take() {
            handle.abort();
        }
        
        // Shutdown new systems
        if let Some(federated_bus) = &self.federated_bus {
            federated_bus.shutdown().await?;
        }
        if let Some(new_system) = &self.new_system {
            new_system.shutdown().await?;
        }
        
        // Shutdown old system
        if let Some(old_processor) = &self.old_processor {
            old_processor.stop().await?;
        }
        
        Ok(())
    }
}

/// Common trait for event subscribers to abstract old vs new
#[async_trait]
pub trait EventSubscriberTrait: Send {
    async fn receive(&mut self) -> Result<SessionEvent>;
    fn try_receive(&mut self) -> Result<Option<SessionEvent>>;
}

/// Wrapper for old event subscriber
pub struct OldEventSubscriber {
    inner: SessionEventSubscriber,
}

#[async_trait]
impl EventSubscriberTrait for OldEventSubscriber {
    async fn receive(&mut self) -> Result<SessionEvent> {
        self.inner.receive().await
    }
    
    fn try_receive(&mut self) -> Result<Option<SessionEvent>> {
        self.inner.try_receive()
    }
}

/// Wrapper for new event subscriber
pub struct NewEventSubscriber {
    inner: Box<dyn infra_common::events::api::EventSubscriber<SessionEvent>>,
}

#[async_trait]
impl EventSubscriberTrait for NewEventSubscriber {
    async fn receive(&mut self) -> Result<SessionEvent> {
        let event_arc = self.inner.receive().await
            .map_err(|e| SessionError::internal(&format!("New subscriber receive failed: {}", e)))?;
            
        // Extract from Arc
        match Arc::try_unwrap(event_arc) {
            Ok(event) => Ok(event),
            Err(arc) => Ok((*arc).clone()),
        }
    }
    
    fn try_receive(&mut self) -> Result<Option<SessionEvent>> {
        match self.inner.try_receive() {
            Ok(Some(event_arc)) => {
                match Arc::try_unwrap(event_arc) {
                    Ok(event) => Ok(Some(event)),
                    Err(arc) => Ok(Some((*arc).clone())),
                }
            },
            Ok(None) => Ok(None),
            Err(e) => Err(SessionError::internal(&format!("New subscriber try_receive failed: {}", e))),
        }
    }
}

impl Default for EventAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::*;
    
    #[tokio::test]
    async fn test_adapter_with_new_system() {
        let config = EventAdapterConfig {
            use_new_publisher: true,
            use_new_subscriber: true,
            enable_bridge: false,
            bridge_capacity: 1000,
        };
        
        let mut adapter = EventAdapter::with_config(config);
        adapter.initialize().await.unwrap();
        
        // Test publishing
        let event = SessionEvent::SessionCreated {
            session_id: SessionId::new(),
            from: "test@example.com".to_string(),
            to: "user@example.com".to_string(),
            call_state: CallState::Initiating,
        };
        
        adapter.publish_event(event).await.unwrap();
        
        adapter.shutdown().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_adapter_with_old_system() {
        let config = EventAdapterConfig {
            use_new_publisher: false,
            use_new_subscriber: false,
            enable_bridge: false,
            bridge_capacity: 1000,
        };
        
        let mut adapter = EventAdapter::with_config(config);
        adapter.initialize().await.unwrap();
        
        // Test publishing
        let event = SessionEvent::SessionCreated {
            session_id: SessionId::new(),
            from: "test@example.com".to_string(),
            to: "user@example.com".to_string(),
            call_state: CallState::Initiating,
        };
        
        adapter.publish_event(event).await.unwrap();
        
        adapter.shutdown().await.unwrap();
    }
}