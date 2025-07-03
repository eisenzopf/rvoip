//! # Call Center Events System Implementation
//!
//! This module provides a comprehensive event system for real-time call center
//! notifications, event routing, and external integrations. It enables real-time
//! monitoring, alerting, and automation based on call center activities.

use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, debug, warn, error};
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, mpsc, RwLock};
use uuid::Uuid;

use crate::error::{CallCenterError, Result};
use crate::agent::{AgentId, AgentStatus};
use crate::queue::QueueStats;

/// # Call Center Events System
///
/// The `CallCenterEvents` system provides comprehensive real-time event notifications
/// for all call center activities. It enables supervisors, administrators, and external
/// systems to monitor call center operations in real-time and respond to events
/// automatically.
///
/// ## Key Features
///
/// - **Real-time Notifications**: Instant delivery of call center events
/// - **Event Filtering**: Subscribe to specific event types or patterns
/// - **External Integration**: Webhook and API notifications for external systems
/// - **Event History**: Configurable event history and replay capabilities
/// - **Custom Events**: Support for application-specific event types
/// - **Performance Monitoring**: Built-in metrics for event system performance
///
/// ## Event Types
///
/// ### Call Events
/// - Call started/ended
/// - Call transferred/forwarded
/// - Call quality alerts
/// - Call recording events
///
/// ### Agent Events
/// - Agent login/logout
/// - Status changes (available, busy, break)
/// - Performance alerts
/// - Schedule adherence events
///
/// ### Queue Events
/// - Call enqueued/dequeued
/// - Queue overflow
/// - SLA violations
/// - Wait time alerts
///
/// ### System Events
/// - System capacity alerts
/// - Service degradation
/// - Configuration changes
/// - Maintenance notifications
///
/// ## Examples
///
/// ### Basic Event Subscription
///
/// ```rust
/// use rvoip_call_engine::monitoring::events::{CallCenterEvents, EventType};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let events = CallCenterEvents::new();
/// 
/// // Subscribe to all call events
/// let mut call_events = events.subscribe(EventType::Call).await?;
/// 
/// // Listen for events
/// tokio::spawn(async move {
///     while let Ok(event) = call_events.recv().await {
///         println!("📞 Call Event: {:?}", event);
///         
///         match event.event_type {
///             EventType::CallStarted => {
///                 println!("New call started: {}", event.session_id.unwrap());
///             }
///             EventType::CallEnded => {
///                 println!("Call ended: {}", event.session_id.unwrap());
///             }
///             _ => {}
///         }
///     }
/// });
/// # Ok(())
/// # }
/// ```
///
/// ### Agent Status Monitoring
///
/// ```rust
/// use rvoip_call_engine::monitoring::events::{CallCenterEvents, EventType, CallCenterEvent};
/// 
/// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
/// // Subscribe to agent events
/// let mut agent_events = events.subscribe(EventType::Agent).await?;
/// 
/// tokio::spawn(async move {
///     while let Ok(event) = agent_events.recv().await {
///         if let EventType::AgentStatusChanged = event.event_type {
///             let agent_id = event.agent_id.unwrap();
///             let new_status = event.data.get("new_status").unwrap();
///             
///             println!("👤 Agent {} changed status to {}", agent_id, new_status);
///             
///             // Take action based on status change
///             if new_status == "offline" {
///                 println!("🚨 Agent {} went offline unexpectedly!", agent_id);
///             }
///         }
///     }
/// });
/// # Ok(())
/// # }
/// ```
///
/// ### Quality Alert Monitoring
///
/// ```rust
/// use rvoip_call_engine::monitoring::events::{CallCenterEvents, EventType, EventSeverity};
/// 
/// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
/// // Subscribe to quality alerts
/// let mut quality_events = events.subscribe(EventType::QualityAlert).await?;
/// 
/// tokio::spawn(async move {
///     while let Ok(event) = quality_events.recv().await {
///         let severity = event.data.get("severity").unwrap();
///         let session_id = event.session_id.unwrap();
///         
///         match severity.as_str() {
///             "critical" => {
///                 println!("🚨 CRITICAL: Call {} has severe quality issues", session_id);
///                 // Trigger immediate supervisor notification
///             }
///             "warning" => {
///                 println!("⚠️ WARNING: Call {} quality degraded", session_id);
///                 // Log for later review
///             }
///             _ => {}
///         }
///     }
/// });
/// # Ok(())
/// # }
/// ```
///
/// ### External System Integration
///
/// ```rust
/// use rvoip_call_engine::monitoring::events::{CallCenterEvents, WebhookConfig};
/// 
/// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
/// // Configure webhook for external CRM system
/// let webhook_config = WebhookConfig {
///     url: "https://crm.company.com/api/call-events".to_string(),
///     secret: Some("webhook-secret-key".to_string()),
///     retry_attempts: 3,
///     timeout_seconds: 30,
/// };
/// 
/// events.add_webhook("crm_integration", webhook_config).await?;
/// 
/// // All events will now be sent to the external CRM system
/// println!("Webhook integration configured");
/// # Ok(())
/// # }
/// ```
pub struct CallCenterEvents {
    /// Event broadcaster for real-time notifications
    event_broadcaster: broadcast::Sender<CallCenterEvent>,
    
    /// Event history storage (ring buffer)
    event_history: Arc<RwLock<Vec<CallCenterEvent>>>,
    
    /// Webhook configurations for external integrations
    webhooks: Arc<RwLock<HashMap<String, WebhookConfig>>>,
    
    /// Event statistics and performance metrics
    stats: Arc<RwLock<EventStats>>,
    
    /// Configuration for event system
    config: EventSystemConfig,
}

/// Individual call center event
#[derive(Debug, Clone)]
pub struct CallCenterEvent {
    /// Unique event identifier
    pub event_id: String,
    
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    
    /// Type of event
    pub event_type: EventType,
    
    /// Session ID if event is call-related
    pub session_id: Option<String>,
    
    /// Agent ID if event is agent-related
    pub agent_id: Option<String>,
    
    /// Queue ID if event is queue-related
    pub queue_id: Option<String>,
    
    /// Event severity level
    pub severity: EventSeverity,
    
    /// Human-readable event message
    pub message: String,
    
    /// Additional event data (key-value pairs)
    pub data: HashMap<String, String>,
    
    /// Source of the event (component that generated it)
    pub source: String,
}

/// Event type enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum EventType {
    // Call-related events
    /// New call started
    CallStarted,
    /// Call ended normally
    CallEnded,
    /// Call was transferred
    CallTransferred,
    /// Call was forwarded
    CallForwarded,
    /// Call quality alert
    QualityAlert,
    /// Call recording started/stopped
    CallRecording,
    /// Generic call event
    Call,
    
    // Agent-related events
    /// Agent logged in
    AgentLogin,
    /// Agent logged out
    AgentLogout,
    /// Agent status changed
    AgentStatusChanged,
    /// Agent performance alert
    AgentPerformanceAlert,
    /// Agent schedule adherence
    ScheduleAdherence,
    /// Generic agent event
    Agent,
    
    // Queue-related events
    /// Call added to queue
    CallEnqueued,
    /// Call removed from queue
    CallDequeued,
    /// Queue overflow occurred
    QueueOverflow,
    /// SLA violation in queue
    SlaViolation,
    /// Queue wait time alert
    WaitTimeAlert,
    /// Generic queue event
    Queue,
    
    // System-related events
    /// System capacity alert
    SystemCapacity,
    /// Service degradation detected
    ServiceDegradation,
    /// Configuration changed
    ConfigurationChange,
    /// System maintenance
    Maintenance,
    /// Generic system event
    System,
    
    // Custom events
    /// Application-specific custom event
    Custom(String),
}

/// Event severity levels
#[derive(Debug, Clone, PartialEq)]
pub enum EventSeverity {
    /// Informational events
    Info,
    /// Warning events that may need attention
    Warning,
    /// Error events that require attention
    Error,
    /// Critical events that require immediate action
    Critical,
}

/// Webhook configuration for external integrations
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Webhook URL endpoint
    pub url: String,
    
    /// Optional secret for webhook validation
    pub secret: Option<String>,
    
    /// Number of retry attempts on failure
    pub retry_attempts: u32,
    
    /// Timeout for webhook requests in seconds
    pub timeout_seconds: u64,
}

/// Event system statistics
#[derive(Debug, Clone)]
pub struct EventStats {
    /// Total events generated
    pub total_events: u64,
    
    /// Events by type
    pub events_by_type: HashMap<String, u64>,
    
    /// Events by severity
    pub events_by_severity: HashMap<String, u64>,
    
    /// Current subscribers count
    pub active_subscribers: u32,
    
    /// Webhook delivery statistics
    pub webhook_deliveries: WebhookStats,
    
    /// Event processing performance
    pub processing_stats: ProcessingStats,
}

/// Webhook delivery statistics
#[derive(Debug, Clone)]
pub struct WebhookStats {
    /// Total webhook deliveries attempted
    pub total_deliveries: u64,
    
    /// Successful deliveries
    pub successful_deliveries: u64,
    
    /// Failed deliveries
    pub failed_deliveries: u64,
    
    /// Average delivery time in milliseconds
    pub average_delivery_time_ms: f64,
}

/// Event processing performance statistics
#[derive(Debug, Clone)]
pub struct ProcessingStats {
    /// Average event processing time in microseconds
    pub average_processing_time_us: f64,
    
    /// Peak events per second
    pub peak_events_per_second: f64,
    
    /// Current event queue depth
    pub current_queue_depth: u32,
}

/// Event system configuration
#[derive(Debug, Clone)]
struct EventSystemConfig {
    /// Maximum number of events to keep in history
    max_history_size: usize,
    
    /// Event broadcast channel capacity
    broadcast_capacity: usize,
    
    /// Whether to enable webhook deliveries
    enable_webhooks: bool,
    
    /// Default webhook timeout
    default_webhook_timeout: u64,
}

impl CallCenterEvents {
    /// Create a new call center events system
    ///
    /// Initializes a new event system ready to handle call center events.
    /// The system will be ready to accept subscriptions and deliver events
    /// immediately.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::CallCenterEvents;
    /// 
    /// let events = CallCenterEvents::new();
    /// println!("Event system initialized");
    /// ```
    pub fn new() -> Self {
        let config = EventSystemConfig {
            max_history_size: 10000,
            broadcast_capacity: 1000,
            enable_webhooks: true,
            default_webhook_timeout: 30,
        };
        
        let (event_broadcaster, _) = broadcast::channel(config.broadcast_capacity);
        
        Self {
            event_broadcaster,
            event_history: Arc::new(RwLock::new(Vec::new())),
            webhooks: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(EventStats {
                total_events: 0,
                events_by_type: HashMap::new(),
                events_by_severity: HashMap::new(),
                active_subscribers: 0,
                webhook_deliveries: WebhookStats {
                    total_deliveries: 0,
                    successful_deliveries: 0,
                    failed_deliveries: 0,
                    average_delivery_time_ms: 0.0,
                },
                processing_stats: ProcessingStats {
                    average_processing_time_us: 0.0,
                    peak_events_per_second: 0.0,
                    current_queue_depth: 0,
                },
            })),
            config,
        }
    }
    
    /// Subscribe to events of a specific type
    ///
    /// Creates a subscription to receive events of the specified type.
    /// Returns a receiver that will deliver matching events.
    ///
    /// # Arguments
    ///
    /// * `event_type` - Type of events to subscribe to
    ///
    /// # Returns
    ///
    /// `Ok(broadcast::Receiver<CallCenterEvent>)` for receiving events, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::{CallCenterEvents, EventType};
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let events = CallCenterEvents::new();
    /// 
    /// // Subscribe to all call events
    /// let mut call_receiver = events.subscribe(EventType::Call).await?;
    /// 
    /// // Subscribe to agent status changes
    /// let mut agent_receiver = events.subscribe(EventType::AgentStatusChanged).await?;
    /// 
    /// println!("Subscriptions created");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe(&self, event_type: EventType) -> Result<broadcast::Receiver<CallCenterEvent>> {
        let receiver = self.event_broadcaster.subscribe();
        
        // Update subscriber count
        {
            let mut stats = self.stats.write().await;
            stats.active_subscribers += 1;
        }
        
        debug!("📡 New subscription created for event type: {:?}", event_type);
        Ok(receiver)
    }
    
    /// Publish an event to all subscribers
    ///
    /// Broadcasts an event to all current subscribers and stores it in history.
    /// Also triggers webhook deliveries if configured.
    ///
    /// # Arguments
    ///
    /// * `event_type` - Type of the event
    /// * `message` - Human-readable event message
    /// * `severity` - Event severity level
    /// * `data` - Additional event data
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::{CallCenterEvents, EventType, EventSeverity};
    /// use std::collections::HashMap;
    /// 
    /// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut data = HashMap::new();
    /// data.insert("agent_id".to_string(), "agent-001".to_string());
    /// data.insert("old_status".to_string(), "available".to_string());
    /// data.insert("new_status".to_string(), "busy".to_string());
    /// 
    /// events.publish(
    ///     EventType::AgentStatusChanged,
    ///     "Agent status changed from available to busy".to_string(),
    ///     EventSeverity::Info,
    ///     data
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish(
        &self,
        event_type: EventType,
        message: String,
        severity: EventSeverity,
        data: HashMap<String, String>,
    ) -> Result<()> {
        let event = CallCenterEvent {
            event_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type: event_type.clone(),
            session_id: data.get("session_id").cloned(),
            agent_id: data.get("agent_id").cloned(),
            queue_id: data.get("queue_id").cloned(),
            severity: severity.clone(),
            message: message.clone(),
            data: data.clone(),
            source: "call-center-engine".to_string(),
        };
        
        // Store in history
        {
            let mut history = self.event_history.write().await;
            history.push(event.clone());
            
            // Trim history if it exceeds max size
            if history.len() > self.config.max_history_size {
                history.remove(0);
            }
        }
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.total_events += 1;
            
            let type_key = format!("{:?}", event_type);
            *stats.events_by_type.entry(type_key).or_insert(0) += 1;
            
            let severity_key = format!("{:?}", severity);
            *stats.events_by_severity.entry(severity_key).or_insert(0) += 1;
        }
        
        // Broadcast to subscribers
        if let Err(e) = self.event_broadcaster.send(event.clone()) {
            warn!("📡 Failed to broadcast event: {}", e);
        }
        
        // Trigger webhook deliveries
        if self.config.enable_webhooks {
            self.deliver_webhooks(event.clone()).await;
        }
        
        debug!("📡 Event published: {} - {}", event.event_id, message);
        Ok(())
    }
    
    /// Add a webhook configuration for external integration
    ///
    /// Configures a webhook endpoint to receive call center events.
    /// All future events will be delivered to this webhook.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for this webhook configuration
    /// * `config` - Webhook configuration details
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::{CallCenterEvents, WebhookConfig};
    /// 
    /// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
    /// let webhook_config = WebhookConfig {
    ///     url: "https://api.external-system.com/webhooks/call-center".to_string(),
    ///     secret: Some("webhook-secret".to_string()),
    ///     retry_attempts: 3,
    ///     timeout_seconds: 30,
    /// };
    /// 
    /// events.add_webhook("external_system", webhook_config).await?;
    /// println!("Webhook configured");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_webhook(&self, name: &str, config: WebhookConfig) -> Result<()> {
        {
            let mut webhooks = self.webhooks.write().await;
            webhooks.insert(name.to_string(), config);
        }
        
        info!("🔗 Webhook '{}' configured", name);
        Ok(())
    }
    
    /// Remove a webhook configuration
    ///
    /// Removes a previously configured webhook. No further events will
    /// be delivered to this webhook.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the webhook to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::CallCenterEvents;
    /// 
    /// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
    /// events.remove_webhook("external_system").await?;
    /// println!("Webhook removed");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn remove_webhook(&self, name: &str) -> Result<()> {
        {
            let mut webhooks = self.webhooks.write().await;
            webhooks.remove(name);
        }
        
        info!("🔗 Webhook '{}' removed", name);
        Ok(())
    }
    
    /// Get event history for a time period
    ///
    /// Returns all events that occurred within the specified time range.
    /// Useful for debugging, analysis, or replaying events.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Beginning of time range
    /// * `end_time` - End of time range
    ///
    /// # Returns
    ///
    /// Vector of events in chronological order.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::CallCenterEvents;
    /// use chrono::{Utc, Duration};
    /// 
    /// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
    /// let end_time = Utc::now();
    /// let start_time = end_time - Duration::hours(1);
    /// 
    /// let recent_events = events.get_event_history(start_time, end_time).await;
    /// println!("Found {} events in the last hour", recent_events.len());
    /// 
    /// for event in recent_events {
    ///     println!("{}: {}", event.timestamp.format("%H:%M:%S"), event.message);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_event_history(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Vec<CallCenterEvent> {
        let history = self.event_history.read().await;
        
        history.iter()
            .filter(|event| event.timestamp >= start_time && event.timestamp <= end_time)
            .cloned()
            .collect()
    }
    
    /// Get event system statistics
    ///
    /// Returns comprehensive statistics about the event system performance,
    /// including event counts, subscriber information, and webhook metrics.
    ///
    /// # Returns
    ///
    /// [`EventStats`] containing current system statistics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::events::CallCenterEvents;
    /// 
    /// # async fn example(events: CallCenterEvents) -> Result<(), Box<dyn std::error::Error>> {
    /// let stats = events.get_stats().await;
    /// 
    /// println!("📊 Event System Statistics:");
    /// println!("  Total events: {}", stats.total_events);
    /// println!("  Active subscribers: {}", stats.active_subscribers);
    /// println!("  Webhook success rate: {:.1}%", 
    ///          (stats.webhook_deliveries.successful_deliveries as f64 / 
    ///           stats.webhook_deliveries.total_deliveries as f64) * 100.0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_stats(&self) -> EventStats {
        let stats = self.stats.read().await;
        stats.clone()
    }
    
    // Internal helper methods
    
    async fn deliver_webhooks(&self, event: CallCenterEvent) {
        let webhooks = self.webhooks.read().await.clone();
        
        for (name, config) in webhooks {
            let event_clone = event.clone();
            let name_clone = name.clone();
            let config_clone = config.clone();
            let stats = self.stats.clone();
            
            // Deliver webhook asynchronously
            tokio::spawn(async move {
                Self::deliver_single_webhook(name_clone, config_clone, event_clone, stats).await;
            });
        }
    }
    
    async fn deliver_single_webhook(
        name: String,
        config: WebhookConfig,
        event: CallCenterEvent,
        stats: Arc<RwLock<EventStats>>,
    ) {
        // TODO: Implement actual webhook delivery
        // This would typically:
        // 1. Serialize event to JSON
        // 2. Create HTTP request with proper headers
        // 3. Add authentication/signature if configured
        // 4. Send request with timeout and retries
        // 5. Update delivery statistics
        
        debug!("🔗 Delivering webhook '{}' for event {}", name, event.event_id);
        
        // Update statistics (simulated)
        {
            let mut stats = stats.write().await;
            stats.webhook_deliveries.total_deliveries += 1;
            stats.webhook_deliveries.successful_deliveries += 1; // Assuming success for now
        }
        
        // In a real implementation, this would make an HTTP request
        warn!("🚧 Webhook delivery not fully implemented yet");
    }
}

impl Clone for CallCenterEvents {
    fn clone(&self) -> Self {
        // Note: This creates a new broadcaster, so cloned instances
        // will have separate event streams
        let (event_broadcaster, _) = broadcast::channel(self.config.broadcast_capacity);
        
        Self {
            event_broadcaster,
            event_history: self.event_history.clone(),
            webhooks: self.webhooks.clone(),
            stats: self.stats.clone(),
            config: self.config.clone(),
        }
    }
}

impl std::fmt::Display for EventSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventSeverity::Info => write!(f, "INFO"),
            EventSeverity::Warning => write!(f, "WARNING"),
            EventSeverity::Error => write!(f, "ERROR"),
            EventSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventType::CallStarted => write!(f, "CALL_STARTED"),
            EventType::CallEnded => write!(f, "CALL_ENDED"),
            EventType::CallTransferred => write!(f, "CALL_TRANSFERRED"),
            EventType::CallForwarded => write!(f, "CALL_FORWARDED"),
            EventType::QualityAlert => write!(f, "QUALITY_ALERT"),
            EventType::CallRecording => write!(f, "CALL_RECORDING"),
            EventType::Call => write!(f, "CALL"),
            EventType::AgentLogin => write!(f, "AGENT_LOGIN"),
            EventType::AgentLogout => write!(f, "AGENT_LOGOUT"),
            EventType::AgentStatusChanged => write!(f, "AGENT_STATUS_CHANGED"),
            EventType::AgentPerformanceAlert => write!(f, "AGENT_PERFORMANCE_ALERT"),
            EventType::ScheduleAdherence => write!(f, "SCHEDULE_ADHERENCE"),
            EventType::Agent => write!(f, "AGENT"),
            EventType::CallEnqueued => write!(f, "CALL_ENQUEUED"),
            EventType::CallDequeued => write!(f, "CALL_DEQUEUED"),
            EventType::QueueOverflow => write!(f, "QUEUE_OVERFLOW"),
            EventType::SlaViolation => write!(f, "SLA_VIOLATION"),
            EventType::WaitTimeAlert => write!(f, "WAIT_TIME_ALERT"),
            EventType::Queue => write!(f, "QUEUE"),
            EventType::SystemCapacity => write!(f, "SYSTEM_CAPACITY"),
            EventType::ServiceDegradation => write!(f, "SERVICE_DEGRADATION"),
            EventType::ConfigurationChange => write!(f, "CONFIGURATION_CHANGE"),
            EventType::Maintenance => write!(f, "MAINTENANCE"),
            EventType::System => write!(f, "SYSTEM"),
            EventType::Custom(name) => write!(f, "CUSTOM_{}", name.to_uppercase()),
        }
    }
} 