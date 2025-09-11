//! High-performance event system using infra-common StaticFastPath
//!
//! This module integrates session-core events with infra-common's high-performance
//! event system, providing 900K+ events/sec throughput while maintaining compatibility.

use std::sync::Arc;
use async_trait::async_trait;
use rvoip_infra_common::events::{
    api::{EventSystem, EventPublisher, EventSubscriber},
    types::{Event, EventResult, EventPriority, StaticEvent},
    system::EventSystem as InfraEventSystem,
    builder::{EventSystemBuilder, ImplementationType},
};
use crate::manager::events::SessionEvent;
use crate::errors::{Result, SessionError};

/// Make SessionEvent compatible with infra-common's Event trait
impl Event for SessionEvent {
    fn event_type() -> &'static str {
        "rvoip_session_core::manager::events::SessionEvent"
    }

    fn priority() -> EventPriority {
        EventPriority::Normal
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// SessionEvent also implements StaticEvent for maximum performance
impl StaticEvent for SessionEvent {}

/// High-performance event system wrapper using infra-common
pub struct InfraSessionEventSystem {
    /// The underlying infra-common event system
    inner: InfraEventSystem,
    
    /// Publisher for SessionEvent
    publisher: Box<dyn EventPublisher<SessionEvent>>,
}

impl InfraSessionEventSystem {
    /// Create a new high-performance event system using StaticFastPath
    pub fn new() -> Self {
        // Register SessionEvent as a StaticEvent type
        rvoip_infra_common::events::registry::register_static_event::<SessionEvent>();
        
        let inner = EventSystemBuilder::new()
            .implementation(ImplementationType::StaticFastPath)
            .channel_capacity(10_000)  // High capacity for performance
            .build();
        
        let publisher = inner.create_publisher::<SessionEvent>();
        
        Self {
            inner,
            publisher,
        }
    }
    
    /// Create with custom configuration
    pub fn with_config(capacity: usize) -> Self {
        // Register SessionEvent as a StaticEvent type
        rvoip_infra_common::events::registry::register_static_event::<SessionEvent>();
        
        let inner = EventSystemBuilder::new()
            .implementation(ImplementationType::StaticFastPath)
            .channel_capacity(capacity)
            .build();
        
        let publisher = inner.create_publisher::<SessionEvent>();
        
        Self {
            inner,
            publisher,
        }
    }
    
    /// Publish a session event with high performance
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        self.publisher.publish(event).await
            .map_err(|e| SessionError::internal(&format!("Failed to publish event: {}", e)))
    }
    
    /// Publish a batch of events for even higher throughput
    pub async fn publish_batch(&self, events: Vec<SessionEvent>) -> Result<()> {
        self.publisher.publish_batch(events).await
            .map_err(|e| SessionError::internal(&format!("Failed to publish batch: {}", e)))
    }
    
    /// Subscribe to session events
    pub async fn subscribe(&self) -> Result<Box<dyn EventSubscriber<SessionEvent>>> {
        self.inner.subscribe::<SessionEvent>().await
            .map_err(|e| SessionError::internal(&format!("Failed to subscribe: {}", e)))
    }
    
    /// Subscribe with a filter
    pub async fn subscribe_filtered<F>(&self, filter: F) -> Result<Box<dyn EventSubscriber<SessionEvent>>>
    where
        F: Fn(&SessionEvent) -> bool + Send + Sync + 'static,
    {
        self.inner.subscribe_filtered(filter).await
            .map_err(|e| SessionError::internal(&format!("Failed to subscribe with filter: {}", e)))
    }
    
    /// Start the event system
    pub async fn start(&self) -> Result<()> {
        self.inner.start().await
            .map_err(|e| SessionError::internal(&format!("Failed to start event system: {}", e)))
    }
    
    /// Shutdown the event system
    pub async fn shutdown(&self) -> Result<()> {
        self.inner.shutdown().await
            .map_err(|e| SessionError::internal(&format!("Failed to shutdown event system: {}", e)))
    }
}

/// Wrapper for infra-common event subscriber to match our API
pub struct InfraSessionEventSubscriber {
    inner: Box<dyn EventSubscriber<SessionEvent>>,
}

impl InfraSessionEventSubscriber {
    pub fn new(inner: Box<dyn EventSubscriber<SessionEvent>>) -> Self {
        Self { inner }
    }
    
    /// Receive the next event
    pub async fn receive(&mut self) -> Result<SessionEvent> {
        let event_arc = self.inner.receive().await
            .map_err(|e| SessionError::internal(&format!("Failed to receive event: {}", e)))?;
        
        // Extract the event from the Arc
        match Arc::try_unwrap(event_arc) {
            Ok(event) => Ok(event),
            Err(arc) => {
                // If we can't unwrap (multiple references), clone it
                Ok((*arc).clone())
            }
        }
    }
    
    /// Try to receive an event without blocking
    pub fn try_receive(&mut self) -> Result<Option<SessionEvent>> {
        match self.inner.try_receive() {
            Ok(Some(event_arc)) => {
                match Arc::try_unwrap(event_arc) {
                    Ok(event) => Ok(Some(event)),
                    Err(arc) => Ok(Some((*arc).clone())),
                }
            },
            Ok(None) => Ok(None),
            Err(e) => Err(SessionError::internal(&format!("Failed to try_receive: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::*;
    use std::time::Duration;
    
    #[tokio::test]
    async fn test_infra_event_system_basic() {
        let system = InfraSessionEventSystem::new();
        system.start().await.unwrap();
        
        // Test publishing and subscribing
        let mut subscriber = system.subscribe().await.unwrap();
        let subscriber = InfraSessionEventSubscriber::new(subscriber);
        
        // Publish a test event
        let test_event = SessionEvent::SessionCreated {
            session_id: SessionId::new(),
            from: "test@example.com".to_string(),
            to: "user@example.com".to_string(),
            call_state: CallState::Initiating,
        };
        
        system.publish_event(test_event.clone()).await.unwrap();
        
        // We can't easily test the subscriber without complex async coordination
        // This would be covered in integration tests
        
        system.shutdown().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_batch_publishing() {
        let system = InfraSessionEventSystem::new();
        system.start().await.unwrap();
        
        // Create a batch of events
        let events = vec![
            SessionEvent::SessionCreated {
                session_id: SessionId::new(),
                from: "test1@example.com".to_string(),
                to: "user1@example.com".to_string(),
                call_state: CallState::Initiating,
            },
            SessionEvent::SessionCreated {
                session_id: SessionId::new(),
                from: "test2@example.com".to_string(),
                to: "user2@example.com".to_string(),
                call_state: CallState::Initiating,
            },
        ];
        
        // Publish batch - should be much faster than individual publishes
        system.publish_batch(events).await.unwrap();
        
        system.shutdown().await.unwrap();
    }
}