//! # Supervisor Monitoring Implementation
//!
//! This module provides real-time monitoring and oversight capabilities for call center
//! supervisors, including live dashboard data, agent performance tracking, call quality
//! monitoring, and intervention tools.

use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn, error};
use chrono::{DateTime, Utc};

use crate::error::{CallCenterError, Result};
use crate::orchestrator::CallCenterEngine;
use crate::agent::{Agent, AgentId, AgentStatus};
use crate::queue::QueueStats;

/// # Supervisor Monitor for Call Center Oversight
///
/// The `SupervisorMonitor` provides comprehensive real-time monitoring and management
/// capabilities for call center supervisors. It offers live statistics, agent performance
/// tracking, call quality monitoring, and intervention tools to ensure optimal call
/// center operations.
///
/// ## Key Features
///
/// - **Real-time Dashboard**: Live call center statistics and KPI monitoring
/// - **Agent Oversight**: Individual agent performance and status tracking
/// - **Call Quality Monitoring**: Real-time call quality alerts and interventions
/// - **Queue Management**: Queue performance monitoring and manual call assignment
/// - **Performance Analytics**: Historical performance trends and reporting
/// - **Intervention Tools**: Supervisor barge-in, whisper, and coaching capabilities
///
/// ## Usage Scenarios
///
/// ### Real-time Monitoring
/// - Live dashboard updates every few seconds
/// - Service level agreement (SLA) monitoring
/// - Call volume and wait time tracking
/// - Agent availability and utilization rates
///
/// ### Quality Assurance
/// - Call quality score monitoring
/// - Automated alerts for poor quality calls
/// - Agent coaching opportunities identification
/// - Customer satisfaction correlation
///
/// ### Operational Management
/// - Manual call assignment override
/// - Queue overflow management
/// - Agent scheduling adherence monitoring
/// - Performance target tracking
///
/// ## Examples
///
/// ### Basic Supervisor Dashboard
///
/// ```rust
/// use rvoip_call_engine::monitoring::SupervisorMonitor;
/// use rvoip_call_engine::orchestrator::CallCenterEngine;
/// use std::sync::Arc;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = CallCenterEngine::new(Default::default(), None).await?;
/// let supervisor = SupervisorMonitor::new(engine.clone());
/// 
/// // Get real-time statistics
/// let stats = supervisor.get_realtime_stats().await;
/// println!("📊 Call Center Status:");
/// println!("  Active calls: {}", stats.active_calls);
/// println!("  Available agents: {}", stats.available_agents);
/// println!("  Service level: {:.1}%", stats.service_level_percentage);
/// println!("  Average wait time: {}s", stats.average_wait_time_seconds);
/// 
/// // Check for alerts
/// let alerts = supervisor.get_active_alerts().await;
/// for alert in alerts {
///     println!("🚨 Alert: {} - {}", alert.severity, alert.message);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Agent Performance Monitoring
///
/// ```rust
/// use rvoip_call_engine::monitoring::SupervisorMonitor;
/// use std::sync::Arc;
/// 
/// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
/// // Monitor specific agent performance
/// let agent_id = "agent-001";
/// let performance = supervisor.get_agent_performance(agent_id).await?;
/// 
/// println!("👤 Agent Performance: {}", agent_id);
/// println!("  Calls handled: {}", performance.calls_handled_today);
/// println!("  Average handle time: {}s", performance.average_handle_time);
/// println!("  Customer satisfaction: {:.1}", performance.customer_satisfaction_score);
/// println!("  Utilization rate: {:.1}%", performance.utilization_percentage);
/// 
/// // Get agent's current status
/// if let Some(status) = supervisor.get_agent_current_status(agent_id).await? {
///     println!("  Current status: {:?}", status.status);
///     println!("  Status since: {}", status.since);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Call Quality Monitoring
///
/// ```rust
/// use rvoip_call_engine::monitoring::SupervisorMonitor;
/// 
/// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
/// // Monitor call quality in real-time
/// let quality_alerts = supervisor.get_quality_alerts().await;
/// 
/// for alert in quality_alerts {
///     println!("📞 Poor Quality Call: {}", alert.session_id);
///     println!("  MOS Score: {:.2}", alert.mos_score);
///     println!("  Packet Loss: {:.1}%", alert.packet_loss_percentage);
///     println!("  Agent: {}", alert.agent_id);
///     
///     // Supervisor can intervene
///     if alert.mos_score < 2.0 {
///         supervisor.schedule_coaching_session(&alert.agent_id, 
///             "Call quality improvement needed").await?;
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Manual Call Assignment
///
/// ```rust
/// use rvoip_call_engine::monitoring::SupervisorMonitor;
/// use rvoip_session_core::SessionId;
/// 
/// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
/// // Override automatic routing for VIP customer
/// let vip_session_id = SessionId::new(); // In practice, from queue
/// let preferred_agent = "senior-agent-001";
/// 
/// match supervisor.force_assign_call(&vip_session_id, preferred_agent).await {
///     Ok(_) => {
///         println!("✅ VIP call manually assigned to senior agent");
///     }
///     Err(e) => {
///         println!("❌ Manual assignment failed: {}", e);
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct SupervisorMonitor {
    /// Reference to the call center engine for accessing system data
    engine: Arc<CallCenterEngine>,
    
    /// Cache for performance metrics to reduce database load
    metrics_cache: Arc<tokio::sync::RwLock<MetricsCache>>,
    
    /// Active quality alerts that need supervisor attention
    active_alerts: Arc<tokio::sync::RwLock<Vec<QualityAlert>>>,
}

/// Real-time call center statistics for supervisor dashboard
#[derive(Debug, Clone)]
pub struct SupervisorStats {
    /// Number of currently active calls
    pub active_calls: usize,
    
    /// Number of calls currently in all queues
    pub queued_calls: usize,
    
    /// Number of agents currently available
    pub available_agents: usize,
    
    /// Number of agents currently busy on calls
    pub busy_agents: usize,
    
    /// Service level percentage (calls answered within target time)
    pub service_level_percentage: f64,
    
    /// Average wait time for calls in queue (seconds)
    pub average_wait_time_seconds: u64,
    
    /// Current call center capacity utilization
    pub capacity_utilization_percentage: f64,
    
    /// Total calls handled today
    pub calls_handled_today: u64,
    
    /// Timestamp of this statistics snapshot
    pub timestamp: DateTime<Utc>,
}

/// Individual agent performance metrics
#[derive(Debug, Clone)]
pub struct AgentPerformance {
    /// Agent identifier
    pub agent_id: String,
    
    /// Number of calls handled today
    pub calls_handled_today: u32,
    
    /// Average handling time in seconds
    pub average_handle_time: u64,
    
    /// Customer satisfaction score (1.0 - 5.0)
    pub customer_satisfaction_score: f32,
    
    /// Agent utilization percentage
    pub utilization_percentage: f64,
    
    /// First call resolution rate
    pub first_call_resolution_rate: f64,
    
    /// Number of escalations today
    pub escalations_today: u32,
    
    /// Current status information
    pub current_status: AgentStatusInfo,
}

/// Current agent status information
#[derive(Debug, Clone)]
pub struct AgentStatusInfo {
    /// Current agent status
    pub status: AgentStatus,
    
    /// Time when status was last changed
    pub since: DateTime<Utc>,
    
    /// Duration in current status (seconds)
    pub duration_seconds: u64,
}

/// Call quality alert for supervisor attention
#[derive(Debug, Clone)]
pub struct QualityAlert {
    /// Session ID of the call with quality issues
    pub session_id: String,
    
    /// Agent handling the call
    pub agent_id: String,
    
    /// Customer identifier
    pub customer_id: Option<String>,
    
    /// Mean Opinion Score (1.0 - 5.0, lower is worse)
    pub mos_score: f32,
    
    /// Packet loss percentage
    pub packet_loss_percentage: f32,
    
    /// Alert severity level
    pub severity: AlertSeverity,
    
    /// Time when alert was generated
    pub alert_time: DateTime<Utc>,
    
    /// Alert message
    pub message: String,
}

/// Alert severity levels
#[derive(Debug, Clone, PartialEq)]
pub enum AlertSeverity {
    /// Information only, no action required
    Info,
    /// Warning, may need attention
    Warning,
    /// Critical, requires immediate attention
    Critical,
}

/// System-wide alerts that need supervisor attention
#[derive(Debug, Clone)]
pub struct SystemAlert {
    /// Alert type identifier
    pub alert_type: String,
    
    /// Alert severity
    pub severity: AlertSeverity,
    
    /// Human-readable alert message
    pub message: String,
    
    /// Time when alert was generated
    pub timestamp: DateTime<Utc>,
    
    /// Additional context data
    pub context: HashMap<String, String>,
}

/// Internal metrics cache to reduce database load
#[derive(Debug, Clone)]
struct MetricsCache {
    /// Cached supervisor statistics
    stats: Option<SupervisorStats>,
    
    /// Cache timestamp
    last_updated: DateTime<Utc>,
    
    /// Cache validity duration in seconds
    cache_duration_seconds: u64,
}

impl SupervisorMonitor {
    /// Create a new supervisor monitor
    ///
    /// Initializes a new supervisor monitor connected to the specified call center engine.
    /// The monitor will provide real-time access to call center statistics, agent performance,
    /// and quality metrics.
    ///
    /// # Arguments
    ///
    /// * `engine` - Shared reference to the call center engine
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// use rvoip_call_engine::orchestrator::CallCenterEngine;
    /// use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorMonitor::new(engine);
    /// 
    /// // Monitor is ready to provide supervisor capabilities
    /// println!("Supervisor monitor initialized");
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self {
            engine,
            metrics_cache: Arc::new(tokio::sync::RwLock::new(MetricsCache {
                stats: None,
                last_updated: Utc::now(),
                cache_duration_seconds: 5, // Cache for 5 seconds
            })),
            active_alerts: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }
    
    /// Get real-time call center statistics
    ///
    /// Returns comprehensive real-time statistics about the call center operation,
    /// including active calls, agent availability, service levels, and performance metrics.
    /// Results are cached briefly to reduce database load.
    ///
    /// # Returns
    ///
    /// [`SupervisorStats`] containing current call center metrics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// let stats = supervisor.get_realtime_stats().await;
    /// 
    /// println!("📊 Call Center Dashboard");
    /// println!("  Active calls: {}", stats.active_calls);
    /// println!("  Queued calls: {}", stats.queued_calls);
    /// println!("  Available agents: {}", stats.available_agents);
    /// println!("  Service level: {:.1}%", stats.service_level_percentage);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_realtime_stats(&self) -> SupervisorStats {
        // Check cache first
        {
            let cache = self.metrics_cache.read().await;
            if let Some(stats) = &cache.stats {
                let cache_age = Utc::now().signed_duration_since(cache.last_updated);
                if cache_age.num_seconds() < cache.cache_duration_seconds as i64 {
                    return stats.clone();
                }
            }
        }
        
        // Cache miss or expired, fetch fresh data
        let engine_stats = self.engine.get_stats().await;
        
        // Calculate service level and other derived metrics
        let service_level_percentage = self.calculate_service_level().await;
        let average_wait_time = self.calculate_average_wait_time().await;
        let capacity_utilization = self.calculate_capacity_utilization().await;
        let calls_handled_today = self.get_calls_handled_today().await;
        
        let stats = SupervisorStats {
            active_calls: engine_stats.active_calls,
            queued_calls: engine_stats.queued_calls,
            available_agents: engine_stats.available_agents,
            busy_agents: engine_stats.busy_agents,
            service_level_percentage,
            average_wait_time_seconds: average_wait_time,
            capacity_utilization_percentage: capacity_utilization,
            calls_handled_today,
            timestamp: Utc::now(),
        };
        
        // Update cache
        {
            let mut cache = self.metrics_cache.write().await;
            cache.stats = Some(stats.clone());
            cache.last_updated = Utc::now();
        }
        
        stats
    }
    
    /// Get performance metrics for a specific agent
    ///
    /// Returns detailed performance analytics for the specified agent,
    /// including call statistics, quality metrics, and productivity measures.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Identifier of the agent to analyze
    ///
    /// # Returns
    ///
    /// `Ok(AgentPerformance)` with the agent's metrics, or error if agent not found.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// let performance = supervisor.get_agent_performance("agent-001").await?;
    /// 
    /// println!("Agent Performance Report:");
    /// println!("  Calls today: {}", performance.calls_handled_today);
    /// println!("  Avg handle time: {}s", performance.average_handle_time);
    /// println!("  Satisfaction: {:.1}/5.0", performance.customer_satisfaction_score);
    /// println!("  Utilization: {:.1}%", performance.utilization_percentage);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_agent_performance(&self, agent_id: &str) -> Result<AgentPerformance> {
        // TODO: Implement comprehensive agent performance calculation
        // This would typically involve:
        // 1. Querying call history for the agent
        // 2. Calculating average handle time
        // 3. Determining customer satisfaction scores
        // 4. Computing utilization rates
        // 5. Analyzing first call resolution rates
        
        warn!("🚧 get_agent_performance not fully implemented yet");
        
        // Return placeholder data for now
        Ok(AgentPerformance {
            agent_id: agent_id.to_string(),
            calls_handled_today: 0,
            average_handle_time: 0,
            customer_satisfaction_score: 0.0,
            utilization_percentage: 0.0,
            first_call_resolution_rate: 0.0,
            escalations_today: 0,
            current_status: AgentStatusInfo {
                status: AgentStatus::Offline,
                since: Utc::now(),
                duration_seconds: 0,
            },
        })
    }
    
    /// Get current status information for an agent
    ///
    /// Returns the current status of the specified agent including status type,
    /// duration in current status, and last status change time.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Identifier of the agent to check
    ///
    /// # Returns
    ///
    /// `Ok(Some(AgentStatusInfo))` if agent found, `Ok(None)` if not found, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(status) = supervisor.get_agent_current_status("agent-001").await? {
    ///     println!("Agent Status: {:?}", status.status);
    ///     println!("Since: {}", status.since);
    ///     println!("Duration: {}s", status.duration_seconds);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_agent_current_status(&self, agent_id: &str) -> Result<Option<AgentStatusInfo>> {
        // TODO: Query agent registry for current status
        warn!("🚧 get_agent_current_status not fully implemented yet");
        
        // Return placeholder data
        Ok(Some(AgentStatusInfo {
            status: AgentStatus::Available,
            since: Utc::now(),
            duration_seconds: 0,
        }))
    }
    
    /// Get active call quality alerts
    ///
    /// Returns a list of current call quality alerts that need supervisor attention.
    /// This includes calls with poor audio quality, high packet loss, or other
    /// technical issues affecting customer experience.
    ///
    /// # Returns
    ///
    /// Vector of [`QualityAlert`] entries requiring attention.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// let alerts = supervisor.get_quality_alerts().await;
    /// 
    /// for alert in alerts {
    ///     println!("⚠️ Quality Alert: {}", alert.message);
    ///     println!("  Call: {}", alert.session_id);
    ///     println!("  Agent: {}", alert.agent_id);
    ///     println!("  MOS Score: {:.2}", alert.mos_score);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_quality_alerts(&self) -> Vec<QualityAlert> {
        let alerts = self.active_alerts.read().await;
        alerts.clone()
    }
    
    /// Get active system alerts
    ///
    /// Returns a list of current system-wide alerts that need supervisor attention.
    /// This includes capacity issues, service degradation, and other operational
    /// problems affecting the call center.
    ///
    /// # Returns
    ///
    /// Vector of [`SystemAlert`] entries requiring attention.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// let alerts = supervisor.get_active_alerts().await;
    /// 
    /// for alert in alerts {
    ///     println!("🚨 System Alert [{}]: {}", alert.severity, alert.message);
    ///     println!("  Type: {}", alert.alert_type);
    ///     println!("  Time: {}", alert.timestamp);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_active_alerts(&self) -> Vec<SystemAlert> {
        // TODO: Implement system alert collection
        // This would typically check:
        // 1. System capacity thresholds
        // 2. Service level violations
        // 3. Agent availability issues
        // 4. Queue overflow situations
        // 5. Technical system problems
        
        warn!("🚧 get_active_alerts not fully implemented yet");
        
        // Return placeholder alerts
        vec![
            SystemAlert {
                alert_type: "service_level".to_string(),
                severity: AlertSeverity::Warning,
                message: "Service level below target (78.5% vs 80% target)".to_string(),
                timestamp: Utc::now(),
                context: {
                    let mut ctx = HashMap::new();
                    ctx.insert("current_sla".to_string(), "78.5".to_string());
                    ctx.insert("target_sla".to_string(), "80.0".to_string());
                    ctx
                },
            }
        ]
    }
    
    /// Force assignment of a call to a specific agent
    ///
    /// Overrides the automatic routing system to assign a specific call directly
    /// to a chosen agent. This is typically used for VIP customers, escalations,
    /// or when supervisor intervention is required.
    ///
    /// # Arguments
    ///
    /// * `session_id` - ID of the call session to assign
    /// * `agent_id` - ID of the agent to receive the call
    ///
    /// # Returns
    ///
    /// `Ok(())` if assignment successful, or error if assignment failed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// use rvoip_session_core::SessionId;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// let session_id = SessionId::new();
    /// let senior_agent = "senior-agent-001";
    /// 
    /// // Manually assign VIP customer to senior agent
    /// supervisor.force_assign_call(&session_id, senior_agent).await?;
    /// println!("Call assigned to {}", senior_agent);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn force_assign_call(&self, session_id: &rvoip_session_core::SessionId, agent_id: &str) -> Result<()> {
        // TODO: Implement forced call assignment
        // This would typically:
        // 1. Validate that the agent is available
        // 2. Remove the call from any queue
        // 3. Directly assign the call to the agent
        // 4. Notify the routing system of the override
        // 5. Log the manual assignment for audit purposes
        
        warn!("🚧 force_assign_call not fully implemented yet");
        info!("Manual call assignment requested: session {:?} -> agent {}", session_id, agent_id);
        
        Ok(())
    }
    
    /// Schedule a coaching session for an agent
    ///
    /// Creates a coaching session request for the specified agent, typically
    /// triggered by performance issues, quality alerts, or development needs.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - ID of the agent needing coaching
    /// * `reason` - Reason for the coaching session
    ///
    /// # Returns
    ///
    /// `Ok(())` if coaching session scheduled, or error if scheduling failed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::SupervisorMonitor;
    /// 
    /// # async fn example(supervisor: SupervisorMonitor) -> Result<(), Box<dyn std::error::Error>> {
    /// supervisor.schedule_coaching_session(
    ///     "agent-001", 
    ///     "Call quality improvement needed - MOS scores below target"
    /// ).await?;
    /// 
    /// println!("Coaching session scheduled");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn schedule_coaching_session(&self, agent_id: &str, reason: &str) -> Result<()> {
        // TODO: Implement coaching session scheduling
        warn!("🚧 schedule_coaching_session not fully implemented yet");
        info!("Coaching session scheduled for agent {} - Reason: {}", agent_id, reason);
        
        Ok(())
    }
    
    // Private helper methods
    
    async fn calculate_service_level(&self) -> f64 {
        // TODO: Calculate actual service level from call history
        85.0 // Placeholder
    }
    
    async fn calculate_average_wait_time(&self) -> u64 {
        // TODO: Calculate actual wait time from queue statistics
        45 // Placeholder: 45 seconds
    }
    
    async fn calculate_capacity_utilization(&self) -> f64 {
        // TODO: Calculate actual capacity utilization
        75.0 // Placeholder: 75% utilization
    }
    
    async fn get_calls_handled_today(&self) -> u64 {
        // TODO: Query database for calls handled today
        156 // Placeholder
    }
}

impl Clone for SupervisorMonitor {
    fn clone(&self) -> Self {
        Self {
            engine: Arc::clone(&self.engine),
            metrics_cache: Arc::clone(&self.metrics_cache),
            active_alerts: Arc::clone(&self.active_alerts),
        }
    }
}

impl std::fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSeverity::Info => write!(f, "INFO"),
            AlertSeverity::Warning => write!(f, "WARNING"),
            AlertSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
} 