//! # Supervisor API for Call Center Oversight
//!
//! This module provides comprehensive supervisor APIs for real-time monitoring and management
//! of call center operations. Supervisors can monitor agents, track call performance, manage
//! queues, and perform coaching activities through this interface.
//!
//! ## Overview
//!
//! The Supervisor API enables comprehensive oversight of call center operations, providing
//! real-time visibility into agent performance, call routing efficiency, and queue management.
//! It serves as the primary interface for call center supervisors to monitor and manage
//! day-to-day operations.
//!
//! ## Key Features
//!
//! - **Real-Time Monitoring**: Live agent status and call tracking
//! - **Performance Analytics**: Comprehensive metrics and reporting
//! - **Queue Management**: Queue statistics and manual call routing
//! - **Call Listening**: Monitor live calls for quality assurance
//! - **Agent Coaching**: Send coaching messages during calls
//! - **Bridge Management**: Track and manage active call bridges
//! - **Force Assignment**: Manual call routing when needed
//!
//! ## Monitoring Capabilities
//!
//! ### Agent Monitoring
//! - Real-time agent status (Available, Busy, Offline)
//! - Active call count per agent
//! - Agent performance metrics and scores
//! - Skills and capabilities tracking
//!
//! ### Call Monitoring
//! - Active calls across all agents
//! - Call duration and status tracking
//! - Queue assignment and routing history
//! - Bridge information for connected calls
//!
//! ### Queue Monitoring
//! - Queue depth and wait times
//! - Routing performance metrics
//! - Service level achievements
//! - Overflow and abandonment tracking
//!
//! ## Examples
//!
//! ### Basic Supervisor Dashboard
//!
//! ```rust
//! use rvoip_call_engine::api::SupervisorApi;
//! use rvoip_call_engine::CallCenterEngine;
//! use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize supervisor API
//! let engine = CallCenterEngine::new(Default::default(), None).await?;
//! let supervisor = SupervisorApi::new(engine);
//! 
//! // Get real-time statistics
//! let stats = supervisor.get_stats().await;
//! println!("📊 Call Center Overview:");
//! println!("  Active calls: {}", stats.active_calls);
//! println!("  Available agents: {}", stats.available_agents);
//! println!("  Busy agents: {}", stats.busy_agents);
//! println!("  Queued calls: {}", stats.queued_calls);
//! 
//! // List all agents with their status
//! let agents = supervisor.list_agents().await;
//! println!("\n👥 Agent Status:");
//! for agent in agents {
//!     println!("  {} ({}): {:?} - {} active calls", 
//!              agent.agent_id.0, agent.agent_id.0, agent.status, agent.current_calls);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Monitoring and Management
//!
//! ```rust
//! use rvoip_call_engine::api::SupervisorApi;
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(Default::default(), None).await?;
//! let supervisor = SupervisorApi::new(engine);
//! 
//! // Monitor all queues
//! let queue_stats = supervisor.get_all_queue_stats().await?;
//! 
//! for (queue_id, stats) in queue_stats {
//!     println!("📋 Queue: {}", queue_id);
//!     println!("  Queued calls: {}", stats.total_calls);
//!     println!("  Average wait time: {}s", stats.average_wait_time_seconds);
//!     println!("  Longest wait: {}s", stats.longest_wait_time_seconds);
//!     
//!     // Check for high queue depth
//!     if stats.total_calls > 10 {
//!         println!("  ⚠️ High queue depth detected!");
//!         
//!         // Get detailed queue information
//!         let queued_calls = supervisor.get_queued_calls(&queue_id).await;
//!         for call in queued_calls.iter().take(5) {
//!             println!("    📞 Call {} waiting for {}s", 
//!                      call.session_id, call.queue_time_seconds);
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Performance Analytics
//!
//! ```rust
//! use rvoip_call_engine::api::SupervisorApi;
//! use chrono::{Utc, Duration};
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(Default::default(), None).await?;
//! let supervisor = SupervisorApi::new(engine);
//! 
//! // Get performance metrics for the last hour
//! let end_time = Utc::now();
//! let start_time = end_time - Duration::hours(1);
//! 
//! let metrics = supervisor.get_performance_metrics(start_time, end_time).await;
//! 
//! println!("📈 Performance Metrics (Last Hour):");
//! println!("  Total calls: {}", metrics.total_calls);
//! println!("  Calls answered: {} ({:.1}%)", 
//!          metrics.calls_answered,
//!          metrics.calls_answered as f32 / metrics.total_calls as f32 * 100.0);
//! println!("  Calls abandoned: {} ({:.1}%)",
//!          metrics.calls_abandoned,
//!          metrics.calls_abandoned as f32 / metrics.total_calls as f32 * 100.0);
//! println!("  Average wait time: {:.1}s", metrics.average_wait_time_ms as f32 / 1000.0);
//! println!("  Average handle time: {:.1}s", metrics.average_handle_time_ms as f32 / 1000.0);
//! println!("  Service level: {:.1}%", metrics.service_level_percentage);
//! 
//! // Alert on poor performance
//! if metrics.service_level_percentage < 80.0 {
//!     println!("🚨 Service level below target (80%)!");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Coaching and Call Monitoring
//!
//! ```rust
//! use rvoip_call_engine::api::SupervisorApi;
//! use rvoip_call_engine::agent::AgentId;
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(Default::default(), None).await?;
//! let supervisor = SupervisorApi::new(engine);
//! 
//! let agent_id = AgentId::from("agent-001");
//! 
//! // Monitor agent's active calls
//! let agent_calls = supervisor.monitor_agent_calls(&agent_id).await;
//! 
//! for call in agent_calls {
//!     println!("📞 Agent {} handling call {}", agent_id, call.session_id);
//!     println!("  Duration: {}s", call.duration_seconds);
//!     println!("  Caller ID: {:?}", call.caller_id);
//!     
//!     // Listen to the call for quality monitoring
//!     if let Some(bridge_id) = supervisor.listen_to_call(&call.session_id).await? {
//!         println!("  🎧 Listening to call on bridge: {}", bridge_id);
//!     }
//!     
//!     // Send coaching message if call is long
//!     if call.duration_seconds > 300 { // 5 minutes
//!         supervisor.coach_agent(
//!             &agent_id,
//!             "Consider wrapping up the call soon - queue is building"
//!         ).await?;
//!         println!("  💬 Coaching message sent to agent");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Manual Call Assignment
//!
//! ```rust
//! use rvoip_call_engine::api::SupervisorApi;
//! use rvoip_call_engine::agent::AgentId;
//! use rvoip_session_core::SessionId;
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(Default::default(), None).await?;
//! let supervisor = SupervisorApi::new(engine);
//! 
//! // Find a VIP call that needs immediate attention
//! let all_calls = supervisor.list_active_calls().await;
//! 
//! for call in all_calls {
//!     // Check if this is a VIP customer call waiting in queue
//!     if call.queue_id == Some("vip".to_string()) && 
//!        call.queue_time_seconds > 30 {
//!         
//!         println!("🌟 VIP call {} waiting {}s - needs immediate attention", 
//!                  call.session_id, call.queue_time_seconds);
//!         
//!         // Find best available agent for VIP handling
//!         let agents = supervisor.list_agents().await;
//!         let vip_agent = agents.iter()
//!             .find(|agent| agent.skills.contains(&"vip".to_string()) && 
//!                          agent.current_calls == 0);
//!         
//!         if let Some(agent) = vip_agent {
//!             // Force assign the call to the VIP specialist
//!             supervisor.force_assign_call(
//!                 call.session_id,
//!                 agent.agent_id.clone()
//!             ).await?;
//!             
//!             println!("✅ VIP call assigned to specialist: {}", agent.agent_id.0);
//!         } else {
//!             println!("⚠️ No VIP specialists available");
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Real-Time Dashboard Integration
//!
//! ```rust
//! use rvoip_call_engine::api::SupervisorApi;
//! use tokio::time::{interval, Duration};
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(Default::default(), None).await?;
//! let supervisor = SupervisorApi::new(engine);
//! 
//! // Real-time dashboard updates every 10 seconds
//! let mut interval = interval(Duration::from_secs(10));
//! 
//! loop {
//!     interval.tick().await;
//!     
//!     // Get current statistics
//!     let stats = supervisor.get_stats().await;
//!     let queue_stats = supervisor.get_all_queue_stats().await?;
//!     
//!     // Calculate key metrics
//!     let total_calls: usize = queue_stats.iter()
//!         .map(|(_, stats)| stats.total_calls)
//!         .sum();
//!     
//!     let max_wait_time: u64 = queue_stats.iter()
//!         .map(|(_, stats)| stats.longest_wait_time_seconds)
//!         .max()
//!         .unwrap_or(0);
//!     
//!     // Display dashboard
//!     println!("\n🖥️ Real-time Dashboard Update");
//!     println!("┌─────────────────────────────────────┐");
//!     println!("│ Active Calls: {:>3} | Queued: {:>3}     │", 
//!              stats.active_calls, total_calls);
//!     println!("│ Available: {:>3} | Busy: {:>6}       │", 
//!              stats.available_agents, stats.busy_agents);
//!     println!("│ Max Wait: {:>3}s | Queues: {:>3}      │", 
//!              max_wait_time, queue_stats.len());
//!     println!("└─────────────────────────────────────┘");
//!     
//!     // Alert conditions
//!     if total_calls > 20 {
//!         println!("🚨 HIGH QUEUE VOLUME ALERT!");
//!     }
//!     if max_wait_time > 180 {
//!         println!("⏰ LONG WAIT TIME ALERT!");
//!     }
//!     
//!     // In a real implementation, this would break based on some condition
//!     break;
//! }
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use chrono::{DateTime, Utc};
use rvoip_session_core::{SessionId, BridgeId};

use crate::{
    agent::AgentId,
    error::Result as CallCenterResult,
    orchestrator::{CallCenterEngine, types::{AgentInfo, CallInfo, OrchestratorStats}},
    queue::QueueStats,
};

/// # Supervisor API for Call Center Oversight
/// 
/// The `SupervisorApi` provides comprehensive monitoring and management capabilities
/// for call center supervisors. It enables real-time oversight of agents, calls,
/// queues, and system performance with advanced features for coaching and manual
/// call routing.
/// 
/// ## Core Capabilities
/// 
/// ### Monitoring
/// - **Agent Monitoring**: Real-time agent status and performance tracking
/// - **Call Monitoring**: Active call tracking with detailed information
/// - **Queue Monitoring**: Queue statistics and wait time analysis
/// - **Performance Analytics**: Comprehensive metrics and reporting
/// 
/// ### Management
/// - **Manual Routing**: Force assignment of calls to specific agents
/// - **Call Listening**: Monitor live calls for quality assurance
/// - **Agent Coaching**: Send coaching messages during active calls
/// - **Bridge Management**: Track and manage call connections
/// 
/// ## Thread Safety
/// 
/// The `SupervisorApi` is thread-safe and can be cloned for use across multiple
/// tasks or components. It maintains a reference to the underlying call center
/// engine which handles all coordination and state management.
/// 
/// ## Examples
/// 
/// ### Basic Usage
/// 
/// ```rust
/// use rvoip_call_engine::api::SupervisorApi;
/// use rvoip_call_engine::CallCenterEngine;
/// use std::sync::Arc;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = CallCenterEngine::new(Default::default(), None).await?;
/// let supervisor = SupervisorApi::new(engine);
/// 
/// // Get real-time statistics
/// let stats = supervisor.get_stats().await;
/// println!("Active calls: {}", stats.active_calls);
/// 
/// // List all agents
/// let agents = supervisor.list_agents().await;
/// println!("Total agents: {}", agents.len());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct SupervisorApi {
    engine: Arc<CallCenterEngine>,
}

impl SupervisorApi {
    /// Create a new supervisor API instance
    /// 
    /// Initializes a new supervisor API connected to the specified call center engine.
    /// The API provides access to all monitoring and management capabilities.
    /// 
    /// # Arguments
    /// 
    /// * `engine` - Shared reference to the call center engine
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_call_engine::CallCenterEngine;
    /// use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self { engine }
    }
    
    /// Get real-time orchestrator statistics
    /// 
    /// Returns comprehensive real-time statistics about the call center including
    /// active calls, agent availability, queue depths, and routing performance.
    /// This is the primary method for getting an overview of system status.
    /// 
    /// # Returns
    /// 
    /// `OrchestratorStats` containing:
    /// - Active calls count
    /// - Available/busy agents count
    /// - Queue depths across all queues
    /// - Routing performance metrics
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// let stats = supervisor.get_stats().await;
    /// 
    /// println!("📊 Call Center Status:");
    /// println!("  Active calls: {}", stats.active_calls);
    /// println!("  Available agents: {}", stats.available_agents);
    /// println!("  Busy agents: {}", stats.busy_agents);
    /// println!("  Queued calls: {}", stats.queued_calls);
    /// 
    /// // Check for capacity issues
    /// if stats.queued_calls > stats.available_agents * 2 {
    ///     println!("⚠️ High queue to agent ratio detected!");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_stats(&self) -> OrchestratorStats {
        self.engine.get_stats().await
    }
    
    /// List all agents with their current status
    /// 
    /// Returns detailed information for each registered agent including their
    /// current status, active calls, skills, and performance metrics. This
    /// provides a comprehensive view of agent availability and activity.
    /// 
    /// # Returns
    /// 
    /// Vector of `AgentInfo` containing:
    /// - Agent identification and contact information
    /// - Current status (available, busy, offline)
    /// - Active calls count and details
    /// - Skills and capabilities
    /// - Performance score and metrics
    /// - Last activity timestamp
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// let agents = supervisor.list_agents().await;
    /// 
    /// println!("👥 Agent Overview:");
    /// for agent in agents {
    ///     let status_icon = match agent.status {
    ///         _ if agent.current_calls > 0 => "📞",
    ///         _ => "🟢",
    ///     };
    ///     
    ///     println!("  {} {} ({}): {} active calls, skills: {:?}", 
    ///              status_icon, agent.agent_id.0, agent.agent_id.0, 
    ///              agent.current_calls, agent.skills);
    ///     
    ///     // Highlight performance issues
    ///     if agent.performance_score < 0.7 {
    ///         println!("    ⚠️ Performance below threshold");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        self.engine.list_agents().await
    }
    
    /// Get detailed information about a specific agent
    /// 
    /// Retrieves comprehensive information for a single agent including their
    /// current status, active calls, and performance metrics.
    /// 
    /// # Arguments
    /// 
    /// * `agent_id` - Identifier of the agent to retrieve
    /// 
    /// # Returns
    /// 
    /// `Some(AgentInfo)` if agent found, `None` if agent doesn't exist.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_call_engine::agent::AgentId;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// let agent_id = AgentId::from("agent-001");
    /// 
    /// if let Some(agent) = supervisor.get_agent_details(&agent_id).await {
    ///     println!("🔍 Agent Details:");
    ///     println!("  Agent ID: {}", agent.agent_id.0);
    ///     println!("  Status: {:?}", agent.status);
    ///     println!("  Active calls: {}", agent.current_calls);
    ///     println!("  Performance: {:.1}%", agent.performance_score * 100.0);
    ///     println!("  Skills: {:?}", agent.skills);
    /// } else {
    ///     println!("❌ Agent not found");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_agent_details(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        self.engine.get_agent_info(agent_id).await
    }
    
    /// List all active calls
    /// 
    /// Returns comprehensive information about all calls currently active in the
    /// system, including both connected calls and calls waiting in queues.
    /// This provides a system-wide view of call activity.
    /// 
    /// # Returns
    /// 
    /// Vector of `CallInfo` containing:
    /// - Session identification and routing information
    /// - Call status (active, queued, ringing)
    /// - Agent assignment details
    /// - Queue assignment and wait times
    /// - Call duration and timestamps
    /// - Caller identification information
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// let active_calls = supervisor.list_active_calls().await;
    /// 
    /// println!("📞 Active Calls Overview:");
    /// for call in active_calls {
    ///     println!("  Call {}: {:?}", call.session_id, call.status);
    ///     
    ///     if let Some(agent_id) = &call.agent_id {
    ///         println!("    Agent: {}", agent_id);
    ///     }
    ///     
    ///     if let Some(queue_id) = &call.queue_id {
    ///         let wait_time = call.queue_time_seconds;
    ///         println!("    Queue: {} (waiting {}s)", queue_id, wait_time);
    ///     }
    ///     
    ///     let duration = call.duration_seconds;
///     println!("    Duration: {}s", duration);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_active_calls(&self) -> Vec<CallInfo> {
        self.engine.active_calls()
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get detailed information about a specific call
    /// 
    /// Retrieves comprehensive information for a single call by its session ID.
    /// This is useful for detailed call analysis and troubleshooting.
    /// 
    /// # Arguments
    /// 
    /// * `session_id` - Session identifier of the call to retrieve
    /// 
    /// # Returns
    /// 
    /// `Some(CallInfo)` if call found, `None` if call doesn't exist or has ended.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_session_core::SessionId;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// let session_id = SessionId::new(); // In practice, get from active calls
    /// 
    /// if let Some(call) = supervisor.get_call_details(&session_id).await {
    ///     println!("📞 Call Details:");
    ///     println!("  Session: {}", call.session_id);
    ///     println!("  Status: {:?}", call.status);
    ///     println!("  Caller ID: {:?}", call.caller_id);
    ///     
    ///     if let Some(agent) = &call.agent_id {
    ///         println!("  Assigned Agent: {}", agent);
    ///     }
    ///     
    ///     let duration = call.duration_seconds;
///     println!("  Duration: {}m {}s", duration / 60, duration % 60);
    /// } else {
    ///     println!("❌ Call not found or has ended");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_call_details(&self, session_id: &SessionId) -> Option<CallInfo> {
        self.engine.active_calls()
            .get(session_id)
            .map(|entry| entry.clone())
    }
    
    /// Monitor calls assigned to a specific agent
    /// 
    /// Returns information about all calls currently assigned to the specified
    /// agent. This is useful for agent-specific monitoring and coaching.
    /// 
    /// # Arguments
    /// 
    /// * `agent_id` - Identifier of the agent to monitor
    /// 
    /// # Returns
    /// 
    /// Vector of `CallInfo` for all calls assigned to the agent.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_call_engine::agent::AgentId;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// let agent_id = AgentId::from("agent-001");
    /// 
    /// let agent_calls = supervisor.monitor_agent_calls(&agent_id).await;
    /// 
    /// println!("📞 Agent {} Calls:", agent_id);
/// for call in &agent_calls {
///     let duration = call.duration_seconds;
///     println!("  Call {}: {}m {}s", 
///              call.session_id, duration / 60, duration % 60);
    ///     
    ///     println!("    From: {}", call.caller_id);
    ///     
    ///     // Alert on long calls
    ///     if duration > 600 { // 10 minutes
    ///         println!("    ⚠️ Long call duration");
    ///     }
    /// }
    /// 
    /// if agent_calls.is_empty() {
    ///     println!("  No active calls");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn monitor_agent_calls(&self, agent_id: &AgentId) -> Vec<CallInfo> {
        self.engine.active_calls()
            .iter()
            .filter(|entry| entry.value().agent_id.as_ref() == Some(agent_id))
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get queue statistics for all queues
    /// 
    /// Returns comprehensive statistics for all configured queues including
    /// call counts, wait times, and performance metrics.
    /// 
    /// # Returns
    /// 
    /// `Ok(Vec<(String, QueueStats)>)` with queue name and statistics pairs,
    /// or error if statistics retrieval fails.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// let queue_stats = supervisor.get_all_queue_stats().await?;
    /// 
    /// println!("📋 Queue Statistics:");
    /// for (queue_id, stats) in queue_stats {
    ///     println!("  Queue: {}", queue_id);
    ///     println!("    Total calls: {}", stats.total_calls);
    ///     println!("    Avg wait: {}s", stats.average_wait_time_seconds);
    ///     println!("    Max wait: {}s", stats.longest_wait_time_seconds);
    ///     
    ///     // Alert on high wait times
    ///     if stats.average_wait_time_seconds > 120 {
    ///         println!("    🚨 High average wait time!");
    ///     }
    ///     
    ///     // Alert on queue depth
    ///     if stats.total_calls > 15 {
    ///         println!("    ⚠️ High queue depth!");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_all_queue_stats(&self) -> CallCenterResult<Vec<(String, QueueStats)>> {
        self.engine.get_queue_stats().await
    }
    
    /// Get calls in a specific queue
    /// 
    /// Returns information about all calls currently waiting in the specified
    /// queue. This provides detailed queue analysis and wait time monitoring.
    /// 
    /// # Arguments
    /// 
    /// * `queue_id` - Identifier of the queue to examine
    /// 
    /// # Returns
    /// 
    /// Vector of `CallInfo` for all calls in the specified queue.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// let vip_queue_calls = supervisor.get_queued_calls("vip").await;
    /// 
    /// print!("🌟 VIP Queue Status:");
    /// if vip_queue_calls.is_empty() {
    ///     println!("  No calls waiting");
    /// } else {
    ///     for (index, call) in vip_queue_calls.iter().enumerate() {
    ///         let wait_time = call.queue_time_seconds;
    ///         println!("  {}. Call {} - waiting {}s", 
    ///                  index + 1, call.session_id, wait_time);
    ///         
    ///         println!("     From: {}", call.caller_id);
    ///         
    ///         // VIP calls should be prioritized
    ///         if wait_time > 60 {
    ///             println!("     🚨 VIP customer waiting too long!");
    ///         }
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_queued_calls(&self, queue_id: &str) -> Vec<CallInfo> {
        self.engine.active_calls()
            .iter()
            .filter(|entry| {
                let call = entry.value();
                call.queue_id.as_ref().map(|q| q == queue_id).unwrap_or(false) &&
                matches!(call.status, crate::orchestrator::types::CallStatus::Queued)
            })
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// List all active bridges (connected calls)
    /// 
    /// Returns information about all active call bridges in the system.
    /// Bridges represent connected calls between parties and are used for
    /// call monitoring and management purposes.
    /// 
    /// # Returns
    /// 
    /// Vector of `BridgeInfo` containing bridge identification and connection details.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// let bridges = supervisor.list_active_bridges().await;
    /// 
    /// println!("🌉 Active Bridges:");
    /// for bridge in &bridges {
    ///     println!("  Bridge {}: {} participants", bridge.id, bridge.participant_count);
    ///     
    ///     for session in &bridge.sessions {
    ///         println!("    Session: {}", session);
    ///     }
    /// }
    /// 
    /// if bridges.is_empty() {
    ///     println!("  No active bridges");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_active_bridges(&self) -> Vec<rvoip_session_core::BridgeInfo> {
        self.engine.list_active_bridges().await
    }
    
    /// Force assign a queued call to a specific agent
    /// 
    /// Allows supervisors to manually route calls when automatic routing is
    /// insufficient or when special handling is required. This bypasses normal
    /// routing rules and assigns the call directly to the specified agent.
    /// 
    /// # Arguments
    /// 
    /// * `session_id` - Session identifier of the call to assign
    /// * `agent_id` - Identifier of the agent to receive the call
    /// 
    /// # Returns
    /// 
    /// `Ok(())` if assignment successful, or error if assignment fails.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_call_engine::agent::AgentId;
    /// use rvoip_session_core::SessionId;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// // Find a high-priority call that needs immediate attention
    /// let queued_calls = supervisor.get_queued_calls("vip").await;
    /// 
    /// if let Some(urgent_call) = queued_calls.first() {
    ///     // Find an available specialist agent
    ///     let agents = supervisor.list_agents().await;
    ///     let specialist = agents.iter()
    ///         .find(|agent| agent.skills.contains(&"vip".to_string()) && 
    ///                      agent.current_calls == 0);
    ///     
    ///     if let Some(agent) = specialist {
            ///         // Force assign the urgent call
        ///         supervisor.force_assign_call(
        ///             urgent_call.session_id.clone(),
        ///             agent.agent_id.clone()
        ///         ).await?;
    ///         
    ///         println!("✅ Urgent call assigned to specialist: {}", agent.agent_id.0);
    ///     } else {
    ///         println!("⚠️ No specialist agents available");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn force_assign_call(
        &self, 
        session_id: SessionId, 
        agent_id: AgentId
    ) -> CallCenterResult<()> {
        self.engine.assign_agent_to_call(session_id, agent_id).await
    }
    
    /// Get performance metrics for a specific time period
    /// 
    /// Returns comprehensive performance analytics for the specified time range,
    /// including call volumes, service levels, and timing metrics. This data
    /// is essential for performance monitoring and reporting.
    /// 
    /// # Arguments
    /// 
    /// * `start_time` - Beginning of the analysis period
    /// * `end_time` - End of the analysis period
    /// 
    /// # Returns
    /// 
    /// `PerformanceMetrics` containing comprehensive call center performance data.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use chrono::{Utc, Duration};
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// // Get metrics for the last 4 hours
    /// let end_time = Utc::now();
    /// let start_time = end_time - Duration::hours(4);
    /// 
    /// let metrics = supervisor.get_performance_metrics(start_time, end_time).await;
    /// 
    /// println!("📊 Performance Report ({} to {})", 
    ///          start_time.format("%H:%M"), end_time.format("%H:%M"));
    /// println!("┌─────────────────────────────────────────┐");
    /// println!("│ Total Calls: {:>6} | Answered: {:>6}   │", 
    ///          metrics.total_calls, metrics.calls_answered);
    /// println!("│ Queued: {:>6} | Abandoned: {:>6}       │", 
    ///          metrics.calls_queued, metrics.calls_abandoned);
    /// println!("│ Avg Wait: {:>4}s | Avg Handle: {:>4}s   │", 
    ///          metrics.average_wait_time_ms / 1000,
    ///          metrics.average_handle_time_ms / 1000);
    /// println!("│ Service Level: {:>6.1}%              │", 
    ///          metrics.service_level_percentage);
    /// println!("└─────────────────────────────────────────┘");
    /// 
    /// // Performance alerts
    /// if metrics.service_level_percentage < 80.0 {
    ///     println!("🚨 Service level below target!");
    /// }
    /// 
    /// if metrics.average_wait_time_ms > 120000 { // 2 minutes
    ///     println!("⏰ Average wait time exceeds target!");
    /// }
    /// 
    /// let answer_rate = metrics.calls_answered as f32 / metrics.total_calls as f32 * 100.0;
    /// if answer_rate < 90.0 {
    ///     println!("📞 Answer rate below target: {:.1}%", answer_rate);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_performance_metrics(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> PerformanceMetrics {
        let stats = self.engine.routing_stats().read().await;
        
        // In a real implementation, this would query historical data
        PerformanceMetrics {
            total_calls: (stats.calls_routed_directly + stats.calls_queued) as usize,
            calls_answered: stats.calls_routed_directly as usize,
            calls_queued: stats.calls_queued as usize,
            calls_abandoned: stats.calls_rejected as usize,
            average_wait_time_ms: stats.average_routing_time_ms,
            average_handle_time_ms: 180000, // 3 minutes placeholder
            service_level_percentage: 85.0, // Placeholder
            start_time,
            end_time,
        }
    }
    
    /// Listen to a live call (supervisor monitoring)
    /// 
    /// Enables supervisors to monitor live calls for quality assurance and
    /// coaching purposes. Returns the bridge ID that can be used to join
    /// the call in listen-only mode.
    /// 
    /// # Arguments
    /// 
    /// * `session_id` - Session identifier of the call to monitor
    /// 
    /// # Returns
    /// 
    /// `Ok(Some(BridgeId))` if call can be monitored, `Ok(None)` if call
    /// not found or not bridged, or error if monitoring fails.
    /// 
    /// # Note
    /// 
    /// Actual implementation requires additional session-core support
    /// for listen-only mode participation in bridges.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_session_core::SessionId;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// 
    /// // Monitor a specific call for quality assurance
    /// let active_calls = supervisor.list_active_calls().await;
    /// 
    /// for call in active_calls {
    ///     // Focus on longer calls for quality monitoring
///     if call.duration_seconds > 300 { // 5+ minutes
    ///         match supervisor.listen_to_call(&call.session_id).await? {
    ///             Some(bridge_id) => {
    ///                 println!("🎧 Monitoring call {} on bridge {}", 
    ///                          call.session_id, bridge_id);
    ///                 
    ///                 if let Some(agent_id) = &call.agent_id {
    ///                     println!("   Agent: {}", agent_id);
    ///                 }
    ///                 
    ///                 // In practice, would establish listen-only connection
    ///                 break;
    ///             }
    ///             None => {
    ///                 println!("❌ Cannot monitor call {} - not bridged", call.session_id);
    ///             }
    ///         }
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn listen_to_call(&self, session_id: &SessionId) -> CallCenterResult<Option<BridgeId>> {
        Ok(self.engine.active_calls()
            .get(session_id)
            .and_then(|entry| entry.bridge_id.clone()))
    }
    
    /// Send a message to an agent during a call (coaching)
    /// 
    /// Enables supervisors to send coaching messages to agents during active
    /// calls. This supports real-time coaching and guidance for improved
    /// call handling.
    /// 
    /// # Arguments
    /// 
    /// * `agent_id` - Identifier of the agent to coach
    /// * `message` - Coaching message to send to the agent
    /// 
    /// # Returns
    /// 
    /// `Ok(())` if message sent successfully, or error if coaching fails.
    /// 
    /// # Note
    /// 
    /// This is a placeholder implementation. Actual coaching functionality
    /// requires whisper/coaching support in the media layer.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::SupervisorApi;
    /// use rvoip_call_engine::agent::AgentId;
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let supervisor = SupervisorApi::new(engine);
    /// let agent_id = AgentId::from("agent-001");
    /// 
    /// // Monitor agent performance and provide coaching
    /// let agent_calls = supervisor.monitor_agent_calls(&agent_id).await;
    /// 
    /// for call in agent_calls {
///     let duration = call.duration_seconds;
///     
///     // Provide coaching based on call duration
    ///     if duration > 480 { // 8+ minutes
    ///         supervisor.coach_agent(
    ///             &agent_id,
    ///             "Call is running long - consider summarizing and wrapping up"
    ///         ).await?;
    ///         
    ///         println!("💬 Coaching sent: Call wrap-up guidance");
    ///     } else if duration > 240 { // 4+ minutes
    ///         supervisor.coach_agent(
    ///             &agent_id,
    ///             "Midpoint check - ensure you're addressing the customer's main concern"
    ///         ).await?;
    ///         
    ///         println!("💬 Coaching sent: Midpoint guidance");
    ///     }
    /// }
    /// 
    /// // Proactive coaching for new agents
    /// if let Some(agent) = supervisor.get_agent_details(&agent_id).await {
    ///     if agent.performance_score < 0.7 {
    ///         supervisor.coach_agent(
    ///             &agent_id,
    ///             "Remember to use active listening and confirm customer understanding"
    ///         ).await?;
    ///         
    ///         println!("💬 Coaching sent: Performance improvement guidance");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn coach_agent(
        &self,
        agent_id: &AgentId,
        message: &str,
    ) -> CallCenterResult<()> {
        tracing::info!("Supervisor coaching message to {}: {}", agent_id, message);
        // TODO: Implement actual whisper/coaching functionality
        Ok(())
    }
}

/// Performance metrics for a specified time period
/// 
/// Comprehensive call center performance data including call volumes,
/// service levels, timing metrics, and quality indicators. This structure
/// provides the foundation for performance reporting and analysis.
/// 
/// ## Metrics Included
/// 
/// ### Volume Metrics
/// - **Total Calls**: All inbound calls received
/// - **Calls Answered**: Calls successfully connected to agents
/// - **Calls Queued**: Calls that entered queue systems
/// - **Calls Abandoned**: Calls disconnected before being answered
/// 
/// ### Timing Metrics
/// - **Average Wait Time**: Mean time customers wait in queues
/// - **Average Handle Time**: Mean time agents spend on calls
/// - **Service Level**: Percentage of calls answered within target time
/// 
/// ## Examples
/// 
/// ### Performance Analysis
/// 
/// ```rust
/// use rvoip_call_engine::api::supervisor::{SupervisorApi, PerformanceMetrics};
/// use chrono::{Utc, Duration};
/// # use rvoip_call_engine::CallCenterEngine;
/// # use std::sync::Arc;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = CallCenterEngine::new(Default::default(), None).await?;
/// let supervisor = SupervisorApi::new(engine);
/// 
/// let end_time = Utc::now();
/// let start_time = end_time - Duration::hours(1);
/// 
/// let metrics = supervisor.get_performance_metrics(start_time, end_time).await;
/// 
/// // Calculate derived metrics
/// let answer_rate = if metrics.total_calls > 0 {
///     metrics.calls_answered as f32 / metrics.total_calls as f32 * 100.0
/// } else {
///     0.0
/// };
/// 
/// let abandon_rate = if metrics.total_calls > 0 {
///     metrics.calls_abandoned as f32 / metrics.total_calls as f32 * 100.0
/// } else {
///     0.0
/// };
/// 
/// println!("📊 Performance Analysis:");
/// println!("  Answer Rate: {:.1}%", answer_rate);
/// println!("  Abandon Rate: {:.1}%", abandon_rate);
/// println!("  Service Level: {:.1}%", metrics.service_level_percentage);
/// 
/// // Performance targets
/// if answer_rate < 90.0 {
///     println!("🚨 Answer rate below 90% target");
/// }
/// if abandon_rate > 5.0 {
///     println!("⚠️ Abandon rate above 5% target");
/// }
/// if metrics.service_level_percentage < 80.0 {
///     println!("📉 Service level below 80% target");
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// Total number of calls received in the period
    pub total_calls: usize,
    
    /// Number of calls successfully answered by agents
    pub calls_answered: usize,
    
    /// Number of calls that entered queue systems
    pub calls_queued: usize,
    
    /// Number of calls abandoned before being answered
    pub calls_abandoned: usize,
    
    /// Average wait time in queues (milliseconds)
    pub average_wait_time_ms: u64,
    
    /// Average call handling time (milliseconds)
    pub average_handle_time_ms: u64,
    
    /// Service level percentage (calls answered within target time)
    pub service_level_percentage: f32,
    
    /// Start of the measurement period
    pub start_time: DateTime<Utc>,
    
    /// End of the measurement period
    pub end_time: DateTime<Utc>,
}

impl Default for SupervisorApi {
    fn default() -> Self {
        panic!("SupervisorApi requires an engine instance")
    }
} 