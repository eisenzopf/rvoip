//! Plane-aware event routing
//!
//! Routes events efficiently between planes based on affinity and deployment

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use anyhow::Result;
use dashmap::DashMap;
use tokio::sync::RwLock;

use super::{PlaneConfig, FederatedPlane};

/// Event type identifier
pub type EventTypeId = &'static str;

/// Simple event trait for routing (without Serialize/Deserialize)
pub trait RoutableEvent: Send + Sync + 'static {
    fn event_type(&self) -> EventTypeId;
    fn session_id(&self) -> Option<&str>;
}

/// Types of planes in the federated architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlaneType {
    Transport,
    Media,
    Signaling,
}

impl PlaneType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlaneType::Transport => "transport",
            PlaneType::Media => "media",
            PlaneType::Signaling => "signaling",
        }
    }
}

/// Event affinity determines routing behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventAffinity {
    /// Event stays within the plane
    IntraPlane,
    
    /// Event crosses plane boundaries
    InterPlane {
        source: PlaneType,
        target: PlaneType,
        priority: EventPriority,
    },
    
    /// Event needs global broadcast
    GlobalBroadcast,
    
    /// Event can be batched for efficiency
    Batchable {
        max_batch_size: usize,
        timeout: std::time::Duration,
    },
    
    /// Event requires session affinity
    SessionBound {
        session_id: String,
    },
}

/// Event priority for routing decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventPriority {
    Critical,  // Must be delivered immediately
    High,      // Prioritize delivery
    Normal,    // Standard delivery
    Low,       // Can be delayed/batched
}

/// Plane router for intelligent event routing
pub struct PlaneRouter {
    /// Registered planes
    planes: Arc<DashMap<PlaneType, Arc<dyn FederatedPlane>>>,
    
    /// Event routing table
    routing_table: Arc<RwLock<RoutingTable>>,
    
    /// Session affinity mapping
    session_affinity: Arc<DashMap<String, PlaneType>>,
    
    /// Deployment configuration
    deployment_config: Arc<PlaneConfig>,
    
    /// Metrics collector
    metrics: Arc<RoutingMetrics>,
}

impl PlaneRouter {
    /// Create a new plane router
    pub fn new(deployment_config: PlaneConfig) -> Self {
        Self {
            planes: Arc::new(DashMap::new()),
            routing_table: Arc::new(RwLock::new(RoutingTable::default())),
            session_affinity: Arc::new(DashMap::new()),
            deployment_config: Arc::new(deployment_config),
            metrics: Arc::new(RoutingMetrics::new()),
        }
    }
    
    /// Register a plane with the router
    pub async fn register_plane(&self, plane: Arc<dyn FederatedPlane>) -> Result<()> {
        let plane_type = plane.plane_type();
        self.planes.insert(plane_type, plane);
        
        // Update routing table
        let mut table = self.routing_table.write().await;
        table.update_plane_status(plane_type, true);
        
        Ok(())
    }
    
    /// Unregister a plane
    pub async fn unregister_plane(&self, plane_type: PlaneType) -> Result<()> {
        self.planes.remove(&plane_type);
        
        // Update routing table
        let mut table = self.routing_table.write().await;
        table.update_plane_status(plane_type, false);
        
        Ok(())
    }
    
    /// Route an event to the appropriate plane(s)
    pub async fn route_event(&self, event: Arc<dyn RoutableEvent>) -> Result<()> {
        let affinity = self.determine_affinity(&event).await?;
        
        match affinity {
            EventAffinity::IntraPlane => {
                // Event stays within originating plane
                self.route_intra_plane(event).await?;
            }
            EventAffinity::InterPlane { target, priority, .. } => {
                // Route to specific target plane
                self.route_inter_plane(event, target, priority).await?;
            }
            EventAffinity::GlobalBroadcast => {
                // Broadcast to all planes
                self.broadcast_event(event).await?;
            }
            EventAffinity::Batchable { .. } => {
                // Add to batch for later delivery
                self.add_to_batch(event).await?;
            }
            EventAffinity::SessionBound { session_id } => {
                // Route based on session affinity
                self.route_session_bound(event, &session_id).await?;
            }
        }
        
        self.metrics.record_routed_event();
        Ok(())
    }
    
    /// Determine event affinity based on event type and content
    async fn determine_affinity(&self, event: &Arc<dyn RoutableEvent>) -> Result<EventAffinity> {
        // This would be implemented based on event type and routing rules
        // For now, returning a default
        Ok(EventAffinity::IntraPlane)
    }
    
    /// Route event within the same plane
    async fn route_intra_plane(&self, event: Arc<dyn RoutableEvent>) -> Result<()> {
        // Implementation would depend on the specific plane
        Ok(())
    }
    
    /// Route event to another plane
    async fn route_inter_plane(
        &self,
        event: Arc<dyn RoutableEvent>,
        target: PlaneType,
        priority: EventPriority,
    ) -> Result<()> {
        if let Some(plane) = self.planes.get(&target) {
            // In distributed mode, this would use network transport
            // In monolithic mode, this is just a direct call
            match priority {
                EventPriority::Critical => {
                    // Send immediately
                    self.send_to_plane(plane.value(), event).await?;
                }
                _ => {
                    // Can be queued or batched
                    self.queue_for_plane(target, event).await?;
                }
            }
        } else {
            anyhow::bail!("Target plane {:?} not registered", target);
        }
        Ok(())
    }
    
    /// Broadcast event to all planes
    async fn broadcast_event(&self, event: Arc<dyn RoutableEvent>) -> Result<()> {
        for plane in self.planes.iter() {
            self.send_to_plane(plane.value(), event.clone()).await?;
        }
        Ok(())
    }
    
    /// Add event to batch for efficient delivery
    async fn add_to_batch(&self, event: Arc<dyn RoutableEvent>) -> Result<()> {
        // Implementation would batch events for network efficiency
        Ok(())
    }
    
    /// Route based on session affinity
    async fn route_session_bound(&self, event: Arc<dyn RoutableEvent>, session_id: &str) -> Result<()> {
        if let Some(plane_type) = self.session_affinity.get(session_id) {
            if let Some(plane) = self.planes.get(&plane_type) {
                self.send_to_plane(plane.value(), event).await?;
            }
        }
        Ok(())
    }
    
    /// Send event to a specific plane
    async fn send_to_plane(&self, plane: &Arc<dyn FederatedPlane>, event: Arc<dyn RoutableEvent>) -> Result<()> {
        // In monolithic mode, this is a direct call
        // In distributed mode, this would serialize and send over network
        if self.deployment_config.is_local() {
            // Direct in-process delivery
            self.deliver_local(plane, event).await
        } else {
            // Network delivery
            self.deliver_remote(plane, event).await
        }
    }
    
    /// Deliver event locally (in-process)
    async fn deliver_local(&self, _plane: &Arc<dyn FederatedPlane>, _event: Arc<dyn RoutableEvent>) -> Result<()> {
        // Direct function call or channel send
        Ok(())
    }
    
    /// Deliver event remotely (over network)
    async fn deliver_remote(&self, _plane: &Arc<dyn FederatedPlane>, _event: Arc<dyn RoutableEvent>) -> Result<()> {
        // Serialize and send over network transport
        Ok(())
    }
    
    /// Queue event for later delivery to plane
    async fn queue_for_plane(&self, _plane_type: PlaneType, _event: Arc<dyn RoutableEvent>) -> Result<()> {
        // Add to plane-specific queue
        Ok(())
    }
}

/// Routing table for event routing decisions
#[derive(Debug, Default)]
struct RoutingTable {
    /// Event type to plane mapping
    event_routes: DashMap<EventTypeId, PlaneType>,
    
    /// Plane availability status
    plane_status: DashMap<PlaneType, bool>,
    
    /// Custom routing rules
    custom_rules: Vec<RoutingRule>,
}

impl RoutingTable {
    /// Update plane availability status
    fn update_plane_status(&mut self, plane_type: PlaneType, available: bool) {
        self.plane_status.insert(plane_type, available);
    }
    
    /// Add routing rule
    fn add_rule(&mut self, rule: RoutingRule) {
        self.custom_rules.push(rule);
    }
    
    /// Get target plane for event type
    fn get_target(&self, event_type: EventTypeId) -> Option<PlaneType> {
        self.event_routes.get(&event_type).map(|entry| *entry.value())
    }
}

/// Custom routing rule
#[derive(Debug, Clone)]
struct RoutingRule {
    /// Rule name
    name: String,
    
    /// Condition to match
    condition: RoutingCondition,
    
    /// Action to take if matched
    action: RoutingAction,
    
    /// Rule priority
    priority: u32,
}

/// Routing condition
#[derive(Clone)]
enum RoutingCondition {
    /// Match event type
    EventType(EventTypeId),
    
    /// Match event property
    EventProperty { key: String, value: String },
    
    /// Match source plane
    SourcePlane(PlaneType),
    
    /// Custom predicate
    Custom(Arc<dyn Fn(&dyn RoutableEvent) -> bool + Send + Sync>),
}

impl std::fmt::Debug for RoutingCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutingCondition::EventType(et) => write!(f, "EventType({})", et),
            RoutingCondition::EventProperty { key, value } => write!(f, "EventProperty {{ key: {}, value: {} }}", key, value),
            RoutingCondition::SourcePlane(sp) => write!(f, "SourcePlane({:?})", sp),
            RoutingCondition::Custom(_) => write!(f, "Custom(<function>)"),
        }
    }
}

/// Routing action
#[derive(Clone)]
enum RoutingAction {
    /// Route to specific plane
    RouteTo(PlaneType),
    
    /// Broadcast to all planes
    Broadcast,
    
    /// Drop the event
    Drop,
    
    /// Transform and forward
    Transform(Arc<dyn Fn(Arc<dyn RoutableEvent>) -> Arc<dyn RoutableEvent> + Send + Sync>),
}

impl std::fmt::Debug for RoutingAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutingAction::RouteTo(pt) => write!(f, "RouteTo({:?})", pt),
            RoutingAction::Broadcast => write!(f, "Broadcast"),
            RoutingAction::Drop => write!(f, "Drop"),
            RoutingAction::Transform(_) => write!(f, "Transform(<function>)"),
        }
    }
}

/// Metrics for routing performance
struct RoutingMetrics {
    events_routed: std::sync::atomic::AtomicU64,
    routing_errors: std::sync::atomic::AtomicU64,
    average_latency_ns: std::sync::atomic::AtomicU64,
}

impl RoutingMetrics {
    fn new() -> Self {
        Self {
            events_routed: std::sync::atomic::AtomicU64::new(0),
            routing_errors: std::sync::atomic::AtomicU64::new(0),
            average_latency_ns: std::sync::atomic::AtomicU64::new(0),
        }
    }
    
    fn record_routed_event(&self) {
        self.events_routed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    
    fn record_routing_error(&self) {
        self.routing_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}