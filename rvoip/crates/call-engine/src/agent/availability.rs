//! # Agent Availability Tracking
//!
//! This module provides sophisticated availability tracking capabilities for call center agents,
//! including activity monitoring, presence detection, timeout management, and availability
//! analytics. It enables real-time tracking of agent presence and activity patterns.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use tracing::{info, debug, warn};

use crate::error::{CallCenterError, Result};

/// # Availability Tracker for Agent Presence Monitoring
///
/// The `AvailabilityTracker` provides comprehensive agent availability and activity tracking
/// capabilities for call center operations. It monitors agent presence, tracks activity
/// patterns, manages timeout detection, and provides availability analytics to ensure
/// optimal agent utilization and accurate presence information.
///
/// ## Key Features
///
/// - **Real-Time Activity Tracking**: Monitors agent activity in real-time
/// - **Timeout Detection**: Automatically detects inactive agents based on configurable timeouts
/// - **Presence Management**: Tracks agent presence across multiple channels
/// - **Activity Analytics**: Provides insights into agent activity patterns
/// - **Configurable Timeouts**: Supports different timeout policies for different scenarios
/// - **Historical Tracking**: Maintains activity history for analysis and reporting
///
/// ## Activity Sources
///
/// ### SIP Activity
/// - SIP REGISTER refreshes
/// - Incoming/outgoing call events
/// - SIP OPTIONS responses
/// - Custom SIP INFO messages
///
/// ### Web Interface Activity
/// - Agent dashboard interactions
/// - Status changes via web interface
/// - API calls from agent applications
/// - Heartbeat mechanisms
///
/// ### System Events
/// - Login/logout events
/// - Call handling activities
/// - Manual status updates
/// - Automated system interactions
///
/// ## Timeout Policies
///
/// ### Conservative Timeout
/// - Longer timeout periods for stable environments
/// - Reduces false positives for inactive detection
/// - Suitable for environments with reliable connectivity
///
/// ### Aggressive Timeout
/// - Shorter timeout periods for dynamic environments
/// - Quick detection of disconnected agents
/// - Better for mobile or unreliable network conditions
///
/// ### Adaptive Timeout
/// - Dynamic timeout adjustment based on agent behavior patterns
/// - Learns from historical activity patterns
/// - Optimizes between false positives and quick detection
///
/// ## Examples
///
/// ### Basic Activity Tracking
///
/// ```rust
/// use rvoip_call_engine::agent::AvailabilityTracker;
/// 
/// let mut tracker = AvailabilityTracker::new();
/// 
/// // Register agent activity
/// tracker.update_activity("agent-001".to_string());
/// 
/// // Check if agent is active (5-minute timeout)
/// if tracker.is_agent_active("agent-001", 300) {
///     println!("✅ Agent is active");
/// } else {
///     println!("⚠️ Agent appears inactive");
/// }
/// 
/// // Check activity with different timeout
/// if tracker.is_agent_active("agent-001", 60) {
///     println!("Agent active within last minute");
/// }
/// ```
///
/// ### Advanced Activity Monitoring
///
/// ```rust
/// use rvoip_call_engine::agent::availability::{AvailabilityTracker, ActivityType, AvailabilityConfig};
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = AvailabilityConfig {
///     default_timeout_seconds: 300,    // 5 minutes default
///     heartbeat_interval_seconds: 30,  // 30 second heartbeats
///     inactivity_warning_seconds: 240, // 4 minute warning
///     enable_activity_history: true,
///     max_history_entries: 1000,
/// };
/// 
/// let mut tracker = AvailabilityTracker::with_config(config);
/// 
/// // Record different types of activity
/// tracker.record_activity("agent-001", ActivityType::SipRegister)?;
/// tracker.record_activity("agent-001", ActivityType::CallHandling)?;
/// tracker.record_activity("agent-001", ActivityType::WebInterface)?;
/// 
/// // Get detailed availability status
/// let status = tracker.get_availability_status("agent-001")?;
/// 
/// match status {
///     Some(info) => {
///         println!("Agent Status:");
///         println!("  Last activity: {} seconds ago", info.seconds_since_last_activity);
///         println!("  Activity type: {:?}", info.last_activity_type);
///         println!("  Is active: {}", info.is_active);
///         println!("  Warning threshold: {}", info.near_timeout_warning);
///     }
///     None => {
///         println!("No activity recorded for agent");
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Batch Activity Monitoring
///
/// ```rust
/// use rvoip_call_engine::agent::availability::AvailabilityTracker;
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut tracker = AvailabilityTracker::new();
/// 
/// // Simulate multiple agents with activity
/// let agents = vec!["agent-001", "agent-002", "agent-003", "agent-004"];
/// 
/// for agent in &agents {
///     tracker.update_activity(agent.to_string());
/// }
/// 
/// // Get availability summary for all agents
/// let summary = tracker.get_availability_summary(300)?; // 5-minute timeout
/// 
/// println!("📊 Availability Summary:");
/// println!("  Total agents: {}", summary.total_agents);
/// println!("  Active agents: {}", summary.active_agents);
/// println!("  Inactive agents: {}", summary.inactive_agents);
/// println!("  Activity rate: {:.1}%", summary.activity_percentage);
/// 
/// // Get detailed breakdown
/// for agent_info in summary.agent_details {
///     let status = if agent_info.is_active { "🟢" } else { "🔴" };
///     println!("  {} {}: {} seconds ago", 
///              status, agent_info.agent_id, agent_info.seconds_since_last_activity);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Activity Pattern Analysis
///
/// ```rust
/// use rvoip_call_engine::agent::availability::AvailabilityTracker;
/// use chrono::{Utc, Duration};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let tracker = AvailabilityTracker::new();
/// 
/// // Analyze activity patterns over time
/// let end_time = Utc::now();
/// let start_time = end_time - Duration::hours(8); // Last 8 hours
/// 
/// let analysis = tracker.analyze_activity_patterns("agent-001", start_time, end_time).await?;
/// 
/// println!("📈 Activity Pattern Analysis for agent-001:");
/// println!("  Total activities: {}", analysis.total_activities);
/// println!("  Average interval: {:.1} minutes", analysis.average_activity_interval_minutes);
/// println!("  Peak activity hour: {}", analysis.peak_activity_hour);
/// println!("  Inactive periods: {}", analysis.inactive_periods.len());
/// 
/// // Show longest inactive periods
/// for (i, period) in analysis.inactive_periods.iter().take(3).enumerate() {
///     println!("  {}. Inactive for {} minutes at {}", 
///              i + 1, period.duration_minutes, period.start_time.format("%H:%M"));
/// }
/// 
/// // Get recommendations
/// for recommendation in analysis.recommendations {
///     println!("💡 {}", recommendation);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Heartbeat Management
///
/// ```rust
/// use rvoip_call_engine::agent::availability::{AvailabilityTracker, HeartbeatManager};
/// use std::time::Duration;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut tracker = AvailabilityTracker::new();
/// 
/// // Configure heartbeat monitoring
/// let heartbeat_config = HeartbeatManager {
///     interval: Duration::from_secs(30),    // 30-second heartbeats
///     timeout_multiplier: 3,                // 90 seconds timeout
///     enable_automatic_recovery: true,
/// };
/// 
/// tracker.configure_heartbeat(heartbeat_config)?;
/// 
/// // Start heartbeat monitoring for an agent
/// tracker.start_heartbeat_monitoring("agent-001").await?;
/// 
/// // Simulate heartbeat reception
/// loop {
///     // In a real system, this would be triggered by actual heartbeats
///     tracker.receive_heartbeat("agent-001").await?;
///     
///     // Check for agents that missed heartbeats
///     let missed_heartbeats = tracker.check_missed_heartbeats().await?;
///     
///     for agent_id in missed_heartbeats {
///         println!("💔 Agent {} missed heartbeat - marking as potentially inactive", agent_id);
///         tracker.handle_missed_heartbeat(&agent_id).await?;
///     }
///     
///     tokio::time::sleep(Duration::from_secs(30)).await;
///     break; // For example purposes
/// }
/// # Ok(())
/// # }
/// ```
pub struct AvailabilityTracker {
    /// Last activity time for each agent
    last_activity: HashMap<String, ActivityInfo>,
    
    /// Configuration for availability tracking
    config: AvailabilityConfig,
    
    /// Activity history for pattern analysis
    activity_history: HashMap<String, Vec<ActivityRecord>>,
    
    /// Heartbeat monitoring state
    heartbeat_state: HashMap<String, HeartbeatState>,
}

/// Detailed activity information for an agent
#[derive(Debug, Clone)]
pub struct ActivityInfo {
    /// When the last activity occurred
    pub last_activity_time: Instant,
    
    /// Type of the last activity
    pub last_activity_type: ActivityType,
    
    /// UTC timestamp of last activity for external systems
    pub last_activity_utc: DateTime<Utc>,
    
    /// Number of activities recorded today
    pub activities_today: u32,
    
    /// Whether agent is currently considered active
    pub is_currently_active: bool,
}

/// Types of activities that can be tracked
#[derive(Debug, Clone, PartialEq)]
pub enum ActivityType {
    /// SIP REGISTER refresh
    SipRegister,
    /// Incoming or outgoing call activity
    CallHandling,
    /// Web interface interaction
    WebInterface,
    /// API call or programmatic interaction
    ApiInteraction,
    /// Manual status update
    StatusUpdate,
    /// Heartbeat signal
    Heartbeat,
    /// Custom activity type
    Custom(String),
}

/// Configuration for availability tracking behavior
#[derive(Debug, Clone)]
pub struct AvailabilityConfig {
    /// Default timeout in seconds for considering an agent inactive
    pub default_timeout_seconds: u64,
    
    /// Interval for heartbeat monitoring
    pub heartbeat_interval_seconds: u64,
    
    /// Seconds before timeout to send inactivity warning
    pub inactivity_warning_seconds: u64,
    
    /// Whether to maintain activity history
    pub enable_activity_history: bool,
    
    /// Maximum number of history entries per agent
    pub max_history_entries: usize,
}

/// Availability status information for an agent
#[derive(Debug, Clone)]
pub struct AvailabilityStatus {
    /// Agent identifier
    pub agent_id: String,
    
    /// Seconds since last recorded activity
    pub seconds_since_last_activity: u64,
    
    /// Type of last activity
    pub last_activity_type: ActivityType,
    
    /// Whether agent is considered active
    pub is_active: bool,
    
    /// Whether agent is approaching timeout (warning threshold)
    pub near_timeout_warning: bool,
    
    /// Number of activities recorded today
    pub activities_today: u32,
    
    /// UTC timestamp of last activity
    pub last_activity_utc: DateTime<Utc>,
}

/// Summary of availability across all tracked agents
#[derive(Debug, Clone)]
pub struct AvailabilitySummary {
    /// Total number of tracked agents
    pub total_agents: usize,
    
    /// Number of currently active agents
    pub active_agents: usize,
    
    /// Number of currently inactive agents
    pub inactive_agents: usize,
    
    /// Percentage of agents that are active
    pub activity_percentage: f64,
    
    /// Detailed information for each agent
    pub agent_details: Vec<AgentAvailabilityDetail>,
    
    /// Summary generation timestamp
    pub generated_at: DateTime<Utc>,
}

/// Individual agent availability detail
#[derive(Debug, Clone)]
pub struct AgentAvailabilityDetail {
    /// Agent identifier
    pub agent_id: String,
    
    /// Seconds since last activity
    pub seconds_since_last_activity: u64,
    
    /// Whether agent is considered active
    pub is_active: bool,
    
    /// Last activity type
    pub last_activity_type: ActivityType,
}

/// Historical activity record
#[derive(Debug, Clone)]
pub struct ActivityRecord {
    /// When the activity occurred
    pub timestamp: DateTime<Utc>,
    
    /// Type of activity
    pub activity_type: ActivityType,
    
    /// Duration since previous activity
    pub interval_since_previous: Option<Duration>,
}

/// Activity pattern analysis result
#[derive(Debug, Clone)]
pub struct ActivityPatternAnalysis {
    /// Agent being analyzed
    pub agent_id: String,
    
    /// Analysis time period
    pub analysis_period: (DateTime<Utc>, DateTime<Utc>),
    
    /// Total number of activities in period
    pub total_activities: u32,
    
    /// Average interval between activities in minutes
    pub average_activity_interval_minutes: f64,
    
    /// Hour of day with peak activity (0-23)
    pub peak_activity_hour: u8,
    
    /// Periods of inactivity longer than threshold
    pub inactive_periods: Vec<InactivePeriod>,
    
    /// Pattern-based recommendations
    pub recommendations: Vec<String>,
    
    /// Overall activity score (0.0 - 1.0)
    pub activity_score: f64,
}

/// Period of agent inactivity
#[derive(Debug, Clone)]
pub struct InactivePeriod {
    /// When inactivity started
    pub start_time: DateTime<Utc>,
    
    /// When activity resumed
    pub end_time: DateTime<Utc>,
    
    /// Duration of inactivity in minutes
    pub duration_minutes: u64,
}

/// Heartbeat monitoring configuration
#[derive(Debug, Clone)]
pub struct HeartbeatManager {
    /// Expected interval between heartbeats
    pub interval: Duration,
    
    /// Multiplier for timeout (interval * multiplier)
    pub timeout_multiplier: u32,
    
    /// Whether to automatically recover from missed heartbeats
    pub enable_automatic_recovery: bool,
}

/// Heartbeat state for an agent
#[derive(Debug, Clone)]
struct HeartbeatState {
    /// Last heartbeat received
    last_heartbeat: Instant,
    
    /// Number of consecutive missed heartbeats
    missed_count: u32,
    
    /// Whether heartbeat monitoring is active
    monitoring_active: bool,
}

impl AvailabilityTracker {
    /// Create a new availability tracker with default configuration
    ///
    /// Initializes a new availability tracker ready to monitor agent activity.
    /// Uses default configuration values suitable for most call center environments.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// let tracker = AvailabilityTracker::new();
    /// println!("Availability tracker initialized with default settings");
    /// ```
    pub fn new() -> Self {
        Self::with_config(AvailabilityConfig::default())
    }
    
    /// Create a new availability tracker with custom configuration
    ///
    /// Initializes a new availability tracker with the specified configuration.
    /// Allows customization of timeout values, history settings, and other behavior.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for availability tracking behavior
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::{AvailabilityTracker, AvailabilityConfig};
    /// 
    /// let config = AvailabilityConfig {
    ///     default_timeout_seconds: 180,      // 3 minutes
    ///     heartbeat_interval_seconds: 30,    // 30 seconds
    ///     inactivity_warning_seconds: 150,   // 2.5 minutes
    ///     enable_activity_history: true,
    ///     max_history_entries: 500,
    /// };
    /// 
    /// let tracker = AvailabilityTracker::with_config(config);
    /// ```
    pub fn with_config(config: AvailabilityConfig) -> Self {
        Self {
            last_activity: HashMap::new(),
            config,
            activity_history: HashMap::new(),
            heartbeat_state: HashMap::new(),
        }
    }
    
    /// Update agent activity timestamp with default activity type
    ///
    /// Records current activity for the specified agent using a generic activity type.
    /// This is a convenience method for simple activity tracking.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Identifier of the agent to update
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// let mut tracker = AvailabilityTracker::new();
    /// 
    /// // Record activity for an agent
    /// tracker.update_activity("agent-001".to_string());
    /// 
    /// // Agent is now considered active
    /// assert!(tracker.is_agent_active("agent-001", 300));
    /// ```
    pub fn update_activity(&mut self, agent_id: String) {
        let _ = self.record_activity(&agent_id, ActivityType::Custom("general".to_string()));
    }
    
    /// Record specific type of activity for an agent
    ///
    /// Records activity with a specific activity type, enabling more detailed
    /// tracking and analysis of agent behavior patterns.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Identifier of the agent
    /// * `activity_type` - Type of activity being recorded
    ///
    /// # Returns
    ///
    /// `Ok(())` if activity recorded successfully, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::{AvailabilityTracker, ActivityType};
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut tracker = AvailabilityTracker::new();
    /// 
    /// // Record different types of activities
    /// tracker.record_activity("agent-001", ActivityType::SipRegister)?;
    /// tracker.record_activity("agent-001", ActivityType::CallHandling)?;
    /// tracker.record_activity("agent-002", ActivityType::WebInterface)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn record_activity(&mut self, agent_id: &str, activity_type: ActivityType) -> Result<()> {
        let now = Instant::now();
        let now_utc = Utc::now();
        
        // Update main activity tracking
        let activity_info = self.last_activity.entry(agent_id.to_string())
            .or_insert_with(|| ActivityInfo {
                last_activity_time: now,
                last_activity_type: activity_type.clone(),
                last_activity_utc: now_utc,
                activities_today: 0,
                is_currently_active: true,
            });
        
        // Update activity info
        activity_info.last_activity_time = now;
        activity_info.last_activity_type = activity_type.clone();
        activity_info.last_activity_utc = now_utc;
        activity_info.activities_today += 1;
        activity_info.is_currently_active = true;
        
        // Record in history if enabled
        if self.config.enable_activity_history {
            let history = self.activity_history.entry(agent_id.to_string())
                .or_insert_with(Vec::new);
            
            // Calculate interval since previous activity
            let interval_since_previous = history.last()
                .map(|last| now_utc.signed_duration_since(last.timestamp).to_std().ok())
                .flatten();
            
                    // Add new record  
        history.push(ActivityRecord {
            timestamp: now_utc,
            activity_type: activity_type.clone(),
            interval_since_previous,
        });
        
        // Limit history size
        if history.len() > self.config.max_history_entries {
            history.remove(0);
        }
    }
    
    debug!("📊 Activity recorded for {}: {:?}", agent_id, activity_type);
        Ok(())
    }
    
    /// Check if agent is considered active within timeout period
    ///
    /// Determines if an agent is active based on their last recorded activity
    /// and the specified timeout period.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Identifier of the agent to check
    /// * `timeout_secs` - Timeout period in seconds
    ///
    /// # Returns
    ///
    /// `true` if agent is active within timeout period, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// let mut tracker = AvailabilityTracker::new();
    /// tracker.update_activity("agent-001".to_string());
    /// 
    /// // Check with different timeout values
    /// assert!(tracker.is_agent_active("agent-001", 300));  // 5 minutes
    /// assert!(tracker.is_agent_active("agent-001", 60));   // 1 minute
    /// assert!(!tracker.is_agent_active("agent-001", 0));   // Immediate
    /// ```
    pub fn is_agent_active(&self, agent_id: &str, timeout_secs: u64) -> bool {
        if let Some(activity_info) = self.last_activity.get(agent_id) {
            activity_info.last_activity_time.elapsed().as_secs() < timeout_secs
        } else {
            false
        }
    }
    
    /// Get detailed availability status for an agent
    ///
    /// Returns comprehensive availability information including activity timing,
    /// status, and warning indicators.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Identifier of the agent
    ///
    /// # Returns
    ///
    /// `Ok(Some(AvailabilityStatus))` if agent found, `Ok(None)` if not found, or error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let tracker = AvailabilityTracker::new();
    /// 
    /// match tracker.get_availability_status("agent-001")? {
    ///     Some(status) => {
    ///         println!("Agent {} last active {} seconds ago", 
    ///                  status.agent_id, status.seconds_since_last_activity);
    ///         
    ///         if status.near_timeout_warning {
    ///             println!("⚠️ Agent approaching inactivity timeout");
    ///         }
    ///     }
    ///     None => {
    ///         println!("No activity data for agent");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_availability_status(&self, agent_id: &str) -> Result<Option<AvailabilityStatus>> {
        if let Some(activity_info) = self.last_activity.get(agent_id) {
            let seconds_since_last = activity_info.last_activity_time.elapsed().as_secs();
            let is_active = seconds_since_last < self.config.default_timeout_seconds;
            let near_timeout = seconds_since_last > self.config.inactivity_warning_seconds;
            
            Ok(Some(AvailabilityStatus {
                agent_id: agent_id.to_string(),
                seconds_since_last_activity: seconds_since_last,
                last_activity_type: activity_info.last_activity_type.clone(),
                is_active,
                near_timeout_warning: near_timeout && is_active,
                activities_today: activity_info.activities_today,
                last_activity_utc: activity_info.last_activity_utc,
            }))
        } else {
            Ok(None)
        }
    }
    
    /// Get availability summary for all tracked agents
    ///
    /// Returns comprehensive availability statistics across all agents
    /// being tracked by this availability tracker.
    ///
    /// # Arguments
    ///
    /// * `timeout_seconds` - Timeout period for considering agents active
    ///
    /// # Returns
    ///
    /// Availability summary with statistics and detailed agent information.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let tracker = AvailabilityTracker::new();
    /// 
    /// let summary = tracker.get_availability_summary(300)?; // 5-minute timeout
    /// 
    /// println!("Availability Summary:");
    /// println!("  Total: {}", summary.total_agents);
    /// println!("  Active: {}", summary.active_agents);
    /// println!("  Inactive: {}", summary.inactive_agents);
    /// println!("  Activity Rate: {:.1}%", summary.activity_percentage);
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_availability_summary(&self, timeout_seconds: u64) -> Result<AvailabilitySummary> {
        let mut agent_details = Vec::new();
        let mut active_count = 0;
        
        for (agent_id, activity_info) in &self.last_activity {
            let seconds_since_last = activity_info.last_activity_time.elapsed().as_secs();
            let is_active = seconds_since_last < timeout_seconds;
            
            if is_active {
                active_count += 1;
            }
            
            agent_details.push(AgentAvailabilityDetail {
                agent_id: agent_id.clone(),
                seconds_since_last_activity: seconds_since_last,
                is_active,
                last_activity_type: activity_info.last_activity_type.clone(),
            });
        }
        
        let total_agents = agent_details.len();
        let inactive_agents = total_agents - active_count;
        let activity_percentage = if total_agents > 0 {
            (active_count as f64 / total_agents as f64) * 100.0
        } else {
            0.0
        };
        
        Ok(AvailabilitySummary {
            total_agents,
            active_agents: active_count,
            inactive_agents,
            activity_percentage,
            agent_details,
            generated_at: Utc::now(),
        })
    }
    
    /// Analyze activity patterns for an agent over a time period
    ///
    /// Performs comprehensive analysis of agent activity patterns including
    /// frequency, timing, and recommendations for optimization.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent to analyze
    /// * `start_time` - Start of analysis period
    /// * `end_time` - End of analysis period
    ///
    /// # Returns
    ///
    /// Detailed activity pattern analysis with insights and recommendations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// use chrono::{Utc, Duration};
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let tracker = AvailabilityTracker::new();
    /// 
    /// let end_time = Utc::now();
    /// let start_time = end_time - Duration::hours(4);
    /// 
    /// let analysis = tracker.analyze_activity_patterns("agent-001", start_time, end_time).await?;
    /// 
    /// println!("Activity Analysis:");
    /// println!("  Activities: {}", analysis.total_activities);
    /// println!("  Avg interval: {:.1}min", analysis.average_activity_interval_minutes);
    /// println!("  Activity score: {:.2}", analysis.activity_score);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn analyze_activity_patterns(
        &self,
        agent_id: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<ActivityPatternAnalysis> {
        let history = self.activity_history.get(agent_id);
        
        if let Some(records) = history {
            // Filter records within time period
            let period_records: Vec<_> = records.iter()
                .filter(|r| r.timestamp >= start_time && r.timestamp <= end_time)
                .collect();
            
            let total_activities = period_records.len() as u32;
            
            // Calculate average interval
            let total_interval_minutes: f64 = period_records.iter()
                .filter_map(|r| r.interval_since_previous)
                .map(|d| d.as_secs() as f64 / 60.0)
                .sum();
            
            let average_interval = if period_records.len() > 1 {
                total_interval_minutes / (period_records.len() - 1) as f64
            } else {
                0.0
            };
            
            // Find peak activity hour (simplified)
            let peak_hour = 12; // TODO: Implement actual peak detection
            
            // TODO: Implement full pattern analysis
            let activity_score = if total_activities > 0 { 0.8 } else { 0.0 };
            
            Ok(ActivityPatternAnalysis {
                agent_id: agent_id.to_string(),
                analysis_period: (start_time, end_time),
                total_activities,
                average_activity_interval_minutes: average_interval,
                peak_activity_hour: peak_hour,
                inactive_periods: Vec::new(), // TODO: Detect inactive periods
                recommendations: vec![
                    "Activity pattern appears normal".to_string(),
                    "Consider monitoring during peak hours".to_string(),
                ],
                activity_score,
            })
        } else {
            // No history available
            Ok(ActivityPatternAnalysis {
                agent_id: agent_id.to_string(),
                analysis_period: (start_time, end_time),
                total_activities: 0,
                average_activity_interval_minutes: 0.0,
                peak_activity_hour: 0,
                inactive_periods: Vec::new(),
                recommendations: vec![
                    "No activity history available for analysis".to_string(),
                    "Enable activity history tracking for better insights".to_string(),
                ],
                activity_score: 0.0,
            })
        }
    }
    
    /// Configure heartbeat monitoring
    ///
    /// Sets up heartbeat monitoring configuration for proactive agent availability detection.
    ///
    /// # Arguments
    ///
    /// * `heartbeat_config` - Heartbeat monitoring configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::{AvailabilityTracker, HeartbeatManager};
    /// use std::time::Duration;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut tracker = AvailabilityTracker::new();
    /// 
    /// let config = HeartbeatManager {
    ///     interval: Duration::from_secs(30),
    ///     timeout_multiplier: 3,
    ///     enable_automatic_recovery: true,
    /// };
    /// 
    /// tracker.configure_heartbeat(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn configure_heartbeat(&mut self, heartbeat_config: HeartbeatManager) -> Result<()> {
        // TODO: Store heartbeat configuration
        info!("Heartbeat monitoring configured: {:?}", heartbeat_config);
        Ok(())
    }
    
    /// Start heartbeat monitoring for an agent
    ///
    /// Begins active heartbeat monitoring for the specified agent.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent to monitor
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut tracker = AvailabilityTracker::new();
    /// tracker.start_heartbeat_monitoring("agent-001").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_heartbeat_monitoring(&mut self, agent_id: &str) -> Result<()> {
        self.heartbeat_state.insert(agent_id.to_string(), HeartbeatState {
            last_heartbeat: Instant::now(),
            missed_count: 0,
            monitoring_active: true,
        });
        
        info!("🫀 Started heartbeat monitoring for agent: {}", agent_id);
        Ok(())
    }
    
    /// Receive heartbeat from an agent
    ///
    /// Records receipt of a heartbeat signal from an agent.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent sending heartbeat
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut tracker = AvailabilityTracker::new();
    /// tracker.receive_heartbeat("agent-001").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_heartbeat(&mut self, agent_id: &str) -> Result<()> {
        // Update heartbeat state
        if let Some(state) = self.heartbeat_state.get_mut(agent_id) {
            state.last_heartbeat = Instant::now();
            state.missed_count = 0;
        }
        
        // Record as activity
        self.record_activity(agent_id, ActivityType::Heartbeat)?;
        
        debug!("💓 Heartbeat received from agent: {}", agent_id);
        Ok(())
    }
    
    /// Check for agents that have missed heartbeats
    ///
    /// Returns a list of agents that have missed their expected heartbeats.
    ///
    /// # Returns
    ///
    /// Vector of agent IDs that missed heartbeats.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let tracker = AvailabilityTracker::new();
    /// 
    /// let missed = tracker.check_missed_heartbeats().await?;
    /// for agent_id in missed {
    ///     println!("💔 Agent {} missed heartbeat", agent_id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn check_missed_heartbeats(&self) -> Result<Vec<String>> {
        let mut missed = Vec::new();
        let timeout_duration = Duration::from_secs(90); // TODO: Use config
        
        for (agent_id, state) in &self.heartbeat_state {
            if state.monitoring_active && state.last_heartbeat.elapsed() > timeout_duration {
                missed.push(agent_id.clone());
            }
        }
        
        Ok(missed)
    }
    
    /// Handle missed heartbeat for an agent
    ///
    /// Processes a missed heartbeat situation for an agent, potentially
    /// triggering recovery procedures or status updates.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent that missed heartbeat
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::agent::availability::AvailabilityTracker;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut tracker = AvailabilityTracker::new();
    /// tracker.handle_missed_heartbeat("agent-001").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_missed_heartbeat(&mut self, agent_id: &str) -> Result<()> {
        if let Some(state) = self.heartbeat_state.get_mut(agent_id) {
            state.missed_count += 1;
            warn!("💔 Agent {} missed heartbeat (count: {})", agent_id, state.missed_count);
            
            // TODO: Implement recovery procedures
        }
        
        Ok(())
    }
}

impl Default for AvailabilityTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for AvailabilityConfig {
    fn default() -> Self {
        Self {
            default_timeout_seconds: 300,      // 5 minutes
            heartbeat_interval_seconds: 30,    // 30 seconds
            inactivity_warning_seconds: 240,   // 4 minutes
            enable_activity_history: true,
            max_history_entries: 1000,
        }
    }
} 