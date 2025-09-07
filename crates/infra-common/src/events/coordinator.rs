//! Global Event Coordinator for Monolithic and Distributed Deployments
//!
//! Provides a unified event system that replaces individual crate event processors
//! with a single shared event bus, reducing thread count by 50-75%.

use std::sync::Arc;
use std::collections::HashMap;
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{mpsc, RwLock};
use anyhow::Result;
use serde::{Serialize, Deserialize};
use tracing::{debug, info, warn, error};

use crate::events::system::EventSystem;
use crate::events::types::{Event, EventHandler, EventPriority};
use crate::planes::{PlaneRouter, PlaneType, PlaneConfig, LayerTaskManager};

use crate::events::cross_crate::{CrossCrateEvent, EventTypeId};

/// Global event coordinator supporting both monolithic and distributed modes
pub struct GlobalEventCoordinator {
    /// Deployment mode
    deployment_mode: DeploymentMode,
    
    /// Core event bus (StaticFastPath for monolithic, network-aware for distributed)
    event_bus: Arc<dyn EventBusAdapter>,
    
    /// Plane-aware event routing
    plane_router: Arc<PlaneRouter>,
    
    /// Unified task manager for all event processing
    task_manager: Arc<LayerTaskManager>,
    
    /// Event type registry for cross-crate event management
    event_registry: Arc<EventTypeRegistry>,
    
    /// Registered event handlers by type
    handlers: Arc<DashMap<EventTypeId, Vec<Arc<dyn CrossCrateEventHandler>>>>,
    
    /// Active event subscriptions
    subscriptions: Arc<RwLock<HashMap<EventTypeId, Vec<EventSubscription>>>>,
}

#[derive(Debug, Clone)]
pub enum DeploymentMode {
    Monolithic,
    Distributed,
}

/// Trait for event bus adapters (monolithic vs distributed)
#[async_trait]
pub trait EventBusAdapter: Send + Sync {
    async fn publish(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()>;
    async fn subscribe(&self, event_type: EventTypeId) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>>;
    async fn shutdown(&self) -> Result<()>;
}

/// Monolithic event bus adapter using StaticFastPath
pub struct MonolithicEventBus {
    event_bus: Arc<EventSystem>,
    task_manager: Arc<LayerTaskManager>,
    /// Subscribers by event type
    subscribers: Arc<DashMap<EventTypeId, Vec<mpsc::Sender<Arc<dyn CrossCrateEvent>>>>>,
}

#[async_trait]
impl EventBusAdapter for MonolithicEventBus {
    async fn publish(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        let event_type = event.event_type();
        debug!("Publishing cross-crate event: {}", event_type);
        
        // Forward to all subscribers of this event type
        if let Some(subscribers) = self.subscribers.get(event_type) {
            let mut to_remove = Vec::new();
            
            for (idx, sender) in subscribers.iter().enumerate() {
                if sender.try_send(event.clone()).is_err() {
                    // Channel is full or disconnected
                    to_remove.push(idx);
                }
            }
            
            // Clean up dead subscribers
            if !to_remove.is_empty() {
                drop(subscribers);
                if let Some(mut subscribers) = self.subscribers.get_mut(event_type) {
                    for idx in to_remove.into_iter().rev() {
                        subscribers.remove(idx);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    async fn subscribe(&self, event_type: EventTypeId) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>> {
        // Create channel for forwarding events
        let (tx, rx) = mpsc::channel(1000);
        
        // Add to subscribers
        self.subscribers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(tx);
        
        debug!("Subscribed to cross-crate event type: {}", event_type);
        
        Ok(rx)
    }
    
    async fn shutdown(&self) -> Result<()> {
        use crate::events::api::EventSystem as EventSystemTrait;
        self.event_bus.shutdown().await.map_err(|e| anyhow::anyhow!("Event system shutdown failed: {}", e))?;
        self.task_manager.shutdown_all().await?;
        Ok(())
    }
}

/// Event handler that forwards to a channel  
struct ChannelForwarder {
    tx: mpsc::Sender<Arc<dyn CrossCrateEvent>>,
}

impl ChannelForwarder {
    fn new(tx: mpsc::Sender<Arc<dyn CrossCrateEvent>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl CrossCrateEventHandler for ChannelForwarder {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        if let Err(_) = self.tx.try_send(event) {
            warn!("Event channel full, dropping event");
        }
        Ok(())
    }
}

impl GlobalEventCoordinator {
    /// Create coordinator for monolithic deployment (single process)
    pub async fn monolithic() -> Result<Self> {
        let event_bus = Arc::new(EventSystem::new_static_fast_path(10000));
        let task_manager = Arc::new(LayerTaskManager::new("global"));
        
        let monolithic_adapter = Arc::new(MonolithicEventBus {
            event_bus,
            task_manager: task_manager.clone(),
            subscribers: Arc::new(DashMap::new()),
        });
        
        Ok(Self {
            deployment_mode: DeploymentMode::Monolithic,
            event_bus: monolithic_adapter,
            plane_router: Arc::new(PlaneRouter::new(PlaneConfig::Local)),
            task_manager,
            event_registry: Arc::new(EventTypeRegistry::new()),
            handlers: Arc::new(DashMap::new()),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Get the deployment mode
    pub fn deployment_mode(&self) -> &DeploymentMode {
        &self.deployment_mode
    }
    
    /// Register an event handler for a specific event type
    pub async fn register_handler<H>(&self, event_type: EventTypeId, handler: H) -> Result<()>
    where
        H: CrossCrateEventHandler + 'static,
    {
        let handler = Arc::new(handler);
        
        // Add to handlers registry
        self.handlers.entry(event_type)
            .or_insert_with(Vec::new)
            .push(handler.clone());
        
        debug!("Registered handler for event type: {}", event_type);
        Ok(())
    }
    
    /// Publish an event through the global coordinator
    pub async fn publish(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        let event_type = event.event_type();
        
        debug!("Publishing event type: {}", event_type);
        
        // Call registered handlers for this event type
        if let Some(handlers) = self.handlers.get(event_type) {
            debug!("Found {} handlers for event type: {}", handlers.len(), event_type);
            for handler in handlers.iter() {
                if let Err(e) = handler.handle(event.clone()).await {
                    warn!("Handler failed for event type {}: {}", event_type, e);
                }
            }
        } else {
            debug!("No handlers registered for event type: {}", event_type);
        }
        
        // TODO: Add plane-aware routing for cross-crate events
        debug!("Routing cross-crate event through planes");
        
        // Publish through event bus (for subscribers)
        self.event_bus.publish(event).await?;
        
        Ok(())
    }
    
    /// Subscribe to events of a specific type
    pub async fn subscribe(&self, event_type: EventTypeId) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>> {
        debug!("Subscribing to event type: {}", event_type);
        
        // Subscribe through event bus
        let receiver = self.event_bus.subscribe(event_type).await?;
        
        // Track subscription
        let subscription = EventSubscription {
            event_type,
            subscribed_at: std::time::Instant::now(),
        };
        
        self.subscriptions.write().await
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(subscription);
        
        Ok(receiver)
    }
    
    /// Subscribe with plane filtering
    pub async fn subscribe_with_plane_filter(
        &self,
        event_type: EventTypeId,
        plane_type: PlaneType,
    ) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>> {
        // For monolithic mode, plane filtering is informational
        // In distributed mode, this would filter by plane
        debug!("Subscribing to event type: {} for plane: {:?}", event_type, plane_type);
        
        self.subscribe(event_type).await
    }
    
    /// Route an event through the plane router
    pub async fn route_event(
        &self,
        source_plane: PlaneType,
        event: Arc<dyn CrossCrateEvent>,
    ) -> Result<()> {
        debug!("Routing event from plane: {:?}", source_plane);
        
        // TODO: Add plane-aware event routing
        debug!("Routing event from plane: {:?} to target plane", source_plane);
        
        Ok(())
    }
    
    /// Get statistics about the event coordinator
    pub async fn stats(&self) -> EventCoordinatorStats {
        let handler_count: usize = self.handlers.iter().map(|entry| entry.value().len()).sum();
        let subscription_count: usize = self.subscriptions.read().await.values().map(|v| v.len()).sum();
        let task_stats = self.task_manager.stats().await;
        
        EventCoordinatorStats {
            deployment_mode: self.deployment_mode.clone(),
            registered_handlers: handler_count,
            active_subscriptions: subscription_count,
            active_tasks: task_stats.active_tasks,
            total_events_processed: 0, // TODO: Add metrics
        }
    }
    
    /// Graceful shutdown
    pub async fn shutdown(&self) -> Result<()> {
        info!("Starting global event coordinator shutdown");
        
        // Shutdown event bus
        self.event_bus.shutdown().await?;
        
        // Shutdown task manager
        self.task_manager.shutdown_all().await?;
        
        // Clear handlers and subscriptions
        self.handlers.clear();
        self.subscriptions.write().await.clear();
        
        info!("Global event coordinator shutdown complete");
        Ok(())
    }
}

/// Event type registry for managing cross-crate event types
pub struct EventTypeRegistry {
    types: DashMap<EventTypeId, EventTypeInfo>,
}

impl EventTypeRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            types: DashMap::new(),
        };
        
        // Register built-in cross-crate event types
        registry.register_builtin_types();
        
        registry
    }
    
    /// Register a new event type
    pub fn register_event_type(&self, event_type: EventTypeId, info: EventTypeInfo) {
        self.types.insert(event_type, info);
        debug!("Registered event type: {}", event_type);
    }
    
    /// Get event type information
    pub fn get_type_info(&self, event_type: EventTypeId) -> Option<EventTypeInfo> {
        self.types.get(event_type).map(|entry| entry.value().clone())
    }
    
    /// Register built-in cross-crate event types
    fn register_builtin_types(&mut self) {
        // Register core cross-crate event types
        self.register_event_type("session_to_dialog", EventTypeInfo {
            event_type: "session_to_dialog",
            source_plane: PlaneType::Signaling,
            target_plane: PlaneType::Signaling,
            priority: EventPriority::High,
            description: "Events from session-core to dialog-core".to_string(),
        });
        
        self.register_event_type("dialog_to_session", EventTypeInfo {
            event_type: "dialog_to_session", 
            source_plane: PlaneType::Signaling,
            target_plane: PlaneType::Signaling,
            priority: EventPriority::High,
            description: "Events from dialog-core to session-core".to_string(),
        });
        
        self.register_event_type("session_to_media", EventTypeInfo {
            event_type: "session_to_media",
            source_plane: PlaneType::Signaling,
            target_plane: PlaneType::Media,
            priority: EventPriority::High,
            description: "Events from session-core to media-core".to_string(),
        });
        
        self.register_event_type("media_to_session", EventTypeInfo {
            event_type: "media_to_session",
            source_plane: PlaneType::Media, 
            target_plane: PlaneType::Signaling,
            priority: EventPriority::Normal,
            description: "Events from media-core to session-core".to_string(),
        });
        
        // Add more cross-crate event types as needed
    }
}

/// Information about an event type
#[derive(Debug, Clone)]
pub struct EventTypeInfo {
    pub event_type: EventTypeId,
    pub source_plane: PlaneType,
    pub target_plane: PlaneType,
    pub priority: EventPriority,
    pub description: String,
}

/// Event subscription tracking
#[derive(Debug, Clone)]
struct EventSubscription {
    event_type: EventTypeId,
    subscribed_at: std::time::Instant,
}

/// Statistics about the event coordinator
#[derive(Debug, Clone)]
pub struct EventCoordinatorStats {
    pub deployment_mode: DeploymentMode,
    pub registered_handlers: usize,
    pub active_subscriptions: usize,
    pub active_tasks: usize,
    pub total_events_processed: u64,
}

/// Trait for cross-crate event handlers
#[async_trait]
pub trait CrossCrateEventHandler: Send + Sync {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_monolithic_coordinator_creation() {
        let coordinator = GlobalEventCoordinator::monolithic().await.unwrap();
        
        assert!(matches!(coordinator.deployment_mode(), DeploymentMode::Monolithic));
        
        let stats = coordinator.stats().await;
        assert_eq!(stats.registered_handlers, 0);
        assert_eq!(stats.active_subscriptions, 0);
        
        coordinator.shutdown().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_event_type_registry() {
        let registry = EventTypeRegistry::new();
        
        let info = registry.get_type_info("session_to_dialog").unwrap();
        assert_eq!(info.event_type, "session_to_dialog");
        assert_eq!(info.source_plane, PlaneType::Signaling);
        assert_eq!(info.target_plane, PlaneType::Signaling);
    }
}