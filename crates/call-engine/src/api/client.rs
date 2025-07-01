//! # Call Center Client API for Agent Applications
//!
//! This module provides a high-level, agent-focused API for building call center
//! applications. It offers a simplified interface that abstracts the complexity of
//! the underlying call center infrastructure while providing comprehensive
//! functionality for agent interactions.
//!
//! ## Overview
//!
//! The Call Center Client API is designed for agent applications that need to
//! integrate with the call center system. It provides essential functionality
//! for agent registration, status management, call handling, and system
//! interaction through a clean, ergonomic interface.
//!
//! ## Key Features
//!
//! - **Agent Registration**: Simple agent onboarding and authentication
//! - **Status Management**: Real-time agent status updates and monitoring
//! - **Call Handling**: Integration with session management for call processing
//! - **Queue Monitoring**: Access to queue statistics and performance data
//! - **Session Integration**: Direct access to session-core functionality
//! - **Event Handling**: Comprehensive call and system event management
//!
//! ## Agent Workflow
//!
//! ### Registration Phase
//! 1. Agent application initializes client with configuration
//! 2. Agent registers with their profile and capabilities
//! 3. System assigns session and sets initial status
//! 4. Agent becomes available for call routing
//!
//! ### Active Phase
//! 1. Agent receives calls through the session system
//! 2. Status updates reflect current activity (available, busy)
//! 3. Agent can monitor queue status and performance
//! 4. Call events are handled through the session framework
//!
//! ### Status Management
//! 1. Agent can update status (available, busy, offline)
//! 2. System tracks agent availability and performance
//! 3. Automatic status transitions based on call activity
//! 4. Manual status overrides when needed
//!
//! ## Integration Patterns
//!
//! ### Desktop Applications
//! - Native desktop applications (Electron, Tauri, Qt)
//! - Direct integration with system audio and telephony
//! - Full-featured agent interfaces with advanced controls
//! - Integration with CRM and business applications
//!
//! ### Web Applications
//! - Browser-based agent dashboards
//! - WebRTC integration for audio handling
//! - Real-time updates via WebSocket connections
//! - Cross-platform compatibility
//!
//! ### Mobile Applications
//! - Mobile agent applications for remote work
//! - Push notifications for incoming calls
//! - Location-aware routing and status
//! - Battery-optimized operation
//!
//! ## Examples
//!
//! ### Basic Agent Registration
//!
//! ```rust
//! use rvoip_call_engine::api::CallCenterClient;
//! use rvoip_call_engine::agent::{Agent, AgentStatus};
//! use rvoip_call_engine::config::CallCenterConfig;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize client with configuration
//! let config = CallCenterConfig::default();
//! let client = CallCenterClient::builder()
//!     .with_config(config)
//!     .with_database_path("call_center.db".to_string())
//!     .build()
//!     .await?;
//! 
//! // Create agent profile
//! let agent = Agent {
//!     id: "agent-001".to_string(),
//!     sip_uri: "sip:alice@call-center.com".to_string(),
//!     display_name: "Alice Smith".to_string(),
//!     skills: vec!["sales".to_string(), "english".to_string()],
//!     max_concurrent_calls: 2,
//!     status: AgentStatus::Available,
//!     department: Some("Sales".to_string()),
//!     extension: Some("101".to_string()),
//! };
//! 
//! // Register with call center
//! let session_id = client.register_agent(&agent).await?;
//! println!("✅ Agent registered with session: {}", session_id);
//! 
//! // Agent is now ready to receive calls
//! println!("🟢 Agent {} is now available for calls", agent.display_name);
//! # Ok(())
//! # }
//! ```
//!
//! ### Status Management Workflow
//!
//! ```rust
//! use rvoip_call_engine::api::CallCenterClient;
//! use rvoip_call_engine::agent::{AgentId, AgentStatus};
//! # use rvoip_call_engine::config::CallCenterConfig;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let client = CallCenterClient::builder()
//! #     .with_config(CallCenterConfig::default())
//! #     .build().await?;
//! 
//! let agent_id = AgentId::from("agent-001");
//! 
//! // Agent starts shift - mark as available
//! client.update_agent_status(&agent_id, AgentStatus::Available).await?;
//! println!("🟢 Agent available for calls");
//! 
//! // During lunch break - temporarily offline
//! client.update_agent_status(&agent_id, AgentStatus::Offline).await?;
//! println!("🔴 Agent offline for break");
//! 
//! // Return from break - available again
//! client.update_agent_status(&agent_id, AgentStatus::Available).await?;
//! println!("🟢 Agent back and available");
//! 
//! // End of shift - permanently offline
//! client.update_agent_status(&agent_id, AgentStatus::Offline).await?;
//! println!("👋 Agent signed off for the day");
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Dashboard Integration
//!
//! ```rust
//! use rvoip_call_engine::api::CallCenterClient;
//! use rvoip_call_engine::agent::AgentId;
//! use tokio::time::{interval, Duration};
//! # use rvoip_call_engine::config::CallCenterConfig;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let client = CallCenterClient::builder()
//! #     .with_config(CallCenterConfig::default())
//! #     .build().await?;
//! 
//! let agent_id = AgentId::from("agent-001");
//! 
//! // Dashboard update loop
//! let mut interval = interval(Duration::from_secs(30));
//! 
//! loop {
//!     interval.tick().await;
//!     
//!     // Get current agent information
//!     if let Some(agent_info) = client.get_agent_info(&agent_id).await {
//!         println!("📊 Agent Dashboard Update:");
//!         println!("  Status: {:?}", agent_info.status);
//!         println!("  Active calls: {}", agent_info.current_calls);
//!         println!("  Performance: {:.1}%", agent_info.performance_score * 100.0);
//!         
//!         // Alert on performance issues
//!         if agent_info.performance_score < 0.7 {
//!             println!("⚠️ Performance below threshold - coaching recommended");
//!         }
//!     }
//!     
//!     // Get queue statistics for context
//!     let queue_stats = client.get_queue_stats().await?;
//!     let total_calls: usize = queue_stats.iter()
//!         .map(|(_, stats)| stats.total_calls)
//!         .sum();
//!     
//!     println!("📋 Queue Overview:");
//!     println!("  Total queued calls: {}", total_calls);
//!     
//!     for (queue_id, stats) in queue_stats.iter().take(3) {
//!         println!("  {}: {} calls (avg wait {}s)", 
//!                  queue_id, stats.total_calls, stats.average_wait_time_seconds);
//!     }
//!     
//!     // In a real dashboard, this would break based on user interaction
//!     break;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Session Integration
//!
//! ```rust
//! use rvoip_call_engine::api::CallCenterClient;
//! use rvoip_session_core::{SessionCoordinator, CallHandler};
//! use std::sync::Arc;
//! # use rvoip_call_engine::config::CallCenterConfig;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let client = CallCenterClient::builder()
//! #     .with_config(CallCenterConfig::default())
//! #     .build().await?;
//! 
//! // Get access to underlying session management
//! let session_manager = client.session_manager();
//! 
//! // Set up custom call handler for advanced call processing
//! let call_handler = client.call_handler();
//! 
//! // Register for call events
//! // Note: In practice, you'd implement your own CallHandler
//! // session_manager.set_call_handler(call_handler).await?;
//! 
//! println!("🔧 Advanced session integration configured");
//! 
//! // Access session-core features directly when needed
//! println!("📈 Session Statistics:");
//! println!("  Session manager ready for advanced operations");
//! // Note: Use session_manager methods for specific operations as needed
//! # Ok(())
//! # }
//! ```
//!
//! ### Multi-Agent Application
//!
//! ```rust
//! use rvoip_call_engine::api::CallCenterClient;
//! use rvoip_call_engine::agent::{Agent, AgentId, AgentStatus};
//! use std::collections::HashMap;
//! # use rvoip_call_engine::config::CallCenterConfig;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let client = CallCenterClient::builder()
//! #     .with_config(CallCenterConfig::default())
//! #     .build().await?;
//! 
//! let mut managed_agents = HashMap::new();
//! 
//! // Register multiple agents
//! let agent_configs = vec![
//!     ("agent-001", "Alice Smith", "Sales", vec!["sales", "english"]),
//!     ("agent-002", "Bob Jones", "Support", vec!["support", "technical"]),
//!     ("agent-003", "Carol Garcia", "Billing", vec!["billing", "spanish"]),
//! ];
//! 
//! for (id, name, dept, skills) in agent_configs {
//!     let agent = Agent {
//!         id: id.to_string(),
//!         sip_uri: format!("sip:{}@call-center.com", id.replace("-", "")),
//!         display_name: name.to_string(),
//!         skills: skills.iter().map(|s| s.to_string()).collect(),
//!         max_concurrent_calls: 2,
//!         status: AgentStatus::Available,
//!         department: Some(dept.to_string()),
//!         extension: None,
//!     };
//!     
//!     let session_id = client.register_agent(&agent).await?;
//!     managed_agents.insert(id.to_string(), session_id);
//!     
//!     println!("✅ Registered agent: {} ({})", name, dept);
//! }
//! 
//! println!("🎯 Multi-agent application ready with {} agents", managed_agents.len());
//! 
//! // Monitor all agents
//! for (agent_id, _session_id) in &managed_agents {
//!     let agent_id = AgentId::from(agent_id.clone());
//!     
//!     if let Some(info) = client.get_agent_info(&agent_id).await {
//!         let status_icon = match info.status {
//!             _ if info.current_calls > 0 => "📞",
//!             _ => "🟢",
//!         };
//!         
//!         println!("  {} {} - {} active calls", 
//!                  status_icon, info.agent_id.0, info.current_calls);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Performance Monitoring Integration
//!
//! ```rust
//! use rvoip_call_engine::api::CallCenterClient;
//! use rvoip_call_engine::agent::AgentId;
//! use std::time::Duration;
//! # use rvoip_call_engine::config::CallCenterConfig;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let client = CallCenterClient::builder()
//! #     .with_config(CallCenterConfig::default())
//! #     .build().await?;
//! 
//! let agent_id = AgentId::from("agent-001");
//! 
//! // Performance monitoring dashboard
//! async fn update_performance_dashboard(
//!     client: &CallCenterClient,
//!     agent_id: &AgentId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Get agent performance data
//!     if let Some(agent_info) = client.get_agent_info(agent_id).await {
//!         println!("🎯 Performance Dashboard:");
//!         println!("┌─────────────────────────────────────┐");
//!         println!("│ Agent: {:>28} │", agent_info.agent_id.0);
//!         println!("│ Status: {:>27} │", format!("{:?}", agent_info.status));
//!         println!("│ Active Calls: {:>19} │", agent_info.current_calls);
//!         println!("│ Performance: {:>16.1}% │", agent_info.performance_score * 100.0);
//!         println!("│ Skills: {:>25} │", agent_info.skills.join(", "));
//!         println!("└─────────────────────────────────────┘");
//!         
//!         // Performance recommendations
//!         if agent_info.performance_score < 0.6 {
//!             println!("🚨 Critical: Performance requires immediate attention");
//!         } else if agent_info.performance_score < 0.8 {
//!             println!("⚠️ Warning: Performance below target - coaching recommended");
//!         } else {
//!             println!("✅ Good: Performance meets expectations");
//!         }
//!     }
//!     
//!     // Get system context
//!     let queue_stats = client.get_queue_stats().await?;
//!     let high_volume_queues: Vec<_> = queue_stats.iter()
//!         .filter(|(_, stats)| stats.total_calls > 5)
//!         .collect();
//!     
//!     if !high_volume_queues.is_empty() {
//!         println!("📈 High Volume Queues:");
//!         for (queue_id, stats) in high_volume_queues {
//!             println!("  {} - {} calls in queue", queue_id, stats.total_calls);
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! 
//! // Update dashboard
//! update_performance_dashboard(&client, &agent_id).await?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use async_trait::async_trait;
use rvoip_session_core::{SessionId, CallHandler};

use crate::{
    agent::{Agent, AgentId, AgentStatus},
    error::{CallCenterError, Result as CallCenterResult},
    orchestrator::CallCenterEngine,
};

/// # Call Center Client for Agent Applications
/// 
/// The `CallCenterClient` provides a simplified, high-level interface for agent
/// applications to interact with the call center system. It abstracts complex
/// orchestration details and provides essential functionality through an ergonomic API.
/// 
/// ## Core Capabilities
/// 
/// ### Agent Management
/// - **Registration**: Register agent profiles with the call center
/// - **Status Updates**: Update agent availability and activity status
/// - **Information Retrieval**: Get current agent status and performance data
/// 
/// ### System Integration
/// - **Queue Monitoring**: Access queue statistics and performance metrics
/// - **Session Access**: Direct access to underlying session management
/// - **Event Handling**: Integration with call and system event processing
/// 
/// ## Architecture
/// 
/// The client maintains a connection to the underlying `CallCenterEngine` which
/// handles all orchestration, routing, and coordination. This design allows
/// agent applications to focus on user interface and business logic while
/// leaving complex call center operations to the engine.
/// 
/// ## Thread Safety
/// 
/// The `CallCenterClient` is thread-safe and can be cloned for use across
/// multiple components or tasks. All operations are asynchronous and designed
/// for concurrent access.
/// 
/// ## Examples
/// 
/// ### Basic Client Usage
/// 
/// ```rust
/// use rvoip_call_engine::api::CallCenterClient;
/// use rvoip_call_engine::agent::{Agent, AgentStatus};
/// use rvoip_call_engine::config::CallCenterConfig;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create and configure client
/// let client = CallCenterClient::builder()
///     .with_config(CallCenterConfig::default())
///     .build()
///     .await?;
/// 
/// // Register agent
/// let agent = Agent {
///     id: "agent-001".to_string(),
///     sip_uri: "sip:agent@call-center.com".to_string(),
///     display_name: "Agent Smith".to_string(),
///     skills: vec!["general".to_string()],
///     max_concurrent_calls: 1,
///     status: AgentStatus::Available,
///     department: None,
///     extension: None,
/// };
/// 
/// let session_id = client.register_agent(&agent).await?;
/// println!("Agent registered with session: {}", session_id);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct CallCenterClient {
    engine: Arc<CallCenterEngine>,
}

impl CallCenterClient {
    /// Create a new call center client connected to the given engine
    /// 
    /// Initializes a new client instance that connects to the specified
    /// call center engine. The client provides a simplified interface
    /// for agent applications while leveraging the full capabilities
    /// of the underlying engine.
    /// 
    /// # Arguments
    /// 
    /// * `engine` - Shared reference to the call center engine
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::CallCenterEngine;
    /// use rvoip_call_engine::config::CallCenterConfig;
    /// use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let engine = CallCenterEngine::new(
///     CallCenterConfig::default(),
///     None
/// ).await?;
/// 
/// let client = CallCenterClient::new(engine);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self { engine }
    }
    
    /// Register an agent with the call center
    /// 
    /// Registers a new agent profile with the call center system, establishing
    /// a session and making the agent available for call routing. This is the
    /// primary method for agent onboarding and authentication.
    /// 
    /// # Arguments
    /// 
    /// * `agent` - Complete agent profile including identification, skills, and status
    /// 
    /// # Returns
    /// 
    /// `Ok(SessionId)` with the assigned session identifier if registration
    /// succeeds, or error if registration fails.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::agent::{Agent, AgentStatus};
    /// # use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = CallCenterClient::builder()
    /// #     .with_config(CallCenterConfig::default())
    /// #     .build().await?;
    /// 
    /// let agent = Agent {
    ///     id: "agent001".to_string(),
    ///     sip_uri: "sip:agent001@example.com".to_string(),
    ///     display_name: "Agent 001".to_string(),
    ///     skills: vec!["sales".to_string(), "support".to_string()],
    ///     max_concurrent_calls: 3,
    ///     status: AgentStatus::Available,
    ///     department: Some("sales".to_string()),
    ///     extension: Some("001".to_string()),
    /// };
    /// 
    /// let session_id = client.register_agent(&agent).await?;
    /// println!("Agent registered with session: {}", session_id);
    /// 
    /// // Agent is now ready to receive calls
    /// println!("🟢 {} is available for calls", agent.display_name);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_agent(&self, agent: &Agent) -> CallCenterResult<SessionId> {
        self.engine.register_agent(agent).await
    }
    
    /// Update agent status
    /// 
    /// Changes the operational status of an agent, affecting their availability
    /// for call routing. This method is used to reflect changes in agent
    /// availability, breaks, or shift changes.
    /// 
    /// # Arguments
    /// 
    /// * `agent_id` - Identifier of the agent to update
    /// * `status` - New status for the agent
    /// 
    /// # Returns
    /// 
    /// `Ok(())` if status update succeeds, or error if update fails.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::agent::{AgentId, AgentStatus};
    /// # use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = CallCenterClient::builder()
    /// #     .with_config(CallCenterConfig::default())
    /// #     .build().await?;
    /// 
    /// let agent_id = AgentId::from("agent001");
    /// 
    /// // Mark agent as offline for lunch break
    /// client.update_agent_status(&agent_id, AgentStatus::Offline).await?;
    /// println!("🔴 Agent offline for lunch break");
    /// 
    /// // Mark agent as available after break
    /// client.update_agent_status(&agent_id, AgentStatus::Available).await?;
    /// println!("🟢 Agent back and available for calls");
    /// 
    /// // Handle end-of-shift
    /// client.update_agent_status(&agent_id, AgentStatus::Offline).await?;
    /// println!("👋 Agent signed off for the day");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_agent_status(
        &self, 
        agent_id: &AgentId, 
        status: AgentStatus
    ) -> CallCenterResult<()> {
        self.engine.update_agent_status(agent_id, status).await
    }
    
    /// Get current agent information
    /// 
    /// Retrieves comprehensive information about an agent including their
    /// current status, active calls, performance metrics, and capabilities.
    /// This method is essential for agent monitoring and dashboard displays.
    /// 
    /// # Arguments
    /// 
    /// * `agent_id` - Identifier of the agent to retrieve
    /// 
    /// # Returns
    /// 
    /// `Some(AgentInfo)` with current agent information if found,
    /// `None` if agent doesn't exist.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::agent::AgentId;
    /// # use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = CallCenterClient::builder()
    /// #     .with_config(CallCenterConfig::default())
    /// #     .build().await?;
    /// 
    /// let agent_id = AgentId::from("agent001");
    /// 
    /// if let Some(agent_info) = client.get_agent_info(&agent_id).await {
    ///     println!("📋 Agent Information:");
    ///     println!("  Agent ID: {}", agent_info.agent_id.0);
///     println!("  Status: {:?}", agent_info.status);
///     println!("  Active calls: {}", agent_info.current_calls);
///     println!("  Performance: {:.1}%", agent_info.performance_score * 100.0);
///     println!("  Skills: {:?}", agent_info.skills);
///     
///     // Check for performance issues
///     if agent_info.performance_score < 0.7 {
///         println!("⚠️ Performance below threshold - coaching may be needed");
///     }
///     
///     // Check workload
///     if agent_info.current_calls > 2 {
///         println!("📞 High call volume - {} active calls", agent_info.current_calls);
///     }
    /// } else {
    ///     println!("❌ Agent not found or not registered");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_agent_info(&self, agent_id: &AgentId) -> Option<crate::orchestrator::types::AgentInfo> {
        self.engine.get_agent_info(agent_id).await
    }
    
    /// Get current queue statistics
    /// 
    /// Returns comprehensive statistics for all queues that the agent has
    /// visibility into. This provides essential context for agent applications
    /// to understand system load and performance.
    /// 
    /// # Returns
    /// 
    /// `Ok(Vec<(String, QueueStats)>)` with queue name and statistics pairs,
    /// or error if statistics retrieval fails.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// # use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = CallCenterClient::builder()
    /// #     .with_config(CallCenterConfig::default())
    /// #     .build().await?;
    /// 
    /// let queue_stats = client.get_queue_stats().await?;
    /// 
    /// println!("📊 Queue Statistics:");
    /// for (queue_id, stats) in &queue_stats {
    ///     println!("  Queue: {}", queue_id);
    ///     println!("    Total calls: {}", stats.total_calls);
///     println!("    Avg wait: {}s", stats.average_wait_time_seconds);
///     println!("    Longest wait: {}s", stats.longest_wait_time_seconds);
///     
///     // Alert on concerning conditions
///     if stats.total_calls > 10 {
///         println!("    🚨 High queue volume!");
///     }
///     
///     if stats.longest_wait_time_seconds > 300 {
///         println!("    ⏰ Long wait times detected!");
///     }
///     
///     // Show utilization info
///     println!("    Queue utilization: {} calls", stats.total_calls);
/// }
/// 
/// // Summary statistics
/// let total_calls: usize = queue_stats.iter()
///     .map(|(_, stats)| stats.total_calls)
///     .sum();
    /// 
    /// println!("\n📈 System Summary:");
/// println!("  Total queued calls: {}", total_calls);
/// println!("  Active queues: {}", queue_stats.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_queue_stats(&self) -> CallCenterResult<Vec<(String, crate::queue::QueueStats)>> {
        self.engine.get_queue_stats().await
    }
    
    /// Get the underlying session manager for advanced operations
    /// 
    /// Provides direct access to the session-core functionality for applications
    /// that need to perform advanced session management operations beyond the
    /// simplified client API.
    /// 
    /// # Returns
    /// 
    /// Reference to the underlying `SessionCoordinator` instance.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// # use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = CallCenterClient::builder()
    /// #     .with_config(CallCenterConfig::default())
    /// #     .build().await?;
    /// 
    /// // Access session manager for advanced operations
    /// let session_manager = client.session_manager();
    /// 
    /// // Access advanced session manager functionality
/// println!("🔧 Advanced Session Information:");
/// println!("  Session manager available for advanced operations");
/// // Note: Use session_manager methods like create_session, bridge_sessions, etc.
    /// 
    /// // Access session-core capabilities directly
    /// // let bridges = session_manager.list_bridges().await?;
    /// // let session_details = session_manager.get_session_details(&session_id).await?;
    /// 
    /// println!("🚀 Advanced session capabilities available");
    /// # Ok(())
    /// # }
    /// ```
    pub fn session_manager(&self) -> &Arc<rvoip_session_core::SessionCoordinator> {
        self.engine.session_manager()
    }
    
    /// Get the call handler for this client
    /// 
    /// Returns a call handler that can be used for advanced call event processing
    /// and integration with the session framework. This enables sophisticated
    /// call handling scenarios beyond basic agent operations.
    /// 
    /// # Returns
    /// 
    /// `Arc<dyn CallHandler>` that implements call event handling for the call center.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_session_core::CallHandler;
    /// # use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = CallCenterClient::builder()
    /// #     .with_config(CallCenterConfig::default())
    /// #     .build().await?;
    /// 
    /// // Get call handler for advanced call processing
    /// let call_handler = client.call_handler();
    /// 
    /// println!("📞 Call handler available for advanced integration");
    /// 
    /// // In practice, you might:
    /// // - Register this handler with the session manager
    /// // - Use it for custom call event processing
    /// // - Integrate with external systems or workflows
    /// 
    /// // Example of potential usage:
    /// // session_manager.register_call_handler(call_handler).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn call_handler(&self) -> Arc<dyn CallHandler> {
        Arc::new(crate::orchestrator::handler::CallCenterCallHandler {
            engine: Arc::downgrade(&self.engine),
        })
    }
    
    /// Create a builder for constructing a CallCenterClient
    /// 
    /// Returns a `CallCenterClientBuilder` that provides a fluent interface
    /// for configuring and creating a client instance. This is the recommended
    /// way to create clients with custom configuration.
    /// 
    /// # Returns
    /// 
    /// `CallCenterClientBuilder` instance ready for configuration.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = CallCenterClient::builder()
    ///     .with_config(CallCenterConfig::default())
    ///     .with_database_path("call_center.db".to_string())
    ///     .build()
    ///     .await?;
    /// 
    /// println!("✅ Call center client ready");
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> CallCenterClientBuilder {
        CallCenterClientBuilder::new()
    }
}

/// Builder for creating a CallCenterClient
/// 
/// The `CallCenterClientBuilder` provides a fluent interface for configuring and
/// constructing `CallCenterClient` instances. This builder pattern allows for
/// clean configuration of various client options before creating the final client.
/// 
/// ## Configuration Options
/// 
/// - **Configuration**: Core call center configuration including routing rules
/// - **Database Path**: Optional database path for persistent storage
/// - **Custom Settings**: Additional customization options
/// 
/// ## Examples
/// 
/// ### Basic Client Creation
/// 
/// ```rust
/// use rvoip_call_engine::api::CallCenterClient;
/// use rvoip_call_engine::config::CallCenterConfig;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = CallCenterClient::builder()
///     .with_config(CallCenterConfig::default())
///     .build()
///     .await?;
/// 
/// println!("✅ Client created with default configuration");
/// # Ok(())
/// # }
/// ```
/// 
/// ### Advanced Configuration
/// 
/// ```rust
/// use rvoip_call_engine::api::CallCenterClient;
/// use rvoip_call_engine::config::{CallCenterConfig, QueueConfig, RoutingConfig};
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut config = CallCenterConfig::default();
/// config.routing.enable_load_balancing = true; // Enable load balancing
/// 
/// let client = CallCenterClient::builder()
///     .with_config(config)
///     .with_database_path("production_call_center.db".to_string())
///     .build()
///     .await?;
/// 
/// println!("🚀 Production client ready with custom configuration");
/// # Ok(())
/// # }
/// ```
pub struct CallCenterClientBuilder {
    config: Option<crate::config::CallCenterConfig>,
    db_path: Option<String>,
}

impl CallCenterClientBuilder {
    /// Create a new builder
    /// 
    /// Initializes a new builder instance with no configuration set.
    /// Configuration must be provided before building the client.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::client::CallCenterClientBuilder;
    /// 
    /// let builder = CallCenterClientBuilder::new();
    /// // Further configuration needed before building
    /// ```
    pub fn new() -> Self {
        Self {
            config: None,
            db_path: None,
        }
    }
    
    /// Set the configuration
    /// 
    /// Provides the core call center configuration that defines system behavior,
    /// routing rules, queue settings, and other operational parameters.
    /// 
    /// # Arguments
    /// 
    /// * `config` - Complete call center configuration
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut config = CallCenterConfig::default();
    /// 
    /// // Customize configuration
/// config.routing.enable_time_based_routing = true; // Enable time-based routing
    /// 
    /// let client = CallCenterClient::builder()
    ///     .with_config(config)
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_config(mut self, config: crate::config::CallCenterConfig) -> Self {
        self.config = Some(config);
        self
    }
    
    /// Set the database path
    /// 
    /// Configures the path for persistent database storage. If not provided,
    /// the system will operate with in-memory storage only.
    /// 
    /// # Arguments
    /// 
    /// * `path` - File system path for the database file
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = CallCenterClient::builder()
    ///     .with_config(CallCenterConfig::default())
    ///     .with_database_path("call_center_data.db".to_string())
    ///     .build()
    ///     .await?;
    /// 
    /// println!("💾 Client with persistent database storage");
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_database_path(mut self, path: String) -> Self {
        self.db_path = Some(path);
        self
    }
    
    /// Build the client
    /// 
    /// Constructs the final `CallCenterClient` instance using the provided
    /// configuration. This method initializes the underlying call center
    /// engine and all necessary components.
    /// 
    /// # Returns
    /// 
    /// `Ok(CallCenterClient)` if construction succeeds, or error if
    /// configuration is invalid or initialization fails.
    /// 
    /// # Errors
    /// 
    /// - Configuration not provided
    /// - Database initialization failure
    /// - Engine startup failure
    /// - Network or resource initialization errors
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::CallCenterClient;
    /// use rvoip_call_engine::config::CallCenterConfig;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = CallCenterClient::builder()
    ///     .with_config(CallCenterConfig::default())
    ///     .build()
    ///     .await?;
    /// 
    /// println!("🎯 Call center client successfully created");
    /// 
    /// // Client is now ready for agent registration and operations
    /// # Ok(())
    /// # }
    /// ```
    pub async fn build(self) -> CallCenterResult<CallCenterClient> {
        let config = self.config
            .ok_or_else(|| CallCenterError::configuration("Configuration required"))?;
            
        let engine = CallCenterEngine::new(config, self.db_path).await?;
        Ok(CallCenterClient::new(engine))
    }
}

impl Default for CallCenterClientBuilder {
    fn default() -> Self {
        Self::new()
    }
} 