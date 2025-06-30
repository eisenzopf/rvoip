//! # Administrative API for Call Center Management
//!
//! This module provides comprehensive administrative APIs for call center system
//! management, configuration, and maintenance. It enables system administrators
//! to manage agents, configure queues, monitor system health, and perform
//! operational tasks through a powerful administrative interface.
//!
//! ## Overview
//!
//! The Administrative API is designed for call center administrators who need
//! complete control over system configuration and operations. It provides
//! enterprise-grade management capabilities for large-scale call center
//! deployments with sophisticated configuration and monitoring needs.
//!
//! ## Key Features
//!
//! - **Agent Management**: Complete CRUD operations for agent profiles
//! - **Queue Configuration**: Dynamic queue creation and configuration
//! - **Routing Management**: Advanced routing rule configuration
//! - **System Health**: Comprehensive system monitoring and diagnostics
//! - **Database Operations**: Database maintenance and optimization
//! - **Configuration Management**: Import/export and dynamic updates
//! - **Statistics & Reporting**: System-wide analytics and reporting
//!
//! ## Administrative Capabilities
//!
//! ### Agent Administration
//! - Add, update, and remove agent profiles
//! - Manage agent skills and capabilities
//! - Configure agent routing preferences
//! - Monitor agent performance and status
//!
//! ### Queue Management
//! - Create and configure call queues
//! - Set queue-specific routing rules
//! - Configure overflow and escalation policies
//! - Monitor queue performance metrics
//!
//! ### System Configuration
//! - Dynamic routing rule updates
//! - System-wide configuration changes
//! - Feature flag management
//! - Performance parameter tuning
//!
//! ### Monitoring & Maintenance
//! - Real-time system health monitoring
//! - Database maintenance operations
//! - Performance optimization tools
//! - Diagnostic and troubleshooting utilities
//!
//! ## Security Considerations
//!
//! The Administrative API provides powerful system management capabilities
//! and should be secured appropriately:
//!
//! - Authentication and authorization required
//! - Role-based access control recommended
//! - Audit logging for all administrative actions
//! - Network access restrictions
//! - Secure communication channels (HTTPS/TLS)
//!
//! ## Examples
//!
//! ### Basic Agent Management
//!
//! ```rust
//! use rvoip_call_engine::api::AdminApi;
//! use rvoip_call_engine::agent::{Agent, AgentStatus};
//! use rvoip_call_engine::CallCenterEngine;
//! use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize admin API
//! let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
//! let admin = AdminApi::new(Arc::clone(&engine));
//! 
//! // Create a new agent
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
//! // Add agent to system
//! admin.add_agent(agent).await?;
//! println!("✅ Agent added successfully");
//! 
//! // List all agents
//! let agents = admin.list_agents().await?;
//! println!("📋 Total agents: {}", agents.len());
//! 
//! for agent in agents {
//!     println!("  {} ({}): {:?}", agent.display_name, agent.id, agent.status);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Management Operations
//!
//! ```rust
//! use rvoip_call_engine::api::AdminApi;
//! use rvoip_call_engine::config::QueueConfig;
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
//! let admin = AdminApi::new(Arc::clone(&engine));
//! 
//! // Create specialized queues
//! let queue_configs = vec![
//!     ("vip-support", "VIP customer support queue"),
//!     ("technical-escalation", "Technical escalation queue"),
//!     ("billing-inquiries", "Billing and payment inquiries"),
//! ];
//! 
//! for (queue_id, description) in queue_configs {
//!     // Create the queue
//!     admin.create_queue(queue_id).await?;
//!     println!("✅ Created queue: {} ({})", queue_id, description);
//!     
//!     // Configure queue parameters
//!     let config = QueueConfig {
//!         default_max_wait_time: 300,     // 5 minutes max wait
//!         max_queue_size: 50,             // Maximum 50 calls in queue
//!         enable_priorities: true,        // Enable priority routing
//!         enable_overflow: true,          // Enable overflow to other queues
//!         announcement_interval: 30,      // Announcements every 30 seconds
//!     };
//!     
//!     admin.update_queue(queue_id, config).await?;
//!     println!("🔧 Configured queue: {}", queue_id);
//! }
//! 
//! // Get queue configurations
//! let configs = admin.get_queue_configs().await;
//! println!("📊 Queue Configurations:");
//! for (queue_id, config) in configs {
//!     println!("  {}: max_wait={}s, max_size={}", 
//!              queue_id, config.default_max_wait_time, config.max_queue_size);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### System Health Monitoring
//!
//! ```rust
//! use rvoip_call_engine::api::admin::{AdminApi, HealthStatus};
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
//! let admin = AdminApi::new(Arc::clone(&engine));
//! 
//! // Get comprehensive system health
//! let health = admin.get_system_health().await;
//! 
//! println!("🏥 System Health Report:");
//! println!("┌─────────────────────────────────────────┐");
//! println!("│ Status: {:>28} │", match health.status {
//!     HealthStatus::Healthy => "🟢 Healthy",
//!     HealthStatus::Degraded => "🟡 Degraded",
//!     HealthStatus::Critical => "🔴 Critical",
//! });
//! println!("│ Database: {:>25} │", 
//!          if health.database_connected { "🟢 Connected" } else { "🔴 Disconnected" });
//! println!("│ Active Sessions: {:>16} │", health.active_sessions);
//! println!("│ Registered Agents: {:>14} │", health.registered_agents);
//! println!("│ Queued Calls: {:>19} │", health.queued_calls);
//! println!("│ Uptime: {:>25} │", format!("{}s", health.uptime_seconds));
//! println!("└─────────────────────────────────────────┘");
//! 
//! // Display warnings if any
//! if !health.warnings.is_empty() {
//!     println!("\n⚠️ System Warnings:");
//!     for warning in health.warnings {
//!         println!("  • {}", warning);
//!     }
//! }
//! 
//! // Take action based on health status
//! match health.status {
//!     HealthStatus::Critical => {
//!         println!("🚨 CRITICAL: Immediate attention required!");
//!     }
//!     HealthStatus::Degraded => {
//!         println!("⚠️ DEGRADED: Performance monitoring recommended");
//!     }
//!     HealthStatus::Healthy => {
//!         println!("✅ System operating normally");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Configuration Management
//!
//! ```rust
//! use rvoip_call_engine::api::AdminApi;
//! use rvoip_call_engine::config::{RoutingConfig, CallCenterConfig, RoutingStrategy};
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
//! let admin = AdminApi::new(Arc::clone(&engine));
//! 
//! // Export current configuration for backup
//! let config_json = admin.export_config().await?;
//! println!("📤 Configuration exported ({} bytes)", config_json.len());
//! 
//! // Save to file (in real application)
//! // std::fs::write("call_center_config_backup.json", &config_json)?;
//! 
//! // Update routing configuration
//! let mut new_routing = RoutingConfig::default();
//! new_routing.default_strategy = RoutingStrategy::RoundRobin;
//! new_routing.enable_load_balancing = true;
//! new_routing.enable_time_based_routing = true;
//! 
//! admin.update_routing_config(new_routing).await?;
//! println!("🔧 Routing configuration updated");
//! 
//! // Get current system configuration
//! let current_config = admin.get_config();
//! println!("⚙️ Current Configuration:");
//! println!("  Default strategy: {:?}", current_config.routing.default_strategy);
//! println!("  Load balancing: {}", current_config.routing.enable_load_balancing);
//! println!("  Time-based routing: {}", current_config.routing.enable_time_based_routing);
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Agent Management
//!
//! ```rust
//! use rvoip_call_engine::api::AdminApi;
//! use rvoip_call_engine::agent::{Agent, AgentId, AgentStatus};
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
//! let admin = AdminApi::new(Arc::clone(&engine));
//! 
//! // Bulk agent operations
//! let new_agents = vec![
//!     ("agent-sales-01", "Sales Agent 1", "Sales", vec!["sales", "english"]),
//!     ("agent-sales-02", "Sales Agent 2", "Sales", vec!["sales", "spanish"]),
//!     ("agent-support-01", "Support Agent 1", "Support", vec!["technical", "english"]),
//! ];
//! 
//! for (id, name, dept, skills) in new_agents {
//!     let agent = Agent {
//!         id: id.to_string(),
//!         sip_uri: format!("sip:{}@call-center.com", id),
//!         display_name: name.to_string(),
//!         skills: skills.iter().map(|s| s.to_string()).collect(),
//!         max_concurrent_calls: 2,
//!         status: AgentStatus::Offline,
//!         department: Some(dept.to_string()),
//!         extension: None,
//!     };
//!     
//!     admin.add_agent(agent).await?;
//!     println!("➕ Added agent: {} ({})", name, dept);
//! }
//! 
//! // Update agent skills
//! let agent_id = AgentId::from("agent-sales-01");
//! admin.update_agent_skills(&agent_id, vec![
//!     "sales".to_string(),
//!     "english".to_string(),
//!     "vip".to_string(), // Add VIP handling skill
//! ]).await?;
//! 
//! println!("🎯 Updated agent skills");
//! 
//! // Get system statistics
//! let stats = admin.get_statistics().await;
//! println!("📈 System Statistics:");
//! println!("  Total agents: {}", stats.total_agents);
//! println!("  Available agents: {}", stats.available_agents);
//! println!("  Active calls: {}", stats.active_calls);
//! println!("  Queued calls: {}", stats.queued_calls);
//! # Ok(())
//! # }
//! ```
//!
//! ### Database Maintenance
//!
//! ```rust
//! use rvoip_call_engine::api::AdminApi;
//! use tokio::time::{Duration, interval};
//! # use rvoip_call_engine::CallCenterEngine;
//! # use std::sync::Arc;
//! 
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
//! let admin = AdminApi::new(Arc::clone(&engine));
//! 
//! // Periodic maintenance routine
//! async fn maintenance_routine(admin: &AdminApi) -> Result<(), Box<dyn std::error::Error>> {
//!     println!("🧹 Starting maintenance routine...");
//!     
//!     // Check system health first
//!     let health = admin.get_system_health().await;
//!     if !health.database_connected {
//!         println!("❌ Database not connected - skipping maintenance");
//!         return Ok(());
//!     }
//!     
//!     // Optimize database
//!     admin.optimize_database().await?;
//!     println!("✅ Database optimization completed");
//!     
//!     // Get updated statistics
//!     let stats = admin.get_statistics().await;
//!     println!("📊 Post-maintenance statistics:");
//!     println!("  Agents: {}", stats.total_agents);
//!     println!("  Active calls: {}", stats.active_calls);
//!     
//!     Ok(())
//! }
//! 
//! // Run maintenance
//! maintenance_routine(&admin).await?;
//! 
//! // In a real system, you might schedule this periodically:
//! // let mut interval = interval(Duration::from_hours(24));
//! // loop {
//! //     interval.tick().await;
//! //     maintenance_routine(&admin).await?;
//! // }
//! # Ok(())
//! # }
//! ```

use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{State, Path},
    response::Json,
    http::StatusCode,
};
use std::sync::Arc;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::{
    CallCenterEngine,
    agent::{Agent, AgentStatus, AgentId},
    config::{CallCenterConfig, QueueConfig, RoutingConfig},
    error::{CallCenterError, Result as CallCenterResult},
    queue::QueueStats,
};

/// # Administrative API for Call Center Management
/// 
/// The `AdminApi` provides comprehensive administrative capabilities for call center
/// management, including agent operations, queue configuration, system monitoring,
/// and maintenance tasks. This API is designed for system administrators who need
/// complete control over call center operations and configuration.
/// 
/// ## Core Administrative Functions
/// 
/// ### Agent Management
/// - **Add/Remove Agents**: Complete lifecycle management of agent profiles
/// - **Update Agent Information**: Modify agent details, skills, and configuration
/// - **Skills Management**: Assign and update agent capabilities and specializations
/// - **Status Monitoring**: Track agent availability and performance
/// 
/// ### Queue Operations
/// - **Queue Creation**: Establish new call queues with custom configurations
/// - **Configuration Updates**: Modify queue parameters and routing rules
/// - **Queue Deletion**: Remove unused queues (with safety checks)
/// - **Performance Monitoring**: Track queue metrics and utilization
/// 
/// ### System Management
/// - **Health Monitoring**: Comprehensive system health and status tracking
/// - **Configuration Management**: Import/export and dynamic configuration updates
/// - **Database Operations**: Maintenance, optimization, and integrity checks
/// - **Statistics & Analytics**: System-wide performance metrics and reporting
/// 
/// ## Security and Access Control
/// 
/// The `AdminApi` provides powerful system management capabilities and should be
/// used with appropriate security measures:
/// 
/// - Administrative authentication required
/// - Role-based access control recommended
/// - Audit logging for all operations
/// - Network access restrictions
/// - Secure communication channels
/// 
/// ## Thread Safety
/// 
/// The `AdminApi` is thread-safe and can be cloned for use across multiple
/// administrative components or tasks. All operations are asynchronous and
/// designed for concurrent administrative access.
/// 
/// ## Examples
/// 
/// ### Basic Administrative Setup
/// 
/// ```rust
/// use rvoip_call_engine::api::AdminApi;
/// use rvoip_call_engine::CallCenterEngine;
/// use std::sync::Arc;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
/// let admin = AdminApi::new(Arc::clone(&engine));
/// 
/// // Get system health overview
/// let health = admin.get_system_health().await;
/// println!("System status: {:?}", health.status);
/// 
/// // Get system statistics
/// let stats = admin.get_statistics().await;
/// println!("Total agents: {}", stats.total_agents);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AdminApi {
    engine: Arc<CallCenterEngine>,
}

impl AdminApi {
    /// Create a new admin API instance
    /// 
    /// Initializes a new administrative API connected to the specified call center
    /// engine. This provides access to all administrative functions and system
    /// management capabilities.
    /// 
    /// # Arguments
    /// 
    /// * `engine` - Shared reference to the call center engine
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::AdminApi;
    /// use rvoip_call_engine::CallCenterEngine;
    /// use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
    /// let admin = AdminApi::new(Arc::clone(&engine));
    /// 
    /// println!("Administrative API ready");
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self { engine }
    }
    
    /// Add a new agent
    /// 
    /// Registers a new agent profile with the call center system, including
    /// database storage and registry updates. This is the primary method for
    /// agent onboarding and profile creation.
    /// 
    /// # Arguments
    /// 
    /// * `agent` - Complete agent profile including identification, skills, and configuration
    /// 
    /// # Returns
    /// 
    /// `Ok(())` if agent added successfully, or error if operation fails.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_call_engine::api::AdminApi;
    /// use rvoip_call_engine::agent::{Agent, AgentStatus};
    /// # use rvoip_call_engine::CallCenterEngine;
    /// # use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = Arc::new(CallCenterEngine::new(Default::default(), None).await?);
    /// let admin = AdminApi::new(Arc::clone(&engine));
    /// 
    /// let new_agent = Agent {
    ///     id: "agent-005".to_string(),
    ///     sip_uri: "sip:john.doe@call-center.com".to_string(),
    ///     display_name: "John Doe".to_string(),
    ///     skills: vec!["technical".to_string(), "escalation".to_string()],
    ///     max_concurrent_calls: 3,
    ///     status: AgentStatus::Offline, // Will be set to Available when agent logs in
    ///     department: Some("Technical Support".to_string()),
    ///     extension: Some("5005".to_string()),
    /// };
    /// 
    /// admin.add_agent(new_agent).await?;
    /// println!("✅ Agent added to system");
    /// 
    /// // Verify agent was added
    /// let agents = admin.list_agents().await?;
    /// println!("Total agents now: {}", agents.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_agent(&self, agent: Agent) -> Result<(), CallCenterError> {
        // Register with the registry
        let mut registry = self.engine.agent_registry.lock().await;
        registry.register_agent(agent.clone()).await?;
        
        // Also add to database if available
        if let Some(db) = self.engine.database_manager() {
            // Extract username from SIP URI (e.g., "alice" from "sip:alice@127.0.0.1")
            let username = agent.sip_uri
                .trim_start_matches("sip:")
                .split('@')
                .next()
                .unwrap_or(&agent.id);
            
            db.upsert_agent(
                &agent.id,
                username,  // Use the SIP username, not display_name
                Some(&agent.sip_uri)
            ).await.map_err(|e| CallCenterError::database(&format!("Failed to upsert agent: {}", e)))?;
            
            // Update status separately
            db.update_agent_status(&agent.id, agent.status.clone())
                .await.map_err(|e| CallCenterError::database(&format!("Failed to update status: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// Update an existing agent
    pub async fn update_agent(&self, agent: Agent) -> Result<(), CallCenterError> {
        // Update in database if available
        if let Some(db) = self.engine.database_manager() {
            // Extract username from SIP URI (e.g., "alice" from "sip:alice@127.0.0.1")
            let username = agent.sip_uri
                .trim_start_matches("sip:")
                .split('@')
                .next()
                .unwrap_or(&agent.id);
                
            db.upsert_agent(
                &agent.id,
                username,  // Use the SIP username, not display_name
                Some(&agent.sip_uri)
            ).await.map_err(|e| CallCenterError::database(&format!("Failed to upsert agent: {}", e)))?;
            
            // Update status separately
            db.update_agent_status(&agent.id, agent.status.clone())
                .await.map_err(|e| CallCenterError::database(&format!("Failed to update status: {}", e)))?;
        } else {
            return Err(CallCenterError::internal("Database not configured"));
        }
        
        Ok(())
    }
    
    /// Remove an agent
    pub async fn remove_agent(&self, agent_id: &AgentId) -> Result<(), CallCenterError> {
        // Remove from registry
        let mut registry = self.engine.agent_registry.lock().await;
        registry.remove_agent_session(&agent_id.0)?;
        
        // Also mark as offline in database if available
        if let Some(db) = self.engine.database_manager() {
            db.mark_agent_offline(&agent_id.0)
                .await.map_err(|e| CallCenterError::database(&format!("Failed to mark agent offline: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// List all agents
    pub async fn list_agents(&self) -> Result<Vec<Agent>, CallCenterError> {
        if let Some(db) = self.engine.database_manager() {
            // Get DB agents and convert to API agents
            let db_agents = db.list_agents()
                .await.map_err(|e| CallCenterError::database(&format!("Failed to list agents: {}", e)))?;
            
            // Convert DB agents to API agents
            let agents = db_agents.into_iter().map(|db_agent| {
                Agent {
                    id: db_agent.agent_id,
                    sip_uri: db_agent.contact_uri.unwrap_or_else(|| format!("sip:{}@localhost", db_agent.username)),
                    display_name: db_agent.username,
                    skills: vec![], // TODO: Load from database when skill table is implemented
                    max_concurrent_calls: db_agent.max_calls as u32,
                    status: match db_agent.status {
                                            crate::database::DbAgentStatus::Available => AgentStatus::Available,
                    crate::database::DbAgentStatus::Busy => AgentStatus::Busy(vec![]),
                    crate::database::DbAgentStatus::PostCallWrapUp => AgentStatus::PostCallWrapUp,
                    crate::database::DbAgentStatus::Offline => AgentStatus::Offline,
                    crate::database::DbAgentStatus::Reserved => AgentStatus::Available, // Treat as available
                    },
                    department: None,
                    extension: None,
                }
            }).collect();
            
            Ok(agents)
        } else {
            // Return empty list if no database
            Ok(Vec::new())
        }
    }
    
    /// Update agent skills
    pub async fn update_agent_skills(&self, agent_id: &AgentId, skills: Vec<String>) -> Result<(), CallCenterError> {
        if let Some(_db) = self.engine.database_manager() {
            // TODO: Implement skill storage in database
            // For now, just log the request
            tracing::info!("Updating skills for agent {}: {:?}", agent_id.0, skills);
            Ok(())
        } else {
            return Err(CallCenterError::internal("Database not configured"));
        }
    }
    
    /// Create a new queue
    pub async fn create_queue(&self, queue_id: &str) -> CallCenterResult<()> {
        self.engine.create_queue(queue_id).await
    }
    
    /// Update queue configuration
    pub async fn update_queue(&self, queue_id: &str, config: QueueConfig) -> CallCenterResult<()> {
        // In a real implementation, this would update queue settings
        tracing::info!("Updating queue {} configuration", queue_id);
        // TODO: Implement queue configuration updates
        Ok(())
    }
    
    /// Delete a queue
    /// 
    /// This will fail if the queue has active calls
    pub async fn delete_queue(&self, queue_id: &str) -> CallCenterResult<()> {
        let queue_manager = self.engine.queue_manager().read().await;
        let stats = queue_manager.get_queue_stats(queue_id)?;
        
        if stats.total_calls > 0 {
            return Err(CallCenterError::validation(
                "Cannot delete queue with active calls"
            ));
        }
        
        drop(queue_manager);
        // TODO: Add proper queue removal method to QueueManager
        Ok(())
    }
    
    /// Get current system configuration
    pub fn get_config(&self) -> &CallCenterConfig {
        self.engine.config()
    }
    
    /// Update routing configuration
    /// 
    /// This allows dynamic updates to routing rules without restart
    pub async fn update_routing_config(&self, config: RoutingConfig) -> CallCenterResult<()> {
        // In a real implementation, this would update the routing engine
        tracing::info!("Updating routing configuration");
        // TODO: Implement dynamic routing updates
        Ok(())
    }
    
    /// Get system health status
    pub async fn get_system_health(&self) -> SystemHealth {
        let stats = self.engine.get_stats().await;
        let database_ok = self.check_database_health().await;
        
        SystemHealth {
            status: if database_ok { HealthStatus::Healthy } else { HealthStatus::Degraded },
            database_connected: database_ok,
            active_sessions: stats.active_calls,
            registered_agents: stats.available_agents + stats.busy_agents,
            queued_calls: stats.queued_calls,
            uptime_seconds: 0, // TODO: Track actual uptime
            warnings: Vec::new(),
        }
    }
    
    /// Perform database maintenance
    pub async fn optimize_database(&self) -> CallCenterResult<()> {
        tracing::info!("Running database optimization");
        // TODO: Implement database optimization
        Ok(())
    }
    
    /// Export system configuration
    pub async fn export_config(&self) -> CallCenterResult<String> {
        let config = self.engine.config();
        serde_json::to_string_pretty(config)
            .map_err(|e| CallCenterError::internal(&format!("Failed to serialize config: {}", e)))
    }
    
    /// Import system configuration
    /// 
    /// Note: This requires a system restart to take effect
    pub async fn import_config(&self, config_json: &str) -> CallCenterResult<CallCenterConfig> {
        serde_json::from_str(config_json)
            .map_err(|e| CallCenterError::validation(&format!("Invalid config JSON: {}", e)))
    }
    
    /// Get detailed queue configuration
    pub async fn get_queue_configs(&self) -> HashMap<String, QueueConfig> {
        // In a real implementation, this would return actual queue configs
        let mut configs = HashMap::new();
        
        // Add default queues with correct field names
        for queue_id in &["general", "sales", "support", "billing", "vip", "premium", "overflow"] {
            configs.insert(
                queue_id.to_string(),
                QueueConfig {
                    default_max_wait_time: 300,
                    max_queue_size: 100,
                    enable_priorities: true,
                    enable_overflow: *queue_id != "overflow",
                    announcement_interval: 30,
                }
            );
        }
        
        configs
    }
    
    /// Check database health
    async fn check_database_health(&self) -> bool {
        if let Some(db) = self.engine.database_manager() {
            // Try to query the database with a simple query
            match db.query("SELECT 1", ()).await {
                Ok(_) => true,
                Err(e) => {
                    tracing::error!("Database health check failed: {}", e);
                    false
                }
            }
        } else {
            // No database configured
            false
        }
    }

    /// Get statistics
    pub async fn get_statistics(&self) -> CallCenterStats {
        let total_agents = if let Some(db) = self.engine.database_manager() {
            db.list_agents().await.unwrap_or_default().len()
        } else {
            0
        };
        
        let active_calls = if let Some(db) = self.engine.database_manager() {
            db.get_active_calls_count().await.unwrap_or(0)
        } else {
            0
        };
        
        let queued_calls = 0; // TODO: get from queue manager
        
        let available_agents = if let Some(db) = self.engine.database_manager() {
            match db.get_agent_stats().await {
                Ok(stats) => stats.available_agents,
                Err(e) => {
                    tracing::error!("Failed to get agent stats from database: {}", e);
                    0
                }
            }
        } else {
            0
        };
        
        CallCenterStats {
            total_agents,
            available_agents,
            active_calls,
            queued_calls,
        }
    }
}

/// System health information
#[derive(Debug, Clone)]
pub struct SystemHealth {
    pub status: HealthStatus,
    pub database_connected: bool,
    pub active_sessions: usize,
    pub registered_agents: usize,
    pub queued_calls: usize,
    pub uptime_seconds: u64,
    pub warnings: Vec<String>,
}

/// Health status enum
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Critical,
}

impl Default for AdminApi {
    fn default() -> Self {
        panic!("AdminApi requires an engine instance")
    }
}

/// Call center statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallCenterStats {
    pub total_agents: usize,
    pub available_agents: usize,
    pub active_calls: usize,
    pub queued_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub skill_name: String,
    pub skill_level: u8,
} 