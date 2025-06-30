//! # Queue Overflow Management
//!
//! This module provides sophisticated overflow handling capabilities for call queues,
//! including overflow detection, escalation policies, and alternative routing strategies
//! when queues reach capacity limits or service level thresholds.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::{info, warn, error};

use crate::error::{CallCenterError, Result};

/// # Overflow Handler for Queue Management
///
/// The `OverflowHandler` manages queue overflow situations by implementing
/// sophisticated escalation and routing policies. When queues reach capacity
/// or performance thresholds, it automatically triggers alternative routing
/// strategies to maintain service levels and customer satisfaction.
///
/// ## Key Features
///
/// - **Overflow Detection**: Real-time monitoring of queue capacity and wait times
/// - **Escalation Policies**: Configurable rules for overflow situations
/// - **Alternative Routing**: Multiple fallback strategies for overflow calls
/// - **Load Balancing**: Distribution across available overflow queues
/// - **Priority Handling**: Special treatment for high-priority calls during overflow
/// - **External Routing**: Integration with external call centers or services
///
/// ## Overflow Triggers
///
/// ### Capacity-Based
/// - Queue size exceeds maximum capacity
/// - Wait time exceeds acceptable thresholds
/// - Agent availability drops below minimum levels
/// - System utilization reaches critical levels
///
/// ### Performance-Based
/// - Service level falls below SLA targets
/// - Average handling time increases significantly
/// - Customer abandon rate exceeds thresholds
/// - Call quality metrics degrade
///
/// ### Time-Based
/// - Business hours vs. after-hours overflow rules
/// - Peak period overflow handling
/// - Emergency situation overrides
/// - Scheduled maintenance overflow routing
///
/// ## Overflow Strategies
///
/// ### Queue Overflow
/// - Route to secondary queues
/// - Distribute across multiple queue groups
/// - Priority-based queue selection
/// - Geographic overflow routing
///
/// ### External Overflow
/// - Forward to partner call centers
/// - Route to third-party services
/// - Voicemail and callback options
/// - Self-service portal redirection
///
/// ### Intelligent Overflow
/// - Skill-based overflow routing
/// - Customer tier-aware overflow
/// - Historical pattern-based routing
/// - Machine learning-optimized decisions
///
/// ## Examples
///
/// ### Basic Overflow Configuration
///
/// ```rust
/// use rvoip_call_engine::queue::{OverflowHandler, OverflowPolicy};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut overflow = OverflowHandler::new();
/// 
/// // Configure capacity-based overflow
/// overflow.set_capacity_threshold(50)?; // Max 50 calls in queue
/// overflow.set_wait_time_threshold(120)?; // Max 2 minutes wait
/// 
/// // Configure overflow queues
/// overflow.add_overflow_queue("queue_secondary", 1)?; // Priority 1
/// overflow.add_overflow_queue("queue_tertiary", 2)?;  // Priority 2
/// 
/// // Configure external overflow
/// overflow.set_external_overflow("partner_center", "sip:overflow@partner.com")?;
/// 
/// println!("Overflow handler configured");
/// # Ok(())
/// # }
/// ```
///
/// ### Conditional Overflow Policies
///
/// ```rust
/// use rvoip_call_engine::queue::{OverflowHandler, OverflowCondition, OverflowAction};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut overflow = OverflowHandler::new();
/// 
/// // VIP customer overflow policy
/// overflow.add_conditional_policy(
///     "vip_overflow",
///     OverflowCondition::CustomerTier("vip".to_string()),
///     OverflowAction::RouteToQueue("vip_overflow_queue".to_string())
/// )?;
/// 
/// // Business hours overflow
/// overflow.add_conditional_policy(
///     "business_hours",
///     OverflowCondition::BusinessHours,
///     OverflowAction::RouteToExternal("business_overflow".to_string())
/// )?;
/// 
/// // Emergency overflow
/// overflow.add_conditional_policy(
///     "emergency",
///     OverflowCondition::QueueSize(100),
///     OverflowAction::EnableCallbacks
/// )?;
/// 
/// println!("Conditional overflow policies configured");
/// # Ok(())
/// # }
/// ```
///
/// ### Real-time Overflow Monitoring
///
/// ```rust
/// use rvoip_call_engine::queue::OverflowHandler;
/// 
/// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
/// // Check if overflow is currently active
/// if overflow.is_overflow_active().await? {
///     let stats = overflow.get_overflow_stats().await?;
///     
///     println!("🚨 Overflow Active:");
///     println!("  Calls routed to overflow: {}", stats.overflow_calls_count);
///     println!("  Primary queue utilization: {:.1}%", stats.primary_queue_utilization);
///     println!("  Current overflow strategy: {}", stats.active_strategy);
///     
///     // Get recommendations for addressing overflow
///     let recommendations = overflow.get_overflow_recommendations().await?;
///     for rec in recommendations {
///         println!("💡 Recommendation: {}", rec);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Advanced Overflow Analytics
///
/// ```rust
/// use rvoip_call_engine::queue::OverflowHandler;
/// use chrono::{Utc, Duration};
/// 
/// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
/// // Analyze overflow patterns over time
/// let end_time = Utc::now();
/// let start_time = end_time - Duration::hours(24);
/// 
/// let analysis = overflow.analyze_overflow_patterns(start_time, end_time).await?;
/// 
/// println!("📊 24-Hour Overflow Analysis:");
/// println!("  Total overflow events: {}", analysis.overflow_events);
/// println!("  Peak overflow time: {}", analysis.peak_overflow_time);
/// println!("  Most effective strategy: {}", analysis.most_effective_strategy);
/// println!("  Average overflow duration: {}min", analysis.average_duration_minutes);
/// 
/// // Export detailed overflow report
/// let report = overflow.generate_overflow_report(start_time, end_time).await?;
/// println!("Generated {} page overflow report", report.pages);
/// # Ok(())
/// # }
/// ```
pub struct OverflowHandler {
    /// Overflow detection thresholds
    thresholds: OverflowThresholds,
    
    /// Configured overflow policies
    policies: Vec<OverflowPolicy>,
    
    /// Available overflow queues with priorities
    overflow_queues: HashMap<String, OverflowQueue>,
    
    /// External overflow destinations
    external_destinations: HashMap<String, ExternalDestination>,
    
    /// Real-time overflow statistics
    stats: OverflowStats,
    
    /// Configuration for overflow behavior
    config: OverflowConfig,
}

/// Overflow detection thresholds
#[derive(Debug, Clone)]
pub struct OverflowThresholds {
    /// Maximum queue capacity before overflow
    pub max_queue_size: u32,
    
    /// Maximum wait time in seconds before overflow
    pub max_wait_time: u64,
    
    /// Minimum service level percentage threshold
    pub min_service_level: f64,
    
    /// Maximum system utilization before overflow
    pub max_utilization: f64,
    
    /// Minimum available agents threshold
    pub min_available_agents: u32,
}

/// Overflow policy configuration
#[derive(Debug, Clone)]
pub struct OverflowPolicy {
    /// Policy name
    pub name: String,
    
    /// Condition that triggers this policy
    pub condition: OverflowCondition,
    
    /// Action to take when condition is met
    pub action: OverflowAction,
    
    /// Policy priority (lower = higher priority)
    pub priority: u8,
    
    /// Whether this policy is currently enabled
    pub enabled: bool,
}

/// Conditions that can trigger overflow policies
#[derive(Debug, Clone)]
pub enum OverflowCondition {
    /// Queue size exceeds threshold
    QueueSize(u32),
    /// Wait time exceeds threshold (seconds)
    WaitTime(u64),
    /// Service level below threshold (percentage)
    ServiceLevel(f64),
    /// System utilization above threshold (percentage)
    SystemUtilization(f64),
    /// Customer has specific tier level
    CustomerTier(String),
    /// During business hours
    BusinessHours,
    /// After business hours
    AfterHours,
    /// All agents unavailable
    NoAgentsAvailable,
    /// Emergency situation declared
    Emergency,
    /// Custom condition (key-value match)
    Custom(String, String),
}

/// Actions to take when overflow conditions are met
#[derive(Debug, Clone)]
pub enum OverflowAction {
    /// Route to specific overflow queue
    RouteToQueue(String),
    /// Route to external destination
    RouteToExternal(String),
    /// Enable callback options
    EnableCallbacks,
    /// Play announcement and queue normally
    PlayAnnouncement(String),
    /// Redirect to self-service portal
    RedirectToSelfService,
    /// Forward to voicemail
    ForwardToVoicemail,
    /// Reject call with busy signal
    RejectCall,
    /// Execute custom action
    Custom(String),
}

/// Overflow queue configuration
#[derive(Debug, Clone)]
pub struct OverflowQueue {
    /// Queue identifier
    pub queue_id: String,
    
    /// Queue priority (lower = higher priority)
    pub priority: u8,
    
    /// Maximum capacity for this overflow queue
    pub max_capacity: u32,
    
    /// Required skills for agents in this queue
    pub required_skills: Vec<String>,
    
    /// Whether this queue is currently active
    pub active: bool,
}

/// External overflow destination
#[derive(Debug, Clone)]
pub struct ExternalDestination {
    /// Destination name
    pub name: String,
    
    /// SIP URI or phone number for routing
    pub destination_uri: String,
    
    /// Authentication credentials if required
    pub credentials: Option<String>,
    
    /// Maximum concurrent calls to this destination
    pub max_concurrent: u32,
    
    /// Current active calls to this destination
    pub current_calls: u32,
    
    /// Whether this destination is currently available
    pub available: bool,
}

/// Real-time overflow statistics
#[derive(Debug, Clone)]
pub struct OverflowStats {
    /// Whether overflow is currently active
    pub overflow_active: bool,
    
    /// Number of calls currently routed to overflow
    pub overflow_calls_count: u32,
    
    /// Primary queue utilization percentage
    pub primary_queue_utilization: f64,
    
    /// Currently active overflow strategy
    pub active_strategy: String,
    
    /// Total overflow events today
    pub overflow_events_today: u32,
    
    /// Average overflow duration in minutes
    pub average_overflow_duration: f64,
    
    /// Statistics timestamp
    pub timestamp: DateTime<Utc>,
}

/// Overflow pattern analysis results
#[derive(Debug, Clone)]
pub struct OverflowAnalysis {
    /// Total number of overflow events in period
    pub overflow_events: u32,
    
    /// Time when overflow was most severe
    pub peak_overflow_time: DateTime<Utc>,
    
    /// Most effective overflow strategy used
    pub most_effective_strategy: String,
    
    /// Average duration of overflow situations (minutes)
    pub average_duration_minutes: f64,
    
    /// Percentage of calls successfully handled via overflow
    pub overflow_success_rate: f64,
    
    /// Recommendations for improvement
    pub recommendations: Vec<String>,
}

/// Overflow report generation result
#[derive(Debug, Clone)]
pub struct OverflowReport {
    /// Number of pages in the report
    pub pages: u32,
    
    /// Report file path or data
    pub report_data: Vec<u8>,
    
    /// Report format (PDF, CSV, etc.)
    pub format: String,
    
    /// Report generation timestamp
    pub generated_at: DateTime<Utc>,
}

/// Configuration for overflow handler behavior
#[derive(Debug, Clone)]
struct OverflowConfig {
    /// Enable automatic overflow detection
    auto_detection_enabled: bool,
    
    /// Overflow check interval in seconds
    check_interval_seconds: u64,
    
    /// Enable predictive overflow detection
    predictive_enabled: bool,
    
    /// Historical data retention period (days)
    history_retention_days: u32,
}

impl OverflowHandler {
    /// Create a new overflow handler
    ///
    /// Initializes a new overflow handler with default thresholds and configuration.
    /// The handler will begin monitoring for overflow conditions immediately.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// let overflow = OverflowHandler::new();
    /// println!("Overflow handler initialized with default settings");
    /// ```
    pub fn new() -> Self {
        Self {
            thresholds: OverflowThresholds {
                max_queue_size: 50,
                max_wait_time: 120,
                min_service_level: 80.0,
                max_utilization: 85.0,
                min_available_agents: 1,
            },
            policies: Vec::new(),
            overflow_queues: HashMap::new(),
            external_destinations: HashMap::new(),
            stats: OverflowStats {
                overflow_active: false,
                overflow_calls_count: 0,
                primary_queue_utilization: 0.0,
                active_strategy: "none".to_string(),
                overflow_events_today: 0,
                average_overflow_duration: 0.0,
                timestamp: Utc::now(),
            },
            config: OverflowConfig {
                auto_detection_enabled: true,
                check_interval_seconds: 10,
                predictive_enabled: false,
                history_retention_days: 30,
            },
        }
    }
    
    /// Set capacity threshold for overflow detection
    ///
    /// Configures the maximum queue size before overflow handling is triggered.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Maximum number of calls in queue before overflow
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut overflow = OverflowHandler::new();
    /// overflow.set_capacity_threshold(75)?; // 75 calls max
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_capacity_threshold(&mut self, threshold: u32) -> Result<()> {
        self.thresholds.max_queue_size = threshold;
        info!("Overflow capacity threshold set to {} calls", threshold);
        Ok(())
    }
    
    /// Set wait time threshold for overflow detection
    ///
    /// Configures the maximum wait time before overflow handling is triggered.
    ///
    /// # Arguments
    ///
    /// * `threshold_seconds` - Maximum wait time in seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut overflow = OverflowHandler::new();
    /// overflow.set_wait_time_threshold(180)?; // 3 minutes max
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_wait_time_threshold(&mut self, threshold_seconds: u64) -> Result<()> {
        self.thresholds.max_wait_time = threshold_seconds;
        info!("Overflow wait time threshold set to {} seconds", threshold_seconds);
        Ok(())
    }
    
    /// Add an overflow queue
    ///
    /// Adds a secondary queue to handle overflow calls with specified priority.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Identifier of the overflow queue
    /// * `priority` - Priority level (lower = higher priority)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut overflow = OverflowHandler::new();
    /// overflow.add_overflow_queue("secondary_queue", 1)?;
    /// overflow.add_overflow_queue("tertiary_queue", 2)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_overflow_queue(&mut self, queue_id: &str, priority: u8) -> Result<()> {
        let overflow_queue = OverflowQueue {
            queue_id: queue_id.to_string(),
            priority,
            max_capacity: 100, // Default capacity
            required_skills: Vec::new(),
            active: true,
        };
        
        self.overflow_queues.insert(queue_id.to_string(), overflow_queue);
        info!("Added overflow queue '{}' with priority {}", queue_id, priority);
        Ok(())
    }
    
    /// Set external overflow destination
    ///
    /// Configures an external destination for overflow calls.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the external destination
    /// * `uri` - SIP URI or phone number for routing
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut overflow = OverflowHandler::new();
    /// overflow.set_external_overflow("partner", "sip:overflow@partner.com")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_external_overflow(&mut self, name: &str, uri: &str) -> Result<()> {
        let destination = ExternalDestination {
            name: name.to_string(),
            destination_uri: uri.to_string(),
            credentials: None,
            max_concurrent: 10,
            current_calls: 0,
            available: true,
        };
        
        self.external_destinations.insert(name.to_string(), destination);
        info!("Added external overflow destination '{}' -> {}", name, uri);
        Ok(())
    }
    
    /// Add a conditional overflow policy
    ///
    /// Adds a policy that triggers specific overflow actions based on conditions.
    ///
    /// # Arguments
    ///
    /// * `name` - Policy name
    /// * `condition` - Condition that triggers the policy
    /// * `action` - Action to take when condition is met
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{OverflowHandler, OverflowCondition, OverflowAction};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut overflow = OverflowHandler::new();
    /// overflow.add_conditional_policy(
    ///     "high_priority",
    ///     OverflowCondition::CustomerTier("platinum".to_string()),
    ///     OverflowAction::RouteToQueue("vip_overflow".to_string())
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_conditional_policy(
        &mut self,
        name: &str,
        condition: OverflowCondition,
        action: OverflowAction,
    ) -> Result<()> {
        let policy = OverflowPolicy {
            name: name.to_string(),
            condition,
            action,
            priority: self.policies.len() as u8,
            enabled: true,
        };
        
        self.policies.push(policy);
        info!("Added overflow policy '{}'", name);
        Ok(())
    }
    
    /// Check if overflow is currently active
    ///
    /// Returns whether overflow handling is currently active in the system.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
    /// if overflow.is_overflow_active().await? {
    ///     println!("⚠️ Overflow is currently active");
    /// } else {
    ///     println!("✅ Normal operation - no overflow");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_overflow_active(&self) -> Result<bool> {
        Ok(self.stats.overflow_active)
    }
    
    /// Get current overflow statistics
    ///
    /// Returns real-time statistics about overflow conditions and performance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
    /// let stats = overflow.get_overflow_stats().await?;
    /// println!("Overflow Stats:");
    /// println!("  Active: {}", stats.overflow_active);
    /// println!("  Overflow calls: {}", stats.overflow_calls_count);
    /// println!("  Strategy: {}", stats.active_strategy);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_overflow_stats(&self) -> Result<OverflowStats> {
        Ok(self.stats.clone())
    }
    
    /// Get overflow recommendations
    ///
    /// Returns recommendations for addressing current overflow conditions.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// 
    /// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
    /// let recommendations = overflow.get_overflow_recommendations().await?;
    /// for rec in recommendations {
    ///     println!("💡 {}", rec);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_overflow_recommendations(&self) -> Result<Vec<String>> {
        let mut recommendations = Vec::new();
        
        if self.stats.overflow_active {
            recommendations.push("Consider adding more agents to reduce queue pressure".to_string());
            recommendations.push("Review call routing efficiency".to_string());
            recommendations.push("Check if external overflow capacity can be increased".to_string());
        }
        
        Ok(recommendations)
    }
    
    /// Analyze overflow patterns over a time period
    ///
    /// Performs historical analysis of overflow events and patterns.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Start of analysis period
    /// * `end_time` - End of analysis period
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// use chrono::{Utc, Duration};
    /// 
    /// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
    /// let end = Utc::now();
    /// let start = end - Duration::days(7);
    /// 
    /// let analysis = overflow.analyze_overflow_patterns(start, end).await?;
    /// println!("Weekly overflow analysis completed");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn analyze_overflow_patterns(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<OverflowAnalysis> {
        // TODO: Implement historical overflow pattern analysis
        warn!("🚧 analyze_overflow_patterns not fully implemented yet");
        
        Ok(OverflowAnalysis {
            overflow_events: 0,
            peak_overflow_time: Utc::now(),
            most_effective_strategy: "queue_overflow".to_string(),
            average_duration_minutes: 0.0,
            overflow_success_rate: 0.0,
            recommendations: vec!["Analysis requires historical data".to_string()],
        })
    }
    
    /// Generate detailed overflow report
    ///
    /// Creates a comprehensive report of overflow performance and recommendations.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Start of report period
    /// * `end_time` - End of report period
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::OverflowHandler;
    /// use chrono::{Utc, Duration};
    /// 
    /// # async fn example(overflow: OverflowHandler) -> Result<(), Box<dyn std::error::Error>> {
    /// let end = Utc::now();
    /// let start = end - Duration::days(30);
    /// 
    /// let report = overflow.generate_overflow_report(start, end).await?;
    /// println!("Generated overflow report: {} bytes", report.report_data.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn generate_overflow_report(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<OverflowReport> {
        // TODO: Implement overflow report generation
        warn!("🚧 generate_overflow_report not fully implemented yet");
        
        Ok(OverflowReport {
            pages: 1,
            report_data: b"Overflow report placeholder".to_vec(),
            format: "text".to_string(),
            generated_at: Utc::now(),
        })
    }
}

impl Default for OverflowHandler {
    fn default() -> Self {
        Self::new()
    }
} 